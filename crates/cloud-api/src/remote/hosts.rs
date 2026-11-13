//! Teams (hosts), device identities and proof-of-possession challenges.

use super::{
    binding_digest, db, decode_public_key, parse_body, parse_uuid, proof_message, throttle,
    verify_proof, CHALLENGE_TTL, MAX_LIST, PROTOCOL_VERSION,
};
use crate::{
    api_error::{Authenticated, Kind, RequestId, V3Error},
    AppState,
};
use axum::{
    body::Bytes,
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::{PgPool, Row};
use uuid::Uuid;

const SMALL_BODY: usize = 16 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChallengeRequest {
    purpose: String,
    binding: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnrollRequest {
    #[serde(rename = "challengeId")]
    challenge_id: Uuid,
    binding: Value,
    signature: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateHost {
    name: String,
    #[serde(rename = "protocolVersion")]
    protocol_version: i32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateHost {
    name: Option<String>,
    status: Option<String>,
    #[serde(rename = "expectedRevision")]
    expected_revision: i64,
}

fn valid_name(name: &str) -> bool {
    let trimmed = name.trim();
    !trimmed.is_empty() && trimmed.chars().count() <= 120 && !trimmed.chars().any(char::is_control)
}

fn text_field<'a>(binding: &'a Value, key: &str) -> Option<&'a str> {
    binding.get(key).and_then(Value::as_str)
}

/// Checks the enrollment binding. Host devices must name a host the caller owns.
async fn check_enroll_binding(
    pool: &PgPool,
    rid: &RequestId,
    owner: &str,
    binding: &Value,
) -> Result<(), V3Error> {
    let label = text_field(binding, "label").ok_or(V3Error::new(rid, Kind::InvalidRequest))?;
    if !valid_name(label) {
        return Err(V3Error::new(rid, Kind::InvalidRequest));
    }
    let kind = text_field(binding, "kind").ok_or(V3Error::new(rid, Kind::InvalidRequest))?;
    let key = text_field(binding, "publicKey").ok_or(V3Error::new(rid, Kind::InvalidRequest))?;
    if decode_public_key(key).is_none() {
        return Err(V3Error::new(rid, Kind::InvalidRequest));
    }
    match kind {
        "controller" => {
            if binding.get("hostId").is_some() {
                return Err(V3Error::new(rid, Kind::InvalidRequest));
            }
        }
        "host" => {
            let host = text_field(binding, "hostId")
                .ok_or(V3Error::new(rid, Kind::InvalidRequest))
                .and_then(|raw| parse_uuid(rid, raw))?;
            let found: Option<String> = sqlx::query_scalar(
                "SELECT status FROM cloud_remote_hosts WHERE id=$1 AND owner_user_id=$2",
            )
            .bind(host)
            .bind(owner)
            .fetch_optional(pool)
            .await
            .map_err(db(rid))?;
            match found.as_deref() {
                Some("active") => {}
                Some(_) => return Err(V3Error::new(rid, Kind::Forbidden)),
                None => return Err(V3Error::new(rid, Kind::NotFound)),
            }
        }
        _ => return Err(V3Error::new(rid, Kind::InvalidRequest)),
    }
    Ok(())
}

/// Issues a single-use challenge. The signed message is rebuilt server-side from
/// the stored nonce and binding digest; the client only signs what it receives.
pub async fn create_challenge(
    Authenticated(user): Authenticated,
    Extension(rid): Extension<RequestId>,
    State(state): State<AppState>,
    body: Bytes,
) -> Result<(StatusCode, Json<Value>), V3Error> {
    let request: ChallengeRequest = parse_body(&rid, &body, SMALL_BODY)?;
    let owner = &user.user.id;
    throttle(&state, &rid, owner, "remote-challenge", 60).await?;
    match request.purpose.as_str() {
        "peer" => super::peers::check_binding(&state, &rid, owner, &request.binding).await?,
        "peer-grant" => {
            super::peer_grants::check_binding(&state, &rid, owner, &request.binding).await?
        }
        "enroll" => check_enroll_binding(&state.pool, &rid, owner, &request.binding).await?,
        "ticket" => {
            super::tickets::check_ticket_binding(&state.pool, &rid, owner, &request.binding).await?
        }
        _ => return Err(V3Error::new(&rid, Kind::InvalidRequest)),
    }
    let nonce: [u8; 32] = rand::random();
    let digest = binding_digest(&request.binding);
    let id = Uuid::new_v4();
    let expires =
        chrono::Utc::now() + chrono::Duration::from_std(CHALLENGE_TTL).unwrap_or_default();
    sqlx::query(
        "INSERT INTO cloud_remote_challenges(id,owner_user_id,purpose,nonce,binding_sha256,expires_at) VALUES($1,$2,$3,$4,$5,$6)",
    )
    .bind(id)
    .bind(owner)
    .bind(&request.purpose)
    .bind(nonce.as_slice())
    .bind(&digest)
    .bind(expires)
    .execute(&state.pool)
    .await
    .map_err(db(&rid))?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": id,
            "purpose": request.purpose,
            "nonce": URL_SAFE_NO_PAD.encode(nonce),
            "bindingSha256": digest,
            "expiresAt": expires.to_rfc3339(),
        })),
    ))
}

