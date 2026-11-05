//! Offline coordinated backup for a single-instance local object volume.
use crate::config::StorageConfig;
use futures_util::TryStreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{postgres::PgPoolOptions, Row};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Read, Write},
    path::Path,
    process::Stdio,
    time::{SystemTime, UNIX_EPOCH},
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const MANIFEST: &str = "manifest.json";
const DUMP: &str = "database.dump";
const MAX_MANIFEST: u64 = 16 * 1024 * 1024;
const MAX_OBJECTS: usize = 50_000;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Manifest {
    format: u32,
    created_unix: u64,
    database_sha256: String,
    objects: Vec<Object>,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Object {
    name: String,
    size: u64,
    sha256: String,
}

fn digest_ok(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
fn object_name(key: &str) -> String {
    format!("object-{}", hex::encode(Sha256::digest(key.as_bytes())))
}
fn private_dir(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new().mode(0o700).create(path)?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Err("local backup requires Unix permissions".into())
    }
}
fn regular(path: &Path) -> Result<std::fs::Metadata> {
    let meta = std::fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() || !meta.is_file() {
        return Err("backup contains a non-regular file".into());
    }
    Ok(meta)
}
fn hash_file(path: &Path) -> Result<(u64, String)> {
    let meta = regular(path)?;
    let mut file = std::fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buf = vec![0; 64 * 1024];
    let mut count = 0u64;
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        count = count.checked_add(n as u64).ok_or("file size overflow")?;
        hash.update(&buf[..n]);
    }
    if count != meta.len() {
        return Err("file changed while being read".into());
    }
    Ok((count, hex::encode(hash.finalize())))
}
fn copy_file(src: &Path, dst: &Path) -> Result<(u64, String)> {
    regular(src)?;
    let mut from = std::fs::File::open(src)?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut to = options.open(dst)?;
    let mut hash = Sha256::new();
    let mut count = 0u64;
    let mut buf = vec![0; 64 * 1024];
    loop {
        let n = from.read(&mut buf)?;
        if n == 0 {
            break;
        }
        count = count.checked_add(n as u64).ok_or("file size overflow")?;
        hash.update(&buf[..n]);
        to.write_all(&buf[..n])?;
    }
    to.sync_all()?;
    Ok((count, hex::encode(hash.finalize())))
}
fn sync_dir(path: &Path) -> Result<()> {
    std::fs::File::open(path)?.sync_all()?;
    Ok(())
}

/// Supply libpq settings through the environment rather than exposing the
/// credential-bearing URL in pg_dump/pg_restore process arguments.
fn configure_pg_command(
    command: &mut tokio::process::Command,
    database_url: &str,
) -> Result<String> {
    let url = url::Url::parse(database_url)?;
    if !matches!(url.scheme(), "postgres" | "postgresql") || url.host().is_none() {
        return Err("invalid PostgreSQL connection URL".into());
    }
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("PG") {
            command.env_remove(name);
        }
    }
    let decode = |value: &str| -> Result<String> {
        Ok(percent_encoding::percent_decode_str(value)
            .decode_utf8()?
            .into_owned())
    };
    let host = match url.host().ok_or("database URL has no host")? {
        url::Host::Domain(name) => name.to_owned(),
        url::Host::Ipv4(ip) => ip.to_string(),
        url::Host::Ipv6(ip) => ip.to_string(),
    };
    let database = decode(
        url.path()
            .strip_prefix('/')
            .ok_or("database URL has no database")?,
    )?;
    if database.is_empty()
        || database.contains('=')
        || database.chars().any(char::is_whitespace)
        || database.starts_with("postgres://")
        || database.starts_with("postgresql://")
    {
        return Err("unsupported database name in URL".into());
    }
    command
        .env("PGHOST", host)
        .env("PGPORT", url.port().unwrap_or(5432).to_string())
        .env("PGDATABASE", &database)
        .env("PGCONNECT_TIMEOUT", "10");
    if !url.username().is_empty() {
        command.env("PGUSER", decode(url.username())?);
    }
    if let Some(password) = url.password() {
        command.env("PGPASSWORD", decode(password)?);
    }
    let mut seen = BTreeSet::new();
    for (key, value) in url.query_pairs() {
        if !seen.insert(key.to_string()) {
            return Err("duplicate database URL parameter".into());
        }
        let env = match key.as_ref() {
            "sslmode" => "PGSSLMODE",
            "sslrootcert" => "PGSSLROOTCERT",
            "sslcert" => "PGSSLCERT",
            "sslkey" => "PGSSLKEY",
            "sslpassword" => "PGSSLPASSWORD",
            "connect_timeout" => "PGCONNECT_TIMEOUT",
            "application_name" => "PGAPPNAME",
            "options" => "PGOPTIONS",
            "target_session_attrs" => "PGTARGETSESSIONATTRS",
            "channel_binding" => "PGCHANNELBINDING",
            "gssencmode" => "PGGSSENCMODE",
            _ => {
                return Err(
                    format!("unsupported PostgreSQL URL parameter for backup: {key}").into(),
                )
            }
        };
        command.env(env, value.as_ref());
    }
    Ok(database)
}

