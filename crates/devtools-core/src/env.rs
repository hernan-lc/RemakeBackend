//! EnvironmentManager: read-only inspection of the toolchain and of everything that can
//! invalidate Cargo's cache. It reports; it never sets, clears or edits anything.

use crate::process::FORBIDDEN_ENV;
use crate::util::{command_output, which};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
pub struct Tool {
    pub name: String,
    pub found: bool,
    pub path: Option<String>,
    pub version: Option<String>,
}

/// One thing that changes how Cargo fingerprints builds.
#[derive(Debug, Clone, Serialize)]
pub struct BuildInfluence {
    /// `env`, a config file path, or a script path.
    pub source: String,
    pub name: String,
    pub value: Option<String>,
    pub note: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TargetDirInfo {
    pub path: String,
    pub exists: bool,
    pub fingerprint_dirs: Vec<(String, usize)>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EnvReport {
    pub tools: Vec<Tool>,
    pub influences: Vec<BuildInfluence>,
    pub target_dirs: Vec<TargetDirInfo>,
}

fn tool(name: &str, version_args: &[&str]) -> Tool {
    let path = which(name);
    let version = path
        .as_ref()
        .and_then(|_| command_output(name, version_args))
        .map(|v| v.lines().next().unwrap_or_default().to_string());
    Tool {
        name: name.into(),
        found: path.is_some(),
        path: path.map(|p| p.to_string_lossy().into_owned()),
        version,
    }
}

pub fn tools() -> Vec<Tool> {
    let mut list = vec![
        tool("cargo", &["--version"]),
        tool("rustc", &["--version"]),
        tool("git", &["--version"]),
        tool("node", &["--version"]),
        tool("mold", &["--version"]),
        tool("kache", &["--version"]),
        tool("sccache", &["--version"]),
    ];
    // cargo-nextest is a cargo subcommand, not necessarily a binary named by that.
    let nextest = command_output("cargo", &["nextest", "--version"]);
    list.push(Tool {
        name: "cargo-nextest".into(),
        found: nextest.is_some(),
        path: None,
        version: nextest.map(|v| v.lines().next().unwrap_or_default().to_string()),
    });
    list
}

fn cargo_config_files(root: &Path) -> Vec<PathBuf> {
    let mut files = vec![root.join(".cargo/config.toml"), root.join(".cargo/config")];
    let home = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cargo")));
    if let Some(home) = home {
        files.push(home.join("config.toml"));
        files.push(home.join("config"));
    }
    files.into_iter().filter(|p| p.is_file()).collect()
}

const CONFIG_KEYS: [&str; 5] = [
    "rustc-wrapper",
    "rustflags",
    "linker",
    "target-dir",
    "rustc-workspace-wrapper",
];

/// Scan for things that change fingerprints: variables already exported, cargo config keys,
/// and scripts that export them on the user's behalf.
pub fn influences(root: &Path) -> Vec<BuildInfluence> {
    let mut found = Vec::new();
    for name in FORBIDDEN_ENV {
        if let Ok(value) = std::env::var(name) {
            found.push(BuildInfluence {
                source: "env".into(),
                name: name.into(),
                value: Some(value),
                note: "set in the calling environment; remake-dev inherits it untouched".into(),
            });
        }
    }
    for file in cargo_config_files(root) {
        let Ok(text) = fs::read_to_string(&file) else {
            continue;
        };
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('#') {
                continue;
            }
            for key in CONFIG_KEYS {
                if trimmed.starts_with(key) && trimmed[key.len()..].trim_start().starts_with('=') {
                    found.push(BuildInfluence {
                        source: file.to_string_lossy().into_owned(),
                        name: key.into(),
                        value: Some(trimmed.to_string()),
                        note: "applies to every cargo command, in every profile".into(),
                    });
                }
            }
        }
    }
    if let Ok(read) = fs::read_dir(root.join("scripts")) {
        for entry in read.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "sh") {
                continue;
            }
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            for (n, line) in text.lines().enumerate() {
                let t = line.trim_start();
                if t.starts_with('#') {
                    continue;
                }
                for name in FORBIDDEN_ENV {
                    if t.contains(&format!("export {name}=")) || t.contains(&format!("{name}=\"$"))
                    {
                        found.push(BuildInfluence {
                            source: format!("{}:{}", path.display(), n + 1),
                            name: name.into(),
                            value: None,
                            note: "a script exports this automatically, so the same command gets different \
                                   fingerprints depending on which tools happen to be installed"
                                .into(),
                        });
                    }
                }
            }
        }
    }
    found
}

pub fn target_dirs(root: &Path) -> Vec<TargetDirInfo> {
    let mut dirs = vec![root.join("target")];
    if let Some(custom) = std::env::var_os("CARGO_TARGET_DIR") {
        dirs.push(PathBuf::from(custom));
    }
    dirs.into_iter()
        .map(|dir| {
            let mut counts = Vec::new();
            for profile in ["debug", "release"] {
                let fp = dir.join(profile).join(".fingerprint");
                if let Ok(read) = fs::read_dir(&fp) {
                    counts.push((profile.to_string(), read.count()));
                }
            }
            TargetDirInfo {
                path: dir.to_string_lossy().into_owned(),
                exists: dir.is_dir(),
                fingerprint_dirs: counts,
            }
        })
        .collect()
}

pub fn inspect(root: &Path) -> EnvReport {
    EnvReport {
        tools: tools(),
        influences: influences(root),
        target_dirs: target_dirs(root),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_that_export_build_variables_are_reported_but_not_changed() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("scripts")).unwrap();
        let script = dir.path().join("scripts/dev.sh");
        let body = "#!/bin/bash\n# export RUSTFLAGS=ignored comment\nexport RUSTC_WRAPPER=kache\nexport RUSTFLAGS=\"$RUSTFLAGS -C x\"\n";
        fs::write(&script, body).unwrap();
        let found = influences(dir.path());
        let names: Vec<_> = found
            .iter()
            .filter(|i| i.source.ends_with("dev.sh:3") || i.source.ends_with("dev.sh:4"))
            .map(|i| i.name.as_str())
            .collect();
        assert!(names.contains(&"RUSTC_WRAPPER"), "{found:?}");
        assert!(names.contains(&"RUSTFLAGS"), "{found:?}");
        assert!(
            !found.iter().any(|i| i.source.ends_with("dev.sh:2")),
            "comments are ignored"
        );
        assert_eq!(fs::read_to_string(&script).unwrap(), body);
    }

    #[test]
    fn cargo_config_keys_that_affect_fingerprints_are_found() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join(".cargo")).unwrap();
        fs::write(
            dir.path().join(".cargo/config.toml"),
            "[build]\nrustc-wrapper = \"kache\"\n# linker = \"nope\"\n",
        )
        .unwrap();
        let found = influences(dir.path());
        assert!(found.iter().any(|i| i.name == "rustc-wrapper"));
        assert!(!found
            .iter()
            .any(|i| i.name == "linker" && i.source.contains(&*dir.path().to_string_lossy())));
    }
}
