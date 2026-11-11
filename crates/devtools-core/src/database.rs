//! DatabaseManager: one persistent, reusable local pgrust instance.
//!
//! `up` never builds anything. It reads the manifest, reuses a healthy managed instance or
//! starts the installed binary, and proves it is pgrust with a real SQL round trip. A port held
//! by anything this tool did not start is never reused and never signalled.

use crate::binary::{self, BinaryManager, InstallMeta, Manifest};
use crate::config::{Connection, DatabaseConfig, DevEnvironment};
use crate::health;
use crate::process;
use crate::util::is_identifier;
use anyhow::{bail, Context, Result};
use serde::Serialize;
use sqlx::Connection as _;
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// pgrust needs a bigger stack than the default 8 MiB (KiB).
const STACK_KIB: u64 = 65_520;
const DEFAULT_START_SECS: u64 = 40;
const STOP_TIMEOUT: Duration = Duration::from_secs(30);

/// How long `up` waits for SQL to answer. `REMAKE_DEV_START_TIMEOUT_SECS` overrides it (used
/// by the tests, which start a stand-in server that never answers).
fn start_timeout() -> Duration {
    let secs = std::env::var("REMAKE_DEV_START_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_START_SECS);
    Duration::from_secs(secs)
}

#[derive(Debug, Clone, Serialize)]
pub struct DatabaseStatus {
    pub installed: bool,
    pub running: bool,
    pub pid: Option<u32>,
    pub port: u16,
    pub binary_path: PathBuf,
    pub data_dir: PathBuf,
    pub data_initialized: bool,
    /// `None` when not running (nothing to probe).
    pub healthy: Option<bool>,
    pub server_version: Option<String>,
    pub installed_version: Option<String>,
    pub database: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpOutcome {
    pub status: DatabaseStatus,
    /// An already running, healthy managed instance was reused.
    pub reused: bool,
    pub database_created: bool,
    pub log_file: PathBuf,
}

#[derive(Debug, Clone)]
pub enum InstallSource {
    /// Build the pgrust checkout at this path (the only way cargo ever runs for pgrust).
    Source(PathBuf),
    /// Copy an already built release directory (`postgres`, `initdb`, `share/`).
    Prebuilt(PathBuf),
}

#[derive(Debug, Clone)]
pub struct InstallOptions {
    pub source: InstallSource,
    /// `db install --force`: replace an existing installation.
    pub force: bool,
    /// `db upgrade`: replacing is the point; the install must already exist.
    pub upgrade: bool,
    /// Build without `main_main/full` (faster; no CJK, pgcrypto or Snappy/Brotli).
    pub slim: bool,
    pub features: Option<Vec<String>>,
    /// Allow changing the server version while a data directory exists.
    pub accept_data_risk: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct InstallOutcome {
    pub manifest: Manifest,
    pub replaced: Option<Manifest>,
    pub built_from_source: bool,
}

#[allow(async_fn_in_trait)]
pub trait DatabaseController {
    async fn start(&self) -> Result<DatabaseStatus>;
    async fn stop(&self) -> Result<()>;
    async fn status(&self) -> Result<DatabaseStatus>;
    async fn health_check(&self) -> Result<()>;
}

pub struct DatabaseManager {
    env: DevEnvironment,
    bin: BinaryManager,
}

impl DatabaseManager {
    pub fn new(env: DevEnvironment) -> Self {
        let bin = BinaryManager::new(env.install_dir(), env.root.clone());
        Self { env, bin }
    }

    pub fn env(&self) -> &DevEnvironment {
        &self.env
    }

    pub fn binaries(&self) -> &BinaryManager {
        &self.bin
    }

    fn conn(&self) -> &Connection {
        &self.env.conn
    }

    /// The managed server's pid, only if the pid file names a live process that runs exactly
    /// the installed executable. A stale or recycled pid is forgotten, never signalled.
    pub fn managed_pid(&self) -> Option<u32> {
        let pidfile = self.env.pid_file();
        let pid = match process::read_pid(&pidfile) {
            Some(pid) => pid,
            None => {
                if pidfile.exists() {
                    let _ = fs::remove_file(&pidfile);
                }
                return None;
            }
        };
        let binary = self.bin.binary_path();
        if binary.is_file() && process::pid_alive(pid) && process::pid_belongs_to(pid, &binary) {
            Some(pid)
        } else {
            let _ = fs::remove_file(&pidfile);
            None
        }
    }

    fn instance_file(&self) -> PathBuf {
        self.env.state_dir().join("instance.json")
    }

    /// The port the managed server was started on. It can differ from the configured one if
    /// `DATABASE_URL` or devtools.toml changed since.
    pub fn running_port(&self) -> u16 {
        std::fs::read_to_string(self.instance_file())
            .ok()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .and_then(|v| v["port"].as_u64())
            .and_then(|p| u16::try_from(p).ok())
            .unwrap_or(self.conn().port)
    }

    fn conn_on_running_port(&self) -> Connection {
        Connection {
            port: self.running_port(),
            ..self.conn().clone()
        }
    }

    pub fn data_initialized(&self) -> bool {
        self.env.data_dir().join("PG_VERSION").is_file()
    }

    pub async fn status(&self) -> Result<DatabaseStatus> {
        let installed = self.bin.is_installed();
        let pid = if installed { self.managed_pid() } else { None };
        let (healthy, server_version) = if pid.is_some() {
            let report = health::check(&self.conn_on_running_port()).await;
            (
                Some(report.sql_ok && report.identity_ok),
                report.server_version,
            )
        } else {
            (None, None)
        };
        Ok(DatabaseStatus {
            installed,
            running: pid.is_some(),
            pid,
            port: if pid.is_some() {
                self.running_port()
            } else {
                self.conn().port
            },
            binary_path: self.bin.binary_path(),
            data_dir: self.env.data_dir(),
            data_initialized: self.data_initialized(),
            healthy,
            server_version,
            installed_version: self.bin.load_manifest().ok().map(|m| m.version),
            database: self.conn().database.clone(),
        })
    }

    /// A real SQL round trip, and proof that the server is pgrust.
    pub async fn health_check(&self) -> Result<String> {
        let version = health::server_version(self.conn()).await?;
        health::require_pgrust_identity(&version)?;
        Ok(version)
    }

    /// Create the data directory with the installed `initdb` if it does not exist yet.
    pub fn initialize(&self) -> Result<bool> {
        self.env.require_credentials()?;
        let data = self.env.data_dir();
        if self.data_initialized() {
            return Ok(false);
        }
        if data.exists() && fs::read_dir(&data)?.next().is_some() {
            bail!(
                "refusing to initialize inside a non-empty directory that is not a cluster: {}",
                data.display()
            );
        }
        self.bin.require_binary()?;
        self.env.ensure_dirs()?;
        if let Some(parent) = data.parent() {
            fs::create_dir_all(parent)?;
        }
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let pwfile = self
            .env
            .state_dir()
            .join(format!(".pw-{}-{nanos}", std::process::id()));
        fs::write(&pwfile, format!("{}\n", self.conn().password))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&pwfile, fs::Permissions::from_mode(0o600))?;
        }
        let result = std::process::Command::new(self.bin.initdb_path())
            .arg("-D")
            .arg(&data)
            .args([
                "--no-locale",
                "--encoding",
                "UTF8",
                "-U",
                &self.conn().user,
                "--auth-host=scram-sha-256",
                "--auth-local=scram-sha-256",
            ])
            .arg(format!("--pwfile={}", pwfile.display()))
            .status();
        let _ = fs::remove_file(&pwfile);
        match result {
            Ok(status) if status.success() => Ok(true),
            Ok(status) => bail!("initdb failed ({status})"),
            Err(e) => Err(e).context("failed to run the installed initdb"),
        }
    }

    /// Start the installed server, or reuse the healthy managed one. Never runs cargo.
    pub async fn up(&self) -> Result<UpOutcome> {
        self.env.require_credentials()?;
        // 1. The binary must be installed and match its manifest.
        let manifest = self.bin.verify(false)?;
        self.env.ensure_dirs()?;
        let log_file = self.env.log_file();

        // 2. A managed instance that is already up and healthy is reused as-is.
        if let Some(pid) = self.managed_pid() {
            if self.running_port() != self.conn().port {
                bail!(
                    "the managed pgrust (pid {pid}) is running on port {} but the configuration says {}; \
                     run `remake-dev db restart` to apply the new port",
                    self.running_port(),
                    self.conn().port
                );
            }
            let version = self.health_check().await.with_context(|| {
                format!(
                    "the managed server (pid {pid}) is running but unhealthy; inspect {} or run `remake-dev db restart`",
                    log_file.display()
                )
            })?;
            let created = self.ensure_database(&self.conn().database.clone()).await?;
            let mut status = self.status().await?;
            status.server_version = Some(version);
            return Ok(UpOutcome {
                status,
                reused: true,
                database_created: created,
                log_file,
            });
        }

        // 3. Anything else on the port is not ours: never reuse it, never stop it.
        if process::port_in_use(&self.conn().host, self.conn().port) {
            bail!(
                "port {} is already in use by a process this tool did not start; refusing to reuse \
                 or stop it. Change database.port in devtools.toml (or DATABASE_URL)",
                self.conn().port
            );
        }

        // 4. Create the cluster on first use, then start the server.
        self.initialize()?;
        let socket_dir = self.env.socket_dir();
        fs::create_dir_all(&socket_dir)?;
        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_file)
            .with_context(|| format!("cannot open {}", log_file.display()))?;
        let args = vec![
            "-D".to_string(),
            self.env.data_dir().to_string_lossy().into_owned(),
            "-k".to_string(),
            socket_dir.to_string_lossy().into_owned(),
            "-p".to_string(),
            self.conn().port.to_string(),
            "-c".to_string(),
            format!("listen_addresses={}", self.conn().host),
            "-c".to_string(),
            "io_method=sync".to_string(),
            "-c".to_string(),
            "max_stack_depth=60000".to_string(),
        ];
        let mut child = process::spawn_server(
            &self.bin.binary_path(),
            &args,
            &[("RUST_MIN_STACK", "33554432".to_string())],
            log,
            STACK_KIB,
        )?;
        fs::write(self.env.pid_file(), child.id().to_string())?;
        fs::write(
            self.instance_file(),
            serde_json::json!({ "pid": child.id(), "port": self.conn().port, "started_at": crate::util::now_rfc3339() })
                .to_string(),
        )?;

        // 5. Wait until it answers SQL as pgrust.
        let deadline = Instant::now() + start_timeout();
        loop {
            if let Some(status) = child.try_wait()? {
                let _ = fs::remove_file(self.env.pid_file());
                bail!(
                    "pgrust exited during startup ({status}); inspect {}",
                    log_file.display()
                );
            }
            match health::server_version(self.conn()).await {
                Ok(version) => {
                    if let Err(e) = health::require_pgrust_identity(&version) {
                        let _ = process::stop_pid(child.id(), STOP_TIMEOUT);
                        let _ = fs::remove_file(self.env.pid_file());
                        return Err(e);
                    }
                    break;
                }
                Err(_) if Instant::now() < deadline => {
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
                Err(e) => {
                    // Do not leave a half-started server behind.
                    let _ = process::stop_pid(child.id(), STOP_TIMEOUT);
                    let _ = fs::remove_file(self.env.pid_file());
                    return Err(e).context(format!(
                        "timed out waiting for pgrust; inspect {}",
                        log_file.display()
                    ));
                }
            }
        }
        let created = self.ensure_database(&self.conn().database.clone()).await?;
        let _ = manifest;
        Ok(UpOutcome {
            status: self.status().await?,
            reused: false,
            database_created: created,
            log_file,
        })
    }

    /// Stop the managed server (data is kept). Returns false when nothing managed was running.
    pub async fn down(&self) -> Result<bool> {
        let Some(pid) = self.managed_pid() else {
            return Ok(false);
        };
        process::stop_pid(pid, STOP_TIMEOUT)?;
        let _ = fs::remove_file(self.env.pid_file());
        let _ = fs::remove_file(self.instance_file());
        Ok(true)
    }

    pub async fn restart(&self) -> Result<UpOutcome> {
        self.down().await?;
        self.up().await
    }

    /// The last `lines` lines of the server log.
    pub fn logs(&self, lines: usize) -> Result<String> {
        let path = self.env.log_file();
        let text = fs::read_to_string(&path).with_context(|| {
            format!(
                "no log at {} (the server has not been started yet)",
                path.display()
            )
        })?;
        let all: Vec<&str> = text.lines().collect();
        let start = all.len().saturating_sub(lines);
        Ok(all[start..].join("\n"))
    }

    /// Create `name` if it is missing. `name` is validated, so it is safe to quote.
    pub async fn ensure_database(&self, name: &str) -> Result<bool> {
        if !is_identifier(name) {
            bail!("`{name}` is not a safe database name");
        }
        let mut c = health::connect(self.conn(), "postgres").await?;
        let exists = sqlx::query(&format!(
            "SELECT 1 FROM pg_database WHERE datname = '{name}'"
        ))
        .fetch_optional(&mut c)
        .await?
        .is_some();
        if !exists {
            sqlx::query(&format!("CREATE DATABASE \"{name}\""))
                .execute(&mut c)
                .await?;
        }
        let _ = c.close().await;
        Ok(!exists)
    }

    /// Recreate the isolated test database. This is the only destructive database operation
    /// the tools offer, and it cannot name the development database.
    pub async fn reset_test_database(&self, yes: bool) -> Result<String> {
        self.env.require_credentials()?;
        let name = reset_target(&self.env.config.database, &self.conn().database, yes)?;
        if self.managed_pid().is_none() {
            bail!("the managed pgrust is not running; start it with `remake-dev db up`");
        }
        let mut c = health::connect(self.conn(), "postgres").await?;
        sqlx::query(&format!("DROP DATABASE IF EXISTS \"{name}\""))
            .execute(&mut c)
            .await?;
        sqlx::query(&format!("CREATE DATABASE \"{name}\""))
            .execute(&mut c)
            .await?;
        let _ = c.close().await;
        Ok(name)
    }

    /// URL for the isolated test database, to export as `CLOUD_TEST_DATABASE_URL`.
    pub fn test_database_url(&self) -> String {
        database_url(self.conn(), &self.env.config.database.test_database)
    }

    /// Install (or, with `upgrade`, replace) pgrust. Building from source is the only path
    /// that runs cargo, and it runs here and nowhere else.
    pub fn install(&self, opts: &InstallOptions) -> Result<InstallOutcome> {
        let existing = self.bin.verify(false).ok();
        if opts.upgrade && existing.is_none() {
            bail!("nothing to upgrade: pgrust is not installed. Run `remake-dev db install`");
        }
        if !opts.upgrade && existing.is_some() && !opts.force {
            bail!(
                "pgrust is already installed ({}); use `remake-dev db upgrade` to update it or \
                 `db install --force` to reinstall",
                existing.as_ref().map(|m| m.version.as_str()).unwrap_or("?")
            );
        }
        let (release_dir, meta, built) = match &opts.source {
            InstallSource::Source(path) => {
                let source = absolute(&self.env.root, path);
                binary::ensure_compat_patch(
                    &source,
                    &self
                        .env
                        .root
                        .join("patches/pgrust-mimalloc-glibc-2.44.patch"),
                )?;
                let features = match (&opts.features, opts.slim) {
                    (Some(f), _) => f.clone(),
                    (None, true) => Vec::new(),
                    (None, false) => binary::detect_features(&source),
                };
                let release = binary::build_from_source(&source, &features)?;
                let meta = InstallMeta {
                    source_type: "source".into(),
                    source_commit: binary::git_commit(&source),
                    source_dirty: binary::git_dirty(&source),
                    build_profile: "release".into(),
                    features,
                };
                (release, meta, true)
            }
            InstallSource::Prebuilt(path) => {
                let dir = absolute(&self.env.root, path);
                let meta = InstallMeta {
                    source_type: "prebuilt".into(),
                    source_commit: None,
                    source_dirty: false,
                    build_profile: "release".into(),
                    features: opts.features.clone().unwrap_or_default(),
                };
                (dir, meta, false)
            }
        };
        // A different server version over an existing cluster needs a dump and restore.
        if let (Some(old), true) = (&existing, self.data_initialized()) {
            let new_version = release_version(&release_dir)?;
            if new_version != old.version && !opts.accept_data_risk {
                bail!(
                    "the server version would change ({} -> {}) while a data directory exists at {}. \
                     pgrust data is not guaranteed to carry across versions: pg_dump first, then \
                     re-run with --accept-data-risk",
                    old.version,
                    new_version,
                    self.env.data_dir().display()
                );
            }
        }
        // Building can take hours and does not need the server stopped; swapping the binary does.
        if self.managed_pid().is_some() {
            bail!(
                "the managed pgrust is running; stop it with `remake-dev db down`, then re-run this \
                 command (the build in {} is kept, so nothing is recompiled)",
                release_dir.display()
            );
        }
        let manifest = self.bin.install_from_dir(&release_dir, meta)?;
        Ok(InstallOutcome {
            manifest,
            replaced: existing,
            built_from_source: built,
        })
    }
}

impl DatabaseController for DatabaseManager {
    async fn start(&self) -> Result<DatabaseStatus> {
        Ok(self.up().await?.status)
    }
    async fn stop(&self) -> Result<()> {
        self.down().await.map(|_| ())
    }
    async fn status(&self) -> Result<DatabaseStatus> {
        DatabaseManager::status(self).await
    }
    async fn health_check(&self) -> Result<()> {
        DatabaseManager::health_check(self).await.map(|_| ())
    }
}

fn absolute(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .ok()
            .filter(|p| p.exists())
            .unwrap_or_else(|| root.join(path))
    }
}