/// Registers a device after the caller proves possession of the new key.
pub async fn enroll_device(
    Authenticated(user): Authenticated,
    Extension(rid): Extension<RequestId>,
    State(state): State<AppState>,
    body: Bytes,
) -> Result<(StatusCode, Json<Value>), V3Error> {
    let request: EnrollRequest = parse_body(&rid, &body, SMALL_BODY)?;
    let owner = &user.user.id;
    throttle(&state, &rid, owner, "remote-enroll", 20).await?;
    check_enroll_binding(&state.pool, &rid, owner, &request.binding).await?;
    let key_text = text_field(&request.binding, "publicKey").unwrap_or_default();
    let public_key = decode_public_key(key_text).ok_or(V3Error::new(&rid, Kind::InvalidRequest))?;
    let mut tx = state.pool.begin().await.map_err(db(&rid))?;
    let challenge = sqlx::query(
        "SELECT nonce,binding_sha256 FROM cloud_remote_challenges WHERE id=$1 AND owner_user_id=$2 AND purpose='enroll' AND consumed_at IS NULL AND expires_at>now() FOR UPDATE",
    )
    .bind(request.challenge_id)
    .bind(owner)
    .fetch_optional(&mut *tx)
    .await
    .map_err(db(&rid))?
    .ok_or(V3Error::new(&rid, Kind::Expired))?;
    let nonce: Vec<u8> = challenge.get("nonce");
    let stored_digest: String = challenge.get("binding_sha256");
    if binding_digest(&request.binding) != stored_digest {
        return Err(V3Error::new(&rid, Kind::InvalidRequest));
    }
    let message = proof_message("enroll", &nonce, &stored_digest);
    if !verify_proof(&public_key, &message, &request.signature) {
        return Err(V3Error::new(&rid, Kind::Unauthorized));
    }
    let label = text_field(&request.binding, "label")
        .unwrap_or_default()
        .trim()
        .to_owned();
    let kind = text_field(&request.binding, "kind")
        .unwrap_or_default()
        .to_owned();
    let host_id = match kind.as_str() {
        "host" => Some(parse_uuid(
            &rid,
            text_field(&request.binding, "hostId").unwrap_or_default(),
        )?),
        _ => None,
    };
    if let Some(host) = host_id {
        // One active host identity per team; the lock serializes concurrent enrollments.
        sqlx::query("SELECT id FROM cloud_remote_hosts WHERE id=$1 FOR UPDATE")
            .bind(host)
            .fetch_optional(&mut *tx)
            .await
            .map_err(db(&rid))?;
        let existing: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM cloud_remote_devices WHERE host_id=$1 AND kind='host' AND status='active'",
        )
        .bind(host)
        .fetch_one(&mut *tx)
        .await
        .map_err(db(&rid))?;
        if existing > 0 {
            return Err(V3Error::new(&rid, Kind::Conflict));
        }
    }
    let device_id = Uuid::new_v4();
    let inserted = sqlx::query(
        "INSERT INTO cloud_remote_devices(id,owner_user_id,label,kind,host_id,public_key) VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT (public_key) DO NOTHING",
    )
    .bind(device_id)
    .bind(owner)
    .bind(&label)
    .bind(&kind)
    .bind(host_id)
    .bind(public_key.as_slice())
    .execute(&mut *tx)
    .await
    .map_err(db(&rid))?;
    if inserted.rows_affected() == 0 {
        return Err(V3Error::new(&rid, Kind::Conflict));
    }
    sqlx::query("UPDATE cloud_remote_challenges SET consumed_at=now() WHERE id=$1")
        .bind(request.challenge_id)
        .execute(&mut *tx)
        .await
        .map_err(db(&rid))?;
    audit(&mut tx, &rid, owner, host_id, "device.enrolled", device_id).await?;
    tx.commit().await.map_err(db(&rid))?;
    Ok((
        StatusCode::CREATED,
        Json(
            json!({"id": device_id, "label": label, "kind": kind, "hostId": host_id, "status": "active"}),
        ),
    ))
}

