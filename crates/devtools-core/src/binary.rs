//! BinaryManager: pgrust is built once, explicitly, and then only ever executed.
//!
//! The installed tree lives outside every `target/` directory:
//!
//! ```text
//! .devtools/bin/pgrust/
//!   postgres  initdb  share/  manifest.json
//! ```
//!
//! pgrust reads `share/` from beside its binary, so the whole tree is installed, hashed and
//! replaced as a unit. Nothing in this module runs cargo except [`build_from_source`], which
//! is only reachable from `db install --source` and `db upgrade --source`.

use crate::util::{command_output, copy_dir_all, now_rfc3339, sha256_file, sha256_tree};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const MANIFEST_FILE: &str = "manifest.json";
pub const MANIFEST_SCHEMA: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub schema_version: u32,
    pub engine: String,
    /// Exactly what `postgres --version` printed at install time.
    pub version: String,
    pub source_commit: Option<String>,
    pub source_dirty: bool,
    /// Relative to the checkout when possible, so the manifest survives moving the repo.
    pub binary_path: String,
    pub binary_sha256: String,
    /// Hash of every file of the tree except this manifest.
    pub tree_sha256: String,
    pub source_type: String,
    pub build_profile: String,
    pub features: Vec<String>,
    pub installed_at: String,
}

/// What the caller knows about where a tree came from.
#[derive(Debug, Clone)]
pub struct InstallMeta {
    pub source_type: String,
    pub source_commit: Option<String>,
    pub source_dirty: bool,
    pub build_profile: String,
    pub features: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct BinaryManager {
    pub install_dir: PathBuf,
    /// Checkout root, used only to store a relative `binary_path` in the manifest.
    pub root: PathBuf,
}

pub fn exe_name(base: &str) -> String {
    if cfg!(windows) {
        format!("{base}.exe")
    } else {
        base.to_string()
    }
}

impl BinaryManager {
    pub fn new(install_dir: PathBuf, root: PathBuf) -> Self {
        Self { install_dir, root }
    }

    pub fn binary_path(&self) -> PathBuf {
        self.install_dir.join(exe_name("postgres"))
    }
    pub fn initdb_path(&self) -> PathBuf {
        self.install_dir.join(exe_name("initdb"))
    }
    pub fn share_dir(&self) -> PathBuf {
        self.install_dir.join("share")
    }
    pub fn manifest_path(&self) -> PathBuf {
        self.install_dir.join(MANIFEST_FILE)
    }

    pub fn is_installed(&self) -> bool {
        self.binary_path().is_file()
    }

    /// The installed `postgres` path, or the instruction to install it. Never builds.
    pub fn require_binary(&self) -> Result<PathBuf> {
        let binary = self.binary_path();
        if !binary.is_file() {
            bail!(
                "pgrust is not installed at {}. Run `remake-dev db install --source <pgrust checkout>` \
                 (or `--prebuilt <dir>`); nothing else ever compiles it",
                self.install_dir.display()
            );
        }
        Ok(binary)
    }

    pub fn load_manifest(&self) -> Result<Manifest> {
        let path = self.manifest_path();
        let text = fs::read_to_string(&path).with_context(|| {
            format!(
                "no manifest at {}; reinstall with `remake-dev db install --force`",
                path.display()
            )
        })?;
        let manifest: Manifest = serde_json::from_str(&text)
            .with_context(|| format!("{} is not a valid manifest", path.display()))?;
        if manifest.schema_version != MANIFEST_SCHEMA {
            bail!(
                "manifest schema {} is not supported (expected {MANIFEST_SCHEMA})",
                manifest.schema_version
            );
        }
        if manifest.engine != "pgrust" {
            bail!(
                "manifest engine is `{}`, expected `pgrust`",
                manifest.engine
            );
        }
        Ok(manifest)
    }

