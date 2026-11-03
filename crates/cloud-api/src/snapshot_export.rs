//! Export private candidate files for offline inspection. This is not deployment.
use cloud_storage::{ObjectVersion, QuarantineStore};
use serde::Serialize;
use sqlx::{PgConnection, PgPool, Row};
use std::{
    collections::BTreeSet,
    error::Error,
    fs,
    io::Write,
    path::{Component, Path},
};
use uuid::Uuid;
type Result<T> = std::result::Result<T, Box<dyn Error>>;
const MAX_TOTAL: u64 = 20 * 1024 * 1024 * 1024;
const MARKER: &str = ".cloud-candidate.json";
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct File {
    pub(crate) path: String,
    pub(crate) digest: String,
    size: i64,
    key: String,
    version: String,
    revision: String,
}
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Manifest {
    pub(crate) root: String,
    pub(crate) inventory: String,
    revision: String,
    pub(crate) files: Vec<File>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExportFile<'a> {
    path: &'a str,
    sha256: &'a str,
    size: i64,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExportMarker<'a> {
    candidate_id: Uuid,
    status: &'static str,
    root_fingerprint: &'a str,
    inventory_sha256: &'a str,
    files: Vec<ExportFile<'a>>,
}
fn valid_path(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 2048
        && s.split('/').all(|part| {
            !part.is_empty()
                && !part.starts_with('.')
                && part
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"._+-".contains(&c))
        })
        && Path::new(s)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}