fn release_version(release_dir: &Path) -> Result<String> {
    crate::util::command_output(
        release_dir
            .join(binary::exe_name("postgres"))
            .to_str()
            .unwrap_or_default(),
        &["--version"],
    )
    .with_context(|| {
        format!(
            "{} does not run (`postgres --version` failed)",
            release_dir.display()
        )
    })
}

/// The one database `db reset` may recreate, or the reason it refuses.
pub fn reset_target(config: &DatabaseConfig, dev_database: &str, yes: bool) -> Result<String> {
    if !yes {
        bail!("`db reset` recreates the isolated test database; pass --yes to confirm");
    }
    let name = &config.test_database;
    if !is_identifier(name) {
        bail!("test database `{name}` is not a safe identifier");
    }
    if name == &config.database || name == dev_database {
        bail!("refusing to reset `{name}`: it is the development database");
    }
    Ok(name.clone())
}

fn percent_encode(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

pub fn database_url(conn: &Connection, database: &str) -> String {
    format!(
        "postgres://{}:{}@{}:{}/{}",
        percent_encode(&conn.user),
        percent_encode(&conn.password),
        conn.host,
        conn.port,
        database
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DevConfig;

    #[test]
    fn reset_needs_yes_and_never_names_the_dev_database() {
        let cfg = DatabaseConfig::default();
        assert!(reset_target(&cfg, "remake_dev", false)
            .unwrap_err()
            .to_string()
            .contains("--yes"));
        assert_eq!(
            reset_target(&cfg, "remake_dev", true).unwrap(),
            "cloud_test"
        );
        // Even a hostile config cannot point the reset at the dev database.
        let mut bad = DevConfig::default().database;
        bad.test_database = "remake_dev".into();
        assert!(reset_target(&bad, "remake_dev", true)
            .unwrap_err()
            .to_string()
            .contains("development database"));
        let mut other = DevConfig::default().database;
        other.test_database = "cloud".into();
        assert!(reset_target(&other, "cloud", true).is_err());
    }

    #[test]
    fn database_urls_encode_credentials() {
        let conn = Connection {
            host: "127.0.0.1".into(),
            port: 55432,
            user: "cloud".into(),
            password: "p@ss:w/rd".into(),
            database: "x".into(),
        };
        assert_eq!(
            database_url(&conn, "cloud_test"),
            "postgres://cloud:p%40ss%3Aw%2Frd@127.0.0.1:55432/cloud_test"
        );
    }
}
