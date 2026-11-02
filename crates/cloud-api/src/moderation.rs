//! Review decisions are independent from publication and cannot bypass verification.
use crate::AppState;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use better_auth::axum_integration::AuthUser;
use serde::{Deserialize, Serialize};
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;

fn db(_: sqlx::Error) -> StatusCode {
    tracing::error!("moderation database operation failed");
    StatusCode::INTERNAL_SERVER_ERROR
}
pub(crate) async fn reviewer(
    tx: &mut Transaction<'_, Postgres>,
    user: &str,
) -> Result<(), StatusCode> {
    // Serialize decisions with role revocation; an account needs at least one live role.
    let role:Option<String>=sqlx::query_scalar("SELECT role FROM cloud_account_roles WHERE user_id=$1 AND role IN ('admin','moderator') ORDER BY role FOR SHARE")
        .bind(user).fetch_optional(&mut **tx).await.map_err(db)?;
    if role.is_none() {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Page {
    after: Option<Uuid>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    id: Uuid,
    publisher_id: Uuid,
    namespace: String,
    plugin_id: String,
    version: String,
    target: String,
    sha256: String,
    size: i64,
    key_id: String,
    key_fingerprint: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Queue {
    items: Vec<Candidate>,
    next_cursor: Option<Uuid>,
}
pub async fn queue(
    AuthUser(user): AuthUser,
    State(state): State<AppState>,
    Query(page): Query<Page>,
) -> Result<Json<Queue>, StatusCode> {
    let mut tx = state.pool.begin().await.map_err(db)?;
    reviewer(&mut tx, &user.user.id).await?;
    // Exclude conflicts of interest and records whose verification or authorization is no longer valid.
    let rows=sqlx::query("SELECT r.id,r.publisher_id,p.namespace,r.plugin_id,r.version,r.target,u.expected_sha256,u.expected_size,k.key_id,k.fingerprint FROM cloud_plugin_releases r JOIN cloud_publishers p ON p.id=r.publisher_id JOIN cloud_upload_sessions u ON u.id=r.upload_id AND u.publisher_id=r.publisher_id JOIN cloud_publisher_keys k ON k.id=r.publisher_key_id AND k.publisher_id=r.publisher_id JOIN cloud_verification_jobs j ON j.upload_id=u.id AND j.release_id=r.id WHERE r.status='antivirus_passed' AND j.status='antivirus_passed' AND j.result_class='antivirus_passed' AND p.status='approved' AND k.status='approved' AND r.requested_by<>$1 AND u.requested_by<>$1 AND k.requested_by<>$1 AND NOT EXISTS (SELECT 1 FROM cloud_publisher_memberships m WHERE m.publisher_id=r.publisher_id AND m.user_id=$1) AND ($2::uuid IS NULL OR r.id>$2) ORDER BY r.id LIMIT 51")
        .bind(&user.user.id).bind(page.after).fetch_all(&mut *tx).await.map_err(db)?;
    let more = rows.len() > 50;
    let items: Vec<_> = rows
        .into_iter()
        .take(50)
        .map(|r| Candidate {
            id: r.get("id"),
            publisher_id: r.get("publisher_id"),
            namespace: r.get("namespace"),
            plugin_id: r.get("plugin_id"),
            version: r.get("version"),
            target: r.get("target"),
            sha256: r.get("expected_sha256"),
            size: r.get("expected_size"),
            key_id: r.get("key_id"),
            key_fingerprint: r.get("fingerprint"),
        })
        .collect();
    let next_cursor = if more {
        items.last().map(|i| i.id)
    } else {
        None
    };
    tx.commit().await.map_err(db)?;
    Ok(Json(Queue { items, next_cursor }))
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Decision {
    decision: String,
    reason_code: String,
}
pub async fn decide(
    AuthUser(user): AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(input): Json<Decision>,
) -> Result<StatusCode, StatusCode> {
    let approve = match (input.decision.as_str(), input.reason_code.as_str()) {
        ("approve", "policy_reviewed") => true,
        ("reject", "policy" | "malware" | "other") => false,
        _ => return Err(StatusCode::BAD_REQUEST),
    };
    crate::uploads::limit(&state, &user.user.id, "moderation-decision").await?;
    let mut tx = state.pool.begin().await.map_err(db)?;
    reviewer(&mut tx, &user.user.id).await?;
    // The publisher lock serializes this decision with key, membership, and worker changes.
    let row = sqlx::query(
        "SELECT publisher_id,publisher_key_id,upload_id FROM cloud_plugin_releases WHERE id=$1",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(db)?
    .ok_or(StatusCode::NOT_FOUND)?;
    let publisher: Uuid = row.get("publisher_id");
    let key: Uuid = row.get("publisher_key_id");
    let upload: Uuid = row.get("upload_id");
    let publisher_status: String =
        sqlx::query_scalar("SELECT status FROM cloud_publishers WHERE id=$1 FOR UPDATE")
            .bind(publisher)
            .fetch_one(&mut *tx)
            .await
            .map_err(db)?;
    let member:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM cloud_publisher_memberships WHERE publisher_id=$1 AND user_id=$2)").bind(publisher).bind(&user.user.id).fetch_one(&mut *tx).await.map_err(db)?;
    if member {
        return Err(StatusCode::FORBIDDEN);
    }
    let (key_status, key_requester): (String, String) = sqlx::query_as(
        "SELECT status,requested_by FROM cloud_publisher_keys WHERE id=$1 AND publisher_id=$2 FOR UPDATE",
    )
    .bind(key)
    .bind(publisher)
    .fetch_one(&mut *tx)
    .await
    .map_err(db)?;
    let upload_requester: String = sqlx::query_scalar(
        "SELECT requested_by FROM cloud_upload_sessions WHERE id=$1 AND publisher_id=$2",
    )
    .bind(upload)
    .bind(publisher)
    .fetch_one(&mut *tx)
    .await
    .map_err(db)?;
    let row =
        sqlx::query("SELECT status,requested_by FROM cloud_plugin_releases WHERE id=$1 FOR UPDATE")
            .bind(id)
            .fetch_one(&mut *tx)
            .await
            .map_err(db)?;
    if row.get::<String, _>("requested_by") == user.user.id
        || key_requester == user.user.id
        || upload_requester == user.user.id
    {
        return Err(StatusCode::FORBIDDEN);
    }
    let previous=sqlx::query("SELECT actor_user_id,decision,reason_code FROM cloud_moderation_events WHERE release_id=$1").bind(id).fetch_optional(&mut *tx).await.map_err(db)?;
    if let Some(previous) = previous {
        if previous.get::<String, _>("actor_user_id") == user.user.id
            && previous.get::<String, _>("decision") == input.decision
            && previous.get::<String, _>("reason_code") == input.reason_code
        {
            tx.commit().await.map_err(db)?;
            return Ok(StatusCode::NO_CONTENT);
        }
        return Err(StatusCode::CONFLICT);
    }
    let verification:Option<(String,Option<String>)>=sqlx::query_as("SELECT status,result_class FROM cloud_verification_jobs WHERE upload_id=$1 AND release_id=$2 FOR UPDATE").bind(upload).bind(id).fetch_optional(&mut *tx).await.map_err(db)?;
    if row.get::<String, _>("status") != "antivirus_passed"
        || verification.as_ref().is_none_or(|(s, r)| {
            s != "antivirus_passed" || r.as_deref() != Some("antivirus_passed")
        })
    {
        return Err(StatusCode::CONFLICT);
    }
    if approve && (publisher_status != "approved" || key_status != "approved") {
        return Err(StatusCode::CONFLICT);
    }
    let status = if approve {
        "review_approved"
    } else {
        "review_rejected"
    };
    sqlx::query("UPDATE cloud_plugin_releases SET status=$2 WHERE id=$1")
        .bind(id)
        .bind(status)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    sqlx::query("INSERT INTO cloud_moderation_events (id,release_id,actor_user_id,decision,reason_code) VALUES ($1,$2,$3,$4,$5)")
        .bind(Uuid::new_v4()).bind(id).bind(&user.user.id).bind(&input.decision).bind(&input.reason_code).execute(&mut *tx).await.map_err(db)?;
    sqlx::query("INSERT INTO cloud_audit_events (id,actor_user_id,publisher_id,subject_id,action) VALUES ($1,$2,$3,$4,$5)")
        .bind(Uuid::new_v4()).bind(&user.user.id).bind(publisher).bind(id).bind(if approve {"release.review_approved"} else {"release.review_rejected"}).execute(&mut *tx).await.map_err(db)?;
    tx.commit().await.map_err(db)?;
    Ok(StatusCode::NO_CONTENT)
}

// Never derive Debug: the returned URL is a bearer capability.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Inspection {
    url: String,
    expires_in: u64,
}
async fn inspection_metadata(
    tx: &mut Transaction<'_, Postgres>,
    user: &str,
    id: Uuid,
) -> Result<(Uuid, String, String), StatusCode> {
    reviewer(tx, user).await?;
    let row = sqlx::query(
        "SELECT publisher_id,publisher_key_id,upload_id FROM cloud_plugin_releases WHERE id=$1",
    )
    .bind(id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(db)?
    .ok_or(StatusCode::NOT_FOUND)?;
    let publisher: Uuid = row.get("publisher_id");
    let key: Uuid = row.get("publisher_key_id");
    let upload: Uuid = row.get("upload_id");
    let publisher_status: String =
        sqlx::query_scalar("SELECT status FROM cloud_publishers WHERE id=$1 FOR UPDATE")
            .bind(publisher)
            .fetch_one(&mut **tx)
            .await
            .map_err(db)?;
    let member:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM cloud_publisher_memberships WHERE publisher_id=$1 AND user_id=$2)").bind(publisher).bind(user).fetch_one(&mut **tx).await.map_err(db)?;
    if member {
        return Err(StatusCode::FORBIDDEN);
    }
    let (key_status,key_requester):(String,String)=sqlx::query_as("SELECT status,requested_by FROM cloud_publisher_keys WHERE id=$1 AND publisher_id=$2 FOR UPDATE").bind(key).bind(publisher).fetch_one(&mut **tx).await.map_err(db)?;
    let row=sqlx::query("SELECT r.status AS release_status,r.requested_by AS release_requester,u.requested_by AS upload_requester,u.object_key,u.version_id,j.status AS job_status,j.result_class FROM cloud_plugin_releases r JOIN cloud_upload_sessions u ON u.id=r.upload_id AND u.publisher_id=r.publisher_id JOIN cloud_verification_jobs j ON j.upload_id=u.id AND j.release_id=r.id WHERE r.id=$1 AND u.id=$2")
        .bind(id).bind(upload).fetch_one(&mut **tx).await.map_err(db)?;
    if key_requester == user
        || row.get::<String, _>("release_requester") == user
        || row.get::<String, _>("upload_requester") == user
    {
        return Err(StatusCode::FORBIDDEN);
    }
    if publisher_status != "approved"
        || key_status != "approved"
        || row.get::<String, _>("release_status") != "antivirus_passed"
        || row.get::<String, _>("job_status") != "antivirus_passed"
        || row.get::<Option<String>, _>("result_class").as_deref() != Some("antivirus_passed")
    {
        return Err(StatusCode::CONFLICT);
    }
    let version: Option<String> = row.get("version_id");
    Ok((
        publisher,
        row.get("object_key"),
        version.ok_or(StatusCode::CONFLICT)?,
    ))
}
pub async fn inspection(
    AuthUser(user): AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Inspection>, StatusCode> {
    crate::uploads::limit(&state, &user.user.id, "moderation-inspection").await?;
    let mut tx = state.pool.begin().await.map_err(db)?;
    let metadata = inspection_metadata(&mut tx, &user.user.id, id).await?;
    tx.commit().await.map_err(db)?;
    let store = state
        .object_storage
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let ttl = std::time::Duration::from_secs(60);
    let signed = store
        .presign_inspection(&metadata.1, &metadata.2, ttl)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    // Authorization may have changed while storage was contacted. Do not return a stale capability.
    let mut tx = state.pool.begin().await.map_err(db)?;
    let current = inspection_metadata(&mut tx, &user.user.id, id).await?;
    if current != metadata {
        return Err(StatusCode::CONFLICT);
    }
    sqlx::query("INSERT INTO cloud_audit_events (id,actor_user_id,publisher_id,subject_id,action) VALUES ($1,$2,$3,$4,'release.inspection_granted')")
        .bind(Uuid::new_v4()).bind(&user.user.id).bind(metadata.0).bind(id).execute(&mut *tx).await.map_err(db)?;
    tx.commit().await.map_err(db)?;
    Ok(Json(Inspection {
        url: signed.url,
        expires_in: ttl.as_secs(),
    }))
}