pub async fn list_devices(
    Authenticated(user): Authenticated,
    Extension(rid): Extension<RequestId>,
    State(state): State<AppState>,
) -> Result<Json<Value>, V3Error> {
    let rows = sqlx::query(
        "SELECT id,label,kind,host_id,status,created_at FROM cloud_remote_devices WHERE owner_user_id=$1 ORDER BY id LIMIT $2",
    )
    .bind(&user.user.id)
    .bind(MAX_LIST * 4)
    .fetch_all(&state.pool)
    .await
    .map_err(db(&rid))?;
    let items: Vec<Value> = rows.iter().map(device_view).collect();
    Ok(Json(json!({"items": items})))
}

fn device_view(row: &sqlx::postgres::PgRow) -> Value {
    json!({
        "id": row.get::<Uuid, _>("id"),
        "label": row.get::<String, _>("label"),
        "kind": row.get::<String, _>("kind"),
        "hostId": row.get::<Option<Uuid>, _>("host_id"),
        "status": row.get::<String, _>("status"),
        "createdAt": row.get::<chrono::DateTime<chrono::Utc>, _>("created_at").to_rfc3339(),
    })
}

/// Revokes a device and everything it authorizes, then disconnects its links.
pub async fn revoke_device(
    Authenticated(user): Authenticated,
    Extension(rid): Extension<RequestId>,
    State(state): State<AppState>,
    Path(device_id): Path<String>,
) -> Result<Json<Value>, V3Error> {
    let device = parse_uuid(&rid, &device_id)?;
    let owner = &user.user.id;
    let mut tx = state.pool.begin().await.map_err(db(&rid))?;
    let found: Option<(String, Option<Uuid>)> = sqlx::query_as(
        "SELECT status,host_id FROM cloud_remote_devices WHERE id=$1 AND owner_user_id=$2 FOR UPDATE",
    )
    .bind(device)
    .bind(owner)
    .fetch_optional(&mut *tx)
    .await
    .map_err(db(&rid))?;
    let Some((_, host_id)) = found else {
        return Err(V3Error::new(&rid, Kind::NotFound));
    };
    sqlx::query("UPDATE cloud_remote_devices SET status='revoked', revoked_at=COALESCE(revoked_at, now()) WHERE id=$1")
        .bind(device)
        .execute(&mut *tx)
        .await
        .map_err(db(&rid))?;
    let affected_hosts: Vec<Uuid> = sqlx::query_scalar(
        "UPDATE cloud_remote_grants SET status='revoked', revision=revision+1, confirmed_revision=NULL, updated_at=now() WHERE device_id=$1 AND status<>'revoked' RETURNING host_id",
    )
    .bind(device)
    .fetch_all(&mut *tx)
    .await
    .map_err(db(&rid))?;
    sqlx::query("UPDATE cloud_remote_sessions SET status='revoked', closed_at=now() WHERE device_id=$1 AND status='active'")
        .bind(device)
        .execute(&mut *tx)
        .await
        .map_err(db(&rid))?;
    sqlx::query("UPDATE cloud_remote_tickets SET consumed_at=now() WHERE device_id=$1 AND consumed_at IS NULL")
        .bind(device)
        .execute(&mut *tx)
        .await
        .map_err(db(&rid))?;
    audit(&mut tx, &rid, owner, host_id, "device.revoked", device).await?;
    tx.commit().await.map_err(db(&rid))?;
    state
        .remote
        .close_device(&state.pool, device, host_id, "deviceRevoked")
        .await;
    for host in affected_hosts {
        state.remote.notify_grants_changed(&state.pool, host).await;
    }
    Ok(Json(json!({"id": device, "status": "revoked"})))
}

