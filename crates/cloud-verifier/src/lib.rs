//! Package integrity verification, not moderation or authorization.
//! The caller must supply a separately approved publisher key and recheck its
//! authorization when publishing. No untrusted upload URL is fetched here.
#![forbid(unsafe_code)]
use std::{
    fs::File,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use tempfile::TempDir;
use thiserror::Error;
use tiktools_plugin_api::manifest::{is_plugin_target, is_valid_plugin_id, is_valid_publisher};
use tiktools_plugin_package::{KeyRole, PackageArchive, Sha256Digest, TrustStore, TrustedKey};

// Stricter server-side limits than the Desktop library. Validation should still
// run in a resource-limited worker on untrusted input.
pub const MAX_COMPRESSED_BYTES: u64 = 128 * 1024 * 1024;
pub const MAX_UNPACKED_BYTES: u64 = 256 * 1024 * 1024;
pub const MAX_ENTRIES: usize = 10_000;
pub const MAX_EXPANSION_RATIO: u64 = 200;

#[derive(Debug, Error)]
pub enum VerifyError {
    #[error("invalid expected identity or target")]
    InvalidExpectedMetadata,
    #[error("package digest or metadata does not match the requested release")]
    Mismatch,
    #[error("archive exceeds resource limits")]
    Limits,
    #[error("package is not signed by the approved publisher key")]
    Untrusted,
    #[error("package validation failed")]
    InvalidPackage,
    #[error("verification storage failed")]
    Io(#[from] io::Error),
}

pub struct ExpectedRelease<'a> {
    pub namespace: &'a str,
    pub plugin_id: &'a str,
    pub version: &'a str,
    pub target: &'a str,
    pub sha256: &'a str,
}

/// Grandfathered identifiers owned by the independently approved official
/// publisher. New identifiers must use their publisher namespace.
pub const OFFICIAL_LEGACY_IDS: &[&str] = &[
    "audio.play.process",
    "hotkeys",
    "l4d2.interactive",
    "minecraft.server",
    "peak.game",
    "repo.game",
    "sonicboom.server",
    "textintel",
];

pub fn publisher_allows(namespace: &str, plugin_id: &str) -> bool {
    plugin_id
        .strip_prefix(namespace)
        .is_some_and(|s| s.starts_with('.') && s.len() > 1)
        || namespace == "tiktools" && OFFICIAL_LEGACY_IDS.contains(&plugin_id)
}

/// Locally approved authority is always bounded; registry root delegation
/// must independently authorize each identifier too.
pub fn approved_publisher_key(
    key: tiktools_plugin_package::PublicKey,
    namespace: &str,
) -> TrustedKey {
    let mut trusted = TrustedKey::new(key, KeyRole::Publisher).publisher(namespace);
    if namespace == "tiktools" {
        trusted.scopes.push("tiktools.*".into());
        trusted
            .scopes
            .extend(OFFICIAL_LEGACY_IDS.iter().map(|id| (*id).to_owned()));
    }
    trusted
}

/// Holds the exact private copy that was verified. Persist this copy by digest;
/// never publish the original upload path, which can change while verifying.
pub struct VerifiedArtifact {
    staging: TempDir,
    pub sha256: String,
    pub size: u64,
    pub plugin_id: String,
    pub version: String,
    pub target: String,
    pub publisher_key_id: String,
}
impl VerifiedArtifact {
    pub fn path(&self) -> PathBuf {
        self.staging.path().join("artifact.plugin")
    }
}

/// Validate an already-downloaded file against independently authorized key and release metadata.
/// Does not make a publication decision, scan malware, or establish key ownership.
pub fn verify(
    source: &Path,
    expected: ExpectedRelease<'_>,
    approved_key: &TrustedKey,
) -> Result<VerifiedArtifact, VerifyError> {
    if !is_valid_publisher(expected.namespace)
        || !is_valid_plugin_id(expected.plugin_id)
        || !publisher_allows(expected.namespace, expected.plugin_id)
        || tiktools_plugin_api::manifest::parse_version(expected.version).is_none()
        || !(expected.target == "any" || is_plugin_target(expected.target))
    {
        return Err(VerifyError::InvalidExpectedMetadata);
    }
    let expected_digest =
        Sha256Digest::parse(expected.sha256).ok_or(VerifyError::InvalidExpectedMetadata)?;
    if approved_key.role != KeyRole::Publisher
        || approved_key.publisher.as_deref() != Some(expected.namespace)
        || !approved_key.allows_package(expected.plugin_id)
        || approved_key.revoked
    {
        return Err(VerifyError::Untrusted);
    }
    let staging = tempfile::Builder::new().prefix("cloud-verify-").tempdir()?;
    let artifact = staging.path().join("artifact.plugin");
    let mut input = File::open(source)?;
    if !input.metadata()?.is_file() {
        return Err(VerifyError::InvalidPackage);
    }
    let mut output = File::create(&artifact)?;
    let copied = io::copy(
        &mut Read::by_ref(&mut input).take(MAX_COMPRESSED_BYTES + 1),
        &mut output,
    )?;
    output.flush()?;
    drop(output);
    if copied > MAX_COMPRESSED_BYTES {
        return Err(VerifyError::Limits);
    }
    let archive = PackageArchive::open(&artifact).map_err(|_| VerifyError::InvalidPackage)?;
    let digest = archive.digest().map_err(|_| VerifyError::InvalidPackage)?;
    if digest != expected_digest {
        return Err(VerifyError::Mismatch);
    }
    check_limits(&artifact)?;
    let inspection = archive.inspect().map_err(|_| VerifyError::InvalidPackage)?;
    if inspection.manifest.id != expected.plugin_id
        || inspection.manifest.version != expected.version
        || inspection.manifest.metadata.publisher.as_deref() != Some(expected.namespace)
        || inspection
            .manifest
            .targets
            .first()
            .map(String::as_str)
            .unwrap_or("any")
            != expected.target
    {
        return Err(VerifyError::Mismatch);
    }
    let signature = inspection
        .signature
        .as_ref()
        .ok_or(VerifyError::Untrusted)?;
    let checksums = inspection
        .checksums
        .as_ref()
        .ok_or(VerifyError::Untrusted)?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| VerifyError::InvalidPackage)?
        .as_secs();
    let mut trust = TrustStore::new();
    trust.insert(approved_key.clone(), "cloud-approved-key");
    let verified = trust
        .verify_package(signature, checksums, expected.plugin_id, now)
        .map_err(|_| VerifyError::Untrusted)?;
    // Upstream extraction checks safe paths, duplicate entries and the entire checksum tree.
    let extracted = archive
        .extract(&staging.path().join("extracted"))
        .map_err(|_| VerifyError::InvalidPackage)?;
    if extracted.checksums != *checksums
        || extracted.manifest.id != expected.plugin_id
        || extracted.manifest.version != expected.version
    {
        return Err(VerifyError::Mismatch);
    }
    Ok(VerifiedArtifact {
        staging,
        sha256: digest.to_string(),
        size: copied,
        plugin_id: expected.plugin_id.into(),
        version: expected.version.into(),
        target: expected.target.into(),
        publisher_key_id: verified.key_id.to_string(),
    })
}

