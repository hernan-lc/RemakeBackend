//! Optional, Docker-free pgrust launcher for local RemakeBackend development.
//!
//! Builds the pinned pgrust source locally, without Docker.
//! Keeps an isolated cluster under
//! `~/.local/share/remakebackend/pgrust` (or `$PGRUST_LOCAL_HOME`), and starts
//! it on 127.0.0.1. Migrations stay explicit (`cargo run -p cloud-api -- migrate`).

use std::env;
use std::fs::{self, OpenOptions};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn project_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

/// Home root for releases, data, logs and pid file.
fn home_dir() -> PathBuf {
    if let Ok(dir) = env::var("PGRUST_LOCAL_HOME") {
        if !dir.trim().is_empty() {
            return PathBuf::from(dir);
        }
    }
    if let Ok(xdg) = env::var("XDG_DATA_HOME") {
        if !xdg.trim().is_empty() {
            return PathBuf::from(xdg).join("remakebackend").join("pgrust");
        }
    }
    let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("remakebackend")
        .join("pgrust")
}

fn bin_dir() -> Result<PathBuf, String> {
    let dir = project_dir().join("vendor/pgrust/target/release");
    if !dir.join("postgres").is_file() || !dir.join("initdb").is_file() {
        return Err("pgrust is not built; run `cargo run -p pgrust-local -- install`".into());
    }
    Ok(dir)
}

fn data_dir() -> PathBuf {
    home_dir().join("data")
}

/// Percent-decode a URL component; rejects malformed escapes.
fn unquote(input: &str) -> Result<String, String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                if i + 2 >= bytes.len() {
                    return Err("Invalid percent-encoding".to_string());
                }
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3])
                    .map_err(|_| "Invalid percent-encoding".to_string())?;
                let byte = u8::from_str_radix(hex, 16)
                    .map_err(|_| "Invalid percent-encoding".to_string())?;
                out.push(byte);
                i += 3;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).map_err(|_| "Invalid percent-encoding".to_string())
}

fn is_identifier(name: &str) -> bool {
    let bytes = name.as_bytes();
    if bytes.is_empty() || bytes.len() > 63 {
        return false;
    }
    if !(bytes[0].is_ascii_lowercase() || bytes[0] == b'_') {
        return false;
    }
    bytes
        .iter()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'_')
}

/// Parsed `DATABASE_URL` for the local cluster: (user, password, database, port).
#[derive(Debug, PartialEq, Eq)]
struct DbSettings {
    user: String,
    password: String,
    database: String,
    port: u16,
}

fn parse_database_url(value: &str) -> Result<DbSettings, String> {
    let value = value.trim();
    let (scheme, rest) = value.split_once("://").ok_or_else(|| {
        "Local pgrust requires a postgres:// URL on localhost or 127.0.0.1".to_string()
    })?;
    if scheme != "postgres" && scheme != "postgresql" {
        return Err(
            "Local pgrust requires a postgres:// URL on localhost or 127.0.0.1".to_string(),
        );
    }
    if rest.contains('?') || rest.contains('#') {
        return Err("Unsupported DATABASE_URL parameters for the local pgrust tool".to_string());
    }
    let (authority, path) = rest
        .split_once('/')
        .ok_or_else(|| "DATABASE_URL needs a username, password and database name".to_string())?;
    let db_raw = path.trim_matches('/');
    if db_raw.is_empty() {
        return Err("DATABASE_URL needs a username, password and database name".to_string());
    }
    let database =
        unquote(db_raw).map_err(|_| "Use lowercase PostgreSQL identifiers".to_string())?;
    let (userinfo, hostport) = authority
        .rsplit_once('@')
        .ok_or_else(|| "DATABASE_URL needs a username, password and database name".to_string())?;
    let (user_raw, password_raw) = userinfo
        .split_once(':')
        .ok_or_else(|| "DATABASE_URL needs a username, password and database name".to_string())?;
    let user = unquote(user_raw).map_err(|_| "Use lowercase PostgreSQL identifiers".to_string())?;
    let password =
        unquote(password_raw).map_err(|_| "DATABASE_URL password cannot be empty".to_string())?;
    if user.is_empty() || password.is_empty() || database.is_empty() {
        return Err("DATABASE_URL needs a username, password and database name".to_string());
    }
    if !is_identifier(&user) || !is_identifier(&database) {
        return Err("Use lowercase PostgreSQL identifiers for database and username".to_string());
    }
    if password.is_empty() {
        return Err("DATABASE_URL password cannot be empty".to_string());
    }
    let (host, port) = match hostport.split_once(':') {
        Some((host, port_raw)) => {
            let port: u16 = port_raw.parse().map_err(|_| {
                "Unsupported DATABASE_URL parameters for the local pgrust tool".to_string()
            })?;
            if port == 0 {
                return Err(
                    "Unsupported DATABASE_URL parameters for the local pgrust tool".to_string(),
                );
            }
            (host, port)
        }
        None => (hostport, 5432),
    };
    if host != "localhost" && host != "127.0.0.1" {
        return Err(
            "Local pgrust requires a postgres:// URL on localhost or 127.0.0.1".to_string(),
        );
    }
    Ok(DbSettings {
        user,
        password,
        database,
        port,
    })
}

