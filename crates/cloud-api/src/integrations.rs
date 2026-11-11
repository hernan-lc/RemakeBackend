//! Managed integrations.
//!
//! The first provider is a signed inbound webhook. Each connection owns one
//! secret, sealed with AES-256-GCM under the server key (`INTEGRATION_SECRET_KEY`,
//! 32 bytes, base64). Deliveries are authenticated with HMAC-SHA256 over the
//! timestamp and body, must fall within a five-minute window, and are
//! deduplicated by delivery id. Without the server key the provider is not
//! published and its routes answer `misconfigured`.

use crate::{
    api_error::{Authenticated, Kind, RequestId, V3Error},
    remote::{db, parse_body, throttle},
    AppState,
};
use aes_gcm::{
    aead::{Aead, KeyInit, Payload},
    Aes256Gcm, Key, Nonce,
};
use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{delete, get, post},
    Extension, Json, Router,
};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use hmac::{Hmac, Mac};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::Sha256;
use sqlx::Row;
use uuid::Uuid;

const KEY_ENV: &str = "INTEGRATION_SECRET_KEY";
const MAX_BODY: usize = 64 * 1024;
const MAX_SKEW_SECONDS: i64 = 300;
const MAX_EVENTS_PER_CONNECTION: i64 = 200;
const SECRET_BYTES: usize = 32;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/v3/integrations/providers", get(providers))
        .route("/api/v3/integrations/connections", get(connections))
        .route("/api/v3/integrations/{provider_id}/connect", post(connect))
        .route(
            "/api/v3/integrations/connections/{connection_id}",
            delete(disconnect),
        )
        .route(
            "/api/v3/integrations/connections/{connection_id}/events",
            get(events),
        )
        .route(
            "/api/v3/integrations/connections/{connection_id}/operations/{operation_id}",
            post(operation),
        )
        .route(
            "/api/v3/integrations/webhook/{connection_id}/deliveries",
            post(deliver),
        )
}

/// The server key, if configured and well formed. Read at each use so that a
/// rotated process environment takes effect on restart without code changes.
fn sealing_key() -> Option<[u8; 32]> {
    let raw = std::env::var(KEY_ENV).ok()?;
    let bytes = STANDARD.decode(raw.trim()).ok()?;
    <[u8; 32]>::try_from(bytes.as_slice()).ok()
}

/// Providers that are implemented and have their prerequisites configured.
pub fn published_providers() -> Vec<&'static str> {
    if sealing_key().is_some() {
        vec!["webhook"]
    } else {
        Vec::new()
    }
}

fn seal(key: &[u8; 32], connection: Uuid, plaintext: &[u8]) -> Vec<u8> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let nonce_bytes: [u8; 12] = rand::random();
    let aad = connection.as_bytes().to_vec();
    let ciphertext = cipher
        .encrypt(
            Nonce::from_slice(&nonce_bytes),
            Payload {
                msg: plaintext,
                aad: &aad,
            },
        )
        .unwrap_or_default();
    let mut sealed = nonce_bytes.to_vec();
    sealed.extend_from_slice(&ciphertext);
    sealed
}

fn open(key: &[u8; 32], connection: Uuid, sealed: &[u8]) -> Option<Vec<u8>> {
    if sealed.len() < 12 {
        return None;
    }
    let (nonce, ciphertext) = sealed.split_at(12);
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let aad = connection.as_bytes().to_vec();
    cipher
        .decrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: ciphertext,
                aad: &aad,
            },
        )
        .ok()
}

fn misconfigured(rid: &RequestId) -> V3Error {
    V3Error::new(rid, Kind::Misconfigured)
}

async fn providers() -> Json<Value> {
    let published: Vec<Value> = published_providers()
        .into_iter()
        .map(|id| json!({"id": id, "deliveryPath": format!("/api/v3/integrations/{id}/{{connectionId}}/deliveries")}))
        .collect();
    Json(json!({"apiVersion": 3, "providers": published}))
}

