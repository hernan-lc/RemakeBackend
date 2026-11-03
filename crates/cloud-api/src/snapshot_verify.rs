//! Point-in-time inspection of a private candidate, not publication approval.
use crate::{
    scanner::{ClamAv, Scan},
    snapshot_export, snapshot_worker,
};
use cloud_storage::QuarantineStore;
use sqlx::{PgPool, Row};
use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fs,
    path::Path,
};
use tiktools_plugin_package::{PublicKey, Sha256Digest, TrustedKey};
use tiktools_plugin_registry::{shard_path, CatalogPage, PackageDocument, RegistryConfigDocument};
use uuid::Uuid;
type Result<T> = std::result::Result<T, Box<dyn Error>>;

async fn inventory(pool: &PgPool, id: Uuid) -> Result<Vec<snapshot_worker::Item>> {
    let mut tx = pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
        .execute(&mut *tx)
        .await?;
    let items = snapshot_worker::capture(&mut tx).await?;
    let candidate = sqlx::query("SELECT inventory_sha256,release_count FROM cloud_snapshot_candidates WHERE id=$1 AND status='candidate'").bind(id).fetch_optional(&mut *tx).await?.ok_or("candidate unavailable")?;
    let digest = Sha256Digest::of_bytes(&serde_json::to_vec(&items)?).to_string();
    if items.is_empty()
        || candidate.get::<String, _>("inventory_sha256") != digest
        || candidate.get::<i32, _>("release_count") as usize != items.len()
    {
        return Err("candidate inventory is stale".into());
    }
    let links = sqlx::query("SELECT publication_job_id,prepared_key,prepared_version_id,sha256 FROM cloud_snapshot_candidate_releases WHERE candidate_id=$1 ORDER BY publication_job_id LIMIT 10001").bind(id).fetch_all(&mut *tx).await?;
    if links.len() != items.len()
        || links.iter().zip(&items).any(|(r, i)| {
            r.get::<Uuid, _>("publication_job_id") != i.job
                || r.get::<String, _>("prepared_key") != i.prepared_key
                || r.get::<String, _>("prepared_version_id") != i.prepared_version
                || r.get::<String, _>("sha256") != i.sha256
        })
    {
        return Err("candidate release links do not match inventory".into());
    }
    tx.commit().await?;
    Ok(items)
}

