//! Single-use connection tickets.
//!
//! A ticket is issued only after the caller proves possession of the device key
//! for an exact binding (host, device, role). Host tickets bind to the host
//! device; controller tickets also bind to the active grant and its revision.

use super::{
    binding_digest, db, new_secret, parse_body, parse_uuid, proof_message, secret_hash, throttle,
    verify_proof, TICKET_TTL,
};
use crate::{
    api_error::{Authenticated, Kind, RequestId, V3Error},
    AppState,
};
use axum::{body::Bytes, extract::State, http::StatusCode, Extension, Json};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::{PgPool, Row};
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IssueRequest {
    #[serde(rename = "challengeId")]
    challenge_id: Uuid,
    binding: Value,
    signature: String,
}

struct Binding {
    host: Uuid,
    device: Uuid,
    role: String,
}

fn parse_binding(rid: &RequestId, binding: &Value) -> Result<Binding, V3Error> {
    let text = |key: &str| binding.get(key).and_then(Value::as_str);
    let host = parse_uuid(
        rid,
        text("hostId").ok_or(V3Error::new(rid, Kind::InvalidRequest))?,
    )?;
    let device = parse_uuid(
        rid,
        text("deviceId").ok_or(V3Error::new(rid, Kind::InvalidRequest))?,
    )?;
    let role = text("role").ok_or(V3Error::new(rid, Kind::InvalidRequest))?;
    if !matches!(role, "host" | "controller") || binding.as_object().map(|o| o.len()) != Some(3) {
        return Err(V3Error::new(rid, Kind::InvalidRequest));
    }
    Ok(Binding {
        host,
        device,
        role: role.to_owned(),
    })
}

/// Validates the authority a ticket would carry, before a challenge is issued.
pub async fn check_ticket_binding(
    pool: &PgPool,
    rid: &RequestId,
    owner: &str,
    binding: &Value,
) -> Result<(), V3Error> {
    let parsed = parse_binding(rid, binding)?;
    let device = sqlx::query(
        "SELECT kind,host_id,status FROM cloud_remote_devices WHERE id=$1 AND owner_user_id=$2",
    )
    .bind(parsed.device)
    .bind(owner)
    .fetch_optional(pool)
    .await
    .map_err(db(rid))?
    .ok_or(V3Error::new(rid, Kind::NotFound))?;
    if device.get::<String, _>("status") != "active" {
        return Err(V3Error::new(rid, Kind::Forbidden));
    }
    let host = sqlx::query("SELECT status,owner_user_id FROM cloud_remote_hosts WHERE id=$1")
        .bind(parsed.host)
        .fetch_optional(pool)
        .await
        .map_err(db(rid))?
        .ok_or(V3Error::new(rid, Kind::NotFound))?;
    // Host tickets belong to the team owner; controller tickets belong to a grant holder.
    if parsed.role == "host" && host.get::<String, _>("owner_user_id") != *owner {
        return Err(V3Error::new(rid, Kind::NotFound));
    }
    if host.get::<String, _>("status") != "active" {
        return Err(V3Error::new(rid, Kind::NotFound));
    }
    authority_for(
        pool,
        rid,
        owner,
        &parsed,
        &device.get::<String, _>("kind"),
        device.get::<Option<Uuid>, _>("host_id"),
    )
    .await
    .map(|_| ())
}

/// The grant revision a controller ticket carries, or `None` for host tickets.
async fn authority_for(
    pool: &PgPool,
    rid: &RequestId,
    owner: &str,
    parsed: &Binding,
    kind: &str,
    device_host: Option<Uuid>,
) -> Result<Option<(Uuid, i64)>, V3Error> {
    match parsed.role.as_str() {
        "host" => {
            if kind != "host" || device_host != Some(parsed.host) {
                return Err(V3Error::new(rid, Kind::Forbidden));
            }
            Ok(None)
        }
        _ => {
            if kind != "controller" {
                return Err(V3Error::new(rid, Kind::Forbidden));
            }
            let grant = sqlx::query("SELECT id,revision FROM cloud_remote_grants WHERE host_id=$1 AND device_id=$2 AND status='active' AND requester_user_id=$3")
                .bind(parsed.host)
                .bind(parsed.device)
                .bind(owner)
                .fetch_optional(pool)
                .await
                .map_err(db(rid))?
                .ok_or(V3Error::new(rid, Kind::Forbidden))?;
            Ok(Some((grant.get("id"), grant.get("revision"))))
        }
    }
}