fn database_settings() -> Result<DbSettings, String> {
    if let Ok(value) = env::var("DATABASE_URL") {
        if !value.trim().is_empty() {
            return parse_database_url(&value);
        }
    }
    let dotenv = project_dir().join(".env");
    if let Ok(text) = fs::read_to_string(&dotenv) {
        for line in text.lines() {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix("DATABASE_URL=") {
                let value = rest.trim().trim_matches(|c| c == '"' || c == '\'');
                if !value.is_empty() {
                    return parse_database_url(value);
                }
            }
        }
    }
    Err("Set DATABASE_URL or create .env from .env.example before initializing".to_string())
}

fn install() -> Result<(), String> {
    let source = project_dir().join("vendor/pgrust");
    if !source.join("Cargo.toml").is_file() {
        return Err(
            "pgrust source is missing; run `git submodule update --init --recursive`".into(),
        );
    }
    // The submodule cannot record uncommitted changes in its parent. Apply the
    // maintained compatibility patch on fresh checkouts before building.
    let patch = project_dir().join("patches/pgrust-mimalloc-glibc-2.44.patch");
    let applied = Command::new("git")
        .current_dir(&source)
        .args(["apply", "--unidiff-zero", "--reverse", "--check"])
        .arg(&patch)
        .output()
        .map_err(|e| format!("failed to check pgrust compatibility patch: {e}"))?;
    if !applied.status.success() {
        let result = Command::new("git")
            .current_dir(&source)
            .args(["apply", "--unidiff-zero"])
            .arg(&patch)
            .output()
            .map_err(|e| format!("failed to apply pgrust compatibility patch: {e}"))?;
        if !result.status.success() {
            return Err(format!(
                "cannot apply pgrust compatibility patch; check submodule changes: {}",
                String::from_utf8_lossy(&result.stderr)
            ));
        }
    }
    let status = Command::new("cargo")
        .current_dir(&source)
        .env_remove("CARGO_TARGET_DIR")
        .env_remove("RUSTUP_TOOLCHAIN")
        .args([
            "build",
            "--release",
            "--locked",
            "-p",
            "main_main",
            "-p",
            "initdb",
        ])
        .status()
        .map_err(|e| format!("failed to build pgrust: {e}"))?;
    if !status.success() {
        return Err(format!("pgrust build failed: {status}"));
    }
    bin_dir()?;
    Ok(())
}

