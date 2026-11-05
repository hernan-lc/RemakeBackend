//! Single-host immutable filesystem objects for a private, owner-only volume.
//! Used by the local-volume backend on a single application instance.
use crate::{
    valid_key, valid_prepared_key, valid_public_key, ObjectVersion, StorageError, MAX_ARTIFACT_SIZE,
};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::{
    fs::{self, OpenOptions},
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
};
use uuid::Uuid;

#[derive(Clone)]
pub struct LocalStore {
    root: PathBuf,
    // Shared by all clones; prevents a coordinated backup while serving.
    _lock: Arc<std::fs::File>,
}

struct TempObject(PathBuf);
impl Drop for TempObject {
    fn drop(&mut self) {
        // Also runs if an upload times out or its client disconnects.
        let _ = std::fs::remove_file(&self.0);
    }
}

impl LocalStore {
    /// Open an existing private directory; callers own its creation and backup.
    pub fn open(root: &Path) -> Result<Self, StorageError> {
        let meta = std::fs::symlink_metadata(root).map_err(|_| StorageError::Unavailable)?;
        if !meta.is_dir() || meta.file_type().is_symlink() {
            return Err(StorageError::Invalid);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if meta.permissions().mode() & 0o077 != 0 {
                return Err(StorageError::Invalid);
            }
        }
        #[cfg(not(unix))]
        return Err(StorageError::Invalid);
        #[cfg(unix)]
        Ok(Self {
            root: std::fs::canonicalize(root).map_err(|_| StorageError::Unavailable)?,
            _lock: Arc::new(acquire_lock(root, false)?),
        })
    }

    fn object(&self, key: &str) -> Result<PathBuf, StorageError> {
        if !(valid_key(key) || valid_prepared_key(key) || valid_public_key(key)) {
            return Err(StorageError::Invalid);
        }
        // Hash the complete key: flattening separators is ambiguous when
        // accepted UUID spellings themselves contain hyphens.
        Ok(self.root.join(format!(
            "object-{}",
            hex::encode(Sha256::digest(key.as_bytes()))
        )))
    }