/// All application and worker processes configured for the same local volume
/// must be stopped; the exclusive volume lock prevents them from starting.
/// Direct database and filesystem writers must also be stopped by the operator.
pub async fn backup(config: StorageConfig, database_url: &str, destination: &Path) -> Result<()> {
    let StorageConfig::Local { root, .. } = config else {
        return Err("backup-local requires STORAGE_LOCAL_DIR".into());
    };
    let _guard = cloud_storage::local::backup_lock(&root).map_err(|_| {
        "cannot lock local volume; stop API and workers and check directory permissions"
    })?;
    let root = std::fs::canonicalize(&root)?;
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let parent = std::fs::canonicalize(parent)?;
    if parent.starts_with(&root) {
        return Err("backup destination must be outside the storage volume".into());
    }
    private_dir(destination)?;
    let object_dir = destination.join("objects");
    private_dir(&object_dir)?;
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(database_url)
        .await?;
    let mut tx = pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await?;
    let snapshot: String = sqlx::query_scalar("SELECT pg_export_snapshot()")
        .fetch_one(&mut *tx)
        .await?;
    let mut rows = sqlx::query("SELECT object_key, version_id, expected_size AS size, expected_sha256 AS sha256 FROM cloud_upload_sessions WHERE status='quarantined' UNION ALL SELECT j.prepared_key, j.prepared_version_id, u.expected_size, u.expected_sha256 FROM cloud_publication_jobs j JOIN cloud_plugin_releases r ON r.id=j.release_id JOIN cloud_upload_sessions u ON u.id=r.upload_id WHERE j.prepared_key IS NOT NULL UNION ALL SELECT object_key,version_id,size,sha256 FROM cloud_snapshot_candidate_files UNION ALL SELECT object_key,version_id,size,sha256 FROM cloud_registry_generation_files UNION ALL SELECT object_key,version_id,size,sha256 FROM cloud_registry_generation_releases")
        .fetch(&mut *tx);
    let mut required: BTreeMap<String, (u64, String)> = BTreeMap::new();
    while let Some(row) = rows.try_next().await? {
        let key: String = row.try_get("object_key")?;
        let version: String = row.try_get("version_id")?;
        let size: i64 = row.try_get("size")?;
        let hash: String = row.try_get("sha256")?;
        if !(1..=cloud_storage::MAX_ARTIFACT_SIZE).contains(&size)
            || !digest_ok(&hash)
            || version != hash
        {
            return Err("database references a non-local or invalid artifact version".into());
        }
        let item = (size as u64, hash);
        if let Some(previous) = required.insert(object_name(&key), item.clone()) {
            if previous != item {
                return Err("database contains conflicting artifact references".into());
            }
        }
        if required.len() > MAX_OBJECTS {
            return Err("too many referenced objects for one backup".into());
        }
    }
    drop(rows);
    let mut dump_command = tokio::process::Command::new("pg_dump");
    let _ = configure_pg_command(&mut dump_command, database_url)?;
    let status = tokio::time::timeout(
        std::time::Duration::from_secs(1800),
        dump_command
            .arg("--format=custom")
            .arg("--no-owner")
            .arg("--no-acl")
            .arg("--snapshot")
            .arg(&snapshot)
            .arg("--file")
            .arg(destination.join(DUMP))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .status(),
    )
    .await??;
    if !status.success() {
        return Err("pg_dump failed; check client version and database access".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            destination.join(DUMP),
            std::fs::Permissions::from_mode(0o600),
        )?;
    }
    std::fs::File::open(destination.join(DUMP))?.sync_all()?;
    let (_, database_sha256) = hash_file(&destination.join(DUMP))?;
    let mut objects = Vec::new();
    for entry in std::fs::read_dir(&root)? {
        let entry = entry?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| "invalid volume filename")?;
        if name == ".cloud-storage.lock" || name.starts_with(".tmp-") {
            continue;
        }
        if !name.starts_with("object-") || !digest_ok(&name[7..]) {
            return Err("unexpected entry in local storage directory".into());
        }
        if objects.len() >= MAX_OBJECTS {
            return Err("too many objects for one backup".into());
        }
        let (size, sha256) = copy_file(&entry.path(), &object_dir.join(&name))?;
        if size == 0 || size > cloud_storage::MAX_ARTIFACT_SIZE as u64 {
            return Err("invalid local object size".into());
        }
        if let Some(expected) = required.remove(&name) {
            if expected != (size, sha256.clone()) {
                return Err("local object does not match database reference".into());
            }
        }
        objects.push(Object { name, size, sha256 });
    }
    if !required.is_empty() {
        return Err("one or more database-referenced objects are missing".into());
    }
    objects.sort_by(|a, b| a.name.cmp(&b.name));
    tx.commit().await?;
    sync_dir(&object_dir)?;
    let manifest = Manifest {
        format: 1,
        created_unix: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
        database_sha256,
        objects,
    };
    let bytes = serde_json::to_vec_pretty(&manifest)?;
    if bytes.len() as u64 > MAX_MANIFEST {
        return Err("backup manifest is too large".into());
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(destination.join(MANIFEST))?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    sync_dir(destination)?;
    sync_dir(&parent)?;
    println!(
        "Backup complete: {} objects; verify before relying on it",
        manifest.objects.len()
    );
    Ok(())
}