async fn manifest(conn: &mut PgConnection, id: Uuid) -> Result<Manifest> {
    let row=sqlx::query("SELECT status,root_fingerprint,inventory_sha256,file_count,xmin::text AS revision FROM cloud_snapshot_candidates WHERE id=$1").bind(id).fetch_optional(&mut *conn).await?.ok_or("candidate not found")?;
    if row.get::<String, _>("status") != "candidate" {
        return Err("candidate unavailable".into());
    }
    let expected: i32 = row.get("file_count");
    let rows=sqlx::query("SELECT path,sha256,size,object_key,version_id,xmin::text AS revision FROM cloud_snapshot_candidate_files WHERE candidate_id=$1 ORDER BY path LIMIT 41001").bind(id).fetch_all(&mut *conn).await?;
    if rows.len() != expected as usize || rows.is_empty() || rows.len() > 41000 {
        return Err("candidate file manifest incomplete".into());
    }
    let mut files = Vec::with_capacity(rows.len());
    let mut total = 0_u64;
    for row in rows {
        let file = File {
            path: row.get("path"),
            digest: row.get("sha256"),
            size: row.get("size"),
            key: row.get("object_key"),
            version: row.get("version_id"),
            revision: row.get("revision"),
        };
        if !valid_path(&file.path)
            || file.path == MARKER
            || !(1..=cloud_storage::MAX_ARTIFACT_SIZE).contains(&file.size)
            || file.digest.len() != 64
            || !file
                .digest
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        {
            return Err("invalid candidate file metadata".into());
        }
        let prefix = format!("prepared/{id}/");
        let suffix = file
            .key
            .strip_prefix(&prefix)
            .ok_or("invalid candidate object key")?;
        if Uuid::parse_str(suffix)?.to_string() != suffix
            || file.version.is_empty()
            || file.version == "null"
            || file.version.len() > 1024
        {
            return Err("invalid candidate object version".into());
        }
        total = total
            .checked_add(file.size as u64)
            .ok_or("candidate too large")?;
        if total > MAX_TOTAL {
            return Err("candidate too large".into());
        }
        files.push(file);
    }
    let paths: BTreeSet<_> = files.iter().map(|f| f.path.as_str()).collect();
    for required in [
        "root.json",
        "keys.json",
        "keys.json.sig",
        "config.json",
        "config.json.sig",
    ] {
        if !paths.contains(required) {
            return Err("candidate seed files missing".into());
        }
    }
    Ok(Manifest {
        root: row.get("root_fingerprint"),
        inventory: row.get("inventory_sha256"),
        revision: row.get("revision"),
        files,
    })
}
pub(crate) async fn current(pool: &PgPool, id: Uuid) -> Result<Manifest> {
    let mut tx = pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
        .execute(&mut *tx)
        .await?;
    let value = manifest(&mut tx, id).await?;
    tx.commit().await?;
    Ok(value)
}
pub async fn export(
    pool: &PgPool,
    store: &QuarantineStore,
    id: Uuid,
    destination: &Path,
) -> Result<()> {
    #[cfg(not(unix))]
    {
        let _ = (pool, store, id, destination);
        return Err("private export permissions are not implemented on this platform".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        let initial = current(pool, id).await?;
        // An existing destination is never touched; incomplete exports have no marker.
        fs::DirBuilder::new().mode(0o700).create(destination)?;
        let parent = destination
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::File::open(parent)?.sync_all()?;
        for file in &initial.files {
            let target = destination.join(&file.path);
            if let Some(parent) = target.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            let mut output = tokio::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(target)
                .await?;
            store
                .download_version(
                    &file.key,
                    &ObjectVersion {
                        version_id: file.version.clone(),
                        size: file.size,
                    },
                    &file.digest,
                    &mut output,
                )
                .await?;
            output.sync_all().await?;
        }
        // Recheck the final on-disk bytes before creating the completion marker.
        let verify_dir = destination.to_path_buf();
        let verify_files = initial.files.clone();
        tokio::task::spawn_blocking(move || -> std::result::Result<(), String> {
            let mut dirs = BTreeSet::new();
            dirs.insert(verify_dir.clone());
            for file in verify_files {
                let path = verify_dir.join(&file.path);
                let mut parent = path.parent();
                while let Some(dir) = parent {
                    if !dir.starts_with(&verify_dir) {
                        break;
                    }
                    dirs.insert(dir.to_path_buf());
                    if dir == verify_dir {
                        break;
                    }
                    parent = dir.parent();
                }
                let meta = fs::symlink_metadata(&path)
                    .map_err(|_| "export verification failed".to_owned())?;
                if !meta.file_type().is_file()
                    || meta.len() != file.size as u64
                    || tiktools_plugin_package::Sha256Digest::of_file(&path)
                        .map_err(|_| "export verification failed".to_owned())?
                        .as_hex()
                        != file.digest
                {
                    return Err("export verification failed".into());
                }
            }
            // Sync nested directory entries before publishing the marker.
            let mut dirs: Vec<_> = dirs.into_iter().collect();
            dirs.sort_by_key(|dir| std::cmp::Reverse(dir.components().count()));
            for dir in dirs {
                fs::File::open(dir)
                    .and_then(|file| file.sync_all())
                    .map_err(|_| "export verification failed".to_owned())?;
            }
            Ok(())
        })
        .await
        .map_err(|_| "export verification task failed")??;
        let latest = current(pool, id).await?;
        if latest != initial {
            return Err("candidate manifest changed during export".into());
        }
        let marker = ExportMarker {
            candidate_id: id,
            status: "private-candidate",
            root_fingerprint: &initial.root,
            inventory_sha256: &initial.inventory,
            files: initial
                .files
                .iter()
                .map(|f| ExportFile {
                    path: &f.path,
                    sha256: &f.digest,
                    size: f.size,
                })
                .collect(),
        };
        let mut temporary = tempfile::NamedTempFile::new_in(destination)?;
        serde_json::to_writer(&mut temporary, &marker)?;
        temporary.write_all(b"\n")?;
        temporary.as_file().sync_all()?;
        temporary
            .persist_noclobber(destination.join(MARKER))
            .map_err(|_| "could not finish candidate export")?;
        fs::File::open(destination)?.sync_all()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::valid_path;
    #[test]
    fn candidate_paths_cannot_escape_or_hide_files() {
        for good in [
            "root.json",
            "index/ac/me/acme.plugin.json",
            "packages/acme.plugin/a.plugin",
        ] {
            assert!(valid_path(good));
        }
        for bad in [
            "",
            "/root.json",
            "../root.json",
            "index/../root.json",
            "index/./root.json",
            ".hidden",
            "index/.hidden",
            "a\\b",
            "a\nfile",
        ] {
            assert!(!valid_path(bad));
        }
    }
}
