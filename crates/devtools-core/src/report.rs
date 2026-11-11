//! Doctor and reports.
//!
//! The JSON report has a versioned schema (`schema_version`) so agents can read it without
//! interpreting prose. Test and build results are only ever what was actually recorded: with no
//! run on record the status is `not_run`, never a zero-count pass.

use crate::binary::Manifest;
use crate::build::{last_build, BuildRecord};
use crate::config::DevEnvironment;
use crate::database::DatabaseManager;
use crate::env::{self, EnvReport};
use crate::process;
use crate::tests::{Status, TestRun, TestRunner};
use crate::util::{command_output, now_rfc3339};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;

pub const REPORT_SCHEMA: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Ok,
    Info,
    Warn,
    Fail,
}

#[derive(Debug, Clone, Serialize)]
pub struct Check {
    pub name: String,
    pub level: Level,
    pub detail: String,
}

fn check(name: &str, level: Level, detail: impl Into<String>) -> Check {
    Check {
        name: name.into(),
        level,
        detail: detail.into(),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DoctorReport {
    pub schema_version: u32,
    pub ok: bool,
    pub checks: Vec<Check>,
    pub environment: EnvReport,
}

pub async fn doctor(env: &DevEnvironment) -> DoctorReport {
    let db = DatabaseManager::new(env.clone());
    let bin = db.binaries();
    let mut checks = Vec::new();

    // Rust and Cargo.
    let environment = env::inspect(&env.root);
    for tool in environment
        .tools
        .iter()
        .filter(|t| t.name == "cargo" || t.name == "rustc")
    {
        checks.push(check(
            &tool.name,
            if tool.found { Level::Ok } else { Level::Fail },
            tool.version
                .clone()
                .unwrap_or_else(|| "not found on PATH".into()),
        ));
    }

    // Configuration and credentials.
    let creds_missing = env.require_credentials().is_err();
    checks.push(check(
        "config",
        if creds_missing { Level::Warn } else { Level::Ok },
        if creds_missing {
            "no database credentials (DATABASE_URL, .env or REMAKE_DEV_DB_PASSWORD); commands that use the database will refuse to run".to_string()
        } else {
            format!(
                "connection from {}; devtools.toml {}",
                env.conn_source,
                if env.root.join("devtools.toml").is_file() { "present" } else { "absent (defaults)" }
            )
        },
    ));

    // pgrust install.
    match bin.verify(true) {
        Ok(m) => checks.push(check(
            "pgrust-binary",
            Level::Ok,
            format!(
                "{} (sha256 {}…, {})",
                m.version,
                &m.binary_sha256[..12.min(m.binary_sha256.len())],
                m.source_type
            ),
        )),
        Err(e) => checks.push(check("pgrust-binary", Level::Fail, format!("{e:#}"))),
    }
    if bin.is_installed() {
        let support = bin.initdb_path().is_file() && bin.share_dir().is_dir();
        checks.push(check(
            "pgrust-support-files",
            if support { Level::Ok } else { Level::Fail },
            if support {
                "initdb and share/ are installed beside the server"
            } else {
                "initdb or share/ is missing"
            },
        ));
    }

    // Data directory and permissions.
    let data = env.data_dir();
    if db.data_initialized() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&data)
                .map(|m| m.permissions().mode() & 0o777)
                .unwrap_or(0);
            checks.push(check(
                "data-dir",
                if mode & 0o077 == 0 {
                    Level::Ok
                } else {
                    Level::Warn
                },
                format!(
                    "{} (mode {:o}{})",
                    data.display(),
                    mode,
                    if mode & 0o077 == 0 {
                        ""
                    } else {
                        "; the server wants 0700"
                    }
                ),
            ));
        }
        #[cfg(not(unix))]
        checks.push(check("data-dir", Level::Ok, data.display().to_string()));
    } else {
        checks.push(check(
            "data-dir",
            Level::Info,
            format!("{} will be created by `db up`", data.display()),
        ));
    }
    let writable = env.ensure_dirs().is_ok();
    checks.push(check(
        "devtools-dir",
        if writable { Level::Ok } else { Level::Fail },
        if writable {
            format!("{} is writable", env.devtools_dir().display())
        } else {
            format!("cannot create {}", env.devtools_dir().display())
        },
    ));

    // Instance, port and SQL.
    let status = db.status().await.ok();
    let running = status.as_ref().is_some_and(|s| s.running);
    if running {
        let s = status.as_ref().unwrap();
        let healthy = s.healthy == Some(true);
        checks.push(check(
            "instance",
            if healthy { Level::Ok } else { Level::Fail },
            format!(
                "pid {}, port {}, {}",
                s.pid.unwrap_or(0),
                s.port,
                s.server_version.clone().unwrap_or_else(|| format!(
                    "no SQL answer as user `{}` with the connection from {}; the managed instance only \
                     accepts the credentials it was initialised with (set DATABASE_URL or \
                     REMAKE_DEV_DB_PASSWORD to match them)",
                    env.conn.user, env.conn_source
                ))
            ),
        ));
    } else if process::port_in_use(&env.conn.host, env.conn.port) {
        checks.push(check(
            "port",
            Level::Warn,
            format!("port {} is held by a process remake-dev did not start; `db up` will refuse to use it", env.conn.port),
        ));
    } else {
        checks.push(check(
            "instance",
            Level::Info,
            format!("not running; port {} is free", env.conn.port),
        ));
    }

    // Migrations: files on disk versus what the dev database has applied.
    let on_disk = fs::read_dir(env.root.join("migrations"))
        .map(|r| {
            r.flatten()
                .filter(|e| e.path().extension().is_some_and(|x| x == "sql"))
                .count()
        })
        .unwrap_or(0);
    if running {
        match applied_migrations(env, db.running_port()).await {
            Ok(Some(applied)) => checks.push(check(
                "migrations",
                if applied as usize >= on_disk {
                    Level::Ok
                } else {
                    Level::Warn
                },
                format!(
                    "{applied} applied, {on_disk} on disk{}",
                    if (applied as usize) < on_disk {
                        " — run `cloud-api migrate`"
                    } else {
                        ""
                    }
                ),
            )),
            Ok(None) => checks.push(check(
                "migrations",
                Level::Info,
                format!("none applied yet, {on_disk} on disk"),
            )),
            Err(e) => checks.push(check(
                "migrations",
                Level::Info,
                format!("could not read the migration table: {e:#}"),
            )),
        }
    } else {
        checks.push(check(
            "migrations",
            Level::Info,
            format!("{on_disk} on disk; database not running, applied count unknown"),
        ));
    }

    // Anything that changes Cargo's fingerprints (reported, never changed).
    for i in &environment.influences {
        checks.push(check(
            "build-cache",
            Level::Warn,
            format!("{} `{}` — {}", i.source, i.name, i.note),
        ));
    }
    if environment.influences.is_empty() {
        checks.push(check(
            "build-cache",
            Level::Ok,
            "no wrapper, rustflags or target-dir override found",
        ));
    }
    for t in environment
        .tools
        .iter()
        .filter(|t| ["mold", "kache", "sccache", "cargo-nextest"].contains(&t.name.as_str()))
    {
        checks.push(check(
            &format!("optional:{}", t.name),
            Level::Info,
            if t.found {
                format!(
                    "available ({}); remake-dev does not enable it for you",
                    t.version.clone().unwrap_or_default()
                )
            } else {
                "not installed (optional)".into()
            },
        ));
    }

    let ok = !checks.iter().any(|c| c.level == Level::Fail);
    DoctorReport {
        schema_version: REPORT_SCHEMA,
        ok,
        checks,
        environment,
    }
}

