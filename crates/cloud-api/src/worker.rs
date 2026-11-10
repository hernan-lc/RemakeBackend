//! One-shot integrity worker. Run as an externally sandboxed, resource-limited process.
use cloud_storage::{ObjectVersion, QuarantineStore, StorageError};
use cloud_verifier::{ExpectedRelease, VerifyError};
use sqlx::{PgPool, Row};
use std::error::Error;
use tiktools_plugin_package::{KeyId, PublicKey};
use uuid::Uuid;

type Result<T> = std::result::Result<T, Box<dyn Error>>;
pub(crate) struct Job {
    pub(crate) upload: Uuid,
    pub(crate) release: Uuid,
    pub(crate) publisher: Uuid,
    pub(crate) key_row: Uuid,
    pub(crate) lease: Uuid,
    pub(crate) object_key: String,
    pub(crate) version_id: String,
    pub(crate) size: i64,
    pub(crate) digest: String,
    pub(crate) namespace: String,
    pub(crate) plugin_id: String,
    pub(crate) version: String,
    pub(crate) target: String,
    pub(crate) key_id: String,
    pub(crate) public_key: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    Passed,
    Detected,
    Invalid,
    Retry,
}

async fn claim(pool: &PgPool) -> Result<Option<Job>> {
    let mut tx = pool.begin().await?;
    // A crashed final attempt cannot leave a job permanently running.
    sqlx::query("UPDATE cloud_verification_jobs SET status='failed', lease_id=NULL, lease_expires_at=NULL, result_class='temporary_failure' WHERE status='running' AND lease_expires_at<=now() AND attempts>=3").execute(&mut *tx).await?;
    let upload:Option<Uuid>=sqlx::query_scalar("SELECT upload_id FROM cloud_verification_jobs WHERE release_id IS NOT NULL AND attempts<3 AND (status='queued' OR (status='running' AND lease_expires_at<=now())) ORDER BY created_at,upload_id FOR UPDATE SKIP LOCKED LIMIT 1")
        .fetch_optional(&mut *tx).await?;
    let Some(upload) = upload else {
        tx.commit().await?;
        return Ok(None);
    };
    let lease = Uuid::new_v4();
    sqlx::query("UPDATE cloud_verification_jobs SET status='running',lease_id=$2,lease_expires_at=now()+interval '10 minutes',attempts=attempts+1,result_class=NULL WHERE upload_id=$1").bind(upload).bind(lease).execute(&mut *tx).await?;
    let r=sqlx::query("SELECT r.id AS release, r.publisher_id, r.publisher_key_id, r.plugin_id,r.version,r.target,p.namespace,u.object_key,u.version_id,u.expected_size,u.expected_sha256,k.key_id,k.public_key FROM cloud_plugin_releases r JOIN cloud_upload_sessions u ON u.id=r.upload_id AND u.publisher_id=r.publisher_id JOIN cloud_publishers p ON p.id=r.publisher_id JOIN cloud_publisher_keys k ON k.id=r.publisher_key_id AND k.publisher_id=r.publisher_id WHERE r.upload_id=$1 AND r.status='awaiting_verification' AND u.status='quarantined'")
        .bind(upload).fetch_optional(&mut *tx).await?;
    let Some(r) = r else {
        sqlx::query("UPDATE cloud_verification_jobs SET status='failed',lease_id=NULL,lease_expires_at=NULL,result_class='temporary_failure' WHERE upload_id=$1 AND lease_id=$2").bind(upload).bind(lease).execute(&mut *tx).await?;
        tx.commit().await?;
        return Ok(None);
    };
    let job = Job {
        upload,
        lease,
        release: r.get("release"),
        publisher: r.get("publisher_id"),
        key_row: r.get("publisher_key_id"),
        object_key: r.get("object_key"),
        version_id: r.get("version_id"),
        size: r.get("expected_size"),
        digest: r.get("expected_sha256"),
        namespace: r.get("namespace"),
        plugin_id: r.get("plugin_id"),
        version: r.get("version"),
        target: r.get("target"),
        key_id: r.get("key_id"),
        public_key: r.get("public_key"),
    };
    tx.commit().await?;
    Ok(Some(job))
}
pub(crate) async fn verify_artifact(
    job: &Job,
    store: &QuarantineStore,
    scanner: &crate::scanner::ClamAv,
) -> std::result::Result<cloud_verifier::VerifiedArtifact, Outcome> {
    let directory = match tempfile::Builder::new().prefix("cloud-download-").tempdir() {
        Ok(d) => d,
        Err(_) => return Err(Outcome::Retry),
    };
    let path = directory.path().join("source.plugin");
    let mut file = match tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .await
    {
        Ok(f) => f,
        Err(_) => return Err(Outcome::Retry),
    };
    match store
        .download_version(
            &job.object_key,
            &ObjectVersion {
                version_id: job.version_id.clone(),
                size: job.size,
            },
            &job.digest,
            &mut file,
        )
        .await
    {
        Ok(()) => {}
        Err(StorageError::Unavailable) => return Err(Outcome::Retry),
        Err(_) => return Err(Outcome::Invalid),
    }
    drop(file);
    let (namespace, plugin_id, version, target, digest, key_id, public_key) = (
        job.namespace.clone(),
        job.plugin_id.clone(),
        job.version.clone(),
        job.target.clone(),
        job.digest.clone(),
        job.key_id.clone(),
        job.public_key.clone(),
    );
    match tokio::task::spawn_blocking(move || {
        let key_id = KeyId::parse(&key_id).map_err(|_| VerifyError::Untrusted)?;
        let key = PublicKey {
            key_id,
            algorithm: "ed25519".into(),
            public_key,
        };
        let trusted = cloud_verifier::approved_publisher_key(key, &namespace);
        cloud_verifier::verify(
            &path,
            ExpectedRelease {
                namespace: &namespace,
                plugin_id: &plugin_id,
                version: &version,
                target: &target,
                sha256: &digest,
            },
            &trusted,
        )
    })
    .await
    {
        Ok(Ok(verified)) => match scanner
            .scan(&verified.path(), verified.size, &verified.sha256)
            .await
        {
            crate::scanner::Scan::Clean => Ok(verified),
            crate::scanner::Scan::Detected => Err(Outcome::Detected),
            crate::scanner::Scan::Unavailable => Err(Outcome::Retry),
        },
        Ok(Err(VerifyError::Io(_))) | Err(_) => Err(Outcome::Retry),
        Ok(Err(_)) => Err(Outcome::Invalid),
    }
}
async fn finish(
    pool: &PgPool,
    job: &Job,
    outcome: Outcome,
    presentation: Option<&cloud_verifier::presentation::Presentation>,
) -> Result<()> {
    let mut tx = pool.begin().await?;
    // Serialize against publisher suspension and key revocation, using the same lock order as API mutations.
    let status: Option<String> =
        sqlx::query_scalar("SELECT status FROM cloud_publishers WHERE id=$1 FOR UPDATE")
            .bind(job.publisher)
            .fetch_optional(&mut *tx)
            .await?;
    let key: Option<String> = sqlx::query_scalar(
        "SELECT status FROM cloud_publisher_keys WHERE id=$1 AND publisher_id=$2 FOR UPDATE",
    )
    .bind(job.key_row)
    .bind(job.publisher)
    .fetch_optional(&mut *tx)
    .await?;
    let lease:Option<i32>=sqlx::query_scalar("SELECT attempts FROM cloud_verification_jobs WHERE upload_id=$1 AND status='running' AND lease_id=$2 AND lease_expires_at>now() FOR UPDATE").bind(job.upload).bind(job.lease).fetch_optional(&mut *tx).await?;
    let Some(attempts) = lease else {
        tx.commit().await?;
        return Ok(());
    };
    let (job_status, release_status, result_class) = if key.as_deref() != Some("approved") {
        ("rejected", Some("rejected"), "key_not_authorized")
    } else if status.as_deref() != Some("approved") {
        if attempts >= 3 {
            ("failed", None, "temporary_failure")
        } else {
            ("queued", None, "temporary_failure")
        }
    } else {
        match outcome {
            Outcome::Passed => (
                "antivirus_passed",
                Some("antivirus_passed"),
                "antivirus_passed",
            ),
            Outcome::Invalid => ("rejected", Some("rejected"), "invalid_package"),
            Outcome::Detected => ("rejected", Some("rejected"), "antivirus_alert"),
            Outcome::Retry if attempts >= 3 => ("failed", None, "temporary_failure"),
            Outcome::Retry => ("queued", None, "temporary_failure"),
        }
    };
    sqlx::query("UPDATE cloud_verification_jobs SET status=$3,lease_id=NULL,lease_expires_at=NULL,result_class=$4 WHERE upload_id=$1 AND lease_id=$2").bind(job.upload).bind(job.lease).bind(job_status).bind(result_class).execute(&mut *tx).await?;
    if let Some(status) = release_status {
        sqlx::query("UPDATE cloud_plugin_releases SET status=$2 WHERE id=$1 AND status='awaiting_verification'").bind(job.release).bind(status).execute(&mut *tx).await?;
        if let ("antivirus_passed", Some(p)) = (status, presentation) {
            // Display metadata is written only with the passing transition, from the verified manifest.
            sqlx::query("UPDATE cloud_plugin_releases SET display_name=$2, summary=$3, categories=$4, authors=$5, engine=$6, homepage=$7, dependencies=$8 WHERE id=$1")
                .bind(job.release)
                .bind(&p.name)
                .bind(&p.summary)
                .bind(&p.categories)
                .bind(&p.authors)
                .bind(&p.engine)
                .bind(&p.homepage)
                .bind(serde_json::to_value(&p.dependencies).unwrap_or_default())
                .execute(&mut *tx)
                .await?;
        }
        sqlx::query("INSERT INTO cloud_audit_events (id,actor_user_id,publisher_id,subject_id,action) VALUES ($1,'system:integrity-worker',$2,$3,$4)").bind(Uuid::new_v4()).bind(job.publisher).bind(job.release).bind(if status=="rejected" {"release.verification_rejected"} else {"release.antivirus_passed"}).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(())
}
pub async fn once(
    pool: &PgPool,
    store: &QuarantineStore,
    scanner: &crate::scanner::ClamAv,
) -> Result<bool> {
    let Some(job) = claim(pool).await? else {
        return Ok(false);
    };
    let allowed: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM cloud_publishers p JOIN cloud_publisher_keys k ON k.publisher_id=p.id WHERE p.id=$1 AND p.status='approved' AND k.id=$2 AND k.status='approved')")
        .bind(job.publisher).bind(job.key_row).fetch_one(pool).await?;
    let mut presentation: Option<cloud_verifier::presentation::Presentation> = None;
    let outcome = if allowed {
        match Box::pin(verify_artifact(&job, store, scanner)).await {
            Ok(verified) => {
                presentation = Some(verified.presentation.clone());
                Outcome::Passed
            }
            Err(outcome) => outcome,
        }
    } else {
        Outcome::Retry
    };
    finish(pool, &job, outcome, presentation.as_ref()).await?;
    Ok(true)
}
