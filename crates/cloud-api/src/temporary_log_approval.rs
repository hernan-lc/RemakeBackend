//! Temporary, explicitly enabled operator override. Possession of service-log
//! access and an authenticated admin session replaces independent review here.
use crate::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use better_auth::{
    axum_integration::AuthUser,
    rate_limit::{RateLimitPolicy, RateLimiter},
};
use better_auth_core::error::AuthError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Row, Transaction};
use std::time::Duration;
use uuid::Uuid;

type Error = (StatusCode, Json<serde_json::Value>);
fn error(status: StatusCode, code: &str) -> Error {
    (status, Json(serde_json::json!({ "error": code })))
}
fn db(_: sqlx::Error) -> Error {
    tracing::error!("temporary approval database operation failed");
    error(StatusCode::INTERNAL_SERVER_ERROR, "internal_error")
}
async fn rate_limit(state: &AppState, actor: &str, name: &str, limit: i64) -> Result<(), Error> {
    let storage = state
        .auth
        .secondary_storage
        .clone()
        .ok_or_else(|| error(StatusCode::SERVICE_UNAVAILABLE, "unavailable"))?;
    let policy = RateLimitPolicy::new(name, limit, Duration::from_secs(60))
        .map_err(|_| error(StatusCode::INTERNAL_SERVER_ERROR, "internal_error"))?;
    RateLimiter::new(storage, "cloud:temporary-approval")
        .enforce(&policy, actor)
        .await
        .map_err(|e| match e {
            AuthError::RateLimited { .. } => error(StatusCode::TOO_MANY_REQUESTS, "rate_limited"),
            _ => error(StatusCode::SERVICE_UNAVAILABLE, "unavailable"),
        })
}
async fn admin(tx: &mut Transaction<'_, Postgres>, user: &str) -> Result<(), Error> {
    let role: Option<String> = sqlx::query_scalar(
        "SELECT role FROM cloud_account_roles WHERE user_id=$1 AND role='admin' FOR SHARE",
    )
    .bind(user)
    .fetch_optional(&mut **tx)
    .await
    .map_err(db)?;
    if role.is_none() {
        return Err(error(StatusCode::FORBIDDEN, "forbidden"));
    }
    Ok(())
}
pub(crate) fn digest(secret: &str, challenge: Uuid, code: &str) -> String {
    let mut h = Sha256::new();
    h.update(b"cloud-temporary-log-approval-v1\0");
    h.update((secret.len() as u64).to_be_bytes());
    h.update(secret.as_bytes());
    h.update(challenge.as_bytes());
    h.update(code.as_bytes());
    hex::encode(h.finalize())
}
fn same_digest(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.as_bytes()
            .iter()
            .zip(b.as_bytes())
            .fold(0u8, |v, (x, y)| v | (x ^ y))
            == 0
}
fn generate_code() -> String {
    // Rejection sampling avoids modulo bias; UUID v4 supplies OS randomness.
    loop {
        let id = Uuid::new_v4();
        let n = u32::from_be_bytes(id.as_bytes()[0..4].try_into().unwrap());
        if n < 4_200_000_000 {
            return format!("{:08}", n % 100_000_000);
        }
    }
}
#[derive(Serialize)]
pub struct Availability {
    enabled: bool,
}
pub async fn availability(
    AuthUser(user): AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Availability>, Error> {
    let mut tx = state.pool.begin().await.map_err(db)?;
    admin(&mut tx, &user.user.id).await?;
    tx.commit().await.map_err(db)?;
    Ok(Json(Availability {
        enabled: state.temporary_log_approvals,
    }))
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Start {
    publisher_id: Uuid,
    key_id: Option<Uuid>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Started {
    challenge_id: Uuid,
    expires_in: u32,
    fingerprint: Option<String>,
}

pub async fn start(
    AuthUser(user): AuthUser,
    State(state): State<AppState>,
    Json(input): Json<Start>,
) -> Result<Json<Started>, Error> {
    if !state.temporary_log_approvals {
        return Err(error(StatusCode::NOT_FOUND, "not_found"));
    }
    rate_limit(&state, &user.user.id, "issue", 3).await?;
    let mut tx = state.pool.begin().await.map_err(db)?;
    admin(&mut tx, &user.user.id).await?;
    // Expired codes have no audit value; approval actions are logged separately.
    sqlx::query(
        "DELETE FROM cloud_temporary_log_approvals WHERE expires_at < now() - interval '1 day'",
    )
    .execute(&mut *tx)
    .await
    .map_err(db)?;
    let status: Option<String> =
        sqlx::query_scalar("SELECT status FROM cloud_publishers WHERE id=$1 FOR UPDATE")
            .bind(input.publisher_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(db)?;
    let status = status.ok_or_else(|| error(StatusCode::NOT_FOUND, "not_found"))?;
    let fingerprint = if let Some(key) = input.key_id {
        let row = sqlx::query("SELECT fingerprint,status FROM cloud_publisher_keys WHERE id=$1 AND publisher_id=$2 FOR UPDATE")
            .bind(key).bind(input.publisher_id).fetch_optional(&mut *tx).await.map_err(db)?
            .ok_or_else(|| error(StatusCode::NOT_FOUND, "not_found"))?;
        if status != "approved" || row.get::<String, _>("status") != "pending" {
            return Err(error(StatusCode::CONFLICT, "conflict"));
        }
        Some(row.get("fingerprint"))
    } else {
        if status != "pending" {
            return Err(error(StatusCode::CONFLICT, "conflict"));
        }
        None
    };
    let id = Uuid::new_v4();
    let code = generate_code();
    let hash = digest(&state.auth.options.secret, id, &code);
    sqlx::query("INSERT INTO cloud_temporary_log_approvals(id,actor_user_id,session_id,publisher_id,key_id,code_digest,expires_at) VALUES($1,$2,$3,$4,$5,$6,now()+interval '5 minutes')")
        .bind(id).bind(&user.user.id).bind(&user.session.id).bind(input.publisher_id).bind(input.key_id).bind(hash).execute(&mut *tx).await.map_err(db)?;
    tx.commit().await.map_err(db)?;
    tracing::warn!(challenge_id=%id, publisher_id=%input.publisher_id, key_id=?input.key_id, confirmation_code=%code, "TEMPORARY operator approval code; expires in five minutes");
    Ok(Json(Started {
        challenge_id: id,
        expires_in: 300,
        fingerprint,
    }))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Confirm {
    code: String,
}
pub async fn confirm(
    AuthUser(user): AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(input): Json<Confirm>,
) -> Result<StatusCode, Error> {
    if !state.temporary_log_approvals {
        return Err(error(StatusCode::NOT_FOUND, "not_found"));
    }
    if input.code.len() != 8 || !input.code.bytes().all(|b| b.is_ascii_digit()) {
        return Err(error(StatusCode::BAD_REQUEST, "invalid_code"));
    }
    rate_limit(&state, &user.user.id, "confirm", 12).await?;
    let mut tx = state.pool.begin().await.map_err(db)?;
    admin(&mut tx, &user.user.id).await?;
    let row = sqlx::query("SELECT publisher_id,key_id,code_digest,attempts,consumed_at IS NOT NULL AS consumed,expires_at<=now() AS expired FROM cloud_temporary_log_approvals WHERE id=$1 AND actor_user_id=$2 AND session_id=$3 FOR UPDATE")
        .bind(id).bind(&user.user.id).bind(&user.session.id).fetch_optional(&mut *tx).await.map_err(db)?
        .ok_or_else(|| error(StatusCode::NOT_FOUND, "not_found"))?;
    if row.get::<bool, _>("consumed")
        || row.get::<bool, _>("expired")
        || row.get::<i32, _>("attempts") >= 5
    {
        return Err(error(StatusCode::FORBIDDEN, "expired_or_used"));
    }
    let expected: String = row.get("code_digest");
    if !same_digest(
        &expected,
        &digest(&state.auth.options.secret, id, &input.code),
    ) {
        sqlx::query("UPDATE cloud_temporary_log_approvals SET attempts=attempts+1 WHERE id=$1")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        tx.commit().await.map_err(db)?;
        return Err(error(StatusCode::FORBIDDEN, "invalid_code"));
    }
    let publisher: Uuid = row.get("publisher_id");
    let key: Option<Uuid> = row.get("key_id");
    let status: Option<String> =
        sqlx::query_scalar("SELECT status FROM cloud_publishers WHERE id=$1 FOR UPDATE")
            .bind(publisher)
            .fetch_optional(&mut *tx)
            .await
            .map_err(db)?;
    let action = if let Some(key) = key {
        if status.as_deref() != Some("approved") {
            return Err(error(StatusCode::CONFLICT, "conflict"));
        }
        let result = sqlx::query("UPDATE cloud_publisher_keys SET status='approved' WHERE id=$1 AND publisher_id=$2 AND status='pending'")
            .bind(key).bind(publisher).execute(&mut *tx).await.map_err(db)?;
        if result.rows_affected() != 1 {
            return Err(error(StatusCode::CONFLICT, "conflict"));
        }
        "publisher_key.temporary_operator_approved"
    } else {
        if status.as_deref() != Some("pending") {
            return Err(error(StatusCode::CONFLICT, "conflict"));
        }
        sqlx::query("UPDATE cloud_publishers SET status='approved' WHERE id=$1")
            .bind(publisher)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        "publisher.temporary_operator_approved"
    };
    sqlx::query("UPDATE cloud_temporary_log_approvals SET consumed_at=now() WHERE id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    sqlx::query("INSERT INTO cloud_audit_events(id,actor_user_id,publisher_id,subject_id,action) VALUES($1,$2,$3,$4,$5)")
        .bind(Uuid::new_v4()).bind(&user.user.id).bind(publisher).bind(key).bind(action).execute(&mut *tx).await.map_err(db)?;
    tx.commit().await.map_err(db)?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_codes_are_eight_digits_and_bound_to_challenge_and_secret() {
        let challenge = Uuid::new_v4();
        let other = Uuid::new_v4();
        for _ in 0..100 {
            let code = generate_code();
            assert_eq!(code.len(), 8);
            assert!(code.bytes().all(|c| c.is_ascii_digit()));
            assert_ne!(
                digest("secret-one", challenge, &code),
                digest("secret-two", challenge, &code)
            );
            assert_ne!(
                digest("secret-one", challenge, &code),
                digest("secret-one", other, &code)
            );
        }
    }
}
