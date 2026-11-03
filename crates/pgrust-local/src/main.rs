//! Optional, Docker-free pgrust launcher for local RemakeBackend development.
//!
//! Std-only Rust port of the closed PR's `scripts/pgrust-local.py`: no Python,
//! no Docker. Downloads the official pgrust v0.4-beta tarball with SHA-256
//! verification, keeps an isolated cluster under
//! `~/.local/share/remakebackend/pgrust` (or `$PGRUST_LOCAL_HOME`), and starts
//! it on 127.0.0.1. Migrations stay explicit (`cargo run -p cloud-api -- migrate`).

use std::env;
use std::fs::{self, OpenOptions};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const VERSION: &str = "0.4-beta";
const MAX_ARCHIVE_BYTES: u64 = 2 * 1024 * 1024 * 1024;

fn base_url() -> String {
    format!("https://pgrust.com/downloads/v{VERSION}")
}

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

/// Map `uname`-style system/machine names to a pgrust release triple.
fn platform_name(system: &str, machine: &str) -> Result<&'static str, String> {
    let system = system.to_lowercase();
    let machine = machine.to_lowercase();
    if system != "linux" && system != "darwin" {
        return Err(
            "pgrust v0.4-beta has no native Windows release; use WSL2 for local development"
                .to_string(),
        );
    }
    let arch = match machine.as_str() {
        "x86_64" | "amd64" => "x86_64",
        "aarch64" | "arm64" => "aarch64",
        _ => return Err(format!("Unsupported pgrust architecture: {machine}")),
    };
    if system == "darwin" {
        return Ok(if arch == "aarch64" {
            "macos-arm64"
        } else {
            "macos-x86_64"
        });
    }
    Ok(if arch == "aarch64" {
        "linux-aarch64"
    } else {
        "linux-x86_64"
    })
}

fn host_platform() -> Result<&'static str, String> {
    let system = env::consts::OS;
    let machine = env::consts::ARCH;
    // env::consts uses "macos"/"linux" and "x86_64"/"aarch64", matching platform_name.
    platform_name(system, machine)
}

fn release_dir() -> Result<PathBuf, String> {
    Ok(home_dir()
        .join("releases")
        .join(format!("pgrust-{VERSION}-{}", host_platform()?)))
}