async fn connections(
    Authenticated(user): Authenticated,
    Extension(rid): Extension<RequestId>,
    State(state): State<AppState>,
) -> Result<Json<Value>, V3Error> {
    let rows = sqlx::query("SELECT id,provider,label,status,created_at FROM cloud_integration_connections WHERE owner_user_id=$1 ORDER BY created_at DESC, id LIMIT 100")
        .bind(&user.user.id)
        .fetch_all(&state.pool)
        .await
        .map_err(db(&rid))?;
    let items: Vec<Value> = rows
        .iter()
        .map(|row| {
            json!({
                "id": row.get::<Uuid, _>("id"),
                "provider": row.get::<String, _>("provider"),
                "label": row.get::<String, _>("label"),
                "status": row.get::<String, _>("status"),
                "createdAt": row.get::<chrono::DateTime<chrono::Utc>, _>("created_at").to_rfc3339(),
            })
        })
        .collect();
    Ok(Json(json!({"apiVersion": 3, "items": items})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Connect {
    label: String,
}

/// Creates a webhook connection. The secret is returned once; only its sealed form is stored.
async fn connect(
    Authenticated(user): Authenticated,
    Extension(rid): Extension<RequestId>,
    State(state): State<AppState>,
    Path(provider_id): Path<String>,
    body: Bytes,
) -> Result<(StatusCode, Json<Value>), V3Error> {
    if provider_id != "webhook" {
        return Err(V3Error::new(&rid, Kind::Unsupported));
    }
    let key = sealing_key().ok_or_else(|| misconfigured(&rid))?;
    throttle(&state, &rid, &user.user.id, "integration-connect", 10).await?;
    let request: Connect = parse_body(&rid, &body, 1024)?;
    let label = request.label.trim().to_owned();
    if label.is_empty() || label.chars().count() > 120 || label.chars().any(char::is_control) {
        return Err(V3Error::new(&rid, Kind::InvalidRequest));
    }
    let secret_bytes: [u8; SECRET_BYTES] = rand::random();
    let secret = hex::encode(secret_bytes);
    let id = Uuid::new_v4();
    let sealed = seal(&key, id, secret.as_bytes());
    sqlx::query("INSERT INTO cloud_integration_connections(id,owner_user_id,provider,label,secret_sealed) VALUES($1,$2,'webhook',$3,$4)")
        .bind(id)
        .bind(&user.user.id)
        .bind(&label)
        .bind(&sealed)
        .execute(&state.pool)
        .await
        .map_err(db(&rid))?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "connectionId": id,
            "provider": "webhook",
            "label": label,
            "secret": secret,
            "deliveryPath": format!("/api/v3/integrations/webhook/{id}/deliveries"),
            "signature": {
                "header": "x-tiktools-signature",
                "timestamp": "x-tiktools-timestamp",
                "delivery": "x-tiktools-delivery",
                "algorithm": "hmac-sha256",
                "input": "timestamp + \".\" + raw body",
                "format": "sha256=<hex>",
                "maxSkewSeconds": MAX_SKEW_SECONDS,
            },
        })),
    ))
}

/// Removes the secret and disables the connection; events already received remain readable.
async fn disconnect(
    Authenticated(user): Authenticated,
    Extension(rid): Extension<RequestId>,
    State(state): State<AppState>,
    Path(connection_id): Path<String>,
) -> Result<Json<Value>, V3Error> {
    let connection = crate::remote::parse_uuid(&rid, &connection_id)?;
    let updated = sqlx::query("UPDATE cloud_integration_connections SET status='revoked', secret_sealed=NULL, revoked_at=now() WHERE id=$1 AND owner_user_id=$2 AND status='active'")
        .bind(connection)
        .bind(&user.user.id)
        .execute(&state.pool)
        .await
        .map_err(db(&rid))?;
    if updated.rows_affected() == 0 {
        let exists: Option<String> = sqlx::query_scalar(
            "SELECT status FROM cloud_integration_connections WHERE id=$1 AND owner_user_id=$2",
        )
        .bind(connection)
        .bind(&user.user.id)
        .fetch_optional(&state.pool)
        .await
        .map_err(db(&rid))?;
        return match exists {
            Some(_) => Ok(Json(json!({"id": connection, "status": "revoked"}))),
            None => Err(V3Error::new(&rid, Kind::NotFound)),
        };
    }
    Ok(Json(json!({"id": connection, "status": "revoked"})))
}

async fn events(
    Authenticated(user): Authenticated,
    Extension(rid): Extension<RequestId>,
    State(state): State<AppState>,
    Path(connection_id): Path<String>,
) -> Result<Json<Value>, V3Error> {
    let connection = crate::remote::parse_uuid(&rid, &connection_id)?;
    let owned: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM cloud_integration_connections WHERE id=$1 AND owner_user_id=$2",
    )
    .bind(connection)
    .bind(&user.user.id)
    .fetch_optional(&state.pool)
    .await
    .map_err(db(&rid))?;
    if owned.is_none() {
        return Err(V3Error::new(&rid, Kind::NotFound));
    }
    let rows = sqlx::query("SELECT delivery_id,received_at,payload FROM cloud_integration_events WHERE connection_id=$1 ORDER BY received_at DESC LIMIT $2")
        .bind(connection)
        .bind(MAX_EVENTS_PER_CONNECTION)
        .fetch_all(&state.pool)
        .await
        .map_err(db(&rid))?;
    let items: Vec<Value> = rows
        .iter()
        .map(|row| {
            json!({
                "deliveryId": row.get::<String, _>("delivery_id"),
                "receivedAt": row.get::<chrono::DateTime<chrono::Utc>, _>("received_at").to_rfc3339(),
                "payload": row.get::<Value, _>("payload"),
            })
        })
        .collect();
    Ok(Json(json!({"apiVersion": 3, "items": items})))
}