pub async fn create_host(
    Authenticated(user): Authenticated,
    Extension(rid): Extension<RequestId>,
    State(state): State<AppState>,
    body: Bytes,
) -> Result<(StatusCode, Json<Value>), V3Error> {
    let request: CreateHost = parse_body(&rid, &body, SMALL_BODY)?;
    throttle(&state, &rid, &user.user.id, "remote-host-create", 10).await?;
    if !valid_name(&request.name) {
        return Err(V3Error::new(&rid, Kind::InvalidRequest));
    }
    if request.protocol_version != PROTOCOL_VERSION {
        return Err(V3Error::new(&rid, Kind::Unsupported));
    }
    let id = Uuid::new_v4();
    let name = request.name.trim().to_owned();
    let mut tx = state.pool.begin().await.map_err(db(&rid))?;
    sqlx::query("INSERT INTO cloud_remote_hosts(id,owner_user_id,name,protocol_version) VALUES($1,$2,$3,$4)")
        .bind(id)
        .bind(&user.user.id)
        .bind(&name)
        .bind(request.protocol_version)
        .execute(&mut *tx)
        .await
        .map_err(db(&rid))?;
    audit(&mut tx, &rid, &user.user.id, Some(id), "host.created", id).await?;
    tx.commit().await.map_err(db(&rid))?;
    Ok((
        StatusCode::CREATED,
        Json(
            json!({"id": id, "name": name, "status": "active", "protocolVersion": PROTOCOL_VERSION, "revision": 1}),
        ),
    ))
}

pub async fn list_hosts(
    Authenticated(user): Authenticated,
    Extension(rid): Extension<RequestId>,
    State(state): State<AppState>,
) -> Result<Json<Value>, V3Error> {
    let rows = sqlx::query("SELECT id,name,status,protocol_version,revision FROM cloud_remote_hosts WHERE owner_user_id=$1 ORDER BY id LIMIT $2")
        .bind(&user.user.id)
        .bind(MAX_LIST)
        .fetch_all(&state.pool)
        .await
        .map_err(db(&rid))?;
    let mut items: Vec<Value> = Vec::with_capacity(rows.len());
    for row in &rows {
        let id: Uuid = row.get("id");
        items.push(json!({
            "id": id,
            "name": row.get::<String, _>("name"),
            "status": row.get::<String, _>("status"),
            "protocolVersion": row.get::<i32, _>("protocol_version"),
            "revision": row.get::<i64, _>("revision"),
            "online": state.remote.host_online(&state.pool, id).await,
        }));
    }
    Ok(Json(json!({"items": items})))
}

pub async fn get_host(
    Authenticated(user): Authenticated,
    Extension(rid): Extension<RequestId>,
    State(state): State<AppState>,
    Path(host_id): Path<String>,
) -> Result<Json<Value>, V3Error> {
    let host = parse_uuid(&rid, &host_id)?;
    let row = sqlx::query("SELECT name,status,protocol_version,revision FROM cloud_remote_hosts WHERE id=$1 AND owner_user_id=$2")
        .bind(host)
        .bind(&user.user.id)
        .fetch_optional(&state.pool)
        .await
        .map_err(db(&rid))?
        .ok_or(V3Error::new(&rid, Kind::NotFound))?;
    Ok(Json(json!({
        "id": host,
        "name": row.get::<String, _>("name"),
        "status": row.get::<String, _>("status"),
        "protocolVersion": row.get::<i32, _>("protocol_version"),
        "revision": row.get::<i64, _>("revision"),
        "online": state.remote.host_online(&state.pool, host).await,
    })))
}