fn bin_dir() -> Result<PathBuf, String> {
    let dir = release_dir()?.join("bin");
    for name in ["postgres", "initdb", "psql"] {
        if !dir.join(name).is_file() {
            return Err(format!(
                "Missing {}; run `cargo run -p pgrust-local -- install` first",
                dir.join(name).display()
            ));
        }
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

/// Reject unsafe tar member names before extraction.
fn validate_entry_name(name: &str, expected_root: &str) -> Result<(), String> {
    if name.is_empty() || name.starts_with('/') {
        return Err("pgrust archive contains an unsafe path".to_string());
    }
    let parts: Vec<&str> = name.split('/').collect();
    if parts.contains(&"..") {
        return Err("pgrust archive contains an unsafe path".to_string());
    }
    if parts.first() != Some(&expected_root) {
        return Err("pgrust archive contains an unsafe path".to_string());
    }
    Ok(())
}

fn run(cmd: &mut Command) -> Result<(), String> {
    let status = cmd
        .status()
        .map_err(|e| format!("failed to run command: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("command failed with status {status}"))
    }
}

fn download(url: &str, dest: &Path) -> Result<(), String> {
    let mut cmd = Command::new("curl");
    cmd.args(["-fsSL", "--max-time", "60", "-o"])
        .arg(dest)
        .arg(url);
    run(&mut cmd).map_err(|e| format!("download failed for {url}: {e}"))
}

fn is_hex64(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| b.is_ascii_hexdigit())
}

fn sha256_of_file(path: &Path) -> Result<String, String> {
    // Prefer coreutils; fall back to BSD shasum.
    for (program, args) in [
        ("sha256sum", vec![] as Vec<&str>),
        ("shasum", vec!["-a", "256"]),
    ] {
        let output = Command::new(program).args(&args).arg(path).output();
        if let Ok(output) = output {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let digest = stdout
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .to_lowercase();
                if is_hex64(&digest) {
                    return Ok(digest);
                }
            }
        }
    }
    Err("could not compute SHA-256 (need sha256sum or shasum)".to_string())
}

fn tar_names(archive: &Path) -> Result<Vec<String>, String> {
    let output = Command::new("tar")
        .args(["-tzf"])
        .arg(archive)
        .output()
        .map_err(|e| format!("failed to list archive: {e}"))?;
    if !output.status.success() {
        return Err("failed to list pgrust archive".to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| line.trim_start_matches("./").to_string())
        .collect())
}

/// Walk an extracted tree, rejecting symlinks and enforcing the size cap.
fn check_tree(root: &Path) -> Result<(), String> {
    let mut total: u64 = 0;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries =
            fs::read_dir(&dir).map_err(|e| format!("failed to read {}: {e}", dir.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("failed to read entry: {e}"))?;
            let meta = fs::symlink_metadata(entry.path())
                .map_err(|e| format!("failed to stat entry: {e}"))?;
            if meta.file_type().is_symlink() {
                return Err("pgrust archive contains a link or special file".to_string());
            }
            if meta.is_dir() {
                stack.push(entry.path());
            } else if meta.is_file() {
                total = total.checked_add(meta.len()).ok_or_else(|| {
                    "pgrust archive exceeds the extraction size limit".to_string()
                })?;
                if total > MAX_ARCHIVE_BYTES {
                    return Err("pgrust archive exceeds the extraction size limit".to_string());
                }
            } else {
                return Err("pgrust archive contains a link or special file".to_string());
            }
        }
    }
    Ok(())
}

