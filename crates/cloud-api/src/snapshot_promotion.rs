//! Audited promotion-readiness decisions for private snapshot candidates.
//!
//! A `verified` decision records that the existing point-in-time verifier
//! passed for a candidate when an operator ran it. A `blocked` decision
//! records that it did not. Neither decision deploys a snapshot, changes a
//! release status, or authorizes promotion: a future promotion process must
//! recheck the entire inventory, current authorization, trust material,
//! scanner policy, and every stored object version atomically before any
//! deployment.
use crate::snapshot_export::{self, Manifest};
use sqlx::PgPool;
use std::error::Error;
use uuid::Uuid;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(serde::Serialize)]
pub struct PromotionCheck {
    pub id: Uuid,
    pub candidate_id: Uuid,
    pub inventory_sha256: String,
    pub manifest_sha256: Option<String>,
    pub decision: String,
    pub operator: String,
    pub reason: String,
    pub created_at: String,
}

/// List the recorded promotion-readiness decisions for a candidate, oldest
/// first. Returns an empty list for unknown candidates: listing never fails
/// closed because it changes nothing.
pub async fn list_checks(pool: &PgPool, candidate_id: Uuid) -> Result<Vec<PromotionCheck>> {
    use sqlx::Row;
    let rows = sqlx::query(
        "SELECT id,candidate_id,inventory_sha256,manifest_sha256,decision,operator,reason,to_char(created_at AT TIME ZONE 'UTC','YYYY-MM-DD\"T\"HH24:MI:SS.MS\"Z\"') AS created_at FROM cloud_snapshot_promotion_checks WHERE candidate_id=$1 ORDER BY cloud_snapshot_promotion_checks.created_at,id",
    )
    .bind(candidate_id)
    .fetch_all(pool)
    .await?;
    let mut checks = Vec::with_capacity(rows.len());
    for row in rows {
        checks.push(PromotionCheck {
            id: row.try_get("id")?,
            candidate_id: row.try_get("candidate_id")?,
            inventory_sha256: row.try_get("inventory_sha256")?,
            manifest_sha256: row.try_get("manifest_sha256")?,
            decision: row.try_get("decision")?,
            operator: row.try_get("operator")?,
            reason: row.try_get("reason")?,
            created_at: row.try_get("created_at")?,
        });
    }
    Ok(checks)
}

fn valid_operator(operator: &str) -> bool {
    !operator.is_empty()
        && operator.len() <= 80
        && operator
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.@".contains(&b))
}

/// Record an audited promotion-readiness decision for a private candidate.
///
/// This entry point records a blocked result. Verified results require
/// `record_verified` and the manifest returned by the verifier. `reason` is a
/// short human-readable summary stored verbatim up to 1024 bytes.
/// Returns the new check row id.
pub async fn record_decision(
    pool: &PgPool,
    candidate_id: Uuid,
    decision: &str,
    operator: &str,
    reason: &str,
) -> Result<Uuid> {
    if decision != "blocked" {
        return Err("verified decisions require a verified inventory hash".into());
    }
    record_decision_inner(pool, candidate_id, decision, operator, reason, None).await
}

/// Record success only if the candidate manifest is unchanged since verification.
pub(crate) async fn record_verified(
    pool: &PgPool,
    candidate_id: Uuid,
    manifest: &Manifest,
    operator: &str,
    reason: &str,
) -> Result<Uuid> {
    record_decision_inner(
        pool,
        candidate_id,
        "verified",
        operator,
        reason,
        Some(manifest),
    )
    .await
}

async fn record_decision_inner(
    pool: &PgPool,
    candidate_id: Uuid,
    decision: &str,
    operator: &str,
    reason: &str,
    expected_manifest: Option<&Manifest>,
) -> Result<Uuid> {
    if decision != "verified" && decision != "blocked" {
        return Err("decision must be verified or blocked".into());
    }
    if !valid_operator(operator) {
        return Err("invalid operator label".into());
    }
    if reason.is_empty() || reason.len() > 1024 {
        return Err("reason must be between 1 and 1024 characters".into());
    }
    let mut tx = pool.begin().await?;
    let inventory: Option<String> = sqlx::query_scalar(
        "SELECT inventory_sha256 FROM cloud_snapshot_candidates WHERE id=$1 FOR UPDATE",
    )
    .bind(candidate_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(inventory) = inventory else {
        return Err("candidate not found".into());
    };
    let manifest_hash = if let Some(expected) = expected_manifest {
        // The parent lock prevents new referenced rows while the file locks fence updates/deletes.
        sqlx::query(
            "SELECT path FROM cloud_snapshot_candidate_files WHERE candidate_id=$1 FOR SHARE",
        )
        .bind(candidate_id)
        .fetch_all(&mut *tx)
        .await?;
        let current = snapshot_export::manifest(&mut tx, candidate_id).await?;
        if current != *expected || current.inventory != inventory {
            return Err("candidate manifest changed after verification".into());
        }
        Some(
            tiktools_plugin_package::Sha256Digest::of_bytes(&serde_json::to_vec(expected)?)
                .to_string(),
        )
    } else {
        None
    };
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO cloud_snapshot_promotion_checks(id,candidate_id,inventory_sha256,manifest_sha256,decision,operator,reason) VALUES($1,$2,$3,$4,$5,$6,$7)",
    )
    .bind(id)
    .bind(candidate_id)
    .bind(&inventory)
    .bind(&manifest_hash)
    .bind(decision)
    .bind(operator)
    .bind(reason)
    .execute(&mut *tx)
    .await?;
    let action = if decision == "verified" {
        "snapshot.candidate_verified"
    } else {
        "snapshot.candidate_verification_blocked"
    };
    sqlx::query(
        "INSERT INTO cloud_audit_events(id,actor_user_id,subject_id,action) VALUES($1,$2,$3,$4)",
    )
    .bind(Uuid::new_v4())
    .bind(format!("operator:{operator}"))
    .bind(candidate_id)
    .bind(action)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(id)
}
