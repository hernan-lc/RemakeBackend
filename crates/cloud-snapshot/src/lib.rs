//! Build a private candidate using the pinned TikTools v1 writer.
//! The caller supplies a complete, independently authorized inventory; this crate
//! neither decides eligibility nor publishes or promotes a candidate.
#![forbid(unsafe_code)]
use cloud_verifier::ExpectedRelease;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use tempfile::TempDir;
use thiserror::Error;
use tiktools_plugin_api::manifest::parse_version;
use tiktools_plugin_package::{
    DetachedSignature, KeyRole, KeysDocument, PackageArchive, PublicKey, Sha256Digest, SigningKey,
    TrustStore, TrustedKey,
};
use tiktools_plugin_registry::{
    Advisory, IndexWriter, RegistryConfigDocument, RootDocument, REGISTRY_PROTOCOL_VERSION,
};

const MAX_DOC: u64 = 8 * 1024 * 1024;
const MAX_RELEASES: usize = 10_000;
const MAX_TOTAL_ARTIFACT_BYTES: u64 = 16 * 1024 * 1024 * 1024;
#[derive(Debug, Error)]
pub enum SnapshotError {
    #[error("invalid or untrusted registry seed")]
    InvalidSeed,
    #[error("invalid or unauthorized release inventory")]
    InvalidInventory,
    #[error("release package verification failed")]
    InvalidPackage,
    #[error("registry writer or output validation failed")]
    Writer,
    #[error("snapshot filesystem operation failed")]
    Io(#[from] io::Error),
}

pub struct ReleaseInput<'a> {
    pub source: &'a Path,
    pub expected: ExpectedRelease<'a>,
    /// Must come from independently approved current Cloud authorization.
    pub approved_key: &'a TrustedKey,
    pub channel: &'a str,
    pub yanked: bool,
    pub advisory: Option<Advisory>,
}
/// A candidate exists only while this handle is alive. It is not a published snapshot.
pub struct CandidateSnapshot {
    staging: TempDir,
    pub files: BTreeMap<String, String>,
}
impl CandidateSnapshot {
    pub fn path(&self) -> &Path {
        self.staging.path()
    }
    /// Recheck the exact candidate files and current delegated-key lifetimes.
    /// This does not establish current Cloud authorization or inventory completeness.
    pub fn revalidate(
        &self,
        root_pin: &TrustedKey,
        index_key: &PublicKey,
        releases: &[(&TrustedKey, &str)],
    ) -> Result<(), SnapshotError> {
        revalidate_tree(self.path(), &self.files, root_pin, index_key, releases)
    }
}

/// Authenticate the supplied candidate files against independent trust pins.
/// The caller must separately establish inventory completeness and authorization.
pub fn revalidate_tree(
    path: &Path,
    files: &BTreeMap<String, String>,
    root_pin: &TrustedKey,
    index_key: &PublicKey,
    releases: &[(&TrustedKey, &str)],
) -> Result<(), SnapshotError> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| SnapshotError::InvalidSeed)?
        .as_secs();
    if root_pin.role != KeyRole::Root || root_pin.revoked || !root_pin.is_valid_at(now) {
        return Err(SnapshotError::InvalidSeed);
    }
    for required in [
        "root.json",
        "keys.json",
        "keys.json.sig",
        "config.json",
        "config.json.sig",
    ] {
        if !files.contains_key(required) {
            return Err(SnapshotError::Writer);
        }
    }
    for (name, digest) in files {
        let relative = Path::new(name);
        if name.contains('\\')
            || relative
                .components()
                .any(|p| !matches!(p, std::path::Component::Normal(_)))
        {
            return Err(SnapshotError::Writer);
        }
        let full = path.join(relative);
        let meta = fs::symlink_metadata(&full)?;
        if !meta.file_type().is_file()
            || meta.len() > cloud_verifier::MAX_COMPRESSED_BYTES
            || Sha256Digest::of_file(&full)?.as_hex() != digest
        {
            return Err(SnapshotError::Writer);
        }
    }
    let root: RootDocument = serde_json::from_slice(&read_regular(path, "root.json")?)
        .map_err(|_| SnapshotError::InvalidSeed)?;
    let mut root_ids = BTreeSet::new();
    if root.version != 1
        || root.keys.len() > 64
        || !root.keys.iter().any(|key| key == &root_pin.key)
        || root
            .keys
            .iter()
            .any(|key| key.raw().is_err() || !root_ids.insert(key.key_id.to_string()))
    {
        return Err(SnapshotError::InvalidSeed);
    }
    let mut trust = TrustStore::new();
    trust.insert(root_pin.clone(), "independent-root-pin");
    let key_bytes = read_regular(path, "keys.json")?;
    signed(
        &trust,
        &key_bytes,
        &read_regular(path, "keys.json.sig")?,
        KeyRole::Root,
        now,
    )?;
    let keys = KeysDocument::parse(&key_bytes).map_err(|_| SnapshotError::InvalidSeed)?;
    let mut ids = BTreeSet::new();
    if keys.is_expired(now)
        || keys.keys.len() > 10_000
        || keys.keys.iter().any(|k| {
            k.role == KeyRole::Root
                || k.key.key_id == root_pin.key.key_id
                || k.key.raw().is_err()
                || !ids.insert(k.key.key_id.to_string())
        })
    {
        return Err(SnapshotError::InvalidSeed);
    }
    trust.extend_delegations(&keys, "root-signed-delegations");
    let index = trust
        .get(&index_key.key_id)
        .ok_or(SnapshotError::InvalidSeed)?;
    if index.role != KeyRole::Index
        || index.revoked
        || !index.is_valid_at(now)
        || index.key != *index_key
    {
        return Err(SnapshotError::InvalidSeed);
    }
    let config_bytes = read_regular(path, "config.json")?;
    signed_by(
        &trust,
        &config_bytes,
        &read_regular(path, "config.json.sig")?,
        &index_key.key_id,
        now,
    )?;
    let config: RegistryConfigDocument =
        serde_json::from_slice(&config_bytes).map_err(|_| SnapshotError::InvalidSeed)?;
    if config.protocol != REGISTRY_PROTOCOL_VERSION
        || config.name.trim().is_empty()
        || config.catalog_pages > 64
        || config.expires.is_some_and(|expiry| now > expiry)
    {
        return Err(SnapshotError::InvalidSeed);
    }
    for (approved, id) in releases {
        let key = trust
            .get(&approved.key.key_id)
            .ok_or(SnapshotError::InvalidInventory)?;
        if approved.role != KeyRole::Publisher
            || approved.revoked
            || !approved.is_valid_at(now)
            || key.role != KeyRole::Publisher
            || key.revoked
            || !key.is_valid_at(now)
            || key.key != approved.key
            || key.publisher != approved.publisher
            || !key.allows_package(id)
            || !approved.allows_package(id)
        {
            return Err(SnapshotError::InvalidInventory);
        }
    }
    for name in files
        .keys()
        .filter(|name| name.ends_with(".json") && *name != "root.json" && *name != "keys.json")
    {
        let signature = format!("{name}.sig");
        if !files.contains_key(&signature) {
            return Err(SnapshotError::Writer);
        }
        signed_by(
            &trust,
            &read_regular(path, name)?,
            &read_regular(path, &signature)?,
            &index_key.key_id,
            now,
        )
        .map_err(|_| SnapshotError::Writer)?;
    }
    Ok(())
}