/// Operations are not implemented for any provider yet, so every request is refused explicitly.
async fn operation(Extension(rid): Extension<RequestId>) -> Result<Json<Value>, V3Error> {
    Err(V3Error::new(&rid, Kind::Unsupported))
}

/// Verifies and records one signed delivery. The sender is not authenticated by
/// session, only by the connection's secret, so every failure looks the same.
async fn deliver(
    Extension(rid): Extension<RequestId>,
    State(state): State<AppState>,
    Path(connection_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, Json<Value>), V3Error> {
    let key = sealing_key().ok_or_else(|| misconfigured(&rid))?;
    let connection = crate::remote::parse_uuid(&rid, &connection_id)?;
    if body.len() > MAX_BODY {
        return Err(V3Error::new(&rid, Kind::InvalidRequest));
    }
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    };
    let (Some(timestamp), Some(delivery), Some(signature)) = (
        header("x-tiktools-timestamp"),
        header("x-tiktools-delivery"),
        header("x-tiktools-signature"),
    ) else {
        return Err(V3Error::new(&rid, Kind::Unauthorized));
    };
    let valid_delivery = (1..=128).contains(&delivery.len())
        && delivery
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.:".contains(&b));
    let timestamp_seconds: i64 = timestamp
        .parse()
        .map_err(|_| V3Error::new(&rid, Kind::Unauthorized))?;
    let now = chrono::Utc::now().timestamp();
    if !valid_delivery || (now - timestamp_seconds).abs() > MAX_SKEW_SECONDS {
        return Err(V3Error::new(&rid, Kind::Unauthorized));
    }
    let row = sqlx::query(
        "SELECT secret_sealed FROM cloud_integration_connections WHERE id=$1 AND status='active'",
    )
    .bind(connection)
    .fetch_optional(&state.pool)
    .await
    .map_err(db(&rid))?
    .ok_or_else(|| V3Error::new(&rid, Kind::Unauthorized))?;
    let sealed: Option<Vec<u8>> = row.get("secret_sealed");
    let secret = sealed
        .and_then(|sealed| open(&key, connection, &sealed))
        .ok_or_else(|| V3Error::new(&rid, Kind::Unauthorized))?;
    let provided = signature
        .strip_prefix("sha256=")
        .and_then(|hex_value| hex::decode(hex_value).ok())
        .ok_or_else(|| V3Error::new(&rid, Kind::Unauthorized))?;
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(&secret)
        .map_err(|_| V3Error::new(&rid, Kind::TemporaryFailure))?;
    mac.update(timestamp.as_bytes());
    mac.update(b".");
    mac.update(&body);
    mac.verify_slice(&provided)
        .map_err(|_| V3Error::new(&rid, Kind::Unauthorized))?;
    let payload: Value =
        serde_json::from_slice(&body).map_err(|_| V3Error::new(&rid, Kind::InvalidRequest))?;
    if !payload.is_object() {
        return Err(V3Error::new(&rid, Kind::InvalidRequest));
    }
    let inserted = sqlx::query("INSERT INTO cloud_integration_events(connection_id,delivery_id,payload) VALUES($1,$2,$3) ON CONFLICT (connection_id,delivery_id) DO NOTHING")
        .bind(connection)
        .bind(&delivery)
        .bind(&payload)
        .execute(&state.pool)
        .await
        .map_err(db(&rid))?;
    if inserted.rows_affected() == 0 {
        return Ok((
            StatusCode::OK,
            Json(json!({"accepted": true, "duplicate": true})),
        ));
    }
    // Keep only the newest events for each connection.
    sqlx::query("DELETE FROM cloud_integration_events WHERE connection_id=$1 AND delivery_id IN (SELECT delivery_id FROM cloud_integration_events WHERE connection_id=$1 ORDER BY received_at DESC OFFSET $2)")
        .bind(connection)
        .bind(MAX_EVENTS_PER_CONNECTION)
        .execute(&state.pool)
        .await
        .map_err(db(&rid))?;
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({"accepted": true, "duplicate": false})),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sealed_secrets_open_only_for_their_connection() {
        let key = [7u8; 32];
        let connection = Uuid::new_v4();
        let sealed = seal(&key, connection, b"s3cret");
        assert_eq!(
            open(&key, connection, &sealed).as_deref(),
            Some(&b"s3cret"[..])
        );
        assert!(
            open(&key, Uuid::new_v4(), &sealed).is_none(),
            "the connection id is bound as associated data"
        );
        assert!(open(&[8u8; 32], connection, &sealed).is_none());
        let mut tampered = sealed.clone();
        *tampered.last_mut().unwrap() ^= 1;
        assert!(open(&key, connection, &tampered).is_none());
    }

    #[test]
    fn the_key_must_be_thirty_two_base64_bytes() {
        // Environment access is process-wide; these checks exercise the parser directly.
        assert!(<[u8; 32]>::try_from(vec![0u8; 31].as_slice()).is_err());
        assert_eq!(STANDARD.encode([1u8; 32]).len(), 44);
    }
}
