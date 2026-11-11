//! ProcessManager: spawn, identify and stop processes without a shell in between.
//!
//! The pgrust server needs a larger stack than the usual 8 MiB. The old launcher got it from
//! `sh -c "ulimit -s 65520; exec ..."`; here the limit is applied in the child, between fork
//! and exec, so no shell is involved and the CLI's own limits are untouched.

use anyhow::{bail, Context, Result};
use std::fs;
use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Variables this tool must never set or clear on the user's behalf. They belong to the user
/// (and to mold/kache/sccache); changing them invalidates Cargo's fingerprints.
pub const FORBIDDEN_ENV: [&str; 4] = [
    "RUSTFLAGS",
    "RUSTC_WRAPPER",
    "CARGO_TARGET_DIR",
    "CARGO_BUILD_RUSTFLAGS",
];

/// A `cargo` command that inherits the caller's environment exactly.
pub fn cargo_command(args: &[&str]) -> Command {
    let mut cmd = Command::new("cargo");
    cmd.args(args);
    cmd
}

/// True if the command would set or remove any variable in [`FORBIDDEN_ENV`].
pub fn touches_forbidden_env(cmd: &Command) -> bool {
    cmd.get_envs()
        .any(|(key, _)| FORBIDDEN_ENV.iter().any(|f| key.to_string_lossy() == *f))
}

pub fn port_in_use(host: &str, port: u16) -> bool {
    let Ok(addr) = format!("{host}:{port}").parse::<SocketAddr>() else {
        return false;
    };
    TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_ok()
}

pub fn read_pid(path: &Path) -> Option<u32> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// Alive and not a zombie.
pub fn pid_alive(pid: u32) -> bool {
    #[cfg(target_os = "linux")]
    {
        match fs::read_to_string(format!("/proc/{pid}/stat")) {
            // "pid (comm) S ..." - the state follows the last ')'.
            Ok(stat) => {
                let state = stat
                    .rsplit(')')
                    .next()
                    .and_then(|s| s.trim().chars().next());
                state != Some('Z') && state != Some('X')
            }
            Err(_) => false,
        }
    }
    #[cfg(all(unix, not(target_os = "linux")))]
    {
        // SAFETY: signal 0 only checks that the process exists.
        let rc = unsafe { libc::kill(pid as i32, 0) };
        rc == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

/// Does `pid` run exactly this executable? Used before trusting or signalling a pid file, so a
/// recycled pid belonging to someone else is never touched.
pub fn pid_belongs_to(pid: u32, expected: &Path) -> bool {
    let Ok(expected) = fs::canonicalize(expected) else {
        return false;
    };
    #[cfg(target_os = "linux")]
    {
        fs::read_link(format!("/proc/{pid}/exe"))
            .map(|actual| actual == expected)
            .unwrap_or(false)
    }
    #[cfg(not(target_os = "linux"))]
    {
        Command::new("ps")
            .args(["-p", &pid.to_string(), "-o", "comm="])
            .output()
            .map(|o| {
                o.status.success()
                    && fs::canonicalize(String::from_utf8_lossy(&o.stdout).trim())
                        .map(|p| p == expected)
                        .unwrap_or(false)
            })
            .unwrap_or(false)
    }
}

/// Spawn the server detached from this process (own session, so Ctrl-C on the CLI does not
/// reach it) with the stack soft limit raised to `stack_kib` KiB in the child only.
#[cfg(unix)]
pub fn spawn_server(
    binary: &Path,
    args: &[String],
    envs: &[(&str, String)],
    log: fs::File,
    stack_kib: u64,
) -> Result<Child> {
    use std::os::unix::process::CommandExt;
    let log_err = log.try_clone().context("cannot duplicate the log handle")?;
    let mut cmd = Command::new(binary);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(log)
        .stderr(log_err);
    for (key, value) in envs {
        cmd.env(key, value);
    }
    let want = stack_kib * 1024;
    // SAFETY: only async-signal-safe calls (setsid, getrlimit, setrlimit) run before exec.
    unsafe {
        cmd.pre_exec(move || {
            libc::setsid();
            let mut limit = libc::rlimit {
                rlim_cur: 0,
                rlim_max: 0,
            };
            if libc::getrlimit(libc::RLIMIT_STACK, &mut limit) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            if limit.rlim_cur != libc::RLIM_INFINITY && limit.rlim_cur < want {
                if limit.rlim_max != libc::RLIM_INFINITY && limit.rlim_max < want {
                    return Err(std::io::Error::other(
                        "the hard stack limit is below 65520 KiB; raise it (ulimit -Hs) so pgrust can run",
                    ));
                }
                limit.rlim_cur = want;
                if libc::setrlimit(libc::RLIMIT_STACK, &limit) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
    cmd.spawn()
        .with_context(|| format!("failed to start {}", binary.display()))
}

#[cfg(not(unix))]
pub fn spawn_server(
    _binary: &Path,
    _args: &[String],
    _envs: &[(&str, String)],
    _log: fs::File,
    _stack_kib: u64,
) -> Result<Child> {
    bail!(
        "managing a local pgrust server is only supported on Linux and macOS (use WSL2 on Windows)"
    )
}

/// Ask a verified process to shut down (SIGINT is PostgreSQL's fast shutdown) and wait for it.
#[cfg(unix)]
pub fn stop_pid(pid: u32, timeout: Duration) -> Result<()> {
    // SAFETY: plain signal to a pid the caller verified against the installed executable.
    if unsafe { libc::kill(pid as i32, libc::SIGINT) } != 0 {
        bail!(
            "failed to signal pid {pid}: {}",
            std::io::Error::last_os_error()
        );
    }
    let deadline = Instant::now() + timeout;
    loop {
        // Reap it if it is our own child, so it does not linger as a zombie.
        // SAFETY: WNOHANG waitpid on a pid is harmless when it is not our child.
        unsafe {
            let mut status = 0;
            libc::waitpid(pid as i32, &mut status, libc::WNOHANG);
        }
        if !pid_alive(pid) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            bail!(
                "pid {pid} did not stop within {}s; inspect the log. It was not killed.",
                timeout.as_secs()
            );
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

#[cfg(not(unix))]
pub fn stop_pid(_pid: u32, _timeout: Duration) -> Result<()> {
    bail!("managing a local pgrust server is only supported on Linux and macOS")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cargo_commands_never_touch_the_users_build_environment() {
        let cmd = cargo_command(&["build", "-p", "cloud-api"]);
        assert!(!touches_forbidden_env(&cmd));
        let mut tampered = cargo_command(&["build"]);
        tampered.env("RUSTFLAGS", "-C link-arg=-fuse-ld=mold");
        assert!(touches_forbidden_env(&tampered));
        let mut cleared = cargo_command(&["build"]);
        cleared.env_remove("RUSTC_WRAPPER");
        assert!(touches_forbidden_env(&cleared));
    }

    #[test]
    fn a_listening_port_is_reported_busy_and_a_free_one_is_not() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(port_in_use("127.0.0.1", port));
        drop(listener);
        assert!(!port_in_use("127.0.0.1", port));
    }

    #[test]
    fn our_own_pid_belongs_to_the_test_binary_and_not_to_something_else() {
        let me = std::process::id();
        let exe = std::env::current_exe().unwrap();
        assert!(pid_alive(me));
        assert!(pid_belongs_to(me, &exe));
        assert!(!pid_belongs_to(me, Path::new("/bin/sh")));
        assert!(!pid_belongs_to(u32::MAX - 1, &exe));
    }
}