fn read_regular(base: &Path, name: &str) -> Result<Vec<u8>, SnapshotError> {
    let path = base.join(name);
    let metadata = fs::symlink_metadata(&path)?;
    if !metadata.file_type().is_file() || metadata.len() > MAX_DOC {
        return Err(SnapshotError::InvalidSeed);
    }
    let mut data = Vec::new();
    fs::File::open(path)?
        .take(MAX_DOC + 1)
        .read_to_end(&mut data)?;
    if data.len() as u64 > MAX_DOC {
        return Err(SnapshotError::InvalidSeed);
    }
    Ok(data)
}
fn signed(
    trust: &TrustStore,
    bytes: &[u8],
    signature: &[u8],
    role: KeyRole,
    now: u64,
) -> Result<(), SnapshotError> {
    let sig = DetachedSignature::parse(signature).map_err(|_| SnapshotError::InvalidSeed)?;
    trust
        .verify_detached(&sig, bytes, role, now)
        .map_err(|_| SnapshotError::InvalidSeed)?;
    Ok(())
}
fn signed_by(
    trust: &TrustStore,
    bytes: &[u8],
    signature: &[u8],
    key_id: &tiktools_plugin_package::KeyId,
    now: u64,
) -> Result<(), SnapshotError> {
    let parsed = DetachedSignature::parse(signature).map_err(|_| SnapshotError::InvalidSeed)?;
    if &parsed.key_id != key_id {
        return Err(SnapshotError::InvalidSeed);
    }
    trust
        .verify_detached(&parsed, bytes, KeyRole::Index, now)
        .map_err(|_| SnapshotError::InvalidSeed)?;
    Ok(())
}
fn valid_advisory(advisory: &Option<Advisory>) -> bool {
    advisory.as_ref().is_none_or(|a| {
        !a.message.trim().is_empty()
            && a.message.len() <= 1024
            && !a.message.chars().any(char::is_control)
            && a.url.as_ref().is_none_or(|s| {
                url::Url::parse(s).ok().is_some_and(|u| {
                    u.scheme() == "https"
                        && u.host().is_some()
                        && u.username().is_empty()
                        && u.password().is_none()
                        && u.fragment().is_none()
                })
            })
    })
}
/// `seed` supplies root.json, root-signed keys and index-signed config only.
/// `root_pin` is obtained independently; it must never be read from the seed as trust.
/// Every input key and release must already be authorized and the inventory complete.
pub fn build(
    seed: &Path,
    root_pin: &TrustedKey,
    index_key: SigningKey,
    releases: &[ReleaseInput<'_>],
) -> Result<CandidateSnapshot, SnapshotError> {
    if releases.len() > MAX_RELEASES || root_pin.role != KeyRole::Root || root_pin.revoked {
        return Err(SnapshotError::InvalidInventory);
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| SnapshotError::InvalidSeed)?
        .as_secs();
    if !root_pin.is_valid_at(now) {
        return Err(SnapshotError::InvalidSeed);
    }
    let mut originals = BTreeMap::new();
    for name in [
        "root.json",
        "keys.json",
        "keys.json.sig",
        "config.json",
        "config.json.sig",
    ] {
        originals.insert(name, read_regular(seed, name)?);
    }
    let root: RootDocument =
        serde_json::from_slice(&originals["root.json"]).map_err(|_| SnapshotError::InvalidSeed)?;
    let mut root_ids = BTreeSet::new();
    if root.version != 1
        || root.keys.len() > 64
        || !root.keys.iter().any(|key| key == &root_pin.key)
        || root
            .keys
            .iter()
            .any(|key| key.raw().is_err() || !root_ids.insert(key.key_id.to_string()))
    {
        return Err(SnapshotError::InvalidSeed);
    }
    let mut trust = TrustStore::new();
    trust.insert(root_pin.clone(), "independent-root-pin");
    signed(
        &trust,
        &originals["keys.json"],
        &originals["keys.json.sig"],
        KeyRole::Root,
        now,
    )?;
    let keys =
        KeysDocument::parse(&originals["keys.json"]).map_err(|_| SnapshotError::InvalidSeed)?;
    if keys.is_expired(now) || keys.keys.len() > 10_000 {
        return Err(SnapshotError::InvalidSeed);
    }
    let mut ids = BTreeSet::new();
    for key in &keys.keys {
        if key.role == KeyRole::Root
            || key.key.key_id == root_pin.key.key_id
            || key.key.raw().is_err()
            || !ids.insert(key.key.key_id.to_string())
        {
            return Err(SnapshotError::InvalidSeed);
        }
    }
    trust.extend_delegations(&keys, "root-signed-delegations");
    signed(
        &trust,
        &originals["config.json"],
        &originals["config.json.sig"],
        KeyRole::Index,
        now,
    )?;
    let config: RegistryConfigDocument = serde_json::from_slice(&originals["config.json"])
        .map_err(|_| SnapshotError::InvalidSeed)?;
    if config.protocol != REGISTRY_PROTOCOL_VERSION
        || config.name.trim().is_empty()
        || config.expires.is_some_and(|until| now > until)
    {
        return Err(SnapshotError::InvalidSeed);
    }
    let signer = index_key.public_key();
    let delegated = trust
        .get(&signer.key_id)
        .ok_or(SnapshotError::InvalidSeed)?;
    if delegated.role != KeyRole::Index
        || delegated.revoked
        || !delegated.is_valid_at(now)
        || delegated.key != signer
    {
        return Err(SnapshotError::InvalidSeed);
    }
    let mut identities = BTreeSet::new();
    let mut plugin_ids = BTreeSet::new();
    let mut release_policy: BTreeMap<(String, String), (String, bool, Option<Advisory>)> =
        BTreeMap::new();
    let mut total_size = 0_u64;
    for input in releases {
        let metadata =
            fs::symlink_metadata(input.source).map_err(|_| SnapshotError::InvalidInventory)?;
        if config
            .revoked_digests
            .iter()
            .any(|digest| digest == input.expected.sha256)
            || !matches!(input.channel, "stable" | "beta")
            || input.expected.namespace.len() > 63
            || input.expected.plugin_id.len() > 256
            || input.expected.version.len() > 128
            || input.expected.target.len() > 128
            || input.expected.sha256.len() != 64
            || !valid_advisory(&input.advisory)
            || !metadata.file_type().is_file()
            || metadata.len() == 0
            || metadata.len() > cloud_verifier::MAX_COMPRESSED_BYTES
        {
            return Err(SnapshotError::InvalidInventory);
        }
        total_size = total_size
            .checked_add(metadata.len())
            .ok_or(SnapshotError::InvalidInventory)?;
        if total_size > MAX_TOTAL_ARTIFACT_BYTES {
            return Err(SnapshotError::InvalidInventory);
        }
        let version =
            parse_version(input.expected.version).ok_or(SnapshotError::InvalidInventory)?;
        if version.to_string() != input.expected.version
            || (input.channel == "stable" && !version.pre.is_empty())
        {
            return Err(SnapshotError::InvalidInventory);
        }
        plugin_ids.insert(input.expected.plugin_id);
        if plugin_ids.len() > 6400 {
            return Err(SnapshotError::InvalidInventory);
        }
        let identity = (
            input.expected.plugin_id.to_owned(),
            input.expected.version.to_owned(),
            input.expected.target.to_owned(),
        );
        if !identities.insert(identity) {
            return Err(SnapshotError::InvalidInventory);
        }
        let policy_key = (
            input.expected.plugin_id.to_owned(),
            input.expected.version.to_owned(),
        );
        let policy = (
            input.channel.to_owned(),
            input.yanked,
            input.advisory.clone(),
        );
        if release_policy
            .get(&policy_key)
            .is_some_and(|old| old != &policy)
        {
            return Err(SnapshotError::InvalidInventory);
        }
        release_policy.insert(policy_key, policy);
        let approved = input.approved_key;
        let delegated = trust
            .get(&approved.key.key_id)
            .ok_or(SnapshotError::InvalidInventory)?;
        if approved.role != KeyRole::Publisher
            || approved.revoked
            || !approved.is_valid_at(now)
            || delegated.role != KeyRole::Publisher
            || delegated.revoked
            || !delegated.is_valid_at(now)
            || delegated.key != approved.key
            || delegated.publisher != approved.publisher
            || !delegated.allows_package(input.expected.plugin_id)
            || !approved.allows_package(input.expected.plugin_id)
        {
            return Err(SnapshotError::InvalidInventory);
        }
    }
    let staging = tempfile::Builder::new()
        .prefix("cloud-snapshot-candidate-")
        .tempdir()?;
    for (name, bytes) in &originals {
        fs::write(staging.path().join(name), bytes)?;
    }
    let writer = IndexWriter::open(staging.path(), &config.name, Some(index_key));
    let mut artifacts = BTreeMap::new();
    let mut verified_total = 0_u64;
    let mut ordered: Vec<_> = releases.iter().collect();
    ordered.sort_by(|a, b| {
        (a.expected.plugin_id, a.expected.version, a.expected.target).cmp(&(
            b.expected.plugin_id,
            b.expected.version,
            b.expected.target,
        ))
    });
    for input in ordered {
        let expected = ExpectedRelease {
            namespace: input.expected.namespace,
            plugin_id: input.expected.plugin_id,
            version: input.expected.version,
            target: input.expected.target,
            sha256: input.expected.sha256,
        };
        let artifact = cloud_verifier::verify(input.source, expected, input.approved_key)
            .map_err(|_| SnapshotError::InvalidPackage)?;
        // The pinned devkit's publish-readiness helper is private. Mirror its
        // non-changelog checks here until upstream exposes a reusable API.
        let inspected = PackageArchive::open(artifact.path())
            .and_then(|archive| archive.inspect())
            .map_err(|_| SnapshotError::InvalidPackage)?;
        let manifest = &inspected.manifest;
        if manifest.metadata.license.is_none()
            || manifest.metadata.engine.is_none()
            || manifest.metadata.categories.is_empty()
            || manifest.description.as_deref().is_none_or(str::is_empty)
        {
            return Err(SnapshotError::InvalidPackage);
        }
        verified_total = verified_total
            .checked_add(artifact.size)
            .ok_or(SnapshotError::InvalidInventory)?;
        if verified_total > MAX_TOTAL_ARTIFACT_BYTES {
            return Err(SnapshotError::InvalidInventory);
        }
        let result = writer
            .add(&artifact.path(), input.channel, None)
            .map_err(|_| SnapshotError::Writer)?;
        if result.id != input.expected.plugin_id
            || result.version != input.expected.version
            || result.target != input.expected.target
            || result.sha256 != artifact.sha256
            || result.size != artifact.size
            || result.key_id.as_deref() != Some(artifact.publisher_key_id.as_str())
        {
            return Err(SnapshotError::Writer);
        }
        let relative = Path::new(&result.url);
        if !result.url.starts_with("packages/")
            || result.url.contains('\\')
            || relative
                .components()
                .any(|part| !matches!(part, std::path::Component::Normal(_)))
        {
            return Err(SnapshotError::Writer);
        }
        if artifacts
            .insert(result.url.clone(), artifact.sha256.clone())
            .is_some()
        {
            return Err(SnapshotError::Writer);
        }
        let dest = staging.path().join(relative);
        if Sha256Digest::of_file(&dest)
            .map_err(SnapshotError::Io)?
            .as_hex()
            != artifact.sha256
        {
            return Err(SnapshotError::Writer);
        }
    }
    for ((id, version), (_, yanked, advisory)) in &release_policy {
        if *yanked {
            writer
                .yank(id, version, true)
                .map_err(|_| SnapshotError::Writer)?;
        }
        if let Some(advisory) = advisory {
            writer
                .advise(id, version, Some(advisory.clone()))
                .map_err(|_| SnapshotError::Writer)?;
        }
    }
    writer.rebuild().map_err(|_| SnapshotError::Writer)?;
    let mut files = BTreeMap::new();
    let mut todo = vec![PathBuf::new()];
    while let Some(relative) = todo.pop() {
        for entry in fs::read_dir(staging.path().join(&relative))? {
            let entry = entry?;
            let relative = relative.join(entry.file_name());
            let kind = entry.file_type()?;
            if kind.is_dir() {
                todo.push(relative);
                continue;
            }
            if !kind.is_file() {
                return Err(SnapshotError::Writer);
            }
            let name = relative
                .to_str()
                .ok_or(SnapshotError::Writer)?
                .replace('\\', "/");
            if name.ends_with(".json") && name != "root.json" && name != "keys.json" {
                let bytes =
                    read_regular(staging.path(), &name).map_err(|_| SnapshotError::Writer)?;
                let signature = read_regular(staging.path(), &format!("{name}.sig"))
                    .map_err(|_| SnapshotError::Writer)?;
                signed_by(&trust, &bytes, &signature, &signer.key_id, now)
                    .map_err(|_| SnapshotError::Writer)?;
            }
            if matches!(name.as_str(), "root.json" | "keys.json" | "keys.json.sig")
                && read_regular(staging.path(), &name).map_err(|_| SnapshotError::Writer)?
                    != originals[name.as_str()]
            {
                return Err(SnapshotError::Writer);
            }
            let hash = Sha256Digest::of_file(&entry.path())?.to_string();
            if name.starts_with("packages/")
                && artifacts.remove(&name).as_deref() != Some(hash.as_str())
            {
                return Err(SnapshotError::Writer);
            }
            if files.insert(name, hash).is_some() || files.len() > MAX_RELEASES * 4 + 1000 {
                return Err(SnapshotError::Writer);
            }
        }
    }
    if !artifacts.is_empty() {
        return Err(SnapshotError::Writer);
    }
    // A long build must not succeed after an input delegation or document expires.
    let finished = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| SnapshotError::InvalidSeed)?
        .as_secs();
    if finished < now
        || !root_pin.is_valid_at(finished)
        || keys.is_expired(finished)
        || config.expires.is_some_and(|until| finished > until)
        || !delegated.is_valid_at(finished)
        || releases.iter().any(|input| {
            !input.approved_key.is_valid_at(finished)
                || trust
                    .get(&input.approved_key.key.key_id)
                    .is_none_or(|key| !key.is_valid_at(finished))
        })
    {
        return Err(SnapshotError::InvalidSeed);
    }
    signed(
        &trust,
        &originals["keys.json"],
        &originals["keys.json.sig"],
        KeyRole::Root,
        finished,
    )?;
    signed(
        &trust,
        &originals["config.json"],
        &originals["config.json.sig"],
        KeyRole::Index,
        finished,
    )?;
    Ok(CandidateSnapshot { staging, files })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{atomic::AtomicBool, Arc, RwLock};
    use tiktools_plugin_package::{pack, sign_archive, KeyId, PackRequest, PackageArchive};
    use tiktools_plugin_registry::{
        join_url, AdvisorySeverity, DownloadRequest, Downloader, RegistryClient, RegistryError,
        RegistrySource,
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    struct Fixture {
        _temp: TempDir,
        seed: PathBuf,
        package: PathBuf,
        digest: String,
        root: TrustedKey,
        index_json: Vec<u8>,
        publisher: TrustedKey,
        publisher_json: Vec<u8>,
    }
    impl Fixture {
        fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let seed = temp.path().join("seed");
            let root = SigningKey::generate(KeyId::parse("root-one").unwrap(), None).unwrap();
            let index = SigningKey::generate(KeyId::parse("index-one").unwrap(), None).unwrap();
            let publisher =
                SigningKey::generate(KeyId::parse("acme-one").unwrap(), Some("acme".into()))
                    .unwrap();
            let publisher_json = publisher.to_json();
            let root_pin = TrustedKey::new(root.public_key(), KeyRole::Root);
            let pub_key =
                TrustedKey::new(publisher.public_key(), KeyRole::Publisher).publisher("acme");
            let index_json = index.to_json();
            IndexWriter::open(&seed, "fixture", Some(index))
                .init(
                    &root,
                    vec![
                        TrustedKey::new(
                            SigningKey::from_json(&index_json).unwrap().public_key(),
                            KeyRole::Index,
                        ),
                        pub_key.clone(),
                    ],
                    None,
                    None,
                )
                .unwrap();
            let manifest = temp.path().join("plugin.json");
            fs::write(&manifest,serde_json::to_vec(&serde_json::json!({"schemaVersion":2,"id":"acme.plugin","name":"Test","version":"1.0.0","runtime":"process","entry":"index.js","protocolVersion":1,"publisher":"acme","description":"A test plugin","license":"MIT","engines":{"tiktools":">=0.4.0"},"categories":["audio"]})).unwrap()).unwrap();
            let entry = temp.path().join("entry");
            fs::write(&entry, b"plugin content").unwrap();
            let package = temp.path().join("sample.plugin");
            pack(&PackRequest {
                manifest,
                entry: Some(entry),
                output: package.clone(),
                target: None,
            })
            .unwrap();
            sign_archive(&package, &publisher).unwrap();
            let digest = Sha256Digest::of_file(&package).unwrap().to_string();
            Self {
                _temp: temp,
                seed,
                package,
                digest,
                root: root_pin,
                index_json,
                publisher: pub_key,
                publisher_json,
            }
        }
        fn index(&self) -> SigningKey {
            SigningKey::from_json(&self.index_json).unwrap()
        }
        fn input(&self) -> ReleaseInput<'_> {
            ReleaseInput {
                source: &self.package,
                expected: ExpectedRelease {
                    namespace: "acme",
                    plugin_id: "acme.plugin",
                    version: "1.0.0",
                    target: "any",
                    sha256: &self.digest,
                },
                approved_key: &self.publisher,
                channel: "stable",
                yanked: false,
                advisory: None,
            }
        }
    }

    #[test]
    fn official_legacy_release_requires_independent_root_delegation() {
        let dir = tempfile::tempdir().unwrap();
        let root = SigningKey::generate(KeyId::parse("root-legacy").unwrap(), None).unwrap();
        let index = SigningKey::generate(KeyId::parse("index-legacy").unwrap(), None).unwrap();
        let publisher =
            SigningKey::generate(KeyId::parse("pub-legacy").unwrap(), Some("tiktools".into()))
                .unwrap();
        let approved = cloud_verifier::approved_publisher_key(publisher.public_key(), "tiktools");
        let unscoped =
            TrustedKey::new(publisher.public_key(), KeyRole::Publisher).publisher("tiktools");
        let seed = dir.path().join("scoped");
        IndexWriter::open(
            &seed,
            "fixture",
            Some(SigningKey::from_json(&index.to_json()).unwrap()),
        )
        .init(
            &root,
            vec![
                TrustedKey::new(index.public_key(), KeyRole::Index),
                approved.clone(),
            ],
            None,
            None,
        )
        .unwrap();
        let unscoped_seed = dir.path().join("unscoped");
        IndexWriter::open(
            &unscoped_seed,
            "fixture",
            Some(SigningKey::from_json(&index.to_json()).unwrap()),
        )
        .init(
            &root,
            vec![
                TrustedKey::new(index.public_key(), KeyRole::Index),
                unscoped,
            ],
            None,
            None,
        )
        .unwrap();
        let manifest = dir.path().join("plugin.json");
        fs::write(&manifest, serde_json::to_vec(&serde_json::json!({"schemaVersion":2,"id":"hotkeys","name":"Hotkeys","version":"1.0.0","runtime":"process","entry":"index.js","protocolVersion":1,"publisher":"tiktools","description":"Test plugin","license":"MIT","engines":{"tiktools":">=0.3.3"},"categories":["utilities"]})).unwrap()).unwrap();
        let entry = dir.path().join("entry");
        fs::write(&entry, b"test").unwrap();
        let package = dir.path().join("test.plugin");
        pack(&PackRequest {
            manifest,
            entry: Some(entry),
            output: package.clone(),
            target: None,
        })
        .unwrap();
        sign_archive(&package, &publisher).unwrap();
        let digest = Sha256Digest::of_file(&package).unwrap().to_string();
        let input = || ReleaseInput {
            source: &package,
            expected: ExpectedRelease {
                namespace: "tiktools",
                plugin_id: "hotkeys",
                version: "1.0.0",
                target: "any",
                sha256: &digest,
            },
            approved_key: &approved,
            channel: "stable",
            yanked: false,
            advisory: None,
        };
        let pin = TrustedKey::new(root.public_key(), KeyRole::Root);
        assert!(build(
            &unscoped_seed,
            &pin,
            SigningKey::from_json(&index.to_json()).unwrap(),
            &[input()]
        )
        .is_err());
        let candidate = build(
            &seed,
            &pin,
            SigningKey::from_json(&index.to_json()).unwrap(),
            &[input()],
        )
        .unwrap();
        assert!(candidate
            .revalidate(&pin, &index.public_key(), &[(&approved, "hotkeys")])
            .is_ok());
    }
    #[test]
    fn refuses_untrusted_inputs_and_duplicate_release() {
        let f = Fixture::new();
        let wrong_root = TrustedKey::new(
            SigningKey::generate(KeyId::parse("wrong-root").unwrap(), None)
                .unwrap()
                .public_key(),
            KeyRole::Root,
        );
        assert!(matches!(
            build(&f.seed, &wrong_root, f.index(), &[f.input()]),
            Err(SnapshotError::InvalidSeed)
        ));
        let wrong_index = SigningKey::generate(KeyId::parse("wrong-index").unwrap(), None).unwrap();
        assert!(matches!(
            build(&f.seed, &f.root, wrong_index, &[f.input()]),
            Err(SnapshotError::InvalidSeed)
        ));
        assert!(matches!(
            build(&f.seed, &f.root, f.index(), &[f.input(), f.input()]),
            Err(SnapshotError::InvalidInventory)
        ));
        let oversized = f._temp.path().join("oversized.plugin");
        fs::File::create(&oversized)
            .unwrap()
            .set_len(cloud_verifier::MAX_COMPRESSED_BYTES + 1)
            .unwrap();
        let mut bad = f.input();
        bad.source = &oversized;
        assert!(matches!(
            build(&f.seed, &f.root, f.index(), &[bad]),
            Err(SnapshotError::InvalidInventory)
        ));
        let wrong_digest = "a".repeat(64);
        let mut bad = f.input();
        bad.expected.sha256 = &wrong_digest;
        assert!(matches!(
            build(&f.seed, &f.root, f.index(), &[bad]),
            Err(SnapshotError::InvalidPackage)
        ));
        let fake = TrustedKey::new(
            SigningKey::generate(KeyId::parse("acme-one").unwrap(), Some("acme".into()))
                .unwrap()
                .public_key(),
            KeyRole::Publisher,
        )
        .publisher("acme");
        let mut bad = f.input();
        bad.approved_key = &fake;
        assert!(matches!(
            build(&f.seed, &f.root, f.index(), &[bad]),
            Err(SnapshotError::InvalidInventory)
        ));
        // A correctly signed package still needs the pinned devkit's publish metadata.
        let incomplete_manifest = f._temp.path().join("incomplete.json");
        fs::write(&incomplete_manifest,serde_json::to_vec(&serde_json::json!({"schemaVersion":2,"id":"acme.plugin","name":"Test","version":"1.0.0","runtime":"process","entry":"index.js","protocolVersion":1,"publisher":"acme"})).unwrap()).unwrap();
        let incomplete = f._temp.path().join("incomplete.plugin");
        pack(&PackRequest {
            manifest: incomplete_manifest,
            entry: Some(f._temp.path().join("entry")),
            output: incomplete.clone(),
            target: None,
        })
        .unwrap();
        sign_archive(
            &incomplete,
            &SigningKey::from_json(&f.publisher_json).unwrap(),
        )
        .unwrap();
        let incomplete_digest = Sha256Digest::of_file(&incomplete).unwrap().to_string();
        let mut bad = f.input();
        bad.source = &incomplete;
        bad.expected.sha256 = &incomplete_digest;
        assert!(matches!(
            build(&f.seed, &f.root, f.index(), &[bad]),
            Err(SnapshotError::InvalidPackage)
        ));
        let forged_path = f._temp.path().join("forged.plugin");
        fs::copy(&f.package, &forged_path).unwrap();
        let forged_signer =
            SigningKey::generate(KeyId::parse("acme-one").unwrap(), Some("acme".into())).unwrap();
        sign_archive(&forged_path, &forged_signer).unwrap();
        let forged_digest = Sha256Digest::of_file(&forged_path).unwrap().to_string();
        let mut bad = f.input();
        bad.source = &forged_path;
        bad.expected.sha256 = &forged_digest;
        assert!(matches!(
            build(&f.seed, &f.root, f.index(), &[bad]),
            Err(SnapshotError::InvalidPackage)
        ));
        let mut expired_root = f.root.clone();
        expired_root.not_after = Some(1);
        assert!(matches!(
            build(&f.seed, &expired_root, f.index(), &[f.input()]),
            Err(SnapshotError::InvalidSeed)
        ));
        let config_path = f.seed.join("config.json");
        let original_config = fs::read(&config_path).unwrap();
        let mut corrupt_config = original_config.clone();
        corrupt_config.push(b' ');
        fs::write(&config_path, corrupt_config).unwrap();
        assert!(matches!(
            build(&f.seed, &f.root, f.index(), &[f.input()]),
            Err(SnapshotError::InvalidSeed)
        ));
        fs::write(config_path, original_config).unwrap();
        let mut revoked = f.publisher.clone();
        revoked.revoked = true;
        let mut bad = f.input();
        bad.approved_key = &revoked;
        assert!(matches!(
            build(&f.seed, &f.root, f.index(), &[bad]),
            Err(SnapshotError::InvalidInventory)
        ));
        let mut bad = f.input();
        bad.advisory = Some(Advisory {
            severity: AdvisorySeverity::High,
            message: "danger".into(),
            url: Some("http://untrusted.test".into()),
        });
        assert!(matches!(
            build(&f.seed, &f.root, f.index(), &[bad]),
            Err(SnapshotError::InvalidInventory)
        ));
        let path = f.seed.join("keys.json");
        let mut tampered = fs::read(&path).unwrap();
        tampered.push(b' ');
        fs::write(path, tampered).unwrap();
        assert!(matches!(
            build(&f.seed, &f.root, f.index(), &[f.input()]),
            Err(SnapshotError::InvalidSeed)
        ));
    }
    #[cfg(unix)]
    #[test]
    fn rejects_symlinked_seed_documents() {
        use std::os::unix::fs::symlink;
        let f = Fixture::new();
        let original = f.seed.join("keys.json");
        let moved = f.seed.join("other.json");
        fs::rename(&original, &moved).unwrap();
        symlink(&moved, &original).unwrap();
        assert!(matches!(
            build(&f.seed, &f.root, f.index(), &[f.input()]),
            Err(SnapshotError::InvalidSeed)
        ));
    }
    #[test]
    fn rejects_revoked_inventory_before_building() {
        let f = Fixture::new();
        let digest = Sha256Digest::parse(f.input().expected.sha256).unwrap();
        IndexWriter::open(&f.seed, "fixture", Some(f.index()))
            .revoke_digest(&digest)
            .unwrap();
        assert!(matches!(
            build(&f.seed, &f.root, f.index(), &[f.input()]),
            Err(SnapshotError::InvalidInventory)
        ));
    }
    #[tokio::test]
    async fn candidate_is_verified_by_pinned_tiktools_client() {
        let f = Fixture::new();
        let revoked_digest = Sha256Digest::of_bytes(b"revoked artifact");
        IndexWriter::open(&f.seed, "fixture", Some(f.index()))
            .revoke_digest(&revoked_digest)
            .unwrap();
        let mut input = f.input();
        input.yanked = true;
        input.advisory = Some(Advisory {
            severity: AdvisorySeverity::High,
            message: "Review this version".into(),
            url: Some("https://example.test/advisory".into()),
        });
        let manifest = f._temp.path().join("second.json");
        fs::write(&manifest,serde_json::to_vec(&serde_json::json!({"schemaVersion":2,"id":"acme.other","name":"Other","version":"2.0.0","runtime":"process","entry":"index.js","protocolVersion":1,"publisher":"acme","description":"A test plugin","license":"MIT","engines":{"tiktools":">=0.4.0"},"categories":["audio"]})).unwrap()).unwrap();
        let entry = f._temp.path().join("second-entry");
        fs::write(&entry, b"second plugin").unwrap();
        let second = f._temp.path().join("second.plugin");
        pack(&PackRequest {
            manifest,
            entry: Some(entry),
            output: second.clone(),
            target: None,
        })
        .unwrap();
        sign_archive(&second, &SigningKey::from_json(&f.publisher_json).unwrap()).unwrap();
        let second_digest = Sha256Digest::of_file(&second).unwrap().to_string();
        let second_input = ReleaseInput {
            source: &second,
            expected: ExpectedRelease {
                namespace: "acme",
                plugin_id: "acme.other",
                version: "2.0.0",
                target: "any",
                sha256: &second_digest,
            },
            approved_key: &f.publisher,
            channel: "stable",
            yanked: false,
            advisory: None,
        };
        let candidate = build(&f.seed, &f.root, f.index(), &[input, second_input]).unwrap();
        assert!(candidate.files.contains_key("keys.json.sig"));
        candidate
            .revalidate(
                &f.root,
                &f.index().public_key(),
                &[(&f.publisher, "acme.plugin"), (&f.publisher, "acme.other")],
            )
            .unwrap();
        let wrong_index = SigningKey::generate(KeyId::parse("wrong-index").unwrap(), None)
            .unwrap()
            .public_key();
        assert!(matches!(
            candidate.revalidate(&f.root, &wrong_index, &[(&f.publisher, "acme.plugin")]),
            Err(SnapshotError::InvalidSeed)
        ));
        let config: RegistryConfigDocument =
            serde_json::from_slice(&fs::read(candidate.path().join("config.json")).unwrap())
                .unwrap();
        assert!(config.revoked_digests.contains(&revoked_digest.to_string()));
        let private: serde_json::Value = serde_json::from_slice(&f.index_json).unwrap();
        let secret = private["privateKey"].as_str().unwrap();
        for name in candidate
            .files
            .keys()
            .filter(|name| name.ends_with(".json") || name.ends_with(".sig"))
        {
            assert!(
                !String::from_utf8_lossy(&fs::read(candidate.path().join(name)).unwrap())
                    .contains(secret)
            );
        }
        for (name, hash) in &candidate.files {
            assert_eq!(
                Sha256Digest::of_file(&candidate.path().join(name))
                    .unwrap()
                    .as_hex(),
                hash
            );
        }
        let root = candidate.path().to_path_buf();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let root = root.clone();
                tokio::spawn(async move {
                    let mut request_bytes = Vec::new();
                    let mut buffer = [0u8; 1024];
                    loop {
                        let Ok(n) = socket.read(&mut buffer).await else {
                            return;
                        };
                        if n == 0 || request_bytes.len() + n > 16384 {
                            return;
                        }
                        request_bytes.extend_from_slice(&buffer[..n]);
                        if request_bytes.windows(4).any(|part| part == b"\r\n\r\n") {
                            break;
                        }
                    }
                    let request = String::from_utf8_lossy(&request_bytes);
                    let path = request
                        .lines()
                        .next()
                        .and_then(|line| line.split_whitespace().nth(1))
                        .unwrap_or("")
                        .split('?')
                        .next()
                        .unwrap_or("")
                        .trim_start_matches('/');
                    if path.split('/').any(|part| part == ".." || part == ".") {
                        return;
                    }
                    let (status, body) = match fs::read(root.join(path)) {
                        Ok(body) => ("200 OK", body),
                        Err(_) => ("404 Not Found", Vec::new()),
                    };
                    let header = format!(
                        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = socket.write_all(header.as_bytes()).await;
                    let _ = socket.write_all(&body).await;
                });
            }
        });
        let scratch = tempfile::tempdir().unwrap();
        let client = |name: &str, key| {
            let mut source = RegistrySource::new("fixture", &base).unwrap();
            source.root_keys = vec![key];
            RegistryClient::new(
                scratch.path().join(name),
                scratch.path().join(format!("{name}.json")),
                vec![source],
                Arc::new(RwLock::new(TrustStore::new())),
            )
            .unwrap()
        };
        let registry = client("valid", f.root.key.clone());
        assert!(registry.refresh("fixture").await.unwrap().verified);
        let catalog = registry.catalog_page("fixture", 1).await.unwrap();
        assert!(catalog.entries.iter().any(|entry| entry.id == "acme.other"));
        assert!(!catalog
            .entries
            .iter()
            .any(|entry| entry.id == "acme.plugin"));
        let (_, second_document) = registry.package("acme.other", None).await.unwrap();
        assert_eq!(second_document.releases[0].version, "2.0.0");
        assert!(!second_document.releases[0].yanked);
        let (source, document) = registry.package("acme.plugin", None).await.unwrap();
        let release = &document.releases[0];
        assert!(release.yanked);
        assert_eq!(
            release.advisory.as_ref().unwrap().severity,
            AdvisorySeverity::High
        );
        let artifact = &release.artifacts[0];
        let url = join_url(&source.url, &artifact.url).unwrap();
        let downloaded = Downloader::new(scratch.path().join("downloads"))
            .unwrap()
            .fetch(
                DownloadRequest {
                    url: &url,
                    expected_digest: Some(&artifact.sha256),
                    expected_size: Some(artifact.size),
                },
                &|_, _| {},
                &AtomicBool::new(false),
            )
            .await
            .unwrap();
        let archive = PackageArchive::open(&downloaded.path).unwrap();
        let inspection = archive.inspect().unwrap();
        registry
            .trust()
            .read()
            .unwrap()
            .verify_package(
                inspection.signature.as_ref().unwrap(),
                inspection.checksums.as_ref().unwrap(),
                "acme.plugin",
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
            )
            .unwrap();
        archive.extract(&scratch.path().join("extracted")).unwrap();
        let wrong = SigningKey::generate(KeyId::parse("wrong-root").unwrap(), None).unwrap();
        assert!(matches!(
            client("wrong", wrong.public_key()).refresh("fixture").await,
            Err(RegistryError::Signature { .. })
        ));
        let path = candidate.path().join("index/ac/me/acme.plugin.json");
        let mut changed = fs::read(&path).unwrap();
        changed.push(b' ');
        fs::write(path, changed).unwrap();
        assert!(matches!(
            candidate.revalidate(
                &f.root,
                &f.index().public_key(),
                &[(&f.publisher, "acme.plugin")]
            ),
            Err(SnapshotError::Writer)
        ));
        assert!(matches!(
            client("tampered", f.root.key.clone())
                .package("acme.plugin", None)
                .await,
            Err(RegistryError::Signature { .. })
        ));
        server.abort();
    }
}
