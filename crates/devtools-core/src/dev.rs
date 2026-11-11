//! Development mode: reuse pgrust, run migrations, run the backend, restart it when its code
//! changes. pgrust is never rebuilt or restarted by a code change.
//!
//! This replaces `scripts/dev-admin.sh`. Unlike that script it does not export `RUSTC_WRAPPER`
//! or `RUSTFLAGS`: the build runs in the caller's environment exactly as it is.

use crate::build;
use crate::config::DevEnvironment;
use crate::database::{database_url, DatabaseManager};
use anyhow::{bail, Context, Result};
use notify::{EventKind, RecursiveMode, Watcher};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::process::{Child, Command};

#[derive(Debug, Clone, Default)]
pub struct DevOptions {
    /// Also start the admin UI (Vite) on 127.0.0.1:5173, as `dev-admin.sh` did.
    pub ui: bool,
    /// Override `development.watch` from devtools.toml.
    pub watch: Option<bool>,
    /// Use whatever answers at `DATABASE_URL` (for example the Docker Compose database) instead
    /// of starting the local pgrust.
    pub no_db: bool,
    /// Do not apply migrations before starting the backend.
    pub no_migrate: bool,
}

const IGNORED_DIRS: [&str; 7] = [
    "target",
    ".devtools",
    ".git",
    "node_modules",
    "dist",
    ".local-dev",
    "vendor",
];
/// Workspace members that are not the backend: their changes must not restart it.
const NOT_BACKEND: [&str; 3] = ["devtools", "devtools-core", "pgrust-local"];

/// Is a change at `path` one that should rebuild and restart the backend?
pub fn is_relevant(path: &Path, root: &Path) -> bool {
    let rel = path.strip_prefix(root).unwrap_or(path);
    let parts: Vec<_> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    if parts.iter().any(|p| IGNORED_DIRS.contains(&p.as_str())) {
        return false;
    }
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().into_owned())
        .unwrap_or_default();
    match parts.first().map(String::as_str) {
        Some("crates") => {
            let member_ok = parts
                .get(1)
                .is_some_and(|m| !NOT_BACKEND.contains(&m.as_str()));
            member_ok && matches!(ext.as_str(), "rs" | "toml" | "sql" | "json")
        }
        Some("migrations") => ext == "sql",
        Some("Cargo.lock") | Some("Cargo.toml") => parts.len() == 1,
        _ => false,
    }
}

/// `.local-dev/auth-secret`, created once (0600) from the OS random source.
pub fn ensure_auth_secret(root: &Path) -> Result<String> {
    if let Ok(secret) = std::env::var("AUTH_SECRET") {
        if !secret.trim().is_empty() {
            return Ok(secret);
        }
    }
    let dir = root.join(".local-dev");
    let file = dir.join("auth-secret");
    if let Ok(existing) = fs::read_to_string(&file) {
        if !existing.trim().is_empty() {
            return Ok(existing.trim().to_string());
        }
    }
    fs::create_dir_all(&dir)?;
    let mut bytes = [0u8; 32];
    {
        use std::io::Read;
        fs::File::open("/dev/urandom")
            .context("cannot open /dev/urandom to generate AUTH_SECRET")?
            .read_exact(&mut bytes)?;
    }
    let secret = hex::encode(bytes);
    fs::write(&file, &secret)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&file, fs::Permissions::from_mode(0o600))?;
    }
    Ok(secret)
}

/// The variables the backend needs in development. Anything the caller already exported wins.
/// The build variables in `process::FORBIDDEN_ENV` are never among them.
pub fn backend_env(
    env: &DevEnvironment,
    auth_secret: &str,
    ui: bool,
    existing: &dyn Fn(&str) -> Option<String>,
) -> Vec<(String, String)> {
    let wanted = [
        ("DATABASE_URL", database_url(&env.conn, &env.conn.database)),
        ("AUTH_SECRET", auth_secret.to_string()),
        ("BIND_ADDR", "127.0.0.1:3000".to_string()),
        (
            "APP_ORIGIN",
            if ui {
                "http://127.0.0.1:5173"
            } else {
                "http://127.0.0.1:3000"
            }
            .to_string(),
        ),
        ("CLOUD_API_URL", "http://127.0.0.1:3000".to_string()),
    ];
    wanted
        .into_iter()
        .filter(|(key, _)| existing(key).is_none_or(|v| v.trim().is_empty()))
        .map(|(k, v)| (k.to_string(), v))
        .collect()
}