fn verify_manifest(root: &Path) -> Result<Manifest> {
    let manifest_file = root.join(MANIFEST);
    if regular(&manifest_file)?.len() > MAX_MANIFEST {
        return Err("backup manifest is too large".into());
    }
    let manifest: Manifest = serde_json::from_slice(&std::fs::read(manifest_file)?)?;
    if manifest.format != 1 || !digest_ok(&manifest.database_sha256) {
        return Err("invalid backup manifest".into());
    }
    if manifest.objects.len() > MAX_OBJECTS {
        return Err("too many objects in backup".into());
    }
    if hash_file(&root.join(DUMP))?.1 != manifest.database_sha256 {
        return Err("database dump checksum mismatch".into());
    }
    let object_dir = root.join("objects");
    let metadata = std::fs::symlink_metadata(&object_dir)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("invalid objects directory".into());
    }
    let mut names = BTreeSet::new();
    for object in &manifest.objects {
        if !object.name.starts_with("object-")
            || !digest_ok(&object.name[7..])
            || !digest_ok(&object.sha256)
            || object.size == 0
            || object.size > cloud_storage::MAX_ARTIFACT_SIZE as u64
            || !names.insert(&object.name)
        {
            return Err("invalid object entry".into());
        }
        if hash_file(&object_dir.join(&object.name))? != (object.size, object.sha256.clone()) {
            return Err("object checksum or size mismatch".into());
        }
    }
    if std::fs::read_dir(&object_dir)?.count() != names.len() {
        return Err("unexpected objects in backup".into());
    }
    Ok(manifest)
}

pub fn verify(root: &Path) -> Result<()> {
    let manifest = verify_manifest(root)?;
    println!("Backup verified: {} objects", manifest.objects.len());
    Ok(())
}