/// Updates a team with an expected revision. Disabling a team closes its links.
pub async fn update_host(
    Authenticated(user): Authenticated,
    Extension(rid): Extension<RequestId>,
    State(state): State<AppState>,
    Path(host_id): Path<String>,
    body: Bytes,
) -> Result<Json<Value>, V3Error> {
    let host = parse_uuid(&rid, &host_id)?;
    let request: UpdateHost = parse_body(&rid, &body, SMALL_BODY)?;
    if let Some(name) = &request.name {
        if !valid_name(name) {
            return Err(V3Error::new(&rid, Kind::InvalidRequest));
        }
    }
    if let Some(status) = &request.status {
        if status != "active" && status != "disabled" {
            return Err(V3Error::new(&rid, Kind::InvalidRequest));
        }
    }
    let mut tx = state.pool.begin().await.map_err(db(&rid))?;
    let current = sqlx::query("SELECT revision,status FROM cloud_remote_hosts WHERE id=$1 AND owner_user_id=$2 FOR UPDATE")
        .bind(host)
        .bind(&user.user.id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db(&rid))?
        .ok_or(V3Error::new(&rid, Kind::NotFound))?;
    if current.get::<i64, _>("revision") != request.expected_revision {
        return Err(V3Error::new(&rid, Kind::Conflict));
    }
    let name = request.name.as_ref().map(|n| n.trim().to_owned());
    let revision: i64 = sqlx::query_scalar(
        "UPDATE cloud_remote_hosts SET name=COALESCE($2,name), status=COALESCE($3,status), revision=revision+1, updated_at=now() WHERE id=$1 RETURNING revision",
    )
    .bind(host)
    .bind(name)
    .bind(request.status.as_deref())
    .fetch_one(&mut *tx)
    .await
    .map_err(db(&rid))?;
    let disabled = request.status.as_deref() == Some("disabled");
    if disabled {
        sqlx::query("UPDATE cloud_remote_sessions SET status='closed', closed_at=now() WHERE host_id=$1 AND status='active'")
            .bind(host)
            .execute(&mut *tx)
            .await
            .map_err(db(&rid))?;
        sqlx::query("UPDATE cloud_remote_tickets SET consumed_at=now() WHERE host_id=$1 AND consumed_at IS NULL")
            .bind(host)
            .execute(&mut *tx)
            .await
            .map_err(db(&rid))?;
    }
    audit(
        &mut tx,
        &rid,
        &user.user.id,
        Some(host),
        "host.updated",
        host,
    )
    .await?;
    tx.commit().await.map_err(db(&rid))?;
    if disabled {
        state
            .remote
            .close_host(&state.pool, host, "hostDisabled")
            .await;
    }
    let row =
        sqlx::query("SELECT name,status,protocol_version FROM cloud_remote_hosts WHERE id=$1")
            .bind(host)
            .fetch_one(&state.pool)
            .await
            .map_err(db(&rid))?;
    Ok(Json(json!({
        "id": host,
        "name": row.get::<String, _>("name"),
        "status": row.get::<String, _>("status"),
        "protocolVersion": row.get::<i32, _>("protocol_version"),
        "revision": revision,
        "online": state.remote.host_online(&state.pool, host).await,
    })))
}

pub(crate) async fn audit(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    rid: &RequestId,
    actor: &str,
    host: Option<Uuid>,
    action: &str,
    subject: Uuid,
) -> Result<(), V3Error> {
    sqlx::query("INSERT INTO cloud_remote_audit(id,actor_user_id,host_id,action,subject_id) VALUES($1,$2,$3,$4,$5)")
        .bind(Uuid::new_v4())
        .bind(actor)
        .bind(host)
        .bind(action)
        .bind(subject.to_string())
        .execute(&mut **tx)
        .await
        .map_err(db(rid))?;
    Ok(())
}

/// Shared helper: fetch a team owned by the caller or fail without revealing others.
pub(crate) async fn owned_host(
    pool: &PgPool,
    rid: &RequestId,
    owner: &str,
    host: Uuid,
) -> Result<(i64, String), V3Error> {
    let row = sqlx::query(
        "SELECT revision,status FROM cloud_remote_hosts WHERE id=$1 AND owner_user_id=$2",
    )
    .bind(host)
    .bind(owner)
    .fetch_optional(pool)
    .await
    .map_err(db(rid))?
    .ok_or(V3Error::new(rid, Kind::NotFound))?;
    Ok((row.get("revision"), row.get("status")))
}
