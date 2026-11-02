//! Run only against disposable S3-compatible test storage with prepared buckets.
use cloud_storage::{Config, QuarantineStore, StorageError};
use sha2::{Digest, Sha256};
use std::{error::Error, time::Duration};
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let endpoint = std::env::var("CLOUD_TEST_S3_ENDPOINT")?;
    let settings = |bucket: &str| Config {
        endpoint: endpoint.clone(),
        region: "us-east-1".into(),
        bucket: bucket.into(),
        access_key: "test".into(),
        secret_key: "test".into(),
    };
    let store = QuarantineStore::new(settings("cloud-quarantine"))?;
    let unversioned = QuarantineStore::new(settings("cloud-unversioned"))?;
    let key = format!("quarantine/{}/{}", Uuid::new_v4(), Uuid::new_v4());
    let bytes = b"disposable test artifact";
    let digest = hex::encode(Sha256::digest(bytes));
    assert!(matches!(
        unversioned
            .presign_upload(&key, bytes.len() as i64, &digest, Duration::from_secs(60))
            .await,
        Err(StorageError::VersioningRequired)
    ));
    let signed = store
        .presign_upload(&key, bytes.len() as i64, &digest, Duration::from_secs(60))
        .await?;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let put = || {
        let mut r = client.put(&signed.url);
        for (k, v) in &signed.headers {
            r = r.header(k, v);
        }
        r.body(bytes.to_vec())
    };
    let response = put().send().await?;
    if !response.status().is_success() {
        return Err(format!("upload returned {}", response.status()).into());
    }
    let captured = store.capture(&key, bytes.len() as i64).await?;
    assert!(!captured.version_id.is_empty());
    let mut downloaded = Vec::new();
    store
        .download_version(&key, &captured, &digest, &mut downloaded)
        .await?;
    assert_eq!(downloaded, bytes);
    let mut rejected = Vec::new();
    assert!(matches!(
        store
            .download_version(&key, &captured, &"0".repeat(64), &mut rejected)
            .await,
        Err(StorageError::InvalidObject)
    ));
    assert!(matches!(
        store.capture(&key, bytes.len() as i64 + 1).await,
        Err(StorageError::InvalidObject)
    ));
    let repeated = put().send().await?;
    assert_eq!(repeated.status(), reqwest::StatusCode::PRECONDITION_FAILED);
    assert_eq!(
        store.capture(&key, bytes.len() as i64).await?.version_id,
        captured.version_id
    );
    println!("emulator accepted the conditional upload and returned an immutable version");
    Ok(())
}