fn initialize(settings: &DbSettings) -> Result<(), String> {
    if data_dir().join("PG_VERSION").is_file() {
        return Ok(());
    }
    if data_dir().exists()
        && fs::read_dir(data_dir())
            .map(|mut entries| entries.next().is_some())
            .unwrap_or(false)
    {
        return Err(format!(
            "Refusing to overwrite a nonempty pgrust directory: {}",
            data_dir().display()
        ));
    }
    let bin = bin_dir()?;
    let version = Command::new(bin.join("postgres"))
        .arg("--version")
        .output()
        .map_err(|e| format!("failed to execute pgrust postgres: {e}"))?;
    if !version.status.success() {
        return Err(format!("pgrust postgres --version failed ({}); the upstream binary may be incompatible with this system; see docs/LOCAL-PGRUST.md", version.status));
    }
    fs::create_dir_all(home_dir()).map_err(|e| format!("failed to create home: {e}"))?;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pwfile = home_dir().join(format!(".pw-{}-{nanos}", std::process::id()));
    fs::write(&pwfile, format!("{}\n", settings.password))
        .map_err(|e| format!("failed to write pwfile: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&pwfile, fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("failed to chmod pwfile: {e}"))?;
    }
    let result = Command::new(bin.join("initdb"))
        .args([
            "-D",
            &data_dir().to_string_lossy(),
            "--no-locale",
            "--encoding",
            "UTF8",
            "-U",
            &settings.user,
            "--auth-host=scram-sha-256",
            "--auth-local=scram-sha-256",
            &format!("--pwfile={}", pwfile.display()),
        ])
        .status()
        .map_err(|e| format!("failed to run initdb: {e}"));
    let _ = fs::remove_file(&pwfile);
    match result {
        Ok(status) if status.success() => {
            println!("Initialized private cluster at {}", data_dir().display());
            Ok(())
        }
        _ => Err("initdb failed".to_string()),
    }
}

fn query(settings: &DbSettings, sql: &str, database: &str) -> Result<String, String> {
    let output = Command::new("psql")
        .args([
            "-X",
            "-A",
            "-t",
            "-v",
            "ON_ERROR_STOP=1",
            "-h",
            "127.0.0.1",
            "-p",
            &settings.port.to_string(),
            "-U",
            &settings.user,
            "-d",
            database,
            "-c",
            sql,
        ])
        .env("PGPASSWORD", &settings.password)
        .env("RUST_MIN_STACK", "33554432")
        .output()
        .map_err(|e| format!("psql failed: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "psql failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn port_busy(port: u16) -> bool {
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
    TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_ok()
}

/// Return the managed postgres pid after verifying it belongs to this install.
fn verified_pid() -> Result<Option<u32>, String> {
    let pidfile = home_dir().join("pgrust.pid");
    if !pidfile.is_file() {
        return Ok(None);
    }
    let text = fs::read_to_string(&pidfile).map_err(|e| format!("failed to read pidfile: {e}"))?;
    let pid: u32 = match text.trim().parse() {
        Ok(pid) => pid,
        Err(_) => {
            let _ = fs::remove_file(&pidfile);
            return Ok(None);
        }
    };
    let expected = bin_dir()?.join("postgres").to_string_lossy().into_owned();
    if pid_belongs_to(pid, &expected) {
        Ok(Some(pid))
    } else {
        let _ = fs::remove_file(&pidfile);
        Ok(None)
    }
}

fn pid_belongs_to(pid: u32, expected_binary: &str) -> bool {
    #[cfg(target_os = "linux")]
    {
        fs::read_link(format!("/proc/{pid}/exe"))
            .ok()
            .zip(fs::canonicalize(expected_binary).ok())
            .is_some_and(|(actual, expected)| actual == expected)
    }
    #[cfg(not(target_os = "linux"))]
    {
        Command::new("ps")
            .args(["-p", &pid.to_string(), "-o", "comm="])
            .output()
            .map(|output| {
                output.status.success()
                    && String::from_utf8_lossy(&output.stdout).trim() == expected_binary
            })
            .unwrap_or(false)
    }
}

fn pid_alive(pid: u32) -> bool {
    #[cfg(target_os = "linux")]
    {
        if Path::new(&format!("/proc/{pid}")).exists() {
            return true;
        }
    }
    Command::new("ps")
        .args(["-p", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn up() -> Result<(), String> {
    let settings = database_settings()?;
    install()?;
    initialize(&settings)?;
    if verified_pid()?.is_some() {
        println!("pgrust already running");
        return Ok(());
    }
    if port_busy(settings.port) {
        return Err(format!(
            "Port {} already in use; refusing to connect to an unrelated server",
            settings.port
        ));
    }
    let rundir = home_dir().join("run");
    fs::create_dir_all(&rundir).map_err(|e| format!("failed to create run dir: {e}"))?;
    let log = home_dir().join("pgrust.log");
    let output = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log)
        .map_err(|e| format!("failed to open log: {e}"))?;
    let stderr = output
        .try_clone()
        .map_err(|e| format!("failed to dup log: {e}"))?;
    // pgrust's recommended max_stack_depth requires a larger process stack
    // limit than the usual 8 MiB shell default. Only change it in the child.
    let mut child = Command::new("sh")
        .args([
            "-c",
            "ulimit -s 65520 || { echo 'Cannot raise stack limit to 65520 KiB; check the hard stack limit' >&2; exit 1; }; exec \"$@\"",
            "pgrust-local",
        ])
        .arg(bin_dir()?.join("postgres"))
        .args([
            "-D",
            &data_dir().to_string_lossy(),
            "-k",
            &rundir.to_string_lossy(),
            "-p",
            &settings.port.to_string(),
            "-c",
            "listen_addresses=127.0.0.1",
            "-c",
            "io_method=sync",
            "-c",
            "max_stack_depth=60000",
        ])
        .env("PGPASSWORD", &settings.password)
        .env("RUST_MIN_STACK", "33554432")
        .stdin(Stdio::null())
        .stdout(output)
        .stderr(stderr)
        .spawn()
        .map_err(|e| format!("failed to start postgres: {e}"))?;
    let pid = child.id();
    fs::write(home_dir().join("pgrust.pid"), pid.to_string())
        .map_err(|e| format!("failed to write pidfile: {e}"))?;
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|e| format!("failed to check pgrust startup: {e}"))?
        {
            let _ = fs::remove_file(home_dir().join("pgrust.pid"));
            return Err(format!(
                "pgrust exited during startup ({status}); inspect {}",
                log.display()
            ));
        }
        match query(&settings, "SELECT version()", "postgres") {
            Ok(version) => {
                if !version.to_lowercase().contains("pgrust") {
                    return Err("Connected database does not identify as pgrust".to_string());
                }
                break;
            }
            Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(250)),
            Err(e) => {
                if Instant::now() >= deadline {
                    return Err(format!(
                        "Timed out waiting for pgrust; inspect {}",
                        log.display()
                    ));
                }
                let _ = e;
                std::thread::sleep(Duration::from_millis(250));
            }
        }
    }
    let exists = query(
        &settings,
        &format!(
            "SELECT 1 FROM pg_database WHERE datname = '{}'",
            settings.database
        ),
        "postgres",
    )?;
    if exists != "1" {
        query(
            &settings,
            &format!("CREATE DATABASE \"{}\"", settings.database),
            "postgres",
        )?;
    }
    println!("pgrust running on 127.0.0.1:{} (pid {pid})", settings.port);
    println!("Next: cargo run -p cloud-api -- migrate && cargo run -p cloud-api");
    Ok(())
}

fn down() -> Result<(), String> {
    let Some(pid) = verified_pid()? else {
        println!("pgrust is not running (managed process)");
        return Ok(());
    };
    let status = Command::new("kill")
        .args(["-INT", &pid.to_string()])
        .status()
        .map_err(|e| format!("failed to signal pgrust: {e}"))?;
    if !status.success() {
        return Err("failed to signal pgrust".to_string());
    }
    let deadline = Instant::now() + Duration::from_secs(20);
    while pid_alive(pid) {
        if Instant::now() >= deadline {
            return Err(
                "Server did not stop within 20 seconds; inspect its log and PID".to_string(),
            );
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    let _ = fs::remove_file(home_dir().join("pgrust.pid"));
    println!("pgrust stopped; data retained");
    Ok(())
}

fn status() -> Result<(), String> {
    match verified_pid()? {
        Some(pid) => println!("managed pgrust: running (pid {pid})"),
        None => println!("managed pgrust: stopped"),
    }
    println!("data: {}", data_dir().display());
    Ok(())
}

fn usage() -> &'static str {
    "usage: pgrust-local <install|up|down|status>"
}

fn main() {
    let command = env::args().nth(1);
    let result = match command.as_deref() {
        Some("install") => install(),
        Some("up") => up(),
        Some("down") => down(),
        Some("status") => status(),
        _ => {
            eprintln!("{}", usage());
            std::process::exit(2);
        }
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn database_url_valid() {
        let settings =
            parse_database_url("postgres://cloud:local-only-password@localhost:5433/cloud")
                .unwrap();
        assert_eq!(
            settings,
            DbSettings {
                user: "cloud".into(),
                password: "local-only-password".into(),
                database: "cloud".into(),
                port: 5433,
            }
        );
    }

    #[test]
    fn database_url_default_port() {
        let settings = parse_database_url("postgres://cloud:pw@127.0.0.1/cloud").unwrap();
        assert_eq!(settings.port, 5432);
    }

    #[test]
    fn database_url_rejects_remote() {
        assert!(
            parse_database_url("postgres://cloud:pass@example.com/cloud")
                .unwrap_err()
                .contains("localhost")
        );
    }

    #[test]
    fn database_url_rejects_injected_identifier() {
        assert!(
            parse_database_url("postgres://cloud:pass@localhost/cloud%22%3Bdrop")
                .unwrap_err()
                .contains("identifiers")
        );
    }

    #[test]
    fn database_url_rejects_missing_password() {
        assert!(parse_database_url("postgres://cloud@localhost/cloud").is_err());
    }
}