    /// Write to a private temporary file, verify bytes, then publish without
    /// replacing any existing object. A crash may leave an unreferenced temp.
    pub async fn put<R: AsyncRead + Unpin>(
        &self,
        key: &str,
        size: i64,
        sha256: &str,
        mut input: R,
    ) -> Result<ObjectVersion, StorageError> {
        let target = self.object(key)?;
        if !(1..=MAX_ARTIFACT_SIZE).contains(&size) || !valid_digest(sha256) {
            return Err(StorageError::Invalid);
        }
        let temp = self.root.join(format!(".tmp-{}", Uuid::new_v4()));
        let result = async {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                options.mode(0o600);
            }
            let mut file = options
                .open(&temp)
                .await
                .map_err(|_| StorageError::Unavailable)?;
            let _cleanup = TempObject(temp.clone());
            let mut digest = Sha256::new();
            let mut total = 0i64;
            let mut buffer = vec![0u8; 64 * 1024];
            loop {
                let n = input
                    .read(&mut buffer)
                    .await
                    .map_err(|_| StorageError::Unavailable)?;
                if n == 0 {
                    break;
                }
                total = total
                    .checked_add(n as i64)
                    .ok_or(StorageError::InvalidObject)?;
                if total > size {
                    return Err(StorageError::InvalidObject);
                }
                digest.update(&buffer[..n]);
                file.write_all(&buffer[..n])
                    .await
                    .map_err(|_| StorageError::Unavailable)?;
            }
            if total != size || hex::encode(digest.finalize()) != sha256 {
                return Err(StorageError::InvalidObject);
            }
            file.sync_all()
                .await
                .map_err(|_| StorageError::Unavailable)?;
            drop(file);
            // Hard links are atomic and never replace a previous object.
            if let Err(error) = fs::hard_link(&temp, &target).await {
                if error.kind() != std::io::ErrorKind::AlreadyExists {
                    return Err(StorageError::Unavailable);
                }
                // A retry is safe only when the existing immutable object is
                // exactly the object promised by this reservation.
                let version = ObjectVersion {
                    version_id: sha256.to_owned(),
                    size,
                };
                self.download(key, &version, tokio::io::sink()).await?;
                return Ok(version);
            }
            let dir = fs::File::open(&self.root)
                .await
                .map_err(|_| StorageError::Unavailable)?;
            dir.sync_all()
                .await
                .map_err(|_| StorageError::Unavailable)?;
            Ok(ObjectVersion {
                version_id: sha256.to_owned(),
                size,
            })
        }
        .await;
        result
    }

    /// Read the exact content-addressed version, checking both size and hash.
    /// On failure callers must discard any bytes already written to the sink.
    pub async fn download<W: AsyncWrite + Unpin>(
        &self,
        key: &str,
        version: &ObjectVersion,
        mut output: W,
    ) -> Result<(), StorageError> {
        let path = self.object(key)?;
        if !(1..=MAX_ARTIFACT_SIZE).contains(&version.size) || !valid_digest(&version.version_id) {
            return Err(StorageError::Invalid);
        }
        let meta = fs::symlink_metadata(&path)
            .await
            .map_err(|_| StorageError::Unavailable)?;
        if !meta.is_file() || meta.file_type().is_symlink() || meta.len() != version.size as u64 {
            return Err(StorageError::InvalidObject);
        }
        let mut input = fs::File::open(path)
            .await
            .map_err(|_| StorageError::Unavailable)?;
        let mut total = 0i64;
        let mut digest = Sha256::new();
        let mut buffer = vec![0u8; 64 * 1024];
        loop {
            let n = input
                .read(&mut buffer)
                .await
                .map_err(|_| StorageError::Unavailable)?;
            if n == 0 {
                break;
            }
            total = total
                .checked_add(n as i64)
                .ok_or(StorageError::InvalidObject)?;
            if total > version.size {
                return Err(StorageError::InvalidObject);
            }
            digest.update(&buffer[..n]);
            output
                .write_all(&buffer[..n])
                .await
                .map_err(|_| StorageError::Unavailable)?;
        }
        output
            .flush()
            .await
            .map_err(|_| StorageError::Unavailable)?;
        if total != version.size || hex::encode(digest.finalize()) != version.version_id {
            return Err(StorageError::InvalidObject);
        }
        Ok(())
    }

    /// Capture the current immutable object identity for a completed upload.
    pub async fn capture(
        &self,
        key: &str,
        expected_size: i64,
    ) -> Result<ObjectVersion, StorageError> {
        let path = self.object(key)?;
        if !(1..=MAX_ARTIFACT_SIZE).contains(&expected_size) {
            return Err(StorageError::Invalid);
        }
        let meta = fs::symlink_metadata(&path)
            .await
            .map_err(|_| StorageError::Unavailable)?;
        if !meta.is_file() || meta.file_type().is_symlink() || meta.len() != expected_size as u64 {
            return Err(StorageError::InvalidObject);
        }
        let mut input = fs::File::open(path)
            .await
            .map_err(|_| StorageError::Unavailable)?;
        let mut total = 0i64;
        let mut digest = Sha256::new();
        let mut buffer = vec![0u8; 64 * 1024];
        loop {
            let n = input
                .read(&mut buffer)
                .await
                .map_err(|_| StorageError::Unavailable)?;
            if n == 0 {
                break;
            }
            total = total
                .checked_add(n as i64)
                .ok_or(StorageError::InvalidObject)?;
            if total > expected_size {
                return Err(StorageError::InvalidObject);
            }
            digest.update(&buffer[..n]);
        }
        if total != expected_size {
            return Err(StorageError::InvalidObject);
        }
        Ok(ObjectVersion {
            version_id: hex::encode(digest.finalize()),
            size: total,
        })
    }

    pub async fn length(&self, key: &str) -> Result<i64, StorageError> {
        let meta = fs::symlink_metadata(self.object(key)?)
            .await
            .map_err(|_| StorageError::Unavailable)?;
        if !meta.is_file()
            || meta.file_type().is_symlink()
            || !(1..=MAX_ARTIFACT_SIZE as u64).contains(&meta.len())
        {
            return Err(StorageError::InvalidObject);
        }
        Ok(meta.len() as i64)
    }

    /// Garbage collection is only safe for keys which will never be written again.
    pub async fn delete_version(&self, key: &str, version: &str) -> Result<(), StorageError> {
        if !valid_digest(version) {
            return Err(StorageError::Invalid);
        }
        let size = self.length(key).await?;
        self.download(
            key,
            &ObjectVersion {
                version_id: version.to_owned(),
                size,
            },
            tokio::io::sink(),
        )
        .await?;
        fs::remove_file(self.object(key)?)
            .await
            .map_err(|_| StorageError::Unavailable)?;
        fs::File::open(&self.root)
            .await
            .map_err(|_| StorageError::Unavailable)?
            .sync_all()
            .await
            .map_err(|_| StorageError::Unavailable)?;
        Ok(())
    }
}

