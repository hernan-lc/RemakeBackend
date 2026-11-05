//! Process-level checks for the deployment entry points: Railway configuration
//! validation, the one-shot migrate command, and the HTTP server.
//!
//! Tests marked `#[ignore]` need `CLOUD_TEST_DATABASE_URL` pointing at a
//! disposable PostgreSQL database and run with `-- --ignored --test-threads=1`.
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    process::{Child, Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

const BIN: &str = env!("CARGO_BIN_EXE_cloud-api");
const AUTH_SECRET: &str = "0123456789abcdef0123456789abcdef";

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

// Start from an empty environment so a developer's or CI's own RAILWAY_* or
// PORT variables cannot change what these tests observe.
fn command(database_url: &str) -> Command {
    let mut command = Command::new(BIN);
    command
        .env_clear()
        .env("DATABASE_URL", database_url)
        .env("AUTH_SECRET", AUTH_SECRET)
        .env("APP_ORIGIN", "http://127.0.0.1:3000")
        .env("RUST_LOG", "cloud_api=info");
    command
}

fn database_url() -> String {
    std::env::var("CLOUD_TEST_DATABASE_URL")
        .expect("set CLOUD_TEST_DATABASE_URL to a disposable PostgreSQL database")
}

// Combined stdout and stderr with ANSI color codes removed: tracing colors its
// output even when it is captured through a pipe, which would split field names.
fn logs(output: &Output) -> String {
    let raw = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    strip_ansi(&raw)
}

fn strip_ansi(text: &str) -> String {
    let mut plain = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(character) = chars.next() {
        if character == '\u{1b}' {
            // Control sequence: ESC '[' parameters, terminated by an ASCII letter.
            for next in chars.by_ref() {
                if next.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            plain.push(character);
        }
    }
    plain
}

fn migrate(database_url: &str) -> Output {
    command(database_url)
        .arg("migrate")
        .env("BIND_ADDR", format!("127.0.0.1:{}", free_port()))
        .output()
        .unwrap()
}

/// Minimal HTTP/1.1 GET returning the response status code.
fn http_status(port: u16, path: &str) -> Option<u16> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
    )
    .ok()?;
    let mut response = String::new();
    stream.read_to_string(&mut response).ok()?;
    response.split_whitespace().nth(1)?.parse().ok()
}

struct KillOnDrop(Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn railway_configuration_is_rejected_before_connecting() {
    // The database host below is unreachable on purpose: validation must fail first.
    let output = command("postgres://deploy-user@127.0.0.1:1/unused")
        .env("RAILWAY_ENVIRONMENT_NAME", "production")
        .env("PORT", "8080")
        .env("BIND_ADDR", "127.0.0.1:8080")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let text = logs(&output);
    assert!(text.contains("loopback"), "{text}");
    assert!(text.contains("PORT"), "error should name the fix: {text}");
    assert!(!text.contains("deploy-user"), "DATABASE_URL leaked: {text}");
    assert!(!text.contains(AUTH_SECRET), "AUTH_SECRET leaked: {text}");
}

#[test]
#[ignore = "needs CLOUD_TEST_DATABASE_URL pointing at a disposable PostgreSQL database"]
fn migrate_mode_applies_schema_and_exits_without_listening() {
    let url = database_url();
    let port = free_port();
    let output = command(&url)
        .arg("migrate")
        .env("BIND_ADDR", format!("127.0.0.1:{port}"))
        .output()
        .unwrap();
    let text = logs(&output);
    assert!(output.status.success(), "{text}");
    assert!(text.contains("startup mode"), "{text}");
    assert!(text.contains("mode=\"migrate\""), "{text}");
    assert!(
        text.contains("migrations applied; exiting without starting the HTTP server"),
        "{text}"
    );
    assert!(!text.contains("HTTP listener bound"), "{text}");
    // The port must be free after the one-shot process exits.
    TcpListener::bind(("127.0.0.1", port)).expect("migrate mode must not hold the listener");
}

#[test]
#[ignore = "needs CLOUD_TEST_DATABASE_URL pointing at a disposable PostgreSQL database"]
fn server_mode_listens_and_reports_ready_after_migrations() {
    let url = database_url();
    let migrated = migrate(&url);
    assert!(migrated.status.success(), "{}", logs(&migrated));

    let port = free_port();
    let child = command(&url)
        .env("BIND_ADDR", format!("127.0.0.1:{port}"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let _server = KillOnDrop(child);

    let deadline = Instant::now() + Duration::from_secs(60);
    while http_status(port, "/health/live") != Some(200) {
        assert!(Instant::now() < deadline, "HTTP listener never became live");
        thread::sleep(Duration::from_millis(200));
    }
    assert_eq!(http_status(port, "/health/ready"), Some(200));
}