    /// Check the install is the one the manifest describes. The quick check hashes the server
    /// binary; the deep check hashes the whole tree. Neither runs cargo.
    pub fn verify(&self, deep: bool) -> Result<Manifest> {
        self.require_binary()?;
        let manifest = self.load_manifest()?;
        for needed in [self.initdb_path(), self.share_dir()] {
            if !needed.exists() {
                bail!("{} is missing from the install", needed.display());
            }
        }
        let actual = sha256_file(&self.binary_path())?;
        if actual != manifest.binary_sha256 {
            bail!(
                "the installed postgres does not match its manifest (sha256 {} != {}); \
                 reinstall with `remake-dev db install --force`",
                actual,
                manifest.binary_sha256
            );
        }
        if deep {
            let tree = sha256_tree(&self.install_dir, &[MANIFEST_FILE])?;
            if tree != manifest.tree_sha256 {
                bail!("the installed tree does not match its manifest (tree sha256 differs)");
            }
        }
        Ok(manifest)
    }

    /// Install a built pgrust release directory (`postgres`, `initdb`, `share/`).
    ///
    /// The tree is copied to a staging directory, validated by running it, and only then
    /// swapped in. A failure at any step leaves the previous installation untouched.
    pub fn install_from_dir(&self, release_dir: &Path, meta: InstallMeta) -> Result<Manifest> {
        for name in [exe_name("postgres"), exe_name("initdb")] {
            if !release_dir.join(&name).is_file() {
                bail!(
                    "{} has no `{name}`; is this a pgrust release directory?",
                    release_dir.display()
                );
            }
        }
        let share = release_dir.join("share");
        if !share.is_dir() || fs::read_dir(&share)?.next().is_none() {
            bail!(
                "{} has no populated share/ directory; pgrust reads it from beside the binary",
                release_dir.display()
            );
        }
        let parent = self
            .install_dir
            .parent()
            .context("install directory has no parent")?;
        fs::create_dir_all(parent)?;
        let name = self
            .install_dir
            .file_name()
            .context("install directory has no name")?
            .to_string_lossy()
            .into_owned();
        let staging = parent.join(format!("{name}.staging-{}", std::process::id()));
        if staging.exists() {
            fs::remove_dir_all(&staging)?;
        }
        let staged = (|| -> Result<Manifest> {
            fs::create_dir_all(&staging)?;
            for bin in [exe_name("postgres"), exe_name("initdb"), exe_name("psql")] {
                let from = release_dir.join(&bin);
                if from.is_file() {
                    fs::copy(&from, staging.join(&bin))
                        .with_context(|| format!("cannot copy {}", from.display()))?;
                }
            }
            copy_dir_all(&share, &staging.join("share"))?;
            let version = command_output(
                staging
                    .join(exe_name("postgres"))
                    .to_str()
                    .unwrap_or_default(),
                &["--version"],
            )
            .context("the staged postgres does not run (`postgres --version` failed)")?;
            if !version.to_lowercase().contains("pgrust") {
                bail!("the staged postgres does not identify as pgrust: `{version}`");
            }
            command_output(
                staging
                    .join(exe_name("initdb"))
                    .to_str()
                    .unwrap_or_default(),
                &["--version"],
            )
            .context("the staged initdb does not run (`initdb --version` failed)")?;
            let final_binary = self.binary_path();
            let binary_path = final_binary
                .strip_prefix(&self.root)
                .unwrap_or(&final_binary)
                .to_string_lossy()
                .into_owned();
            let manifest = Manifest {
                schema_version: MANIFEST_SCHEMA,
                engine: "pgrust".into(),
                version,
                source_commit: meta.source_commit,
                source_dirty: meta.source_dirty,
                binary_path,
                binary_sha256: sha256_file(&staging.join(exe_name("postgres")))?,
                tree_sha256: sha256_tree(&staging, &[MANIFEST_FILE])?,
                source_type: meta.source_type,
                build_profile: meta.build_profile,
                features: meta.features,
                installed_at: now_rfc3339(),
            };
            fs::write(
                staging.join(MANIFEST_FILE),
                serde_json::to_string_pretty(&manifest)? + "\n",
            )?;
            Ok(manifest)
        })();
        let manifest = match staged {
            Ok(manifest) => manifest,
            Err(e) => {
                let _ = fs::remove_dir_all(&staging);
                return Err(e);
            }
        };
        let previous = parent.join(format!("{name}.previous-{}", std::process::id()));
        if self.install_dir.exists() {
            fs::rename(&self.install_dir, &previous)
                .context("cannot move the old install aside")?;
        }
        if let Err(e) = fs::rename(&staging, &self.install_dir) {
            if previous.exists() {
                let _ = fs::rename(&previous, &self.install_dir);
            }
            let _ = fs::remove_dir_all(&staging);
            return Err(e)
                .context("cannot move the new install into place; the old one was restored");
        }
        let _ = fs::remove_dir_all(&previous);
        Ok(manifest)
    }
}

/// Features worth building pgrust with. A pgrust that defines `main_main/full` ships CJK
/// conversions, pgcrypto and the Snappy/Brotli Parquet codecs only through it; an older one
/// has no such feature and builds as-is.
pub fn detect_features(source: &Path) -> Vec<String> {
    let manifest = source.join("crates/backend/main/main_main/Cargo.toml");
    match fs::read_to_string(manifest) {
        Ok(text) if text.lines().any(|l| l.trim_start().starts_with("full =")) => {
            vec!["main_main/full".to_string()]
        }
        _ => Vec::new(),
    }
}

pub fn git_commit(dir: &Path) -> Option<String> {
    command_output("git", &["-C", dir.to_str()?, "rev-parse", "HEAD"])
}

pub fn git_dirty(dir: &Path) -> bool {
    command_output(
        "git",
        &["-C", dir.to_str().unwrap_or("."), "status", "--porcelain"],
    )
    .map(|s| !s.is_empty())
    .unwrap_or(false)
}

/// Apply the tracked mimalloc/glibc 2.44 compatibility patch to a source tree that lacks the
/// fix. A tree that already has it is left alone.
pub fn ensure_compat_patch(source: &Path, patch: &Path) -> Result<()> {
    if !patch.is_file() {
        return Ok(());
    }
    let reverse_ok = Command::new("git")
        .current_dir(source)
        .args(["apply", "--unidiff-zero", "--reverse", "--check"])
        .arg(patch)
        .output()?
        .status
        .success();
    if reverse_ok {
        return Ok(());
    }
    let applied = Command::new("git")
        .current_dir(source)
        .args(["apply", "--unidiff-zero"])
        .arg(patch)
        .output()?;
    if !applied.status.success() {
        bail!(
            "cannot apply the pgrust compatibility patch; check the source tree for local changes: {}",
            String::from_utf8_lossy(&applied.stderr)
        );
    }
    Ok(())
}

/// Build pgrust from a source checkout. This is the only place cargo is run for pgrust, and it
/// is reached only from `db install --source` / `db upgrade --source`.
///
/// The build uses pgrust's own release profile and pinned toolchain. The user's
/// `RUSTFLAGS`, `RUSTC_WRAPPER` and linker settings are inherited untouched; the output goes to
/// `<source>/target` through `--target-dir`, so the backend's `target/` is never involved.
pub fn build_from_source(source: &Path, features: &[String]) -> Result<PathBuf> {
    if !source.join("Cargo.toml").is_file() {
        bail!(
            "{} is not a pgrust checkout (no Cargo.toml); for a submodule run \
             `git submodule update --init --recursive`",
            source.display()
        );
    }
    let target = source.join("target");
    let mut cmd = Command::new("cargo");
    cmd.current_dir(source)
        .env_remove("RUSTUP_TOOLCHAIN") // let pgrust's rust-toolchain.toml pick its own compiler
        .args([
            "build",
            "--release",
            "--locked",
            "-p",
            "main_main",
            "-p",
            "initdb",
        ]);
    for feature in features {
        cmd.args(["--features", feature]);
    }
    cmd.arg("--target-dir").arg(&target);
    let status = cmd
        .status()
        .context("failed to run cargo to build pgrust")?;
    if !status.success() {
        bail!("pgrust build failed ({status})");
    }
    Ok(target.join("release"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// A release directory whose `postgres`/`initdb` are tiny scripts that report a version.
    fn fake_release(dir: &Path, ident: &str) {
        for bin in ["postgres", "initdb"] {
            let path = dir.join(bin);
            fs::write(&path, format!("#!/bin/sh\necho '{ident}'\n")).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        fs::create_dir_all(dir.join("share/timezone")).unwrap();
        fs::write(dir.join("share/postgres.bki"), b"bki").unwrap();
    }

    fn meta() -> InstallMeta {
        InstallMeta {
            source_type: "prebuilt".into(),
            source_commit: Some("abc".into()),
            source_dirty: false,
            build_profile: "release".into(),
            features: vec![],
        }
    }

    #[test]
    fn missing_binary_gives_the_install_instruction_and_never_builds() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = BinaryManager::new(dir.path().join("bin/pgrust"), dir.path().to_path_buf());
        let err = mgr.require_binary().unwrap_err().to_string();
        assert!(err.contains("db install"), "{err}");
        assert!(!mgr.is_installed());
    }

    #[test]
    fn install_verify_and_detect_tampering() {
        let dir = tempfile::tempdir().unwrap();
        let rel = dir.path().join("rel");
        fs::create_dir(&rel).unwrap();
        fake_release(&rel, "pgrust 9.9-test");
        let mgr = BinaryManager::new(
            dir.path().join(".devtools/bin/pgrust"),
            dir.path().to_path_buf(),
        );
        let manifest = mgr.install_from_dir(&rel, meta()).unwrap();
        assert_eq!(manifest.engine, "pgrust");
        assert_eq!(manifest.version, "pgrust 9.9-test");
        assert_eq!(manifest.binary_path, ".devtools/bin/pgrust/postgres");
        assert_eq!(mgr.verify(true).unwrap(), manifest);
        // Changing the backend never changes the installed binary or its hash.
        fs::write(dir.path().join("backend.rs"), b"fn changed() {}").unwrap();
        assert_eq!(
            mgr.verify(true).unwrap().binary_sha256,
            manifest.binary_sha256
        );
        // Tampering is caught.
        fs::write(mgr.binary_path(), b"#!/bin/sh\necho 'pgrust evil'\n").unwrap();
        assert!(mgr
            .verify(false)
            .unwrap_err()
            .to_string()
            .contains("does not match"));
    }

    #[test]
    fn a_failed_install_keeps_the_previous_one() {
        let dir = tempfile::tempdir().unwrap();
        let good = dir.path().join("good");
        fs::create_dir(&good).unwrap();
        fake_release(&good, "pgrust 1.0");
        let mgr = BinaryManager::new(dir.path().join("bin/pgrust"), dir.path().to_path_buf());
        let first = mgr.install_from_dir(&good, meta()).unwrap();
        // Not pgrust: must be refused, and the installed one must be unchanged.
        let bad = dir.path().join("bad");
        fs::create_dir(&bad).unwrap();
        fake_release(&bad, "postgres (PostgreSQL) 18");
        let err = mgr.install_from_dir(&bad, meta()).unwrap_err().to_string();
        assert!(err.contains("does not identify as pgrust"), "{err}");
        assert_eq!(mgr.verify(true).unwrap(), first);
        // No staging leftovers.
        let leftovers: Vec<_> = fs::read_dir(dir.path().join("bin"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(leftovers, vec!["pgrust".to_string()]);
    }

    #[test]
    fn a_release_without_share_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let rel = dir.path().join("rel");
        fs::create_dir(&rel).unwrap();
        fake_release(&rel, "pgrust 1.0");
        fs::remove_dir_all(rel.join("share")).unwrap();
        let mgr = BinaryManager::new(dir.path().join("bin/pgrust"), dir.path().to_path_buf());
        assert!(mgr
            .install_from_dir(&rel, meta())
            .unwrap_err()
            .to_string()
            .contains("share"));
    }

    #[test]
    fn features_are_detected_from_the_source_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let toml = dir.path().join("crates/backend/main/main_main");
        fs::create_dir_all(&toml).unwrap();
        fs::write(toml.join("Cargo.toml"), "[features]\nfull = [\"x\"]\n").unwrap();
        assert_eq!(
            detect_features(dir.path()),
            vec!["main_main/full".to_string()]
        );
        fs::write(toml.join("Cargo.toml"), "[features]\ndefault = []\n").unwrap();
        assert!(detect_features(dir.path()).is_empty());
    }
}
