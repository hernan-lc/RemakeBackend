//! `clean` removes only what the tools themselves regenerate. It has no switch that deletes the
//! installed pgrust, a data directory or any Cargo cache: those are removed by hand, on purpose.

use crate::config::DevEnvironment;
use anyhow::Result;
use serde::Serialize;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize)]
pub struct CleanOutcome {
    pub removed: Vec<PathBuf>,
    pub kept: Vec<PathBuf>,
}

pub fn clean(env: &DevEnvironment) -> Result<CleanOutcome> {
    let mut removed = Vec::new();
    // Generated logs of earlier runs, reports and the recorded test/build results.
    for dir in [env.reports_dir(), env.logs_dir()] {
        if let Ok(read) = fs::read_dir(&dir) {
            for entry in read.flatten() {
                let path = entry.path();
                // The running server's log is kept: it is not a temporary file.
                if path == env.log_file() {
                    continue;
                }
                if path.is_dir() {
                    fs::remove_dir_all(&path)?;
                } else {
                    fs::remove_file(&path)?;
                }
                removed.push(path);
            }
        }
    }
    if let Ok(read) = fs::read_dir(env.state_dir()) {
        for entry in read.flatten() {
            let path = entry.path();
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if name.starts_with("last_") || name.starts_with(".pw-") {
                fs::remove_file(&path)?;
                removed.push(path);
            }
        }
    }
    let kept = vec![
        env.install_dir(),
        env.data_dir(),
        env.pid_file(),
        env.log_file(),
    ];
    Ok(CleanOutcome { removed, kept })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{parse_database_url, DevConfig};

    #[test]
    fn clean_keeps_the_install_the_database_and_the_pid() {
        let dir = tempfile::tempdir().unwrap();
        let env = DevEnvironment {
            root: dir.path().to_path_buf(),
            config: DevConfig::default(),
            conn: parse_database_url("postgres://cloud:pw@127.0.0.1:5999/cloud").unwrap(),
            conn_source: "test".into(),
        };
        env.ensure_dirs().unwrap();
        fs::create_dir_all(env.install_dir()).unwrap();
        fs::write(env.install_dir().join("postgres"), b"bin").unwrap();
        fs::create_dir_all(env.data_dir()).unwrap();
        fs::write(env.data_dir().join("PG_VERSION"), b"18").unwrap();
        fs::write(env.pid_file(), b"1").unwrap();
        fs::write(env.log_file(), b"log").unwrap();
        fs::write(env.reports_dir().join("latest.json"), b"{}").unwrap();
        fs::write(env.state_dir().join("last_tests.json"), b"[]").unwrap();
        let out = clean(&env).unwrap();
        assert_eq!(out.removed.len(), 2, "{out:?}");
        assert!(env.install_dir().join("postgres").is_file());
        assert!(env.data_dir().join("PG_VERSION").is_file());
        assert!(env.pid_file().is_file() && env.log_file().is_file());
        assert!(!env.reports_dir().join("latest.json").exists());
    }
}
