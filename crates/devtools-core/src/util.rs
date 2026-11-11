//! Small helpers: hashing, timestamps, identifiers, process lookups. Std only plus sha2/hex.

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Hash of a directory tree: sorted relative paths with the sha256 of each file.
/// `skip` names (file names, at any depth) are left out, e.g. the manifest itself.
pub fn sha256_tree(root: &Path, skip: &[&str]) -> Result<String> {
    let mut files = Vec::new();
    collect_files(root, root, skip, &mut files)?;
    files.sort();
    let mut hasher = Sha256::new();
    for rel in files {
        hasher.update(rel.to_string_lossy().as_bytes());
        hasher.update([0]);
        hasher.update(sha256_file(&root.join(&rel))?.as_bytes());
        hasher.update(*b"\n");
    }
    Ok(hex::encode(hasher.finalize()))
}

fn collect_files(base: &Path, dir: &Path, skip: &[&str], out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("cannot read {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if skip.contains(&name.as_str()) {
            continue;
        }
        let kind = entry.file_type()?;
        if kind.is_dir() {
            collect_files(base, &path, skip, out)?;
        } else if kind.is_file() {
            out.push(path.strip_prefix(base)?.to_path_buf());
        }
    }
    Ok(())
}

/// Copy a directory recursively, keeping file permissions. Symlinks are copied as the
/// file they point to (the install tree must not depend on the build directory).
pub fn copy_dir_all(from: &Path, to: &Path) -> Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from).with_context(|| format!("cannot read {}", from.display()))? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        let path = entry.path();
        let meta = fs::metadata(&path)?;
        if meta.is_dir() {
            copy_dir_all(&path, &target)?;
        } else {
            fs::copy(&path, &target).with_context(|| {
                format!("cannot copy {} -> {}", path.display(), target.display())
            })?;
        }
    }
    Ok(())
}

pub fn now_rfc3339() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    rfc3339_from_unix(secs)
}

/// RFC 3339 UTC timestamp from unix seconds (civil-from-days, Howard Hinnant).
pub fn rfc3339_from_unix(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { y + 1 } else { y };
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

/// A PostgreSQL identifier this tool is willing to put inside quotes.
pub fn is_identifier(name: &str) -> bool {
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

/// Absolute path of an executable on PATH, if any.
pub fn which(cmd: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths)
        .map(|dir| dir.join(cmd))
        .find(|candidate| candidate.is_file())
}

/// `<cmd> <args>` stdout trimmed, or None when the command is missing or fails.
///
/// A binary that was copied a moment ago can still be open for writing in another thread's
/// forked child ("text file busy"); that clears within milliseconds, so it is retried.
pub fn command_output(cmd: &str, args: &[&str]) -> Option<String> {
    let mut attempts = 0;
    let out = loop {
        match std::process::Command::new(cmd).args(args).output() {
            Ok(out) => break out,
            Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy && attempts < 20 => {
                attempts += 1;
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            Err(_) => return None,
        }
    };
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3339_known_values() {
        assert_eq!(rfc3339_from_unix(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339_from_unix(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(rfc3339_from_unix(1_700_000_000), "2023-11-14T22:13:20Z");
    }

    #[test]
    fn identifiers() {
        assert!(is_identifier("remake_dev"));
        assert!(!is_identifier("Remake"));
        assert!(!is_identifier("a\"; drop"));
        assert!(!is_identifier(""));
    }

    #[test]
    fn tree_hash_ignores_skipped_names_and_is_stable() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a"), b"1").unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();
        fs::write(dir.path().join("sub/b"), b"2").unwrap();
        let first = sha256_tree(dir.path(), &["manifest.json"]).unwrap();
        fs::write(dir.path().join("manifest.json"), b"x").unwrap();
        assert_eq!(first, sha256_tree(dir.path(), &["manifest.json"]).unwrap());
        fs::write(dir.path().join("sub/b"), b"3").unwrap();
        assert_ne!(first, sha256_tree(dir.path(), &["manifest.json"]).unwrap());
    }
}
