//! Authenticated upload reservations. Quarantined objects are not verified or published.
use crate::AppState;
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use better_auth::{
    axum_integration::AuthUser,
    rate_limit::{RateLimitPolicy, RateLimiter},
};
use cloud_storage::{QuarantineStore, MAX_ARTIFACT_SIZE};
use serde::{Deserialize, Serialize};
use sqlx::{Postgres, Row, Transaction};
use std::{collections::BTreeMap, time::Duration};
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateInput {
    publisher_id: Uuid,
    size: i64,
    sha256: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadOutput {
    id: Uuid,
    url: String,
    headers: BTreeMap<String, String>,
    expires_in: u64,
}
#[derive(Serialize)]
pub struct CompleteOutput {
    id: Uuid,
    status: &'static str,
}

fn db(_: sqlx::Error) -> StatusCode {
    tracing::error!("upload database operation failed");
    StatusCode::INTERNAL_SERVER_ERROR
}
fn storage(_: cloud_storage::StorageError) -> StatusCode {
    StatusCode::SERVICE_UNAVAILABLE
}
fn store(state: &AppState) -> Result<&QuarantineStore, StatusCode> {
    state
        .object_storage
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)
}
pub(crate) async fn limit(state: &AppState, user: &str, action: &str) -> Result<(), StatusCode> {
    let backend = state
        .auth
        .secondary_storage
        .clone()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let policy = RateLimitPolicy::new(action, 30, Duration::from_secs(60))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    RateLimiter::new(backend, "cloud:uploads")
        .enforce(&policy, user)
        .await
        .map_err(|e| match e {
            better_auth_core::error::AuthError::RateLimited { .. } => StatusCode::TOO_MANY_REQUESTS,
            _ => StatusCode::SERVICE_UNAVAILABLE,
        })
}
// Membership mutations and approval also lock the publisher row.
async fn authorize(
    tx: &mut Transaction<'_, Postgres>,
    publisher: Uuid,
    user: &str,
) -> Result<(), StatusCode> {
    let status: Option<String> =
        sqlx::query_scalar("SELECT status FROM cloud_publishers WHERE id = $1 FOR UPDATE")
            .bind(publisher)
            .fetch_optional(&mut **tx)
            .await
            .map_err(db)?;
    let role: Option<String> = sqlx::query_scalar(
        "SELECT role FROM cloud_publisher_memberships WHERE publisher_id = $1 AND user_id = $2",
    )
    .bind(publisher)
    .bind(user)
    .fetch_optional(&mut **tx)
    .await
    .map_err(db)?;
    if status.is_none() || role.is_none() {
        return Err(StatusCode::NOT_FOUND);
    }
    if status.as_deref() != Some("approved") || role.as_deref() == Some("viewer") {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(())
}
fn idem(headers: &HeaderMap) -> Result<&str, StatusCode> {
    let mut values = headers.get_all("idempotency-key").iter();
    let value = values
        .next()
        .ok_or(StatusCode::BAD_REQUEST)?
        .to_str()
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    if values.next().is_some()
        || !(16..=128).contains(&value.len())
        || !value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_.~".contains(&c))
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    Ok(value)
}

pub async fn create(
    AuthUser(principal): AuthUser,
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<CreateInput>,
) -> Result<Json<UploadOutput>, StatusCode> {
    let key = idem(&headers)?;
    if !(1..=MAX_ARTIFACT_SIZE).contains(&input.size)
        || input.sha256.len() != 64
        || !input
            .sha256
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    let store = store(&state)?;
    limit(&state, &principal.user.id, "create").await?;
    let mut tx = state.pool.begin().await.map_err(db)?;
    authorize(&mut tx, input.publisher_id, &principal.user.id).await?;
    let existing = sqlx::query("SELECT id, requested_by, expected_size, expected_sha256, object_key, status, floor(extract(epoch from (expires_at - now())))::bigint AS remaining FROM cloud_upload_sessions WHERE publisher_id = $1 AND idempotency_key = $2")
        .bind(input.publisher_id).bind(key).fetch_optional(&mut *tx).await.map_err(db)?;
    let (id, object_key, remaining) = if let Some(row) = existing {
        if row.get::<String, _>("requested_by") != principal.user.id
            || row.get::<i64, _>("expected_size") != input.size
            || row.get::<String, _>("expected_sha256") != input.sha256
            || row.get::<String, _>("status") != "pending"
        {
            return Err(StatusCode::CONFLICT);
        }
        let remaining: i64 = row.get("remaining");
        if remaining < 1 {
            return Err(StatusCode::GONE);
        }
        (row.get("id"), row.get("object_key"), remaining)
    } else {
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM cloud_upload_sessions WHERE publisher_id = $1 AND status = 'pending' AND expires_at > now()")
            .bind(input.publisher_id).fetch_one(&mut *tx).await.map_err(db)?;
        if count >= 10 {
            return Err(StatusCode::TOO_MANY_REQUESTS);
        }
        let id = Uuid::new_v4();
        let object_key = format!("quarantine/{}/{id}", input.publisher_id);
        sqlx::query("INSERT INTO cloud_upload_sessions (id, publisher_id, requested_by, idempotency_key, expected_size, expected_sha256, object_key, status, expires_at) VALUES ($1,$2,$3,$4,$5,$6,$7,'pending',now() + interval '5 minutes')")
            .bind(id).bind(input.publisher_id).bind(&principal.user.id).bind(key).bind(input.size).bind(&input.sha256).bind(&object_key).execute(&mut *tx).await.map_err(db)?;
        sqlx::query("INSERT INTO cloud_audit_events (id, actor_user_id, publisher_id, action) VALUES ($1,$2,$3,'upload.requested')")
            .bind(Uuid::new_v4()).bind(&principal.user.id).bind(input.publisher_id).execute(&mut *tx).await.map_err(db)?;
        (id, object_key, 299)
    };
    tx.commit().await.map_err(db)?;
    // A failed presign leaves a reservation that can be retried with the same key.
    let ttl = remaining.min(299) as u64;
    let signed = store
        .presign_upload(
            &object_key,
            input.size,
            &input.sha256,
            Duration::from_secs(ttl),
        )
        .await
        .map_err(storage)?;
    Ok(Json(UploadOutput {
        id,
        url: signed.url,
        headers: signed.headers,
        expires_in: ttl,
    }))
}

pub async fn complete(
    AuthUser(principal): AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<CompleteOutput>, StatusCode> {
    let store = store(&state)?;
    limit(&state, &principal.user.id, "complete").await?;
    let mut tx = state.pool.begin().await.map_err(db)?;
    let row = sqlx::query("SELECT publisher_id FROM cloud_upload_sessions WHERE id = $1")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let publisher: Uuid = row.get("publisher_id");
    authorize(&mut tx, publisher, &principal.user.id).await?;
    let row = sqlx::query("SELECT status, object_key, expected_size, expires_at > now() AS active FROM cloud_upload_sessions WHERE id = $1")
        .bind(id).fetch_one(&mut *tx).await.map_err(db)?;
    if row.get::<String, _>("status") == "quarantined" {
        return Ok(Json(CompleteOutput {
            id,
            status: "quarantined",
        }));
    }
    if !row.get::<bool, _>("active") {
        return Err(StatusCode::GONE);
    }
    let object_key: String = row.get("object_key");
    let size: i64 = row.get("expected_size");
    tx.commit().await.map_err(db)?;
    let version = store.capture(&object_key, size).await.map_err(storage)?;
    let mut tx = state.pool.begin().await.map_err(db)?;
    authorize(&mut tx, publisher, &principal.user.id).await?;
    let row = sqlx::query("SELECT status, expires_at > now() AS active FROM cloud_upload_sessions WHERE id = $1 FOR UPDATE")
        .bind(id).fetch_one(&mut *tx).await.map_err(db)?;
    if row.get::<String, _>("status") == "quarantined" {
        return Ok(Json(CompleteOutput {
            id,
            status: "quarantined",
        }));
    }
    if !row.get::<bool, _>("active") {
        return Err(StatusCode::GONE);
    }
    sqlx::query(
        "UPDATE cloud_upload_sessions SET status = 'quarantined', version_id = $2 WHERE id = $1",
    )
    .bind(id)
    .bind(version.version_id)
    .execute(&mut *tx)
    .await
    .map_err(db)?;
    sqlx::query("INSERT INTO cloud_verification_jobs (upload_id, status) VALUES ($1, 'queued')")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    sqlx::query("INSERT INTO cloud_audit_events (id, actor_user_id, publisher_id, action) VALUES ($1,$2,$3,'upload.quarantined')").bind(Uuid::new_v4()).bind(&principal.user.id).bind(publisher).execute(&mut *tx).await.map_err(db)?;
    tx.commit().await.map_err(db)?;
    Ok(Json(CompleteOutput {
        id,
        status: "quarantined",
    }))
}