/// Count of applied sqlx migrations in the development database, `None` if the table is absent.
async fn applied_migrations(env: &DevEnvironment, port: u16) -> Result<Option<i64>> {
    use sqlx::Connection as _;
    // The managed server may run on a port other than the configured one (instance.json).
    let conn = crate::config::Connection { port, ..env.conn.clone() };
    let mut c = crate::health::connect(&conn, &conn.database).await?;
    let exists: Option<String> = sqlx::query_scalar("SELECT to_regclass('_sqlx_migrations')::text")
        .fetch_one(&mut c)
        .await?;
    let count = if exists.is_some() {
        Some(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM _sqlx_migrations")
                .fetch_one(&mut c)
                .await?,
        )
    } else {
        None
    };
    let _ = c.close().await;
    Ok(count)
}

#[derive(Debug, Clone, Serialize)]
pub struct Platform {
    pub os: String,
    pub arch: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DatabaseReport {
    pub engine: String,
    pub installed: bool,
    pub running: bool,
    pub healthy: bool,
    pub port: u16,
    pub server_version: Option<String>,
    pub installed_version: Option<String>,
    pub manifest: Option<Manifest>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TestsReport {
    pub status: Status,
    pub passed: u64,
    pub failed: u64,
    pub skipped: u64,
    pub runs: Vec<TestRun>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BuildReport {
    pub status: Status,
    pub record: Option<BuildRecord>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Diagnostic {
    pub severity: Level,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub schema_version: u32,
    pub project: String,
    pub generated_at: String,
    pub commit: Option<String>,
    pub platform: Platform,
    pub rust: serde_json::Value,
    pub database: DatabaseReport,
    pub tests: TestsReport,
    pub build: BuildReport,
    pub diagnostics: Vec<Diagnostic>,
}

pub fn overall_test_status(runs: &[TestRun]) -> Status {
    if runs.iter().any(|r| r.status == Status::Failed) {
        Status::Failed
    } else if runs.iter().any(|r| r.status == Status::Passed) {
        Status::Passed
    } else if runs.iter().any(|r| r.status == Status::Skipped) {
        Status::Skipped
    } else if runs.iter().any(|r| r.status == Status::Unsupported)
        && runs.iter().all(|r| r.status == Status::Unsupported)
    {
        Status::Unsupported
    } else {
        Status::NotRun
    }
}

pub struct ReportGenerator {
    env: DevEnvironment,
}

impl ReportGenerator {
    pub fn new(env: DevEnvironment) -> Self {
        Self { env }
    }

    pub async fn generate(&self) -> Result<Report> {
        let db = DatabaseManager::new(self.env.clone());
        let status = db.status().await?;
        let manifest = db.binaries().load_manifest().ok();
        let runs = TestRunner::new(self.env.clone()).last_runs();
        let build = last_build(&self.env);
        let doctor = doctor(&self.env).await;
        let diagnostics = doctor
            .checks
            .iter()
            .filter(|c| matches!(c.level, Level::Warn | Level::Fail))
            .map(|c| Diagnostic {
                severity: c.level,
                code: c.name.clone(),
                message: c.detail.clone(),
            })
            .collect();
        Ok(Report {
            schema_version: REPORT_SCHEMA,
            project: "RemakeBackend".into(),
            generated_at: now_rfc3339(),
            commit: command_output(
                "git",
                &[
                    "-C",
                    self.env.root.to_str().unwrap_or("."),
                    "rev-parse",
                    "HEAD",
                ],
            ),
            platform: Platform {
                os: std::env::consts::OS.into(),
                arch: std::env::consts::ARCH.into(),
            },
            rust: serde_json::json!({
                "rustc": command_output("rustc", &["--version"]),
                "cargo": command_output("cargo", &["--version"]),
            }),
            database: DatabaseReport {
                engine: "pgrust".into(),
                installed: status.installed,
                running: status.running,
                healthy: status.healthy == Some(true),
                port: status.port,
                server_version: status.server_version,
                installed_version: status.installed_version,
                manifest,
            },
            tests: TestsReport {
                status: overall_test_status(&runs),
                passed: runs.iter().map(|r| r.passed).sum(),
                failed: runs.iter().map(|r| r.failed).sum(),
                skipped: runs.iter().map(|r| r.skipped).sum(),
                runs,
            },
            build: BuildReport {
                status: build.as_ref().map(|b| b.status).unwrap_or(Status::NotRun),
                record: build,
            },
            diagnostics,
        })
    }

    /// Write the report to the reports directory (`latest.json` / `latest.md`) and return the path.
    pub fn save(&self, report: &Report, format: &str) -> Result<std::path::PathBuf> {
        self.env.ensure_dirs()?;
        let (name, body) = match format {
            "markdown" | "md" => ("latest.md", to_markdown(report)),
            _ => ("latest.json", serde_json::to_string_pretty(report)? + "\n"),
        };
        let path = self.env.reports_dir().join(name);
        fs::write(&path, body)?;
        Ok(path)
    }
}

pub fn to_markdown(r: &Report) -> String {
    let mut out = format!(
        "# {} report\n\n- generated: {}\n- commit: {}\n- platform: {}/{}\n\n",
        r.project,
        r.generated_at,
        r.commit.as_deref().unwrap_or("unknown"),
        r.platform.os,
        r.platform.arch
    );
    out.push_str(&format!(
        "## Database\n\n- engine: {}\n- installed: {} ({})\n- running: {}, healthy: {}, port {}\n- server: {}\n\n",
        r.database.engine,
        r.database.installed,
        r.database.installed_version.as_deref().unwrap_or("-"),
        r.database.running,
        r.database.healthy,
        r.database.port,
        r.database.server_version.as_deref().unwrap_or("-"),
    ));
    out.push_str(&format!(
        "## Tests\n\n- status: {:?}\n- passed {} / failed {} / skipped {}\n\n",
        r.tests.status, r.tests.passed, r.tests.failed, r.tests.skipped
    ));
    for run in &r.tests.runs {
        out.push_str(&format!(
            "- {}: {:?} ({} passed, {} failed, {} skipped, {} ms){}\n",
            run.kind,
            run.status,
            run.passed,
            run.failed,
            run.skipped,
            run.duration_ms,
            run.note
                .as_deref()
                .map(|n| format!(" — {n}"))
                .unwrap_or_default()
        ));
        for f in &run.failures {
            out.push_str(&format!(
                "  - FAILED {}: {}\n",
                f.name,
                f.message.as_deref().unwrap_or("")
            ));
        }
    }
    out.push_str(&format!("\n## Build\n\n- status: {:?}\n\n", r.build.status));
    out.push_str("## Diagnostics\n\n");
    if r.diagnostics.is_empty() {
        out.push_str("none\n");
    }
    for d in &r.diagnostics {
        out.push_str(&format!("- [{:?}] {}: {}\n", d.severity, d.code, d.message));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(kind: &str, status: Status) -> TestRun {
        let mut r = TestRun::not_run(kind, "x");
        r.status = status;
        r
    }

    #[test]
    fn overall_status_never_calls_an_empty_history_a_pass() {
        assert_eq!(overall_test_status(&[]), Status::NotRun);
        assert_eq!(
            overall_test_status(&[run("unit", Status::NotRun)]),
            Status::NotRun
        );
        assert_eq!(
            overall_test_status(&[run("unit", Status::Passed), run("e2e", Status::Unsupported)]),
            Status::Passed
        );
        assert_eq!(
            overall_test_status(&[
                run("unit", Status::Passed),
                run("integration", Status::Failed)
            ]),
            Status::Failed
        );
        assert_eq!(
            overall_test_status(&[run("e2e", Status::Unsupported)]),
            Status::Unsupported
        );
        assert_eq!(
            overall_test_status(&[run("unit", Status::Skipped)]),
            Status::Skipped
        );
    }
}