/// Exclusive advisory lock for an offline backup. Every local-storage API or
/// worker process holds a shared lock. Other database writers must be stopped
/// separately. The lock only coordinates processes on the same filesystem.
pub fn backup_lock(root: &Path) -> Result<std::fs::File, StorageError> {
    let meta = std::fs::symlink_metadata(root).map_err(|_| StorageError::Unavailable)?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return Err(StorageError::Invalid);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o077 != 0 {
            return Err(StorageError::Invalid);
        }
        acquire_lock(root, true)
    }
    #[cfg(not(unix))]
    {
        Err(StorageError::Invalid)
    }
}

#[cfg(unix)]
fn acquire_lock(root: &Path, exclusive: bool) -> Result<std::fs::File, StorageError> {
    use std::os::unix::fs::OpenOptionsExt;
    let path = root.join(".cloud-storage.lock");
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(&path)
        .map_err(|_| StorageError::Unavailable)?;
    let metadata = std::fs::symlink_metadata(path).map_err(|_| StorageError::Unavailable)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(StorageError::Invalid);
    }
    if exclusive {
        fs2::FileExt::try_lock_exclusive(&file)
    } else {
        fs2::FileExt::try_lock_shared(&file)
    }
    .map_err(|_| StorageError::Unavailable)?;
    Ok(file)
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[tokio::test]
    async fn immutable_content_is_bounded_verified_and_never_overwritten() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let store = LocalStore::open(root.path()).unwrap();
        let key = format!("quarantine/{}/{}", Uuid::new_v4(), Uuid::new_v4());
        let data = b"test artifact";
        let digest = hex::encode(Sha256::digest(data));
        assert!(store.put(&key, 3, &digest, &data[..]).await.is_err());
        assert!(store
            .put(&key, data.len() as i64, &"0".repeat(64), &data[..])
            .await
            .is_err());
        let version = store
            .put(&key, data.len() as i64, &digest, &data[..])
            .await
            .unwrap();
        assert_eq!(
            store
                .put(&key, data.len() as i64, &digest, &data[..])
                .await
                .unwrap(),
            version
        );
        let other = b"other content";
        let other_digest = hex::encode(Sha256::digest(other));
        assert!(store
            .put(&key, other.len() as i64, &other_digest, &other[..])
            .await
            .is_err());
        let mut out = Vec::new();
        store.download(&key, &version, &mut out).await.unwrap();
        assert_eq!(out, data);
        assert_eq!(
            store.capture(&key, data.len() as i64).await.unwrap(),
            version
        );
        assert!(store.capture(&key, 1).await.is_err());
        std::fs::write(store.object(&key).unwrap(), b"bad artifact!").unwrap();
        assert!(store
            .download(&key, &version, tokio::io::sink())
            .await
            .is_err());
        assert!(store.object("quarantine/../../etc/passwd").is_err());
    }

    #[tokio::test]
    async fn concurrent_different_uploads_cannot_replace_each_other() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let store = LocalStore::open(root.path()).unwrap();
        let key = format!("quarantine/{}/{}", Uuid::new_v4(), Uuid::new_v4());
        let first = b"first";
        let second = b"other";
        let first_hash = hex::encode(Sha256::digest(first));
        let second_hash = hex::encode(Sha256::digest(second));
        let (a, b) = tokio::join!(
            store.put(&key, 5, &first_hash, &first[..]),
            store.put(&key, 5, &second_hash, &second[..]),
        );
        assert_ne!(a.is_ok(), b.is_ok());
        let version = a.or(b).unwrap();
        store
            .download(&key, &version, tokio::io::sink())
            .await
            .unwrap();
    }

    #[test]
    fn backup_lock_excludes_open_stores_and_other_backups() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let store = LocalStore::open(root.path()).unwrap();
        let clone = store.clone();
        assert!(backup_lock(root.path()).is_err());
        drop(store);
        assert!(backup_lock(root.path()).is_err());
        drop(clone);
        let backup = backup_lock(root.path()).unwrap();
        assert!(LocalStore::open(root.path()).is_err());
        assert!(backup_lock(root.path()).is_err());
        drop(backup);
        assert!(LocalStore::open(root.path()).is_ok());
    }
}
