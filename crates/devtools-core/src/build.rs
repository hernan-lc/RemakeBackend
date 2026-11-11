//! Backend build and check. Only backend packages are built: pgrust is never part of this
//! workspace's build, and these tools are not a dependency of `cloud-api`.

use crate::config::DevEnvironment;
use crate::process;
use crate::tests::Status;
use crate::util::now_rfc3339;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Instant;

/// The backend server and the libraries it is made of.
pub const BACKEND_PACKAGES: [&str; 6] = [
    "cloud-api",
    "cloud-contracts",
    "cloud-verifier",
    "cloud-client",
    "cloud-storage",
    "cloud-snapshot",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildRecord {
    pub kind: String,
    pub status: Status,
    pub duration_ms: u128,
    pub command: Vec<String>,
    pub exit_code: Option<i32>,
    pub at: String,
}

fn state_file(env: &DevEnvironment) -> PathBuf {
    env.state_dir().join("last_build.json")
}

pub fn last_build(env: &DevEnvironment) -> Option<BuildRecord> {
    serde_json::from_str(&fs::read_to_string(state_file(env)).ok()?).ok()
}

fn run_recorded(env: &DevEnvironment, kind: &str, args: Vec<String>) -> Result<BuildRecord> {
    let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
    let mut cmd = process::cargo_command(&borrowed);
    cmd.current_dir(&env.root);
    debug_assert!(!process::touches_forbidden_env(&cmd));
    let started = Instant::now();
    let status = cmd.status().context("failed to run cargo")?;
    let record = BuildRecord {
        kind: kind.into(),
        status: if status.success() {
            Status::Passed
        } else {
            Status::Failed
        },
        duration_ms: started.elapsed().as_millis(),
        command: std::iter::once("cargo".to_string()).chain(args).collect(),
        exit_code: status.code(),
        at: now_rfc3339(),
    };
    if env.ensure_dirs().is_ok() {
        let _ = fs::write(
            state_file(env),
            serde_json::to_string_pretty(&record)? + "\n",
        );
    }
    Ok(record)
}

/// `cargo build -p cloud-api`: the backend only.
pub fn build_backend(env: &DevEnvironment, release: bool) -> Result<BuildRecord> {
    let mut args = vec![
        "build".to_string(),
        "-p".to_string(),
        "cloud-api".to_string(),
    ];
    if release {
        args.push("--release".into());
    }
    run_recorded(env, "build", args)
}

/// Fast checks over the backend packages (and clippy when asked).
pub fn check_backend(env: &DevEnvironment, clippy: bool) -> Result<BuildRecord> {
    let mut args = vec![if clippy { "clippy" } else { "check" }.to_string()];
    for package in BACKEND_PACKAGES {
        args.push("-p".into());
        args.push(package.into());
    }
    if clippy {
        args.push("--all-targets".into());
    }
    run_recorded(env, if clippy { "clippy" } else { "check" }, args)
}

/// Build one package and return the path of its executable, read from cargo's own JSON
/// messages (so a custom target directory is honoured). Compiler diagnostics still render.
pub fn build_executable(env: &DevEnvironment, package: &str) -> Result<PathBuf> {
    let mut cmd = process::cargo_command(&[
        "build",
        "-p",
        package,
        "--message-format=json-render-diagnostics",
    ]);
    cmd.current_dir(&env.root).stdout(Stdio::piped());
    debug_assert!(!process::touches_forbidden_env(&cmd));
    let mut child = cmd.spawn().context("failed to run cargo")?;
    let mut exe = None;
    for line in BufReader::new(child.stdout.take().unwrap())
        .lines()
        .map_while(|l| l.ok())
    {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if value["reason"] == "compiler-artifact" && value["target"]["name"] == package {
            if let Some(path) = value["executable"].as_str() {
                exe = Some(PathBuf::from(path));
            }
        }
    }
    let status = child.wait()?;
    if !status.success() {
        bail!("`cargo build -p {package}` failed ({status})");
    }
    exe.with_context(|| format!("cargo built {package} but reported no executable"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_backend_set_never_contains_the_tools_or_the_old_launcher() {
        assert!(BACKEND_PACKAGES.contains(&"cloud-api"));
        for package in BACKEND_PACKAGES {
            assert!(!package.contains("devtools") && !package.contains("pgrust"));
        }
    }
}