// Initial candidates have no prior releases, yanks or advisories. Verify every
// index entry and artifact against the independently captured Cloud inventory.
pub(crate) fn check_contents(
    path: &Path,
    files: &BTreeMap<String, String>,
    items: &[snapshot_worker::Item],
    approved: &[TrustedKey],
) -> Result<Vec<(String, u64, String)>> {
    let mut scan_files = Vec::new();
    let mut expected = BTreeMap::new();
    let mut ids = BTreeSet::new();
    for (item, key) in items.iter().zip(approved) {
        ids.insert(item.plugin_id.clone());
        if expected
            .insert(
                (
                    item.plugin_id.clone(),
                    item.version.clone(),
                    item.target.clone(),
                ),
                (item, key),
            )
            .is_some()
        {
            return Err("duplicate inventory release target".into());
        }
    }
    let mut paths: BTreeSet<String> = [
        "root.json",
        "keys.json",
        "keys.json.sig",
        "config.json",
        "config.json.sig",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    let mut observed = BTreeSet::new();
    let mut documents = BTreeMap::new();
    for id in &ids {
        let index_path = shard_path(id);
        if !files.contains_key(&index_path) || !files.contains_key(&format!("{index_path}.sig")) {
            return Err("candidate index missing".into());
        }
        paths.insert(index_path.clone());
        paths.insert(format!("{index_path}.sig"));
        let doc: PackageDocument = serde_json::from_slice(&fs::read(path.join(&index_path))?)?;
        doc.validate(id).map_err(|_| "invalid package document")?;
        let latest = doc
            .releases
            .iter()
            .filter_map(|r| r.semver())
            .max()
            .ok_or("empty package document")?;
        let mut channels: BTreeMap<String, (semver::Version, String)> = BTreeMap::new();
        for release in &doc.releases {
            let version = release.semver().ok_or("invalid version")?;
            let previous = channels.get(&release.channel);
            if previous.is_none_or(|(old, _)| version > *old) {
                channels.insert(release.channel.clone(), (version, release.version.clone()));
            }
        }
        let tags: BTreeMap<_, _> = channels
            .into_iter()
            .map(|(channel, (_, version))| (channel, version))
            .collect();
        if doc.dist_tags != tags || !doc.screenshots.is_empty() {
            return Err("invalid package channel tags or screenshots".into());
        }
        for release in &doc.releases {
            let version = tiktools_plugin_api::manifest::parse_version(&release.version)
                .ok_or("invalid version")?;
            if release.yanked
                || release.advisory.is_some()
                || release.channel
                    != if version.pre.is_empty() {
                        "stable"
                    } else {
                        "beta"
                    }
            {
                return Err("unexpected release policy".into());
            }
            for artifact in &release.artifacts {
                let identity = (id.clone(), release.version.clone(), artifact.target.clone());
                let (item, key) = expected
                    .get(&identity)
                    .ok_or("unexpected indexed artifact")?;
                if !observed.insert(identity)
                    || doc.publisher.as_deref() != Some(&item.namespace)
                    || artifact.sha256.as_hex() != item.sha256
                    || artifact.size != item.size as u64
                    || artifact.key_id.as_deref() != Some(&item.key_id)
                    || !artifact.url.starts_with("packages/")
                    || !files.contains_key(&artifact.url)
                    || files.get(&artifact.url) != Some(&item.sha256)
                    || !paths.insert(artifact.url.clone())
                {
                    return Err("candidate artifact does not match inventory".into());
                }
                let verified = cloud_verifier::verify(
                    &path.join(&artifact.url),
                    cloud_verifier::ExpectedRelease {
                        namespace: &item.namespace,
                        plugin_id: id,
                        version: &item.version,
                        target: &item.target,
                        sha256: &item.sha256,
                    },
                    key,
                )?;
                let inspection =
                    tiktools_plugin_package::PackageArchive::open(verified.path())?.inspect()?;
                let manifest = &inspection.manifest;
                if version == latest
                    && (doc.name != manifest.name
                        || doc.publisher != manifest.metadata.publisher
                        || doc.description != manifest.description
                        || doc.icon != manifest.icon
                        || doc.categories != manifest.metadata.categories
                        || doc.tags != manifest.tags
                        || doc.homepage != manifest.metadata.homepage
                        || doc.repository != manifest.metadata.repository
                        || doc.license != manifest.metadata.license)
                {
                    return Err("indexed package metadata differs from latest package".into());
                }
                let mut permissions = manifest.permissions.clone();
                let mut capabilities = manifest.capabilities.clone();
                permissions.sort();
                capabilities.sort();
                let engines: BTreeMap<String, String> = manifest
                    .metadata
                    .engine
                    .iter()
                    .map(|v| {
                        (
                            tiktools_plugin_api::manifest::HOST_ENGINE.to_owned(),
                            v.clone(),
                        )
                    })
                    .collect();
                if release.runtime != manifest.runtime.to_string()
                    || release.engines != engines
                    || release.schema_version != manifest.schema_version
                    || release.protocol_version != manifest.protocol_version
                    || release.abi_version != manifest.abi_version
                    || release.permissions != permissions
                    || release.capabilities != capabilities
                    || release.dependencies != manifest.metadata.dependencies
                    || release.changelog != inspection.changelog
                {
                    return Err("indexed release metadata differs from package".into());
                }
                scan_files.push((artifact.url.clone(), verified.size, item.sha256.clone()));
                if verified.size != item.size as u64 {
                    return Err("candidate artifact size mismatch".into());
                }
            }
        }
        documents.insert(id.clone(), doc);
    }
    if observed.len() != expected.len() {
        return Err("candidate omits authorized artifacts".into());
    }
    let config: RegistryConfigDocument =
        serde_json::from_slice(&fs::read(path.join("config.json"))?)?;
    let mut catalog_ids = BTreeSet::new();
    for number in 1..=config.catalog_pages {
        let name = format!("catalog/page-{number}.json");
        if !files.contains_key(&name) || !files.contains_key(&format!("{name}.sig")) {
            return Err("candidate catalog missing".into());
        }
        paths.insert(name.clone());
        paths.insert(format!("{name}.sig"));
        let page: CatalogPage = serde_json::from_slice(&fs::read(path.join(name))?)?;
        if page.page != number
            || page.pages != config.catalog_pages
            || page.total as usize != ids.len()
        {
            return Err("invalid candidate catalog".into());
        }
        for entry in page.entries {
            let doc = documents.get(&entry.id).ok_or("unexpected catalog entry")?;
            let latest = doc
                .releases
                .iter()
                .max_by(|a, b| a.semver().cmp(&b.semver()))
                .ok_or("empty package")?;
            if entry.name != doc.name
                || entry.publisher != doc.publisher
                || entry.description != doc.description
                || entry.icon != doc.icon
                || entry.categories != doc.categories
                || entry.tags != doc.tags
                || entry.latest != latest.version
                || entry.runtime != latest.runtime
                || entry.updated_at != latest.published_at
            {
                return Err("catalog metadata differs from index".into());
            }
            if !ids.contains(&entry.id) || !catalog_ids.insert(entry.id) {
                return Err("candidate catalog inventory mismatch".into());
            }
        }
    }
    if catalog_ids != ids || paths != files.keys().cloned().collect() {
        return Err("candidate contains missing or unexpected files".into());
    }
    Ok(scan_files)
}

const POLICY_FILES: [&str; 5] = [
    "root.json",
    "keys.json",
    "keys.json.sig",
    "config.json",
    "config.json.sig",
];

fn policy(seed: &Path, root: &TrustedKey, index: &PublicKey) -> Result<BTreeMap<String, Vec<u8>>> {
    let mut bytes = BTreeMap::new();
    let mut hashes = BTreeMap::new();
    for name in POLICY_FILES {
        let value = snapshot_worker::read_file(&seed.join(name), false, 8 * 1024 * 1024)?;
        hashes.insert(name.to_string(), Sha256Digest::of_bytes(&value).to_string());
        bytes.insert(name.to_string(), value);
    }
    cloud_snapshot::revalidate_tree(seed, &hashes, root, index, &[])?;
    Ok(bytes)
}

fn check_policy(
    candidate: &Path,
    baseline: &BTreeMap<String, Vec<u8>>,
    items: &[snapshot_worker::Item],
) -> Result<()> {
    for name in ["root.json", "keys.json", "keys.json.sig"] {
        if fs::read(candidate.join(name))? != baseline[name] {
            return Err("candidate trust material is stale".into());
        }
    }
    let current: RegistryConfigDocument = serde_json::from_slice(&baseline["config.json"])?;
    let actual: RegistryConfigDocument =
        serde_json::from_slice(&fs::read(candidate.join("config.json"))?)?;
    if current.protocol != actual.protocol
        || current.name != actual.name
        || current.description != actual.description
        || current.api != actual.api
        || current.mirrors != actual.mirrors
        || current.revoked_digests != actual.revoked_digests
        || current.expires != actual.expires
        || items
            .iter()
            .any(|item| current.revoked_digests.contains(&item.sha256))
    {
        return Err("candidate registry policy is stale or revokes a release".into());
    }
    Ok(())
}

pub async fn verify(
    pool: &PgPool,
    store: &QuarantineStore,
    scanner: &ClamAv,
    seed: &Path,
    id: Uuid,
    root: &TrustedKey,
    index: &PublicKey,
) -> Result<()> {
    let baseline = policy(seed, root, index)?;
    let initial = inventory(pool, id).await?;
    let manifest = snapshot_export::current(pool, id).await?;
    if manifest.root != root.key.fingerprint()?.to_string()
        || manifest.inventory != Sha256Digest::of_bytes(&serde_json::to_vec(&initial)?).to_string()
    {
        return Err("candidate trust or inventory mismatch".into());
    }
    let temp = tempfile::Builder::new()
        .prefix("cloud-candidate-check-")
        .tempdir()?;
    let path = temp.path().join("candidate");
    snapshot_export::export(pool, store, id, &path).await?;
    let files: BTreeMap<_, _> = manifest
        .files
        .iter()
        .map(|f| (f.path.clone(), f.digest.clone()))
        .collect();
    let approved = initial
        .iter()
        .map(snapshot_worker::Item::approved)
        .collect::<Result<Vec<_>>>()?;
    let refs: Vec<_> = approved
        .iter()
        .zip(&initial)
        .map(|(key, item)| (key, item.plugin_id.as_str()))
        .collect();
    cloud_snapshot::revalidate_tree(&path, &files, root, index, &refs)?;
    check_policy(&path, &baseline, &initial)?;
    let scan_files = check_contents(&path, &files, &initial, &approved)?;
    for (name, size, digest) in scan_files {
        if scanner.scan(&path.join(name), size, &digest).await != Scan::Clean {
            return Err("candidate antivirus scan did not pass".into());
        }
    }
    cloud_snapshot::revalidate_tree(&path, &files, root, index, &refs)?;
    if policy(seed, root, index)? != baseline
        || inventory(pool, id).await? != initial
        || snapshot_export::current(pool, id).await? != manifest
    {
        return Err("candidate changed during verification".into());
    }
    // Authorization may change immediately after this point; promotion requires a fence.
    Ok(())
}
