//! Prepare a private, version-pinned artifact. This does not sign or deploy a registry.
use crate::{
    scanner::ClamAv,
    worker::{self, Job, Outcome},
};
use cloud_storage::{ObjectVersion, QuarantineStore};
use sqlx::{PgPool, Row};
use std::error::Error;
use uuid::Uuid;
type Result<T> = std::result::Result<T, Box<dyn Error>>;
struct Claim {
    id: Uuid,
    job: Job,
    requester: String,
}
enum Prepared {
    Stored(String, ObjectVersion),
    Unauthorized,
    Invalid,
    Alert,
    Retry,
}

async fn claim(pool: &PgPool) -> Result<Option<Claim>> {
    let mut tx = pool.begin().await?;
    sqlx::query("UPDATE cloud_publication_jobs SET status='failed',lease_id=NULL,lease_expires_at=NULL,result_class='temporary_failure' WHERE status='running' AND lease_expires_at<=now() AND attempts>=3").execute(&mut *tx).await?;
    let id:Option<Uuid>=sqlx::query_scalar("SELECT id FROM cloud_publication_jobs WHERE attempts<3 AND (status='queued' OR (status='running' AND lease_expires_at<=now())) ORDER BY created_at,id FOR UPDATE SKIP LOCKED LIMIT 1").fetch_optional(&mut *tx).await?;
    let Some(id) = id else {
        tx.commit().await?;
        return Ok(None);
    };
    let lease = Uuid::new_v4();
    sqlx::query("UPDATE cloud_publication_jobs SET status='running',lease_id=$2,lease_expires_at=now()+interval '10 minutes',attempts=attempts+1,result_class=NULL WHERE id=$1").bind(id).bind(lease).execute(&mut *tx).await?;
    let r=sqlx::query("SELECT q.requested_by,r.id AS release,r.publisher_id,r.publisher_key_id,r.plugin_id,r.version,r.target,p.namespace,u.id AS upload,u.object_key,u.version_id,u.expected_size,u.expected_sha256,k.key_id,k.public_key FROM cloud_publication_jobs q JOIN cloud_plugin_releases r ON r.id=q.release_id JOIN cloud_upload_sessions u ON u.id=r.upload_id AND u.publisher_id=r.publisher_id JOIN cloud_publishers p ON p.id=r.publisher_id JOIN cloud_publisher_keys k ON k.id=r.publisher_key_id AND k.publisher_id=r.publisher_id WHERE q.id=$1 AND u.status='quarantined'")
        .bind(id).fetch_optional(&mut *tx).await?;
    let Some(r) = r else {
        sqlx::query("UPDATE cloud_publication_jobs SET status='blocked',lease_id=NULL,lease_expires_at=NULL,result_class='authorization_changed' WHERE id=$1 AND lease_id=$2").bind(id).bind(lease).execute(&mut *tx).await?;
        tx.commit().await?;
        return Ok(None);
    };
    let requester: String = r.get("requested_by");
    let job = Job {
        upload: r.get("upload"),
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
    Ok(Some(Claim { id, job, requester }))
}
async fn eligible(pool: &PgPool, claim: &Claim) -> Result<bool> {
    Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM cloud_publication_jobs q JOIN cloud_plugin_releases r ON r.id=q.release_id JOIN cloud_publishers p ON p.id=r.publisher_id JOIN cloud_upload_sessions u ON u.id=r.upload_id AND u.publisher_id=r.publisher_id JOIN cloud_publisher_keys k ON k.id=r.publisher_key_id AND k.publisher_id=r.publisher_id JOIN cloud_verification_jobs v ON v.upload_id=r.upload_id AND v.release_id=r.id JOIN cloud_moderation_events m ON m.release_id=r.id WHERE q.id=$1 AND q.requested_by=$2 AND p.status='approved' AND k.status='approved' AND r.status='review_approved' AND v.status='antivirus_passed' AND v.result_class='antivirus_passed' AND m.decision='approve' AND q.requested_by<>r.requested_by AND q.requested_by<>u.requested_by AND q.requested_by<>k.requested_by AND EXISTS(SELECT 1 FROM cloud_account_roles a WHERE a.user_id=q.requested_by AND a.role IN ('admin','moderator')) AND NOT EXISTS(SELECT 1 FROM cloud_publisher_memberships x WHERE x.publisher_id=r.publisher_id AND x.user_id=q.requested_by))").bind(claim.id).bind(&claim.requester).fetch_one(pool).await?)
}
async fn prepare(claim: &Claim, store: &QuarantineStore, scanner: &ClamAv) -> Prepared {
    let verified = match Box::pin(worker::verify_artifact(&claim.job, store, scanner)).await {
        Ok(verified) => verified,
        Err(Outcome::Invalid) => return Prepared::Invalid,
        Err(Outcome::Detected) => return Prepared::Alert,
        Err(_) => return Prepared::Retry,
    };
    match store
        .stage_verified(
            claim.job.publisher,
            claim.job.lease,
            &verified.path(),
            claim.job.size,
            &claim.job.digest,
        )
        .await
    {
        Ok((key, version)) => Prepared::Stored(key, version),
        Err(_) => Prepared::Retry,
    }
}
async fn finish(pool: &PgPool, claim: &Claim, outcome: Prepared) -> Result<()> {
    let job = &claim.job;
    let mut tx = pool.begin().await?;
    // Hold the requester's role while the decision is committed, matching API lock order.
    let role:Option<String>=sqlx::query_scalar("SELECT role FROM cloud_account_roles WHERE user_id=$1 AND role IN ('admin','moderator') ORDER BY role FOR SHARE").bind(&claim.requester).fetch_optional(&mut *tx).await?;
    // The publisher lock serializes against authorization and release mutations.
    sqlx::query("SELECT id FROM cloud_publishers WHERE id=$1 FOR UPDATE")
        .bind(job.publisher)
        .fetch_optional(&mut *tx)
        .await?;
    sqlx::query("SELECT id FROM cloud_publisher_keys WHERE id=$1 AND publisher_id=$2 FOR UPDATE")
        .bind(job.key_row)
        .bind(job.publisher)
        .fetch_optional(&mut *tx)
        .await?;
    let attempts:Option<i32>=sqlx::query_scalar("SELECT attempts FROM cloud_publication_jobs WHERE id=$1 AND status='running' AND lease_id=$2 AND lease_expires_at>now() FOR UPDATE").bind(claim.id).bind(job.lease).fetch_optional(&mut *tx).await?;
    let Some(attempts) = attempts else {
        tx.commit().await?;
        return Ok(());
    };
    // Check the exact metadata and trust state again after all network and scanning work.
    let valid:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM cloud_plugin_releases r JOIN cloud_publishers p ON p.id=r.publisher_id JOIN cloud_upload_sessions u ON u.id=r.upload_id AND u.publisher_id=r.publisher_id JOIN cloud_publisher_keys k ON k.id=r.publisher_key_id AND k.publisher_id=r.publisher_id JOIN cloud_verification_jobs v ON v.upload_id=u.id AND v.release_id=r.id JOIN cloud_moderation_events m ON m.release_id=r.id JOIN cloud_publication_jobs q ON q.release_id=r.id WHERE r.id=$1 AND r.publisher_id=$2 AND r.publisher_key_id=$3 AND u.id=$4 AND p.status='approved' AND k.status='approved' AND r.status='review_approved' AND u.status='quarantined' AND v.status='antivirus_passed' AND v.result_class='antivirus_passed' AND m.decision='approve' AND p.namespace=$5 AND r.plugin_id=$6 AND r.version=$7 AND r.target=$8 AND u.object_key=$9 AND u.version_id=$10 AND u.expected_size=$11 AND u.expected_sha256=$12 AND k.key_id=$13 AND k.public_key=$14 AND q.id=$15 AND q.requested_by=$16 AND q.requested_by<>r.requested_by AND q.requested_by<>u.requested_by AND q.requested_by<>k.requested_by AND NOT EXISTS(SELECT 1 FROM cloud_publisher_memberships x WHERE x.publisher_id=r.publisher_id AND x.user_id=q.requested_by))")
        .bind(job.release).bind(job.publisher).bind(job.key_row).bind(job.upload).bind(&job.namespace).bind(&job.plugin_id).bind(&job.version).bind(&job.target).bind(&job.object_key).bind(&job.version_id).bind(job.size).bind(&job.digest).bind(&job.key_id).bind(&job.public_key).bind(claim.id).bind(&claim.requester).fetch_one(&mut *tx).await?;
    let (status, class, key, version) =
        if !valid || role.is_none() || matches!(outcome, Prepared::Unauthorized) {
            ("blocked", "authorization_changed", None, None)
        } else {
            match outcome {
                Prepared::Stored(key, version) => {
                    ("prepared", "prepared", Some(key), Some(version.version_id))
                }
                Prepared::Invalid => ("rejected", "invalid_package", None, None),
                Prepared::Alert => ("rejected", "antivirus_alert", None, None),
                Prepared::Retry if attempts >= 3 => ("failed", "temporary_failure", None, None),
                Prepared::Retry => ("queued", "temporary_failure", None, None),
                Prepared::Unauthorized => unreachable!(),
            }
        };
    sqlx::query("UPDATE cloud_publication_jobs SET status=$3,lease_id=NULL,lease_expires_at=NULL,result_class=$4,prepared_key=$5,prepared_version_id=$6 WHERE id=$1 AND lease_id=$2").bind(claim.id).bind(job.lease).bind(status).bind(class).bind(key).bind(version).execute(&mut *tx).await?;
    if status != "queued" {
        sqlx::query("INSERT INTO cloud_audit_events (id,actor_user_id,publisher_id,subject_id,action) VALUES ($1,'system:publication-preparer',$2,$3,$4)").bind(Uuid::new_v4()).bind(job.publisher).bind(job.release).bind(match status {"prepared"=>"release.publication_prepared","blocked"=>"release.publication_blocked","rejected"=>"release.publication_rejected",_=>"release.publication_failed"}).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(())
}
pub async fn once(pool: &PgPool, store: &QuarantineStore, scanner: &ClamAv) -> Result<bool> {
    let Some(claim) = claim(pool).await? else {
        return Ok(false);
    };
    let outcome = if eligible(pool, &claim).await? {
        Box::pin(prepare(&claim, store, scanner)).await
    } else {
        Prepared::Unauthorized
    };
    finish(pool, &claim, outcome).await?;
    Ok(true)
}
