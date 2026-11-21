//! Short-lived login codes relayed to the devices already connected through
//! remote control. No email/SMTP in this phase; no raw token is ever stored.
//!
//! The code is stored only as a keyed hash in Better Auth secondary storage.
//! The raw six-digit value exists only in the relay frame sent over the
//! already-authenticated remote-control WebSocket and in the user's head.

use std::{net::SocketAddr, time::Duration};

use axum::{
    extract::{ConnectInfo, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use better_auth::{
    email_password::User as AuthUserRecord,
    rate_limit::{RateLimitPolicy, RateLimiter},
    session::SessionService,
};
use better_auth_core::{options::BaseUrl, StorageValue};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

use crate::AppState;

const CODE_TTL: Duration = Duration::from_secs(5 * 60);
const RESERVATION_TTL: Duration = Duration::from_secs(5 * 60);
const MAX_ATTEMPTS: u32 = 5;
const MAX_DEVICES: i64 = 8;
const MAX_EMAIL: usize = 256;
const MAX_NONCE: usize = 128;

/// Columns selected by `load_user`: id, email, name, email_verified, image,
/// additional_fields.
type UserRow = (String, String, String, bool, Option<String>, Option<String>);

fn hash(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}

fn reservation_key(id: &str) -> String {
    format!("cloud:login-reservation:{}", hash(id))
}

fn code_key(id: &str) -> String {
    format!("cloud:login-code:{}", hash(id))
}

fn code_digest(secret: &str, reservation_id: &str, nonce: &str, code: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"tiktools-login-code-v1\0");
    hasher.update(secret.as_bytes());
    hasher.update(reservation_id.as_bytes());
    hasher.update(b"\0");
    hasher.update(nonce.as_bytes());
    hasher.update(b"\0");
    hasher.update(code.as_bytes());
    hex::encode(hasher.finalize())
}

fn secret(state: &AppState) -> Result<String, StatusCode> {
    let value = state.auth.options.secret.clone();
    if value.is_empty() {
        Err(StatusCode::INTERNAL_SERVER_ERROR)
    } else {
        Ok(value)
    }
}

/// Origin string embedded in the reservation and checked at redeem time, so a
/// code issued by one instance cannot be redeemed by another.
fn origin(state: &AppState) -> String {
    match state.auth.options.base_url.as_ref() {
        Some(BaseUrl::Static(value)) => value.clone(),
        Some(BaseUrl::Dynamic { .. }) | None => state.auth.options.secret.clone(),
    }
}

fn kv(
    state: &AppState,
) -> Result<&std::sync::Arc<dyn better_auth_core::SecondaryStorage>, StatusCode> {
    crate::desktop::kv_storage(state)
}

async fn limit(
    state: &AppState,
    action: &str,
    identity: &str,
    count: i64,
) -> Result<(), StatusCode> {
    let backend = state
        .auth
        .secondary_storage
        .clone()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let policy = RateLimitPolicy::new(action, count, Duration::from_secs(60))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    RateLimiter::new(backend, "cloud:login-code")
        .enforce(&policy, identity)
        .await
        .map_err(|error| match error {
            better_auth_core::error::AuthError::RateLimited { .. } => StatusCode::TOO_MANY_REQUESTS,
            _ => StatusCode::SERVICE_UNAVAILABLE,
        })
}

fn peer_identity(headers: &HeaderMap, peer: SocketAddr) -> String {
    // The proxy boundary already strips untrusted forwarding headers.
    headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| peer.ip().to_string())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReserveInput {
    pub email: String,
    pub device_nonce: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReserveOutput {
    pub reservation_id: String,
    pub device_nonce: String,
    pub requires_oauth: bool,
    pub expires_at: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RelayInput {
    pub reservation_id: String,
    pub device_nonce: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayOutput {
    pub delivered: usize,
    pub expires_at: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RedeemInput {
    pub reservation_id: String,
    pub device_nonce: String,
    pub code: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RedeemOutput {
    pub session_token: String,
    pub expires_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Reservation {
    user_id: Option<String>,
    email: String,
    device_nonce: String,
    origin: String,
    generation: i64,
    expires_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodeRecord {
    digest: String,
    attempts: u32,
    expires_at: u64,
    origin: String,
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    OsRng.fill_bytes(&mut buf);
    hex::encode(buf)
}

async fn load_reservation(state: &AppState, id: &str) -> Result<Reservation, StatusCode> {
    let value = kv(state)?
        .get(&reservation_key(id))
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        .ok_or(StatusCode::UNAUTHORIZED)?;
    serde_json::from_value(value.value).map_err(|_| StatusCode::UNAUTHORIZED)
}

/// Reserve a login operation. An unknown account needs OAuth instead of a code.
pub async fn reserve(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    State(state): State<AppState>,
    Json(input): Json<ReserveInput>,
) -> Result<Json<ReserveOutput>, StatusCode> {
    if input.email.is_empty()
        || input.email.len() > MAX_EMAIL
        || !input.email.contains('@')
        || input.device_nonce.is_empty()
        || input.device_nonce.len() > MAX_NONCE
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    limit(
        &state,
        "reserve",
        &peer_identity(&HeaderMap::new(), peer),
        20,
    )
    .await?;

    let user_id: Option<String> = sqlx::query_scalar("SELECT id FROM \"user\" WHERE email = $1")
        .bind(&input.email)
        .fetch_optional(&state.pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let reservation_id = random_hex(16);
    let generation = match &user_id {
        Some(id) => crate::desktop::generation_of(&state, id).await?,
        None => 0,
    };
    let expires_at = now() + RESERVATION_TTL.as_secs();
    let reservation = Reservation {
        user_id,
        email: input.email.clone(),
        device_nonce: input.device_nonce.clone(),
        origin: origin(&state),
        generation,
        expires_at,
    };
    kv(&state)?
        .set(
            &reservation_key(&reservation_id),
            StorageValue {
                value: serde_json::to_value(&reservation)
                    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
                expires_in: Some(RESERVATION_TTL),
            },
        )
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(Json(ReserveOutput {
        reservation_id,
        device_nonce: input.device_nonce,
        requires_oauth: reservation.user_id.is_none(),
        expires_at,
    }))
}

/// Deliver a one-time code to the devices already connected via remote control.
pub async fn relay(
    State(state): State<AppState>,
    Json(input): Json<RelayInput>,
) -> Result<Json<RelayOutput>, StatusCode> {
    let reservation = load_reservation(&state, &input.reservation_id).await?;
    if reservation.device_nonce != input.device_nonce || reservation.expires_at <= now() {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let user_id = reservation
        .user_id
        .as_deref()
        .ok_or(StatusCode::FORBIDDEN)?; // use OAuth

    let sessions: Vec<Uuid> = sqlx::query_scalar(
        "SELECT s.id FROM cloud_remote_sessions s \
         JOIN cloud_remote_hosts h ON h.id = s.host_id \
         WHERE h.owner_user_id = $1 AND s.status = 'active' \
         ORDER BY s.opened_at DESC LIMIT $2",
    )
    .bind(user_id)
    .bind(MAX_DEVICES)
    .fetch_all(&state.pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if sessions.is_empty() {
        return Err(StatusCode::CONFLICT); // master only: no remote control possible
    }

    let mut code_bytes = [0u8; 4];
    OsRng.fill_bytes(&mut code_bytes);
    let number = u32::from_be_bytes(code_bytes) % 1_000_000;
    let code = format!("{number:06}");
    let expires_at = now() + CODE_TTL.as_secs();
    let record = CodeRecord {
        digest: code_digest(
            &secret(&state)?,
            &input.reservation_id,
            &input.device_nonce,
            &code,
        ),
        attempts: 0,
        expires_at,
        origin: reservation.origin.clone(),
    };
    kv(&state)?
        .set(
            &code_key(&input.reservation_id),
            StorageValue {
                value: serde_json::to_value(&record)
                    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
                expires_in: Some(CODE_TTL),
            },
        )
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;

    let mut delivered = 0usize;
    for session in sessions {
        if state
            .remote
            .route_to_session(
                &state.pool,
                session,
                &serde_json::json!({
                    "type": "login.code",
                    "reservationId": input.reservation_id,
                    "code": code,
                    "expiresAt": expires_at,
                    "origin": reservation.origin,
                }),
            )
            .await
        {
            delivered += 1;
        }
    }
    if delivered == 0 {
        return Err(StatusCode::CONFLICT);
    }
    Ok(Json(RelayOutput {
        delivered,
        expires_at,
    }))
}

async fn load_user(pool: &PgPool, user_id: &str) -> Result<AuthUserRecord, StatusCode> {
    let row: Option<UserRow> = sqlx::query_as(
        "SELECT id, email, name, email_verified, image, additional_fields \
             FROM \"user\" WHERE id = $1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let (id, email, name, email_verified, image, additional) =
        row.ok_or(StatusCode::UNAUTHORIZED)?;
    let additional_fields = additional
        .and_then(|value| {
            serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&value).ok()
        })
        .unwrap_or_default();
    Ok(AuthUserRecord {
        id,
        email,
        name,
        email_verified,
        image,
        additional_fields,
    })
}

/// Redeem a relayed code for a native Bearer session token.
pub async fn redeem(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<RedeemInput>,
) -> Result<Json<RedeemOutput>, StatusCode> {
    let identity = peer_identity(&headers, peer);
    limit(&state, "redeem", &identity, 20).await?;
    if input.code.len() != 6 || !input.code.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let reservation = load_reservation(&state, &input.reservation_id).await?;
    if reservation.device_nonce != input.device_nonce
        || reservation.expires_at <= now()
        || reservation.origin != origin(&state)
    {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let user_id = reservation
        .user_id
        .as_deref()
        .ok_or(StatusCode::FORBIDDEN)?;
    if crate::desktop::generation_of(&state, user_id).await? != reservation.generation {
        return Err(StatusCode::UNAUTHORIZED);
    }

    let key = code_key(&input.reservation_id);
    let raw = kv(&state)?
        .get(&key)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        .ok_or(StatusCode::UNAUTHORIZED)?;
    let mut record: CodeRecord =
        serde_json::from_value(raw.value).map_err(|_| StatusCode::UNAUTHORIZED)?;
    if record.expires_at <= now() || record.origin != reservation.origin {
        let _ = kv(&state)?.delete(&key).await;
        return Err(StatusCode::UNAUTHORIZED);
    }
    let presented = code_digest(
        &secret(&state)?,
        &input.reservation_id,
        &input.device_nonce,
        &input.code,
    );
    if presented != record.digest {
        record.attempts += 1;
        if record.attempts >= MAX_ATTEMPTS {
            let _ = kv(&state)?.delete(&key).await;
        } else {
            let remaining = record.expires_at.saturating_sub(now());
            let _ = kv(&state)?
                .set(
                    &key,
                    StorageValue {
                        value: serde_json::to_value(&record)
                            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
                        expires_in: Some(Duration::from_secs(remaining.max(1))),
                    },
                )
                .await;
        }
        return Err(StatusCode::UNAUTHORIZED);
    }
    let _ = kv(&state)?.get_and_delete(&key).await;

    let user = load_user(&state.pool, user_id).await?;
    let result = SessionService::new(state.auth.clone())
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .create(user, true)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    crate::desktop::invalidate_handoffs(&state, user_id).await?;
    Ok(Json(RedeemOutput {
        session_token: result.session_token,
        expires_at: result.session.expires_at,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_digest_is_bound_to_reservation_nonce_and_secret() {
        let a = code_digest("secret", "reservation", "nonce", "123456");
        assert_eq!(a, code_digest("secret", "reservation", "nonce", "123456"));
        assert_ne!(a, code_digest("secret", "reservation", "nonce", "654321"));
        assert_ne!(a, code_digest("secret", "reservation", "other", "123456"));
        assert_ne!(a, code_digest("other", "reservation", "nonce", "123456"));
        assert_eq!(a.len(), 64);
    }

    #[test]
    fn keys_never_contain_raw_ids() {
        let key = reservation_key("reservation-secret-id");
        assert!(key.starts_with("cloud:login-reservation:"));
        assert!(!key.contains("reservation-secret-id"));
        assert_eq!(key.len(), "cloud:login-reservation:".len() + 64);
    }

    #[test]
    fn attempts_are_bounded() {
        // Five wrong guesses burn the code; the constant is part of the contract.
        assert_eq!(MAX_ATTEMPTS, 5);
        assert_eq!(CODE_TTL, Duration::from_secs(300));
    }
}