fn check_limits(path: &Path) -> Result<(), VerifyError> {
    let mut zip =
        zip::ZipArchive::new(File::open(path)?).map_err(|_| VerifyError::InvalidPackage)?;
    if zip.is_empty() || zip.len() > MAX_ENTRIES {
        return Err(VerifyError::Limits);
    }
    let mut total = 0_u64;
    for index in 0..zip.len() {
        let entry = zip
            .by_index(index)
            .map_err(|_| VerifyError::InvalidPackage)?;
        total = total.checked_add(entry.size()).ok_or(VerifyError::Limits)?;
        if total > MAX_UNPACKED_BYTES
            || (entry.size() > 1024 * 1024
                && entry.size() > entry.compressed_size().saturating_mul(MAX_EXPANSION_RATIO))
        {
            return Err(VerifyError::Limits);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tiktools_plugin_package::{pack, sign_archive, KeyId, PackRequest, SigningKey};
    use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

    #[test]
    fn official_legacy_scopes_are_exact_and_not_available_to_other_publishers() {
        let signing =
            SigningKey::generate(KeyId::parse("test-key").unwrap(), Some("tiktools".into()))
                .unwrap();
        let official = approved_publisher_key(signing.public_key(), "tiktools");
        for id in OFFICIAL_LEGACY_IDS {
            assert!(publisher_allows("tiktools", id));
            assert!(official.allows_package(id));
            assert!(!publisher_allows("other", id));
        }
        assert!(publisher_allows("tiktools", "tiktools.new"));
        assert!(official.allows_package("tiktools.new"));
        assert!(!publisher_allows("tiktools", "sonicboom.other"));
        assert!(!official.allows_package("sonicboom.other"));
        assert!(!publisher_allows("tiktools", "tiktools"));
        let other = approved_publisher_key(signing.public_key(), "other");
        assert!(!other.allows_package("hotkeys"));
    }

    #[test]
    fn signed_official_legacy_package_verifies_only_with_explicit_scope() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = dir.path().join("plugin.json");
        std::fs::write(&manifest, serde_json::to_vec(&serde_json::json!({"schemaVersion":2,"id":"hotkeys","name":"Hotkeys","version":"1.0.0","runtime":"process","entry":"index.js","protocolVersion":1,"publisher":"tiktools"})).unwrap()).unwrap();
        let entry = dir.path().join("entry");
        std::fs::write(&entry, b"test").unwrap();
        let package = dir.path().join("hotkeys.plugin");
        pack(&PackRequest {
            manifest,
            entry: Some(entry),
            output: package.clone(),
            target: None,
        })
        .unwrap();
        let signing = SigningKey::generate(
            KeyId::parse("official-key").unwrap(),
            Some("tiktools".into()),
        )
        .unwrap();
        sign_archive(&package, &signing).unwrap();
        let digest = Sha256Digest::of_file(&package).unwrap().to_string();
        let expected = || ExpectedRelease {
            namespace: "tiktools",
            plugin_id: "hotkeys",
            version: "1.0.0",
            target: "any",
            sha256: &digest,
        };
        let official = approved_publisher_key(signing.public_key(), "tiktools");
        assert!(verify(&package, expected(), &official).is_ok());
        let unscoped =
            TrustedKey::new(signing.public_key(), KeyRole::Publisher).publisher("tiktools");
        assert!(matches!(
            verify(&package, expected(), &unscoped),
            Err(VerifyError::Untrusted)
        ));
    }

    fn setup() -> (TempDir, PathBuf, TrustedKey) {
        let dir = tempfile::tempdir().unwrap();
        let manifest = dir.path().join("plugin.json");
        std::fs::write(&manifest, serde_json::to_vec(&serde_json::json!({"schemaVersion":2,"id":"acme.plugin","name":"Test","version":"1.0.0","runtime":"process","entry":"index.js","protocolVersion":1,"publisher":"acme"})).unwrap()).unwrap();
        let entry = dir.path().join("entry");
        std::fs::write(&entry, b"valid plugin bytes").unwrap();
        let package = dir.path().join("source.plugin");
        pack(&PackRequest {
            manifest,
            entry: Some(entry),
            output: package.clone(),
            target: None,
        })
        .unwrap();
        let key =
            SigningKey::generate(KeyId::parse("acme-key").unwrap(), Some("acme".into())).unwrap();
        sign_archive(&package, &key).unwrap();
        let trusted = TrustedKey::new(key.public_key(), KeyRole::Publisher).publisher("acme");
        (dir, package, trusted)
    }
    fn expected(digest: &str) -> ExpectedRelease<'_> {
        ExpectedRelease {
            namespace: "acme",
            plugin_id: "acme.plugin",
            version: "1.0.0",
            target: "any",
            sha256: digest,
        }
    }
    fn rewrite(source: &Path, destination: &Path, mutate: bool, traversal: bool) {
        let mut input = zip::ZipArchive::new(File::open(source).unwrap()).unwrap();
        let mut output = ZipWriter::new(File::create(destination).unwrap());
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        for index in 0..input.len() {
            let mut entry = input.by_index(index).unwrap();
            let name = entry.name().unwrap().into_owned();
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).unwrap();
            if mutate && name.ends_with("index.js") {
                bytes.push(b'!');
            }
            output.start_file(name, options).unwrap();
            output.write_all(&bytes).unwrap();
        }
        if traversal {
            output.start_file("../../escape", options).unwrap();
            output.write_all(b"bad").unwrap();
        }
        output.finish().unwrap();
    }
    #[test]
    fn verifies_copy_and_rejects_forged_or_mismatched_metadata() {
        let (_dir, package, trusted) = setup();
        let digest = Sha256Digest::of_file(&package).unwrap().to_string();
        let verified = verify(&package, expected(&digest), &trusted).unwrap();
        assert_eq!(verified.sha256, digest);
        assert_eq!(verified.publisher_key_id, "acme-key");
        // Changing the source after verification does not change the retained copy.
        std::fs::write(&package, b"replaced upload").unwrap();
        assert_eq!(
            Sha256Digest::of_file(&verified.path()).unwrap().to_string(),
            digest
        );
        assert!(matches!(
            verify(&package, expected(&digest), &trusted),
            Err(VerifyError::Mismatch)
        ));
        let (_dir, package, trusted) = setup();
        let digest = Sha256Digest::of_file(&package).unwrap().to_string();
        let mut wrong = expected(&digest);
        wrong.version = "1.0.1";
        assert!(matches!(
            verify(&package, wrong, &trusted),
            Err(VerifyError::Mismatch)
        ));
        let mut wrong = expected(&digest);
        wrong.target = "linux-x64-gnu";
        assert!(matches!(
            verify(&package, wrong, &trusted),
            Err(VerifyError::Mismatch)
        ));
        let mut wrong = expected(&digest);
        wrong.namespace = "other";
        assert!(matches!(
            verify(&package, wrong, &trusted),
            Err(VerifyError::InvalidExpectedMetadata)
        ));
        let fake =
            SigningKey::generate(KeyId::parse("acme-key").unwrap(), Some("acme".into())).unwrap();
        let fake = TrustedKey::new(fake.public_key(), KeyRole::Publisher).publisher("acme");
        assert!(matches!(
            verify(&package, expected(&digest), &fake),
            Err(VerifyError::Untrusted)
        ));
        let mut revoked = trusted.clone();
        revoked.revoked = true;
        assert!(matches!(
            verify(&package, expected(&digest), &revoked),
            Err(VerifyError::Untrusted)
        ));
        let mut expired = trusted.clone();
        expired.not_after = Some(1);
        assert!(matches!(
            verify(&package, expected(&digest), &expired),
            Err(VerifyError::Untrusted)
        ));
    }
    #[test]
    fn rejects_tampering_traversal_and_high_expansion() {
        let (dir, package, trusted) = setup();
        for (name, tamper, traversal) in [
            ("tampered.plugin", true, false),
            ("traversal.plugin", false, true),
        ] {
            let path = dir.path().join(name);
            rewrite(&package, &path, tamper, traversal);
            let digest = Sha256Digest::of_file(&path).unwrap().to_string();
            assert!(matches!(
                verify(&path, expected(&digest), &trusted),
                Err(VerifyError::InvalidPackage)
            ));
        }
        let bomb = dir.path().join("bomb.plugin");
        let mut zip = ZipWriter::new(File::create(&bomb).unwrap());
        zip.start_file(
            "large",
            SimpleFileOptions::default().compression_method(CompressionMethod::Deflated),
        )
        .unwrap();
        zip.write_all(&vec![0; 2 * 1024 * 1024]).unwrap();
        zip.finish().unwrap();
        let digest = Sha256Digest::of_file(&bomb).unwrap().to_string();
        assert!(matches!(
            verify(&bomb, expected(&digest), &trusted),
            Err(VerifyError::Limits)
        ));
    }
}
