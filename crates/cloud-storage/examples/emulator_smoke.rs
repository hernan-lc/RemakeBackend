//! Run only against disposable S3-compatible test storage with prepared buckets.
use cloud_storage::{Config, QuarantineStore, StorageError};
use sha2::{Digest, Sha256};
use std::{error::Error, time::Duration};
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let endpoint = std::env::var("CLOUD_TEST_S3_ENDPOINT")?;
    let access_key = std::env::var("CLOUD_TEST_S3_ACCESS_KEY").unwrap_or_else(|_| "test".into());
    let secret_key = std::env::var("CLOUD_TEST_S3_SECRET_KEY").unwrap_or_else(|_| "test".into());
    let (versioned_bucket, unversioned_bucket) =
        if std::env::var_os("CLOUD_TEST_S3_CREATE_BUCKETS").is_some() {
            use aws_sdk_s3::{
                config::{Credentials, Region},
                types::{BucketVersioningStatus, VersioningConfiguration},
            };
            let client = aws_sdk_s3::Client::from_conf(
                aws_sdk_s3::config::Builder::new()
                    .behavior_version_latest()
                    .endpoint_url(&endpoint)
                    .region(Region::new("us-east-1"))
                    .credentials_provider(Credentials::new(
                        &access_key,
                        &secret_key,
                        None,
                        None,
                        "smoke-test",
                    ))
                    .force_path_style(true)
                    .build(),
            );
            let a = format!("cloud-versioned-{}", Uuid::new_v4());
            let b = format!("cloud-unversioned-{}", Uuid::new_v4());
            for bucket in [&a, &b] {
                client.create_bucket().bucket(bucket).send().await?;
            }
            client
                .put_bucket_versioning()
                .bucket(&a)
                .versioning_configuration(
                    VersioningConfiguration::builder()
                        .status(BucketVersioningStatus::Enabled)
                        .build(),
                )
                .send()
                .await?;
            (a, b)
        } else {
            (
                "cloud-quarantine".to_string(),
                "cloud-unversioned".to_string(),
            )
        };
    let settings = |bucket: &str| Config {
        endpoint: endpoint.clone(),
        region: "us-east-1".into(),
        bucket: bucket.into(),
        access_key: access_key.clone(),
        secret_key: secret_key.clone(),
    };
    let store = QuarantineStore::new(settings(&versioned_bucket))?;
    let unversioned = QuarantineStore::new(settings(&unversioned_bucket))?;
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
    // The provider must reject an altered payload and a missing signed header.
    let mut altered = client.put(&signed.url);
    let mut missing_header = client.put(&signed.url);
    for (name, value) in &signed.headers {
        altered = altered.header(name, value);
        if !name.eq_ignore_ascii_case("if-none-match") {
            missing_header = missing_header.header(name, value);
        }
    }
    let mut wrong_bytes = bytes.to_vec();
    wrong_bytes[0] ^= 1;
    assert!(!altered
        .body(wrong_bytes)
        .send()
        .await?
        .status()
        .is_success());
    assert!(!missing_header
        .body(bytes.to_vec())
        .send()
        .await?
        .status()
        .is_success());
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
    let temp = tempfile::tempdir()?;
    let staged_path = temp.path().join("artifact.plugin");
    tokio::fs::write(&staged_path, bytes).await?;
    let (prepared_key, prepared_version) = store
        .stage_verified(
            Uuid::new_v4(),
            Uuid::new_v4(),
            &staged_path,
            bytes.len() as i64,
            &digest,
        )
        .await?;
    let mut prepared = Vec::new();
    store
        .download_version(&prepared_key, &prepared_version, &digest, &mut prepared)
        .await?;
    assert_eq!(prepared, bytes);
    println!("S3-compatible store enforced conditional upload, versioned download and prepared-object readback");
    Ok(())
}
