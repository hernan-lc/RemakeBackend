//! Configuration: `devtools.toml` plus the backend's own `DATABASE_URL`.
//!
//! `DATABASE_URL` stays the single source of truth for the connection. `devtools.toml`
//! only describes where things live and how the tools behave; its connection fields are
//! the fallback when no `DATABASE_URL` is set. Credentials never live in the versioned file.

use crate::util::is_identifier;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub const CONFIG_FILE: &str = "devtools.toml";
pub const DEVTOOLS_DIR: &str = ".devtools";
/// `conn_source` when no `DATABASE_URL`, `.env` entry or `REMAKE_DEV_DB_PASSWORD` was found.
pub const MISSING_CREDENTIALS: &str = "missing";

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct DatabaseConfig {
    pub engine: String,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    pub data_dir: String,
    pub binary_dir: String,
    pub auto_start: bool,
    /// Must stay false: pgrust is only ever built by `remake-dev db install` / `db upgrade`.
    pub auto_build: bool,
    /// Isolated database used by integration tests and the only one `db reset` may touch.
    pub test_database: String,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            engine: "pgrust".into(),
            host: "127.0.0.1".into(),
            port: 55432,
            database: "remake_dev".into(),
            username: "postgres".into(),
            data_dir: ".devtools/database/dev".into(),
            binary_dir: ".devtools/bin/pgrust".into(),
            auto_start: true,
            auto_build: false,
            test_database: "cloud_test".into(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct DevelopmentConfig {
    pub watch: bool,
    pub restart_on_change: bool,
    pub debounce_ms: u64,
}

impl Default for DevelopmentConfig {
    fn default() -> Self {
        Self {
            watch: true,
            restart_on_change: true,
            debounce_ms: 500,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct TestsConfig {
    pub parallel: bool,
    pub isolated_database: bool,
    pub timeout_seconds: u64,
}

impl Default for TestsConfig {
    fn default() -> Self {
        Self {
            parallel: true,
            isolated_database: true,
            timeout_seconds: 120,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct ReportsConfig {
    pub directory: String,
    pub format: String,
}

impl Default for ReportsConfig {
    fn default() -> Self {
        Self {
            directory: ".devtools/reports".into(),
            format: "json".into(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct DevConfig {
    pub database: DatabaseConfig,
    pub development: DevelopmentConfig,
    pub tests: TestsConfig,
    pub reports: ReportsConfig,
}

/// How to reach the local pgrust. The password is never printed or serialized.
#[derive(Clone, PartialEq, Eq)]
pub struct Connection {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub database: String,
}

impl std::fmt::Debug for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connection")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("user", &self.user)
            .field("database", &self.database)
            .field("password", &"<redacted>")
            .finish()
    }
}

/// Everything the tools need to know about the checkout they run in.
#[derive(Debug, Clone)]
pub struct DevEnvironment {
    pub root: PathBuf,
    pub config: DevConfig,
    pub conn: Connection,
    /// Where the connection came from, for `doctor`/`report` (`DATABASE_URL`, `.env`, `devtools.toml`).
    pub conn_source: String,
}

impl DevEnvironment {
    /// Locate the checkout from the current directory (or `REMAKE_DEV_ROOT`) and load it.
    pub fn load() -> Result<Self> {
        Self::load_from(&discover_root()?)
    }

    pub fn load_from(root: &Path) -> Result<Self> {
        let config_path = root.join(CONFIG_FILE);
        let config: DevConfig = if config_path.is_file() {
            let text = fs::read_to_string(&config_path)
                .with_context(|| format!("cannot read {}", config_path.display()))?;
            toml::from_str(&text).with_context(|| format!("invalid {}", config_path.display()))?
        } else {
            DevConfig::default()
        };
        config.validate()?;
        let (conn, conn_source) = resolve_connection(root, &config)?;
        Ok(Self {
            root: root.to_path_buf(),
            config,
            conn,
            conn_source,
        })
    }

    fn rel(&self, value: &str) -> PathBuf {
        let path = Path::new(value);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root.join(path)
        }
    }

    /// Operations that talk to the database call this first. Commands that do not (`setup`,
    /// `doctor`, `clean`, `db install`) work without credentials.
    pub fn require_credentials(&self) -> Result<()> {
        if self.conn_source == MISSING_CREDENTIALS {
            bail!(
                "no database credentials: set DATABASE_URL (or a DATABASE_URL line in .env) or \
                 REMAKE_DEV_DB_PASSWORD; passwords are never read from {CONFIG_FILE}"
            );
        }
        Ok(())
    }

    pub fn devtools_dir(&self) -> PathBuf {
        self.root.join(DEVTOOLS_DIR)
    }
    pub fn install_dir(&self) -> PathBuf {
        self.rel(&self.config.database.binary_dir)
    }
    /// `REMAKE_DEV_DATA_DIR` lets a developer point at an existing cluster (for example
    /// the one the old `pgrust-local` created) without editing the versioned config.
    pub fn data_dir(&self) -> PathBuf {
        match std::env::var("REMAKE_DEV_DATA_DIR") {
            Ok(dir) if !dir.trim().is_empty() => PathBuf::from(dir),
            _ => self.rel(&self.config.database.data_dir),
        }
    }
    pub fn logs_dir(&self) -> PathBuf {
        self.devtools_dir().join("logs")
    }
    pub fn state_dir(&self) -> PathBuf {
        self.devtools_dir().join("state")
    }
    pub fn reports_dir(&self) -> PathBuf {
        self.rel(&self.config.reports.directory)
    }
    pub fn pid_file(&self) -> PathBuf {
        self.state_dir().join("pgrust.pid")
    }
    pub fn log_file(&self) -> PathBuf {
        self.logs_dir().join("pgrust.log")
    }
    /// Unix socket directory. Sockets have a ~100 byte path limit, so a long checkout path
    /// falls back to a short per-user directory in /tmp.
    pub fn socket_dir(&self) -> PathBuf {
        let preferred = self.state_dir().join("run");
        if preferred.as_os_str().len() + "/.s.PGSQL.65535.lock".len() <= 100 {
            return preferred;
        }
        let mut hasher = <sha2::Sha256 as sha2::Digest>::new();
        sha2::Digest::update(&mut hasher, self.root.to_string_lossy().as_bytes());
        let tag = hex::encode(sha2::Digest::finalize(hasher));
        PathBuf::from(format!("/tmp/remake-dev-{}", &tag[..12]))
    }
    pub fn ensure_dirs(&self) -> Result<()> {
        for dir in [
            self.devtools_dir(),
            self.logs_dir(),
            self.state_dir(),
            self.reports_dir(),
        ] {
            fs::create_dir_all(&dir).with_context(|| format!("cannot create {}", dir.display()))?;
        }
        Ok(())
    }
}

impl DevConfig {
    pub fn validate(&self) -> Result<()> {
        let db = &self.database;
        if db.engine != "pgrust" {
            bail!(
                "unsupported database.engine `{}` (only `pgrust` is managed)",
                db.engine
            );
        }
        if db.auto_build {
            bail!(
                "database.auto_build = true is not supported: pgrust is only built by \
                 `remake-dev db install` or `remake-dev db upgrade`"
            );
        }
        if db.host != "127.0.0.1" && db.host != "localhost" {
            bail!("database.host must be 127.0.0.1 or localhost; the managed server only listens on loopback");
        }
        if db.port == 0 {
            bail!("database.port must not be 0");
        }
        for (field, value) in [
            ("database.database", &db.database),
            ("database.username", &db.username),
            ("database.test_database", &db.test_database),
        ] {
            if !is_identifier(value) {
                bail!("{field} `{value}` must be a lowercase PostgreSQL identifier");
            }
        }
        if db.test_database == db.database {
            bail!("database.test_database must differ from database.database");
        }
        Ok(())
    }
}

fn discover_root() -> Result<PathBuf> {
    if let Ok(root) = std::env::var("REMAKE_DEV_ROOT") {
        if !root.trim().is_empty() {
            return Ok(PathBuf::from(root));
        }
    }
    let mut dir = std::env::current_dir()?;
    loop {
        if dir.join(CONFIG_FILE).is_file() || is_workspace_root(&dir) {
            return Ok(dir);
        }
        if !dir.pop() {
            bail!(
                "not inside a RemakeBackend checkout (no {CONFIG_FILE} or workspace Cargo.toml found); \
                 set REMAKE_DEV_ROOT"
            );
        }
    }
}

fn is_workspace_root(dir: &Path) -> bool {
    fs::read_to_string(dir.join("Cargo.toml"))
        .map(|text| text.contains("[workspace]") && dir.join("crates").join("devtools").is_dir())
        .unwrap_or(false)
}

/// Connection precedence: `DATABASE_URL` env, then `.env`, then `devtools.toml`.
fn resolve_connection(root: &Path, config: &DevConfig) -> Result<(Connection, String)> {
    if let Ok(value) = std::env::var("DATABASE_URL") {
        if !value.trim().is_empty() {
            return Ok((parse_database_url(&value)?, "DATABASE_URL".into()));
        }
    }
    if let Ok(text) = fs::read_to_string(root.join(".env")) {
        for line in text.lines() {
            if let Some(rest) = line.trim().strip_prefix("DATABASE_URL=") {
                let value = rest.trim().trim_matches(|c| c == '"' || c == '\'');
                if !value.is_empty() {
                    return Ok((parse_database_url(value)?, ".env".into()));
                }
            }
        }
    }
    let password = std::env::var("REMAKE_DEV_DB_PASSWORD").unwrap_or_default();
    let db = &config.database;
    let source = if password.trim().is_empty() {
        MISSING_CREDENTIALS
    } else {
        CONFIG_FILE
    };
    Ok((
        Connection {
            host: "127.0.0.1".into(),
            port: db.port,
            user: db.username.clone(),
            password,
            database: db.database.clone(),
        },
        source.into(),
    ))
}

fn unquote(input: &str) -> Result<String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                if i + 2 >= bytes.len() {
                    bail!("invalid percent-encoding");
                }
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3])?;
                out.push(u8::from_str_radix(hex, 16).context("invalid percent-encoding")?);
                i += 3;
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    Ok(String::from_utf8(out)?)
}

/// Parse the backend's `DATABASE_URL`. Only loopback hosts are accepted: this tool starts
/// and stops a local server, it never talks to a remote one.
pub fn parse_database_url(value: &str) -> Result<Connection> {
    const LOCAL: &str =
        "the local pgrust tools require a postgres:// URL on localhost or 127.0.0.1";
    const SHAPE: &str = "DATABASE_URL needs a username, password and database name";
    let value = value.trim();
    let (scheme, rest) = value.split_once("://").context(LOCAL)?;
    if scheme != "postgres" && scheme != "postgresql" {
        bail!(LOCAL);
    }
    if rest.contains('?') || rest.contains('#') {
        bail!("unsupported DATABASE_URL parameters for the local pgrust tools");
    }
    let (authority, path) = rest.split_once('/').context(SHAPE)?;
    let db_raw = path.trim_matches('/');
    if db_raw.is_empty() {
        bail!(SHAPE);
    }
    let database = unquote(db_raw)?;
    let (userinfo, hostport) = authority.rsplit_once('@').context(SHAPE)?;
    let (user_raw, password_raw) = userinfo.split_once(':').context(SHAPE)?;
    let user = unquote(user_raw)?;
    let password = unquote(password_raw)?;
    if user.is_empty() || password.is_empty() || database.is_empty() {
        bail!(SHAPE);
    }
    if !is_identifier(&user) || !is_identifier(&database) {
        bail!("use lowercase PostgreSQL identifiers for the database and username");
    }
    let (host, port) = match hostport.split_once(':') {
        Some((host, port_raw)) => {
            let port: u16 = port_raw
                .parse()
                .map_err(|_| anyhow::anyhow!("unsupported DATABASE_URL port"))?;
            if port == 0 {
                bail!("unsupported DATABASE_URL port");
            }
            (host, port)
        }
        None => (hostport, 5432),
    };
    if host != "localhost" && host != "127.0.0.1" {
        bail!(LOCAL);
    }
    Ok(Connection {
        host: "127.0.0.1".into(),
        port,
        user,
        password,
        database,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn database_url_valid() {
        let c = parse_database_url("postgres://cloud:local-only-password@localhost:5433/cloud")
            .unwrap();
        assert_eq!(c.user, "cloud");
        assert_eq!(c.password, "local-only-password");
        assert_eq!(c.database, "cloud");
        assert_eq!(c.port, 5433);
    }

    #[test]
    fn database_url_default_port() {
        assert_eq!(
            parse_database_url("postgres://cloud:pw@127.0.0.1/cloud")
                .unwrap()
                .port,
            5432
        );
    }

    #[test]
    fn database_url_rejects_remote_injection_and_missing_password() {
        assert!(
            parse_database_url("postgres://cloud:pass@example.com/cloud")
                .unwrap_err()
                .to_string()
                .contains("localhost")
        );
        assert!(
            parse_database_url("postgres://cloud:pass@localhost/cloud%22%3Bdrop")
                .unwrap_err()
                .to_string()
                .contains("identifiers")
        );
        assert!(parse_database_url("postgres://cloud@localhost/cloud").is_err());
    }

    #[test]
    fn debug_never_prints_the_password() {
        let c = parse_database_url("postgres://cloud:secret-pw@localhost/cloud").unwrap();
        assert!(!format!("{c:?}").contains("secret-pw"));
    }

    #[test]
    fn config_defaults_follow_the_plan_and_reject_auto_build() {
        let cfg = DevConfig::default();
        cfg.validate().unwrap();
        assert_eq!(cfg.database.port, 55432);
        assert!(!cfg.database.auto_build);
        let mut bad = DevConfig::default();
        bad.database.auto_build = true;
        assert!(bad
            .validate()
            .unwrap_err()
            .to_string()
            .contains("auto_build"));
        let mut same = DevConfig::default();
        same.database.test_database = same.database.database.clone();
        assert!(same.validate().is_err());
    }

    #[test]
    fn partial_toml_uses_defaults() {
        let cfg: DevConfig = toml::from_str("[database]\nport = 6000\n").unwrap();
        assert_eq!(cfg.database.port, 6000);
        assert_eq!(cfg.database.database, "remake_dev");
        assert_eq!(cfg.tests.timeout_seconds, 120);
    }
}