fn install() -> Result<(), String> {
    let plat = host_platform()?;
    let destination = release_dir()?;
    if destination.join("bin/postgres").is_file()
        && destination.join("bin/initdb").is_file()
        && destination.join("bin/psql").is_file()
    {
        println!("Already installed: {}", destination.display());
        return Ok(());
    }
    let home = home_dir();
    fs::create_dir_all(&home).map_err(|e| format!("failed to create {}: {e}", home.display()))?;
    let name = format!("pgrust-{VERSION}-{plat}.tar.gz");
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let work = home.join(format!(".download-{}-{}", std::process::id(), nanos));
    fs::create_dir_all(&work).map_err(|e| format!("failed to create work dir: {e}"))?;
    let cleanup = |_: ()| {
        let _ = fs::remove_dir_all(&work);
    };
    let archive_path = work.join(&name);
    let checksum_path = work.join(format!("{name}.sha256"));
    println!("Downloading official {name} and checksum...");
    let base = base_url();
    if let Err(e) = download(&format!("{base}/{name}"), &archive_path)
        .and_then(|_| download(&format!("{base}/{name}.sha256"), &checksum_path))
    {
        cleanup(());
        return Err(e);
    }
    let sidecar = fs::read_to_string(&checksum_path)
        .map_err(|e| format!("failed to read checksum sidecar: {e}"))?;
    let expected = sidecar
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_lowercase();
    if !is_hex64(&expected) {
        cleanup(());
        return Err("Invalid upstream SHA-256 sidecar".to_string());
    }
    let actual = sha256_of_file(&archive_path)?;
    if actual != expected {
        cleanup(());
        return Err("pgrust archive checksum mismatch; refusing extraction".to_string());
    }
    let expected_root = format!("pgrust-{VERSION}-{plat}");
    for member in tar_names(&archive_path)? {
        validate_entry_name(&member, &expected_root)?;
    }
    let mut extract = Command::new("tar");
    extract
        .args(["-xzf"])
        .arg(&archive_path)
        .arg("-C")
        .arg(&work);
    if run(&mut extract).is_err() {
        cleanup(());
        return Err("failed to extract pgrust archive".to_string());
    }
    let extracted = work.join(&expected_root);
    for item in ["bin/postgres", "bin/initdb", "bin/psql"] {
        if !extracted.join(item).is_file() {
            cleanup(());
            return Err(format!("Upstream archive is missing {item}"));
        }
    }
    check_tree(&extracted)?;
    if destination.exists() {
        cleanup(());
        return Err(format!(
            "Incomplete installation found at {}; inspect it manually",
            destination.display()
        ));
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("failed to create releases dir: {e}"))?;
    }
    fs::rename(&extracted, &destination).map_err(|e| format!("failed to install release: {e}"))?;
    cleanup(());
    println!("Installed {}", destination.display());
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
    let output = Command::new(bin_dir()?.join("psql"))
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
        if let Ok(cmdline) = fs::read(format!("/proc/{pid}/cmdline")) {
            if !cmdline.is_empty() {
                let joined = String::from_utf8_lossy(&cmdline);
                return joined.contains(expected_binary);
            }
            return false;
        }
    }
    // Fallback (macOS, or /proc unavailable): ask ps.
    Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "command="])
        .output()
        .map(|output| {
            output.status.success()
                && String::from_utf8_lossy(&output.stdout).contains(expected_binary)
        })
        .unwrap_or(false)
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
    let child = Command::new(bin_dir()?.join("postgres"))
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
    // Detach: forget the handle so the server outlives this command; the pid
    // file plus binary-path verification owns lifecycle from here.
    std::mem::forget(child);
    fs::write(home_dir().join("pgrust.pid"), pid.to_string())
        .map_err(|e| format!("failed to write pidfile: {e}"))?;
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if !pid_alive(pid) {
            let _ = fs::remove_file(home_dir().join("pgrust.pid"));
            return Err(format!(
                "pgrust exited during startup; inspect {}",
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
    fn platform_linux_x64() {
        assert_eq!(platform_name("Linux", "x86_64").unwrap(), "linux-x86_64");
    }

    #[test]
    fn platform_macos_arm() {
        assert_eq!(platform_name("Darwin", "arm64").unwrap(), "macos-arm64");
    }

    #[test]
    fn platform_windows_rejected() {
        assert!(platform_name("Windows", "x86_64")
            .unwrap_err()
            .contains("WSL2"));
    }

    #[test]
    fn platform_arch_rejected() {
        assert!(platform_name("Linux", "mips").is_err());
    }

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

    #[test]
    fn archive_rejects_traversal() {
        assert!(validate_entry_name(
            "pgrust-0.4-beta-linux-x86_64/../../oops",
            "pgrust-0.4-beta-linux-x86_64"
        )
        .unwrap_err()
        .contains("unsafe path"));
    }

    #[test]
    fn archive_rejects_absolute_and_wrong_root() {
        assert!(validate_entry_name("/etc/passwd", "pgrust-0.4-beta-linux-x86_64").is_err());
        assert!(
            validate_entry_name("other-root/bin/postgres", "pgrust-0.4-beta-linux-x86_64").is_err()
        );
        assert!(validate_entry_name(
            "pgrust-0.4-beta-linux-x86_64/bin/postgres",
            "pgrust-0.4-beta-linux-x86_64"
        )
        .is_ok());
    }

    #[test]
    fn archive_rejects_symlinks_on_disk() {
        // Exercise the real extraction guard: symlinks are rejected, regular
        // files and directories pass.
        let base = env::temp_dir().join(format!(
            "pgrust-local-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(base.join("root/sub")).unwrap();
        fs::write(base.join("root/sub/file"), b"data").unwrap();
        assert!(check_tree(&base.join("root")).is_ok());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("target", base.join("root/sub/link")).unwrap();
            assert!(check_tree(&base.join("root"))
                .unwrap_err()
                .contains("link or special"));
        }
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn checksum_sidecar_shape() {
        assert!(is_hex64(&"a".repeat(64)));
        assert!(!is_hex64("xyz"));
    }
}
