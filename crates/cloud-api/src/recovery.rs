//! Explicit operator recovery of transient job failures. Never changes trust decisions.
use sqlx::PgPool;
use std::error::Error;
use uuid::Uuid;

type Result<T> = std::result::Result<T, Box<dyn Error>>;
#[derive(Clone, Copy)]
pub enum Kind {
    Verification,
    Publication,
}

pub async fn retry(pool: &PgPool, kind: Kind, id: Uuid, operator: &str) -> Result<bool> {
    if operator.is_empty()
        || operator.len() > 80
        || !operator
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.@".contains(&b))
    {
        return Err("invalid operator label".into());
    }
    let mut tx = pool.begin().await?;
    // Conditional row update serializes competing operators and cannot interfere
    // with running leases or permanent rejection/authorization decisions.
    let (query, action) = match kind {
        Kind::Verification => ("UPDATE cloud_verification_jobs j SET status='queued',attempts=0,result_class=NULL WHERE j.upload_id=$1 AND j.status='failed' AND j.result_class='temporary_failure' AND j.lease_id IS NULL AND j.lease_expires_at IS NULL AND EXISTS (SELECT 1 FROM cloud_plugin_releases r JOIN cloud_upload_sessions u ON u.id=r.upload_id AND u.publisher_id=r.publisher_id WHERE r.id=j.release_id AND r.upload_id=j.upload_id AND r.status='awaiting_verification' AND u.status='quarantined') RETURNING (SELECT publisher_id FROM cloud_plugin_releases WHERE id=j.release_id)", "verification.retry_requested"),
        Kind::Publication => ("UPDATE cloud_publication_jobs j SET status='queued',attempts=0,result_class=NULL WHERE j.id=$1 AND j.status='failed' AND j.result_class='temporary_failure' AND j.lease_id IS NULL AND j.lease_expires_at IS NULL AND j.prepared_key IS NULL AND j.prepared_version_id IS NULL AND EXISTS (SELECT 1 FROM cloud_plugin_releases r WHERE r.id=j.release_id AND r.status='review_approved') RETURNING (SELECT publisher_id FROM cloud_plugin_releases WHERE id=j.release_id)", "publication.retry_requested"),
    };
    let publisher: Option<Uuid> = sqlx::query_scalar(query)
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;
    let Some(publisher) = publisher else {
        tx.commit().await?;
        return Ok(false);
    };
    sqlx::query("INSERT INTO cloud_audit_events(id,actor_user_id,publisher_id,subject_id,action) VALUES($1,$2,$3,$4,$5)")
        .bind(Uuid::new_v4()).bind(format!("operator:{operator}")).bind(publisher).bind(id).bind(action).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(true)
}
