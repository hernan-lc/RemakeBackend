//! TestRunner: unit tests without a database, integration tests against an isolated one.
//!
//! * unit: `cargo test --workspace`. The database tests are `#[ignore]`d in the source, so this
//!   never starts pgrust.
//! * integration: reuses the installed pgrust, recreates the isolated test database (never the
//!   development one) and runs exactly the tests marked
//!   `#[ignore = "... CLOUD_TEST_DATABASE_URL"]` against it. Every test gets a private database
//!   cloned from a migrated template (see `cloud-api`'s `test_db`), so with `tests.parallel` they
//!   run on several threads.
//!
//! Results are structured. A run that executed no test is `skipped`, never `passed`.

use crate::config::DevEnvironment;
use crate::database::DatabaseManager;
use crate::process;
use crate::util::now_rfc3339;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

pub const DB_TEST_MARKER: &str = "CLOUD_TEST_DATABASE_URL";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Passed,
    Failed,
    Skipped,
    NotRun,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestKind {
    Unit,
    Integration,
}

impl TestKind {
    pub fn name(self) -> &'static str {
        match self {
            TestKind::Unit => "unit",
            TestKind::Integration => "integration",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestFailure {
    pub name: String,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestRun {
    pub kind: String,
    pub status: Status,
    pub passed: u64,
    pub failed: u64,
    pub skipped: u64,
    pub duration_ms: u128,
    pub command: Vec<String>,
    pub exit_code: Option<i32>,
    pub failures: Vec<TestFailure>,
    pub started_at: String,
    /// Why a run is `not_run` / `unsupported`.
    pub note: Option<String>,
}

impl TestRun {
    pub fn not_run(kind: &str, note: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            status: Status::NotRun,
            passed: 0,
            failed: 0,
            skipped: 0,
            duration_ms: 0,
            command: vec![],
            exit_code: None,
            failures: vec![],
            started_at: now_rfc3339(),
            note: Some(note.into()),
        }
    }
    pub fn unsupported(kind: &str, note: impl Into<String>) -> Self {
        Self {
            status: Status::Unsupported,
            ..Self::not_run(kind, note)
        }
    }
    /// True for anything that must make the process exit non-zero.
    pub fn is_failure(&self) -> bool {
        matches!(self.status, Status::Failed)
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub passed: u64,
    pub failed: u64,
    pub ignored: u64,
    pub suites: u64,
    pub failures: Vec<TestFailure>,
}

/// Fold the output of `cargo test` (any number of test binaries) into one summary.
pub fn parse_cargo_test_output(text: &str) -> Summary {
    let mut summary = Summary::default();
    let lines: Vec<&str> = text.lines().collect();
    for line in &lines {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("test result: ") {
            summary.suites += 1;
            for part in rest.split(';') {
                let part = part.trim();
                let number = |suffix: &str| -> Option<u64> {
                    part.strip_suffix(suffix)
                        .and_then(|p| p.split_whitespace().last())
                        .and_then(|n| n.parse().ok())
                };
                if let Some(n) = number(" passed") {
                    summary.passed += n;
                } else if let Some(n) = number(" failed") {
                    summary.failed += n;
                } else if let Some(n) = number(" ignored") {
                    summary.ignored += n;
                }
            }
        } else if let Some(rest) = line.strip_prefix("test ") {
            if let Some(name) = rest.strip_suffix(" ... FAILED") {
                let name = name.trim().to_string();
                if !summary.failures.iter().any(|f| f.name == name) {
                    summary.failures.push(TestFailure {
                        name,
                        message: None,
                    });
                }
            }
        }
    }
    // "---- name stdout ----" blocks carry the panic message.
    for (i, line) in lines.iter().enumerate() {
        if let Some(name) = line
            .strip_prefix("---- ")
            .and_then(|r| r.strip_suffix(" stdout ----"))
        {
            let message = lines[i + 1..]
                .iter()
                .take(6)
                .map(|l| l.trim())
                .find(|l| !l.is_empty() && !l.starts_with("thread '"))
                .or_else(|| lines.get(i + 1).map(|l| l.trim()))
                .map(|l| l.to_string());
            if let Some(f) = summary.failures.iter_mut().find(|f| f.name == name) {
                f.message = message;
            }
        }
    }
    summary
}

/// A test marked as needing the isolated database, plus any other environment variable its
/// `#[ignore = "..."]` reason says it needs (for example a separately built SDK binary).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct DbTest {
    pub package: String,
    pub name: String,
    pub extra_env: Vec<String>,
}

/// Variable-looking words (`CLOUD_V2_APP_SDK_TEST_BIN`) in an ignore reason, other than the
/// database one that `test --integration` provides itself.
fn extra_env_vars(reason: &str) -> Vec<String> {
    let mut vars: Vec<String> = reason
        .split(|c: char| !(c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'))
        .filter(|w| {
            w.len() > 5
                && w.contains('_')
                && w.chars().next().is_some_and(|c| c.is_ascii_uppercase())
        })
        .filter(|w| *w != DB_TEST_MARKER)
        .map(str::to_string)
        .collect();
    vars.sort();
    vars.dedup();
    vars
}

/// Every test marked as needing the test database.
pub fn discover_database_tests(root: &Path) -> Result<Vec<DbTest>> {
    let mut found = Vec::new();
    let crates = root.join("crates");
    for entry in
        fs::read_dir(&crates).with_context(|| format!("cannot read {}", crates.display()))?
    {
        let entry = entry?;
        let package = entry.file_name().to_string_lossy().into_owned();
        let mut stack = vec![entry.path().join("src"), entry.path().join("tests")];
        while let Some(dir) = stack.pop() {
            let Ok(read) = fs::read_dir(&dir) else {
                continue;
            };
            for item in read {
                let path = item?.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    collect_marked_tests(&path, &package, &mut found)?;
                }
            }
        }
    }
    found.sort();
    found.dedup();
    Ok(found)
}

fn collect_marked_tests(path: &Path, package: &str, out: &mut Vec<DbTest>) -> Result<()> {
    let text = fs::read_to_string(path)?;
    let lines: Vec<&str> = text.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("#[ignore") && trimmed.contains(DB_TEST_MARKER) {
            for next in lines.iter().skip(i + 1).take(6) {
                let t = next.trim();
                let t = t.strip_prefix("pub ").unwrap_or(t);
                let t = t.strip_prefix("async ").unwrap_or(t);
                if let Some(rest) = t.strip_prefix("fn ") {
                    let name: String = rest
                        .chars()
                        .take_while(|c| c.is_alphanumeric() || *c == '_')
                        .collect();
                    if !name.is_empty() {
                        out.push(DbTest {
                            package: package.to_string(),
                            name,
                            extra_env: extra_env_vars(trimmed),
                        });
                    }
                    break;
                }
            }
        }
    }
    Ok(())
}

/// Run a command, forwarding its output live while also capturing it. In `--json` mode the
/// forwarded output goes to stderr so stdout carries only the JSON document.
fn run_streaming(
    mut cmd: std::process::Command,
    to_stderr: bool,
    deadline: Option<Duration>,
) -> Result<(Option<i32>, String)> {
    debug_assert!(!process::touches_forbidden_env(&cmd));
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().context("failed to run cargo")?;
    let captured = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let mut handles = Vec::new();
    let streams: [(Box<dyn std::io::Read + Send>, bool); 2] = [
        (Box::new(child.stdout.take().unwrap()), false),
        (Box::new(child.stderr.take().unwrap()), true),
    ];
    for (stream, is_err) in streams {
        let sink = captured.clone();
        handles.push(std::thread::spawn(move || {
            for line in BufReader::new(stream).lines().map_while(|l| l.ok()) {
                if to_stderr || is_err {
                    eprintln!("{line}");
                } else {
                    println!("{line}");
                }
                if let Ok(mut buf) = sink.lock() {
                    buf.push_str(&line);
                    buf.push('\n');
                }
            }
        }));
    }
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if deadline.is_some_and(|d| started.elapsed() > d) {
            let _ = child.kill();
            let _ = child.wait();
            for h in handles {
                let _ = h.join();
            }
            let text = captured.lock().map(|b| b.clone()).unwrap_or_default();
            return Ok((
                None,
                format!(
                    "{text}\nremake-dev: timed out after {:?}\n",
                    deadline.unwrap()
                ),
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    for h in handles {
        let _ = h.join();
    }
    let _ = std::io::stdout().flush();
    let text = captured.lock().map(|b| b.clone()).unwrap_or_default();
    Ok((status.code(), text))
}

/// Test threads for the database tests: one when `tests.parallel` is off, otherwise twice the
/// CPU count (the tests wait on the database as much as they compute), between 2 and 8.
pub fn test_threads(parallel: bool) -> usize {
    if !parallel {
        return 1;
    }
    let cpus = std::thread::available_parallelism().map_or(2, |n| n.get());
    (cpus * 2).clamp(2, 8)
}

pub struct TestRunner {
    env: DevEnvironment,
    /// Send cargo's live output to stderr (for `--json`).
    pub quiet_stdout: bool,
}

impl TestRunner {
    pub fn new(env: DevEnvironment) -> Self {
        Self {
            env,
            quiet_stdout: false,
        }
    }

    fn state_file(&self) -> PathBuf {
        self.env.state_dir().join("last_tests.json")
    }

    /// Latest recorded run of each kind.
    pub fn last_runs(&self) -> Vec<TestRun> {
        fs::read_to_string(self.state_file())
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    fn record(&self, run: &TestRun) {
        let mut runs = self.last_runs();
        runs.retain(|r| r.kind != run.kind);
        runs.push(run.clone());
        if self.env.ensure_dirs().is_ok() {
            if let Ok(text) = serde_json::to_string_pretty(&runs) {
                let _ = fs::write(self.state_file(), text + "\n");
            }
        }
    }

    fn finish(
        &self,
        kind: &str,
        command: Vec<String>,
        started: Instant,
        code: Option<i32>,
        text: &str,
    ) -> TestRun {
        let summary = parse_cargo_test_output(text);
        let status = if code != Some(0) {
            Status::Failed
        } else if summary.passed == 0 && summary.failed == 0 {
            Status::Skipped // nothing ran: never reported as a pass
        } else {
            Status::Passed
        };
        let run = TestRun {
            kind: kind.into(),
            status,
            passed: summary.passed,
            failed: summary.failed,
            skipped: summary.ignored,
            duration_ms: started.elapsed().as_millis(),
            command,
            exit_code: code,
            failures: summary.failures,
            started_at: now_rfc3339(),
            note: None,
        };
        self.record(&run);
        run
    }

    pub fn run_unit(&self) -> Result<TestRun> {
        let args = ["test", "--workspace"];
        let mut cmd = process::cargo_command(&args);
        cmd.current_dir(&self.env.root);
        let started = Instant::now();
        let (code, text) = run_streaming(cmd, self.quiet_stdout, None)?;
        let command = std::iter::once("cargo".to_string())
            .chain(args.iter().map(|a| a.to_string()))
            .collect();
        Ok(self.finish("unit", command, started, code, &text))
    }

    pub async fn run_integration(&self) -> Result<TestRun> {
        if !self.env.config.tests.isolated_database {
            return Ok(TestRun::unsupported(
                "integration",
                "integration tests only run against an isolated database; tests.isolated_database = false is not supported",
            ));
        }
        let all = discover_database_tests(&self.env.root)?;
        let (tests, unmet): (Vec<DbTest>, Vec<DbTest>) = all.into_iter().partition(|t| {
            t.extra_env
                .iter()
                .all(|v| std::env::var_os(v).is_some_and(|x| !x.is_empty()))
        });
        if tests.is_empty() {
            return Ok(TestRun::not_run(
                "integration",
                format!("no test is marked #[ignore = \"... {DB_TEST_MARKER}\"]"),
            ));
        }
        // Reuse the installed pgrust (this never builds it) and recreate the isolated database.
        let db = DatabaseManager::new(self.env.clone());
        db.up()
            .await
            .context("integration tests need the managed pgrust")?;
        let name = db.reset_test_database(true).await?;
        let url = db.test_database_url();
        eprintln!(
            "remake-dev: running {} database tests against `{name}`",
            tests.len()
        );

        let mut packages: Vec<String> = tests.iter().map(|t| t.package.clone()).collect();
        packages.sort();
        packages.dedup();
        let threads = test_threads(self.env.config.tests.parallel);
        let started = Instant::now();
        let mut combined = String::new();
        let mut code = Some(0);
        let mut all_cmd = Vec::new();
        for package in packages {
            let names: Vec<&str> = tests
                .iter()
                .filter(|t| t.package == package)
                .map(|t| t.name.as_str())
                .collect();
            let mut args: Vec<String> = ["test", "-p", &package, "--", "--ignored"]
                .iter()
                .map(|s| s.to_string())
                .collect();
            args.push(format!("--test-threads={threads}"));
            args.extend(names.iter().map(|n| n.to_string()));
            let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
            let mut cmd = process::cargo_command(&borrowed);
            cmd.current_dir(&self.env.root).env(DB_TEST_MARKER, &url);
            all_cmd = std::iter::once("cargo".to_string())
                .chain(args.clone())
                .collect();
            let cap = Duration::from_secs(
                (self.env.config.tests.timeout_seconds * names.len() as u64).max(900),
            );
            let (c, text) = run_streaming(cmd, self.quiet_stdout, Some(cap))?;
            combined.push_str(&text);
            if c != Some(0) {
                code = c.or(Some(-1));
            }
        }
        let mut run = self.finish("integration", all_cmd, started, code, &combined);
        if !unmet.is_empty() {
            // Not hidden: counted as skipped and named, with the variable each one is missing.
            run.skipped += unmet.len() as u64;
            run.note = Some(format!(
                "skipped, unmet prerequisite: {}",
                unmet
                    .iter()
                    .map(|t| format!("{} (needs {})", t.name, t.extra_env.join(", ")))
                    .collect::<Vec<_>>()
                    .join("; ")
            ));
            self.record(&run);
        }
        Ok(run)
    }

    /// Every configured category. API/E2E needs the web stack and is reported as unsupported
    /// rather than silently skipped.
    pub async fn run_all(&self) -> Result<Vec<TestRun>> {
        let mut runs = vec![self.run_unit()?];
        match self.run_integration().await {
            Ok(run) => runs.push(run),
            Err(e) => runs.push(TestRun::not_run("integration", format!("{e:#}"))),
        }
        let e2e = TestRun::unsupported("e2e", "no API/E2E suite is wired into remake-dev yet");
        self.record(&e2e);
        runs.push(e2e);
        Ok(runs)
    }
}

pub fn exit_code_for(runs: &[TestRun]) -> i32 {
    // A failure, or an integration run that could not happen, is not a green result.
    if runs
        .iter()
        .any(|r| r.is_failure() || (r.status == Status::NotRun && r.kind != "e2e"))
    {
        1
    } else {
        0
    }
}

#[cfg(test)]
mod unit {
    use super::*;

    const SAMPLE: &str = "\
running 3 tests
test a::ok_one ... ok
test a::bad ... FAILED
test a::skipped ... ignored

failures:

---- a::bad stdout ----
thread 'a::bad' panicked at src/a.rs:9:5:
assertion `left == right` failed

failures:
    a::bad

test result: FAILED. 1 passed; 1 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.01s

running 2 tests
test b::one ... ok
test b::two ... ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
";

    #[test]
    fn database_tests_run_serially_unless_parallel_is_enabled() {
        assert_eq!(test_threads(false), 1);
        let threads = test_threads(true);
        assert!((2..=8).contains(&threads), "{threads}");
    }

    #[test]
    fn output_is_folded_across_suites() {
        let s = parse_cargo_test_output(SAMPLE);
        assert_eq!((s.passed, s.failed, s.ignored, s.suites), (3, 1, 1, 2));
        assert_eq!(s.failures.len(), 1);
        assert_eq!(s.failures[0].name, "a::bad");
        assert!(s.failures[0]
            .message
            .as_deref()
            .unwrap()
            .contains("assertion"));
    }

    #[test]
    fn marked_database_tests_are_discovered_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("crates/cloud-api/src");
        fs::create_dir_all(&src).unwrap();
        fs::write(
            src.join("t.rs"),
            "#[tokio::test]\n#[ignore = \"requires a dedicated PostgreSQL database in CLOUD_TEST_DATABASE_URL\"]\nasync fn real_db_flow() {}\n\
             #[test]\n#[ignore = \"requires a running staging registry\"]\nfn needs_registry() {}\n\
             #[test]\nfn plain() {}\n",
        )
        .unwrap();
        assert_eq!(
            discover_database_tests(dir.path()).unwrap(),
            vec![DbTest {
                package: "cloud-api".into(),
                name: "real_db_flow".into(),
                extra_env: vec![]
            }]
        );
        assert_eq!(
            extra_env_vars(
                "requires disposable CLOUD_TEST_DATABASE_URL and CLOUD_V2_APP_SDK_TEST_BIN"
            ),
            vec!["CLOUD_V2_APP_SDK_TEST_BIN".to_string()]
        );
        assert!(extra_env_vars(
            "requires a dedicated PostgreSQL database in CLOUD_TEST_DATABASE_URL"
        )
        .is_empty());
    }

    #[test]
    fn a_run_that_executed_nothing_is_not_a_pass() {
        let dir = tempfile::tempdir().unwrap();
        let env = DevEnvironment {
            root: dir.path().to_path_buf(),
            config: Default::default(),
            conn: crate::config::parse_database_url("postgres://cloud:pw@127.0.0.1:5999/cloud")
                .unwrap(),
            conn_source: "test".into(),
        };
        let runner = TestRunner::new(env);
        let none = runner.finish(
            "unit",
            vec![],
            Instant::now(),
            Some(0),
            "test result: ok. 0 passed; 0 failed; 0 ignored;",
        );
        assert_eq!(none.status, Status::Skipped);
        let pass = runner.finish(
            "unit",
            vec![],
            Instant::now(),
            Some(0),
            "test result: ok. 4 passed; 0 failed; 0 ignored;",
        );
        assert_eq!(pass.status, Status::Passed);
        let fail = runner.finish(
            "unit",
            vec![],
            Instant::now(),
            Some(101),
            "test result: FAILED. 1 passed; 1 failed; 0 ignored;",
        );
        assert_eq!(fail.status, Status::Failed);
        assert_eq!(exit_code_for(std::slice::from_ref(&pass)), 0);
        assert_eq!(exit_code_for(&[pass.clone(), fail]), 1);
        assert_eq!(
            exit_code_for(&[pass.clone(), TestRun::not_run("integration", "x")]),
            1
        );
        assert_eq!(exit_code_for(&[pass, TestRun::unsupported("e2e", "x")]), 0);
    }

    #[test]
    fn statuses_serialize_as_stable_snake_case() {
        assert_eq!(
            serde_json::to_string(&Status::NotRun).unwrap(),
            "\"not_run\""
        );
        assert_eq!(
            serde_json::to_string(&Status::Unsupported).unwrap(),
            "\"unsupported\""
        );
    }
}
