//! A submission binds immutable upload metadata and an approved key; it does not publish.
use crate::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use better_auth::axum_integration::AuthUser;

use crate::dual_auth::DualUser;
use serde::{Deserialize, Serialize};
use sqlx::{Postgres, Row, Transaction};
use tiktools_plugin_api::manifest::{is_plugin_target, is_valid_plugin_id, parse_version};
use uuid::Uuid;

fn db(_: sqlx::Error) -> StatusCode {
    tracing::error!("release database operation failed");
    StatusCode::INTERNAL_SERVER_ERROR
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Submission {
    upload_id: Uuid,
    publisher_key_id: Uuid,
    plugin_id: String,
    version: String,
    target: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdvisoryView {
    severity: String,
    message: String,
    url: Option<String>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Release {
    id: Uuid,
    publisher_id: Uuid,
    upload_id: Uuid,
    publisher_key_id: Uuid,
    plugin_id: String,
    version: String,
    target: String,
    status: String,
    yanked: bool,
    yank_reason: Option<String>,
    advisory: Option<AdvisoryView>,
}
fn release(row: sqlx::postgres::PgRow) -> Release {
    Release {
        id: row.get("id"),
        publisher_id: row.get("publisher_id"),
        upload_id: row.get("upload_id"),
        publisher_key_id: row.get("publisher_key_id"),
        plugin_id: row.get("plugin_id"),
        version: row.get("version"),
        target: row.get("target"),
        status: row.get("status"),
        yanked: false,
        yank_reason: None,
        advisory: None,
    }
}
async fn revocation(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
) -> Result<(bool, Option<String>, Option<AdvisoryView>), StatusCode> {
    let yank: Option<String> =
        sqlx::query_scalar("SELECT reason FROM cloud_release_yanks WHERE release_id=$1")
            .bind(id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(db)?;
    let advisory: Option<AdvisoryView> = sqlx::query(
        "SELECT severity,message,url FROM cloud_release_advisories WHERE release_id=$1",
    )
    .bind(id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(db)?
    .map(|row: sqlx::postgres::PgRow| AdvisoryView {
        severity: row.get("severity"),
        message: row.get("message"),
        url: row.get("url"),
    });
    Ok((yank.is_some(), yank, advisory))
}
const FIELDS: &str = "id,publisher_id,upload_id,publisher_key_id,plugin_id,version,target,status";
async fn authorize(
    tx: &mut Transaction<'_, Postgres>,
    publisher: Uuid,
    user: &str,
    write: bool,
) -> Result<String, StatusCode> {
    let row = sqlx::query("SELECT namespace,status FROM cloud_publishers WHERE id=$1 FOR UPDATE")
        .bind(publisher)
        .fetch_optional(&mut **tx)
        .await
        .map_err(db)?;
    let role: Option<String> = sqlx::query_scalar(
        "SELECT role FROM cloud_publisher_memberships WHERE publisher_id=$1 AND user_id=$2",
    )
    .bind(publisher)
    .bind(user)
    .fetch_optional(&mut **tx)
    .await
    .map_err(db)?;
    let (Some(row), Some(role)) = (row, role) else {
        return Err(StatusCode::NOT_FOUND);
    };
    if write
        && (row.get::<String, _>("status") != "approved"
            || !matches!(role.as_str(), "owner" | "admin" | "publisher"))
    {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(row.get("namespace"))
}
fn valid(input: &Submission, namespace: &str) -> bool {
    input.plugin_id.len() <= 256
        && is_valid_plugin_id(&input.plugin_id)
        && input
            .plugin_id
            .strip_prefix(namespace)
            .is_some_and(|rest| rest.starts_with('.') && rest.len() > 1)
        && input.version.len() <= 128
        && parse_version(&input.version).is_some_and(|version| version.to_string() == input.version)
        && input.target.len() <= 128
        && (input.target == "any" || is_plugin_target(&input.target))
}
pub async fn submit(
    DualUser(user): DualUser,
    State(state): State<AppState>,
    Json(input): Json<Submission>,
) -> Result<(StatusCode, Json<Release>), StatusCode> {
    crate::uploads::limit(&state, &user.user.id, "release-submit").await?;
    let mut tx = state.pool.begin().await.map_err(db)?;
    // Lookup exposes no metadata; the membership check follows before any response.
    let publisher: Uuid =
        sqlx::query_scalar("SELECT publisher_id FROM cloud_upload_sessions WHERE id=$1")
            .bind(input.upload_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(db)?
            .ok_or(StatusCode::NOT_FOUND)?;
    let namespace = authorize(&mut tx, publisher, &user.user.id, true).await?;
    if !valid(&input, &namespace) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let query = format!("SELECT {FIELDS} FROM cloud_plugin_releases WHERE upload_id=$1");
    if let Some(row) = sqlx::query(&query)
        .bind(input.upload_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db)?
    {
        let mut old = release(row);
        let (yanked, yank_reason, advisory) = revocation(&mut tx, old.id).await?;
        old.yanked = yanked;
        old.yank_reason = yank_reason;
        old.advisory = advisory;
        if old.publisher_key_id != input.publisher_key_id
            || old.plugin_id != input.plugin_id
            || old.version != input.version
            || old.target != input.target
        {
            return Err(StatusCode::CONFLICT);
        }
        tx.commit().await.map_err(db)?;
        return Ok((StatusCode::OK, Json(old)));
    }
    let upload_status: String = sqlx::query_scalar(
        "SELECT status FROM cloud_upload_sessions WHERE id=$1 AND publisher_id=$2 FOR UPDATE",
    )
    .bind(input.upload_id)
    .bind(publisher)
    .fetch_one(&mut *tx)
    .await
    .map_err(db)?;
    if upload_status != "quarantined" {
        return Err(StatusCode::CONFLICT);
    }
    let key_status: Option<String> = sqlx::query_scalar(
        "SELECT status FROM cloud_publisher_keys WHERE id=$1 AND publisher_id=$2 FOR UPDATE",
    )
    .bind(input.publisher_key_id)
    .bind(publisher)
    .fetch_optional(&mut *tx)
    .await
    .map_err(db)?;
    if key_status.as_deref() != Some("approved") {
        return Err(StatusCode::CONFLICT);
    }
    let id = Uuid::new_v4();
    let result=sqlx::query("INSERT INTO cloud_plugin_releases (id,publisher_id,upload_id,publisher_key_id,plugin_id,version,target,status,requested_by) VALUES ($1,$2,$3,$4,$5,$6,$7,'awaiting_verification',$8)")
        .bind(id).bind(publisher).bind(input.upload_id).bind(input.publisher_key_id).bind(&input.plugin_id).bind(&input.version).bind(&input.target).bind(&user.user.id).execute(&mut *tx).await;
    if let Err(e) = result {
        if e.as_database_error()
            .is_some_and(|e| e.is_unique_violation())
        {
            return Err(StatusCode::CONFLICT);
        }
        return Err(db(e));
    }
    let result=sqlx::query("UPDATE cloud_verification_jobs SET release_id=$2 WHERE upload_id=$1 AND status='queued' AND release_id IS NULL").bind(input.upload_id).bind(id).execute(&mut *tx).await.map_err(db)?;
    if result.rows_affected() != 1 {
        return Err(StatusCode::CONFLICT);
    }
    sqlx::query("INSERT INTO cloud_audit_events (id,actor_user_id,publisher_id,subject_id,action) VALUES ($1,$2,$3,$4,'release.submitted')")
        .bind(Uuid::new_v4()).bind(&user.user.id).bind(publisher).bind(id).execute(&mut *tx).await.map_err(db)?;
    tx.commit().await.map_err(db)?;
    Ok((
        StatusCode::CREATED,
        Json(Release {
            id,
            publisher_id: publisher,
            upload_id: input.upload_id,
            publisher_key_id: input.publisher_key_id,
            plugin_id: input.plugin_id,
            version: input.version,
            target: input.target,
            status: "awaiting_verification".into(),
            yanked: false,
            yank_reason: None,
            advisory: None,
        }),
    ))
}
pub async fn get(
    DualUser(user): DualUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Release>, StatusCode> {
    let mut tx = state.pool.begin().await.map_err(db)?;
    let query = format!("SELECT {FIELDS} FROM cloud_plugin_releases WHERE id=$1");
    let row = sqlx::query(&query)
        .bind(id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let mut result = release(row);
    authorize(&mut tx, result.publisher_id, &user.user.id, false).await?;
    let (yanked, yank_reason, advisory) = revocation(&mut tx, result.id).await?;
    result.yanked = yanked;
    result.yank_reason = yank_reason;
    result.advisory = advisory;
    tx.commit().await.map_err(db)?;
    Ok(Json(result))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Review {
    decision: String,
    reason_code: String,
    yanked: bool,
    yank_reason: Option<String>,
    advisory: Option<AdvisoryView>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicRevocation {
    id: Uuid,
    plugin_id: String,
    version: String,
    target: String,
    yanked: bool,
    yank_reason: Option<String>,
    advisory: Option<AdvisoryView>,
    key_revoked: bool,
}

/// Public revocation state for registry clients. No authentication: only the
/// release identity, its yank/advisory state, and whether the signing key is
/// revoked are exposed. Tenant, upload, and key identifiers stay hidden.
pub async fn revocation_status(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<PublicRevocation>, StatusCode> {
    let mut tx = state.pool.begin().await.map_err(db)?;
    let row = sqlx::query(
        "SELECT id,plugin_id,version,target,publisher_key_id FROM cloud_plugin_releases WHERE id=$1",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(db)?
    .ok_or(StatusCode::NOT_FOUND)?;
    let (yanked, yank_reason, advisory) = revocation(&mut tx, id).await?;
    let key_status: Option<String> =
        sqlx::query_scalar("SELECT status FROM cloud_publisher_keys WHERE id=$1")
            .bind(row.get::<Uuid, _>("publisher_key_id"))
            .fetch_optional(&mut *tx)
            .await
            .map_err(db)?;
    tx.commit().await.map_err(db)?;
    Ok(Json(PublicRevocation {
        id: row.get("id"),
        plugin_id: row.get("plugin_id"),
        version: row.get("version"),
        target: row.get("target"),
        yanked,
        yank_reason,
        advisory,
        key_revoked: key_status.as_deref() == Some("revoked"),
    }))
}
pub async fn review(
    AuthUser(user): AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Review>, StatusCode> {
    let mut tx = state.pool.begin().await.map_err(db)?;
    let publisher: Uuid =
        sqlx::query_scalar("SELECT publisher_id FROM cloud_plugin_releases WHERE id=$1")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(db)?
            .ok_or(StatusCode::NOT_FOUND)?;
    authorize(&mut tx, publisher, &user.user.id, false).await?;
    let row =
        sqlx::query("SELECT decision,reason_code FROM cloud_moderation_events WHERE release_id=$1")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(db)?
            .ok_or(StatusCode::NOT_FOUND)?;
    let (yanked, yank_reason, advisory) = revocation(&mut tx, id).await?;
    let result = Review {
        decision: row.get("decision"),
        reason_code: row.get("reason_code"),
        yanked,
        yank_reason,
        advisory,
    };
    tx.commit().await.map_err(db)?;
    Ok(Json(result))
}