fn spawn_backend(exe: &Path, vars: &[(String, String)], root: &Path) -> Result<Child> {
    let mut cmd = Command::new(exe);
    cmd.current_dir(root).kill_on_drop(true);
    for (k, v) in vars {
        cmd.env(k, v);
    }
    cmd.spawn()
        .with_context(|| format!("failed to start {}", exe.display()))
}

fn migrate(exe: &Path, vars: &[(String, String)], root: &Path) -> Result<()> {
    let mut cmd = std::process::Command::new(exe);
    cmd.current_dir(root).arg("migrate");
    for (k, v) in vars {
        cmd.env(k, v);
    }
    let status = cmd.status().context("failed to run the migrations")?;
    if !status.success() {
        bail!("`cloud-api migrate` failed ({status})");
    }
    Ok(())
}

fn spawn_ui(root: &Path) -> Result<Child> {
    let admin = root.join("web/admin");
    let vite = admin.join("node_modules/vite/bin/vite.js");
    if !vite.is_file() {
        bail!("install the admin UI dependencies first: cd web/admin && npm ci");
    }
    Command::new("node")
        .current_dir(&admin)
        .kill_on_drop(true)
        .arg(&vite)
        .args(["--host", "127.0.0.1", "--port", "5173", "--strictPort"])
        .spawn()
        .context("failed to start the admin UI (is node installed?)")
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(f)
        .await
        .context("build task panicked")?
}