pub async fn issue(
    Authenticated(user): Authenticated,
    Extension(rid): Extension<RequestId>,
    State(state): State<AppState>,
    body: Bytes,
) -> Result<(StatusCode, Json<Value>), V3Error> {
    let request: IssueRequest = parse_body(&rid, &body, 4 * 1024)?;
    let owner = &user.user.id;
    throttle(&state, &rid, owner, "remote-ticket", 60).await?;
    let parsed = parse_binding(&rid, &request.binding)?;
    let digest = binding_digest(&request.binding);
    let mut tx = state.pool.begin().await.map_err(db(&rid))?;
    let challenge = sqlx::query("SELECT nonce,binding_sha256 FROM cloud_remote_challenges WHERE id=$1 AND owner_user_id=$2 AND purpose='ticket' AND consumed_at IS NULL AND expires_at>now() FOR UPDATE")
        .bind(request.challenge_id)
        .bind(owner)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db(&rid))?
        .ok_or(V3Error::new(&rid, Kind::Expired))?;
    let nonce: Vec<u8> = challenge.get("nonce");
    let stored: String = challenge.get("binding_sha256");
    if stored != digest {
        return Err(V3Error::new(&rid, Kind::InvalidRequest));
    }
    let device = sqlx::query("SELECT kind,host_id,status,public_key FROM cloud_remote_devices WHERE id=$1 AND owner_user_id=$2 FOR SHARE")
        .bind(parsed.device)
        .bind(owner)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db(&rid))?
        .ok_or(V3Error::new(&rid, Kind::NotFound))?;
    if device.get::<String, _>("status") != "active" {
        return Err(V3Error::new(&rid, Kind::Forbidden));
    }
    let host =
        sqlx::query("SELECT status,owner_user_id FROM cloud_remote_hosts WHERE id=$1 FOR SHARE")
            .bind(parsed.host)
            .fetch_optional(&mut *tx)
            .await
            .map_err(db(&rid))?
            .ok_or(V3Error::new(&rid, Kind::NotFound))?;
    if parsed.role == "host" && host.get::<String, _>("owner_user_id") != *owner {
        return Err(V3Error::new(&rid, Kind::NotFound));
    }
    if host.get::<String, _>("status") != "active" {
        return Err(V3Error::new(&rid, Kind::Conflict));
    }
    let public_key: Vec<u8> = device.get("public_key");
    let message = proof_message("ticket", &nonce, &stored);
    if !verify_proof(&public_key, &message, &request.signature) {
        return Err(V3Error::new(&rid, Kind::Unauthorized));
    }
    let grant = authority_for(
        &state.pool,
        &rid,
        owner,
        &parsed,
        &device.get::<String, _>("kind"),
        device.get::<Option<Uuid>, _>("host_id"),
    )
    .await?;
    let secret = new_secret();
    let id = Uuid::new_v4();
    let expires = chrono::Utc::now() + chrono::Duration::from_std(TICKET_TTL).unwrap_or_default();
    sqlx::query("INSERT INTO cloud_remote_tickets(id,token_sha256,role,owner_user_id,host_id,device_id,grant_id,grant_revision,expires_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)")
        .bind(id)
        .bind(secret_hash(&secret))
        .bind(&parsed.role)
        .bind(owner)
        .bind(parsed.host)
        .bind(parsed.device)
        .bind(grant.map(|(g, _)| g))
        .bind(grant.map(|(_, r)| r))
        .bind(expires)
        .execute(&mut *tx)
        .await
        .map_err(db(&rid))?;
    sqlx::query("UPDATE cloud_remote_challenges SET consumed_at=now() WHERE id=$1")
        .bind(request.challenge_id)
        .execute(&mut *tx)
        .await
        .map_err(db(&rid))?;
    tx.commit().await.map_err(db(&rid))?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "ticket": secret,
            "role": parsed.role,
            "hostId": parsed.host,
            "expiresAt": expires.to_rfc3339(),
        })),
    ))
}