/// Restore only into an empty database and empty local volume. Existing data
/// is never overwritten. Stop direct database and filesystem writers first.
pub async fn restore(config: StorageConfig, database_url: &str, backup: &Path) -> Result<()> {
    let StorageConfig::Local { root, .. } = config else {
        return Err("restore-local requires STORAGE_LOCAL_DIR".into());
    };
    let _guard = cloud_storage::local::backup_lock(&root).map_err(|_| {
        "cannot lock local volume; stop API and workers and check directory permissions"
    })?;
    let root = std::fs::canonicalize(root)?;
    let backup = std::fs::canonicalize(backup)?;
    if backup.starts_with(&root) || root.starts_with(&backup) {
        return Err("backup and destination volume must be separate".into());
    }
    let manifest = verify_manifest(&backup)?;
    for entry in std::fs::read_dir(&root)? {
        if entry?.file_name() != ".cloud-storage.lock" {
            return Err("restore requires an empty storage directory".into());
        }
    }
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(database_url)
        .await?;
    let occupied: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname NOT LIKE 'pg\\_%' ESCAPE '\\' AND n.nspname <> 'information_schema') OR EXISTS (SELECT 1 FROM pg_namespace WHERE nspname NOT LIKE 'pg\\_%' ESCAPE '\\' AND nspname NOT IN ('information_schema','public')) OR EXISTS (SELECT 1 FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname='public') OR EXISTS (SELECT 1 FROM pg_extension WHERE extname <> 'plpgsql')")
        .fetch_one(&pool).await?;
    if occupied {
        return Err("restore requires an empty target database".into());
    }
    // Verify the exact dump bytes that pg_restore will read, not merely a
    // previously checked path which could have changed in between.
    let mut dump = tempfile::tempfile_in(&root)?;
    let mut source = std::fs::File::open(backup.join(DUMP))?;
    let mut hash = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let n = source.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
        dump.write_all(&buffer[..n])?;
    }
    if hex::encode(hash.finalize()) != manifest.database_sha256 {
        return Err("database dump changed during restore".into());
    }
    use std::io::{Seek, SeekFrom};
    dump.seek(SeekFrom::Start(0))?;
    for object in &manifest.objects {
        let copied = copy_file(
            &backup.join("objects").join(&object.name),
            &root.join(&object.name),
        )?;
        if copied != (object.size, object.sha256.clone()) {
            return Err("object changed during restore".into());
        }
    }
    sync_dir(&root)?;
    let mut restore_command = tokio::process::Command::new("pg_restore");
    let database = configure_pg_command(&mut restore_command, database_url)?;
    let status = tokio::time::timeout(
        std::time::Duration::from_secs(1800),
        restore_command
            .arg("--dbname")
            .arg(database)
            .arg("--exit-on-error")
            .arg("--single-transaction")
            .arg("--no-owner")
            .arg("--no-acl")
            .stdin(Stdio::from(dump))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .status(),
    )
    .await??;
    if !status.success() {
        return Err("pg_restore failed; database transaction rolled back, copied volume files require inspection".into());
    }
    println!(
        "Restore complete: {} objects; run readiness and registry verification before cutover",
        manifest.objects.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn backup_verifier_rejects_corrupt_or_extra_objects() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("objects")).unwrap();
        std::fs::write(dir.path().join(DUMP), b"database").unwrap();
        let name = format!("object-{}", "a".repeat(64));
        std::fs::write(dir.path().join("objects").join(&name), b"object").unwrap();
        let manifest = Manifest {
            format: 1,
            created_unix: 0,
            database_sha256: hex::encode(Sha256::digest(b"database")),
            objects: vec![Object {
                name: name.clone(),
                size: 6,
                sha256: hex::encode(Sha256::digest(b"object")),
            }],
        };
        std::fs::write(
            dir.path().join(MANIFEST),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        assert!(verify(dir.path()).is_ok());
        std::fs::write(dir.path().join("objects").join(&name), b"tamper").unwrap();
        assert!(verify(dir.path()).is_err());
        std::fs::write(dir.path().join("objects").join(&name), b"object").unwrap();
        std::fs::write(dir.path().join("objects/unexpected"), b"x").unwrap();
        assert!(verify(dir.path()).is_err());
    }

    #[test]
    fn postgres_tools_receive_decoded_credentials_without_command_line_secrets() {
        let mut cmd = tokio::process::Command::new("pg_dump");
        let database = configure_pg_command(
            &mut cmd,
            "postgres://user:p%40ss@[::1]:5544/example?sslmode=require",
        )
        .unwrap();
        assert_eq!(database, "example");
        let command = cmd.as_std();
        let env: BTreeMap<_, _> = command
            .get_envs()
            .filter_map(|(k, v)| {
                v.map(|v| {
                    (
                        k.to_string_lossy().into_owned(),
                        v.to_string_lossy().into_owned(),
                    )
                })
            })
            .collect();
        assert_eq!(env["PGHOST"], "::1");
        assert_eq!(env["PGPORT"], "5544");
        assert_eq!(env["PGUSER"], "user");
        assert_eq!(env["PGPASSWORD"], "p@ss");
        assert_eq!(env["PGSSLMODE"], "require");
        assert!(command
            .get_args()
            .all(|arg| !arg.to_string_lossy().contains("p@ss")));
        assert!(configure_pg_command(&mut cmd, "postgres://user@localhost/db?unknown=x").is_err());
        assert!(configure_pg_command(
            &mut cmd,
            "postgres://user@localhost/db?sslmode=require&sslmode=disable"
        )
        .is_err());
    }
}