pub async fn run(env: &DevEnvironment, opts: &DevOptions) -> Result<()> {
    // 1. pgrust: reuse the running instance or start the installed binary. Never builds.
    env.require_credentials()?;
    if opts.no_db {
        println!("--no-db: using the database at DATABASE_URL as is");
    } else {
        let db = DatabaseManager::new(env.clone());
        let up = db.up().await?;
        println!(
            "pgrust {} on 127.0.0.1:{} (pid {})",
            if up.reused { "reused" } else { "started" },
            up.status.port,
            up.status.pid.unwrap_or(0)
        );
    }

    // 2. Backend environment, built without touching the caller's build variables.
    let secret = ensure_auth_secret(&env.root)?;
    let vars = backend_env(env, &secret, opts.ui, &|k| std::env::var(k).ok());

    // 3. Build once, migrate, start.
    let root = env.root.clone();
    let mut exe = {
        let env = env.clone();
        blocking(move || build::build_executable(&env, "cloud-api")).await?
    };
    if !opts.no_migrate {
        migrate(&exe, &vars, &root)?;
    }
    let mut server = Some(spawn_backend(&exe, &vars, &root)?);
    let mut ui = if opts.ui {
        Some(spawn_ui(&root)?)
    } else {
        None
    };
    println!("backend running; the database stays up when you stop this command");

    let watch = opts.watch.unwrap_or(env.config.development.watch);
    let restart = env.config.development.restart_on_change;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<PathBuf>();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(event) = res {
            if matches!(
                event.kind,
                EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
            ) {
                for path in event.paths {
                    let _ = tx.send(path);
                }
            }
        }
    })?;
    if watch {
        for dir in ["crates", "migrations"] {
            if root.join(dir).is_dir() {
                watcher.watch(&root.join(dir), RecursiveMode::Recursive)?;
            }
        }
        watcher
            .watch(&root.join("Cargo.lock"), RecursiveMode::NonRecursive)
            .ok();
        println!(
            "watching for backend changes (debounce {} ms)",
            env.config.development.debounce_ms
        );
    }
    let debounce = Duration::from_millis(env.config.development.debounce_ms);

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            changed = rx.recv(), if watch => {
                let Some(path) = changed else { break };
                if !is_relevant(&path, &root) { continue; }
                // Debounce: absorb the burst of events an editor save produces.
                while let Ok(Some(_)) = tokio::time::timeout(debounce, rx.recv()).await {}
                if !restart {
                    println!("backend changed; restart_on_change is off");
                    continue;
                }
                println!("change in {}: rebuilding the backend", path.strip_prefix(&root).unwrap_or(&path).display());
                let env2 = env.clone();
                match blocking(move || build::build_executable(&env2, "cloud-api")).await {
                    Ok(new_exe) => {
                        if let Some(mut old) = server.take() {
                            let _ = old.kill().await;
                        }
                        exe = new_exe;
                        let migrated = if opts.no_migrate { Ok(()) } else { migrate(&exe, &vars, &root) };
                        match migrated.and_then(|_| spawn_backend(&exe, &vars, &root)) {
                            Ok(child) => server = Some(child),
                            Err(e) => eprintln!("backend not restarted: {e:#}"),
                        }
                    }
                    Err(e) => eprintln!("build failed, keeping the previous backend: {e:#}"),
                }
            }
            status = async {
                match server.as_mut() {
                    Some(child) => child.wait().await,
                    None => std::future::pending().await,
                }
            } => {
                eprintln!("backend exited ({}); waiting for a change to restart it", status?);
                server = None;
            }
        }
    }
    if let Some(mut child) = server.take() {
        let _ = child.kill().await;
    }
    if let Some(mut child) = ui.take() {
        let _ = child.kill().await;
    }
    println!("stopped; pgrust is still running");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{parse_database_url, DevConfig};
    use crate::process::FORBIDDEN_ENV;

    fn env() -> DevEnvironment {
        DevEnvironment {
            root: PathBuf::from("/repo"),
            config: DevConfig::default(),
            conn: parse_database_url("postgres://cloud:pw@127.0.0.1:55432/cloud").unwrap(),
            conn_source: "test".into(),
        }
    }

    #[test]
    fn only_backend_changes_restart_the_backend() {
        let root = Path::new("/repo");
        assert!(is_relevant(
            Path::new("/repo/crates/cloud-api/src/main.rs"),
            root
        ));
        assert!(is_relevant(
            Path::new("/repo/crates/cloud-storage/Cargo.toml"),
            root
        ));
        assert!(is_relevant(
            Path::new("/repo/migrations/20261102000001_publishers.sql"),
            root
        ));
        assert!(is_relevant(Path::new("/repo/Cargo.lock"), root));
        // Not the backend, or noise.
        assert!(!is_relevant(
            Path::new("/repo/crates/devtools/src/main.rs"),
            root
        ));
        assert!(!is_relevant(
            Path::new("/repo/crates/devtools-core/src/dev.rs"),
            root
        ));
        assert!(!is_relevant(Path::new("/repo/target/debug/foo.rs"), root));
        assert!(!is_relevant(
            Path::new("/repo/.devtools/logs/pgrust.log"),
            root
        ));
        assert!(!is_relevant(Path::new("/repo/.git/HEAD"), root));
        assert!(!is_relevant(Path::new("/repo/web/admin/src/app.ts"), root));
        assert!(!is_relevant(
            Path::new("/repo/vendor/pgrust/crates/x/src/lib.rs"),
            root
        ));
        assert!(!is_relevant(
            Path::new("/repo/crates/cloud-api/src/notes.md"),
            root
        ));
    }

    #[test]
    fn backend_env_never_carries_build_variables_and_respects_the_callers() {
        let preset = |k: &str| (k == "BIND_ADDR").then(|| "0.0.0.0:9".to_string());
        let vars = backend_env(&env(), "secret", true, &preset);
        for (key, _) in &vars {
            assert!(!FORBIDDEN_ENV.contains(&key.as_str()), "{key}");
        }
        assert!(
            !vars.iter().any(|(k, _)| k == "BIND_ADDR"),
            "caller's value must win"
        );
        let url = &vars.iter().find(|(k, _)| k == "DATABASE_URL").unwrap().1;
        assert_eq!(url, "postgres://cloud:pw@127.0.0.1:55432/cloud");
        assert_eq!(
            vars.iter().find(|(k, _)| k == "APP_ORIGIN").unwrap().1,
            "http://127.0.0.1:5173"
        );
    }
}
