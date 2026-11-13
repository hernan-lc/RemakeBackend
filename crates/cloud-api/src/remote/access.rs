//! Invitations, join requests, grants and sessions.
//!
//! Inviting never grants access and redeeming never approves. A decision creates
//! a grant that stays `pendingHost` until the host confirms the exact revision
//! it persisted over the relay.

use super::hosts::{audit as audit_from, owned_host};
use super::{
    db, new_secret, parse_body, parse_uuid, secret_hash, throttle, INVITATION_DEFAULT_TTL_SECONDS,
    INVITATION_MAX_TTL_SECONDS, MAX_LIST, MAX_POLICY_BYTES, OPERATIONS, PRESET_ROLES,
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
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;
use uuid::Uuid;

const SMALL_BODY: usize = 4 * 1024;
const POLICY_BODY: usize = MAX_POLICY_BYTES + 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateInvitation {
    #[serde(rename = "requestedRole")]
    requested_role: String,
    #[serde(rename = "ttlSeconds")]
    ttl_seconds: Option<i64>,
    #[serde(rename = "recipientId")]
    recipient_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Redeem {
    token: String,
    #[serde(rename = "deviceId")]
    device_id: Uuid,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Decision {
    decision: String,
    role: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateGrant {
    role: Option<String>,
    policy: Option<Value>,
    #[serde(rename = "expectedRevision")]
    expected_revision: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectRevision {
    #[serde(rename = "expectedRevision")]
    expected_revision: i64,
}

fn valid_policy(rid: &RequestId, role: &str, policy: &Value) -> Result<(), V3Error> {
    let Some(object) = policy.as_object() else {
        return Err(V3Error::new(rid, Kind::InvalidRequest));
    };
    if serde_json::to_vec(policy)
        .map(|b| b.len())
        .unwrap_or(usize::MAX)
        > MAX_POLICY_BYTES
    {
        return Err(V3Error::new(rid, Kind::InvalidRequest));
    }
    if object
        .get("nativeGrant")
        .is_some_and(|grant| role != "custom" || !grant.is_object())
        || object
            .get("nativeClientKind")
            .is_some_and(|kind| !matches!(kind.as_str(), Some("desktop" | "agent")))
    {
        return Err(V3Error::new(rid, Kind::InvalidRequest));
    }
    match role {
        "custom" => {
            // A native-only policy is validated semantically by the host before
            // confirmation. Missing legacy operations still denies every WSS
            // command in role_admits; do not require an unrelated permission set.
            if !object.contains_key("allowedOperations") && object.contains_key("nativeGrant") {
                return Ok(());
            }
            let ops = object
                .get("allowedOperations")
                .and_then(Value::as_array)
                .ok_or(V3Error::new(rid, Kind::InvalidRequest))?;
            let known = ops.len() <= OPERATIONS.len()
                && ops.iter().all(|op| {
                    op.as_str()
                        .is_some_and(|name| OPERATIONS.iter().any(|(known, _)| *known == name))
                });
            if !known {
                return Err(V3Error::new(rid, Kind::InvalidRequest));
            }
            Ok(())
        }
        preset if PRESET_ROLES.contains(&preset) => {
            if object.contains_key("allowedOperations") {
                return Err(V3Error::new(rid, Kind::InvalidRequest));
            }
            Ok(())
        }
        _ => Err(V3Error::new(rid, Kind::InvalidRequest)),
    }
}

pub async fn create_invitation(
    Authenticated(user): Authenticated,
    Extension(rid): Extension<RequestId>,
    State(state): State<AppState>,
    Path(host_id): Path<String>,
    body: Bytes,
) -> Result<(StatusCode, Json<Value>), V3Error> {
    let host = parse_uuid(&rid, &host_id)?;
    let request: CreateInvitation = parse_body(&rid, &body, SMALL_BODY)?;
    throttle(&state, &rid, &user.user.id, "remote-invitation", 20).await?;
    if !PRESET_ROLES.contains(&request.requested_role.as_str()) {
        return Err(V3Error::new(&rid, Kind::InvalidRequest));
    }
    let ttl = request
        .ttl_seconds
        .unwrap_or(INVITATION_DEFAULT_TTL_SECONDS);
    if !(60..=INVITATION_MAX_TTL_SECONDS).contains(&ttl) {
        return Err(V3Error::new(&rid, Kind::InvalidRequest));
    }
    let (_, status) = owned_host(&state.pool, &rid, &user.user.id, host).await?;
    if status != "active" {
        return Err(V3Error::new(&rid, Kind::Conflict));
    }
    let token = new_secret();
    if let Some(recipient) = &request.recipient_id {
        if recipient == &user.user.id || recipient.len() > 256 {
            return Err(V3Error::new(&rid, Kind::InvalidRequest));
        }
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM \"user\" WHERE id=$1)")
            .bind(recipient)
            .fetch_one(&state.pool)
            .await
            .map_err(db(&rid))?;
        if !exists {
            return Err(V3Error::new(&rid, Kind::NotFound));
        }
    }
    let id = Uuid::new_v4();
    let expires = chrono::Utc::now() + chrono::Duration::seconds(ttl);
    sqlx::query("INSERT INTO cloud_remote_invitations(id,host_id,created_by,token_sha256,requested_role,expires_at,recipient_user_id) VALUES($1,$2,$3,$4,$5,$6,$7)")
        .bind(id)
        .bind(host)
        .bind(&user.user.id)
        .bind(secret_hash(&token))
        .bind(&request.requested_role)
        .bind(expires)
        .bind(&request.recipient_id)
        .execute(&state.pool)
        .await
        .map_err(db(&rid))?;
    // The token is returned exactly once; only its hash is stored.
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": id,
            "requestedRole": request.requested_role,
            "status": "open",
            "expiresAt": expires.to_rfc3339(),
            "token": token,
        })),
    ))
}

pub async fn list_invitations(
    Authenticated(user): Authenticated,
    Extension(rid): Extension<RequestId>,
    State(state): State<AppState>,
    Path(host_id): Path<String>,
) -> Result<Json<Value>, V3Error> {
    let host = parse_uuid(&rid, &host_id)?;
    owned_host(&state.pool, &rid, &user.user.id, host).await?;
    let rows = sqlx::query("SELECT id,requested_role,status,expires_at,created_at,redeemed_at FROM cloud_remote_invitations WHERE host_id=$1 ORDER BY created_at DESC, id LIMIT $2")
        .bind(host)
        .bind(MAX_LIST)
        .fetch_all(&state.pool)
        .await
        .map_err(db(&rid))?;
    let items: Vec<Value> = rows
        .iter()
        .map(|row| {
            json!({
                "id": row.get::<Uuid, _>("id"),
                "requestedRole": row.get::<String, _>("requested_role"),
                "status": row.get::<String, _>("status"),
                "expiresAt": row.get::<chrono::DateTime<chrono::Utc>, _>("expires_at").to_rfc3339(),
                "redeemedAt": row.get::<Option<chrono::DateTime<chrono::Utc>>, _>("redeemed_at").map(|t| t.to_rfc3339()),
            })
        })
        .collect();
    Ok(Json(json!({"items": items})))
}

pub async fn revoke_invitation(
    Authenticated(user): Authenticated,
    Extension(rid): Extension<RequestId>,
    State(state): State<AppState>,
    Path((host_id, invitation_id)): Path<(String, String)>,
) -> Result<Json<Value>, V3Error> {
    let host = parse_uuid(&rid, &host_id)?;
    let invitation = parse_uuid(&rid, &invitation_id)?;
    owned_host(&state.pool, &rid, &user.user.id, host).await?;
    let updated = sqlx::query("UPDATE cloud_remote_invitations SET status='revoked' WHERE id=$1 AND host_id=$2 AND status='open'")
        .bind(invitation)
        .bind(host)
        .execute(&state.pool)
        .await
        .map_err(db(&rid))?;
    if updated.rows_affected() == 0 {
        return Err(V3Error::new(&rid, Kind::Conflict));
    }
    Ok(Json(json!({"id": invitation, "status": "revoked"})))
}

/// Redeems an invitation into a pending join request. Single use, expiring, and
/// bound to a controller device owned by the redeeming account.
pub async fn redeem_invitation(
    Authenticated(user): Authenticated,
    Extension(rid): Extension<RequestId>,
    State(state): State<AppState>,
    body: Bytes,
) -> Result<(StatusCode, Json<Value>), V3Error> {
    let request: Redeem = parse_body(&rid, &body, SMALL_BODY)?;
    let owner = &user.user.id;
    throttle(&state, &rid, owner, "remote-redeem", 30).await?;
    let mut tx = state.pool.begin().await.map_err(db(&rid))?;
    let device = sqlx::query(
        "SELECT kind,status FROM cloud_remote_devices WHERE id=$1 AND owner_user_id=$2 FOR UPDATE",
    )
    .bind(request.device_id)
    .bind(owner)
    .fetch_optional(&mut *tx)
    .await
    .map_err(db(&rid))?
    .ok_or(V3Error::new(&rid, Kind::NotFound))?;
    if device.get::<String, _>("kind") != "controller"
        || device.get::<String, _>("status") != "active"
    {
        return Err(V3Error::new(&rid, Kind::Forbidden));
    }
    let invitation = sqlx::query("SELECT id,host_id,requested_role,status,expires_at,recipient_user_id FROM cloud_remote_invitations WHERE token_sha256=$1 FOR UPDATE")
        .bind(secret_hash(&request.token))
        .fetch_optional(&mut *tx)
        .await
        .map_err(db(&rid))?
        .ok_or(V3Error::new(&rid, Kind::NotFound))?;
    let invitation_id: Uuid = invitation.get("id");
    if invitation
        .get::<Option<String>, _>("recipient_user_id")
        .is_some_and(|id| id != *owner)
    {
        return Err(V3Error::new(&rid, Kind::NotFound));
    }
    let host: Uuid = invitation.get("host_id");
    match invitation.get::<String, _>("status").as_str() {
        "open" => {}
        _ => return Err(V3Error::new(&rid, Kind::Conflict)),
    }
    let expires: chrono::DateTime<chrono::Utc> = invitation.get("expires_at");
    if expires <= chrono::Utc::now() {
        return Err(V3Error::new(&rid, Kind::Expired));
    }
    let (_, host_status) = owned_host_any(&mut tx, &rid, host).await?;
    if host_status != "active" {
        return Err(V3Error::new(&rid, Kind::Conflict));
    }
    let live: i64 = sqlx::query_scalar("SELECT count(*) FROM cloud_remote_grants WHERE host_id=$1 AND device_id=$2 AND status<>'revoked'")
        .bind(host)
        .bind(request.device_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(db(&rid))?;
    if live > 0 {
        return Err(V3Error::new(&rid, Kind::Conflict));
    }
    let requested_role: String = invitation.get("requested_role");
    let request_id = Uuid::new_v4();
    sqlx::query("INSERT INTO cloud_remote_requests(id,host_id,invitation_id,requester_user_id,device_id,requested_role) VALUES($1,$2,$3,$4,$5,$6)")
        .bind(request_id)
        .bind(host)
        .bind(invitation_id)
        .bind(owner)
        .bind(request.device_id)
        .bind(&requested_role)
        .execute(&mut *tx)
        .await
        .map_err(db(&rid))?;
    sqlx::query(
        "UPDATE cloud_remote_invitations SET status='redeemed', redeemed_at=now() WHERE id=$1",
    )
    .bind(invitation_id)
    .execute(&mut *tx)
    .await
    .map_err(db(&rid))?;
    audit_from(
        &mut tx,
        &rid,
        owner,
        Some(host),
        "request.created",
        request_id,
    )
    .await?;
    tx.commit().await.map_err(db(&rid))?;
    Ok((
        StatusCode::CREATED,
        Json(
            json!({"requestId": request_id, "hostId": host, "status": "pending", "requestedRole": requested_role}),
        ),
    ))
}

async fn owned_host_any(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    rid: &RequestId,
    host: Uuid,
) -> Result<(i64, String), V3Error> {
    let row = sqlx::query("SELECT revision,status FROM cloud_remote_hosts WHERE id=$1 FOR UPDATE")
        .bind(host)
        .fetch_optional(&mut **tx)
        .await
        .map_err(db(rid))?
        .ok_or(V3Error::new(rid, Kind::NotFound))?;
    Ok((row.get("revision"), row.get("status")))
}

pub async fn list_requests(
    Authenticated(user): Authenticated,
    Extension(rid): Extension<RequestId>,
    State(state): State<AppState>,
    Path(host_id): Path<String>,
) -> Result<Json<Value>, V3Error> {
    let host = parse_uuid(&rid, &host_id)?;
    owned_host(&state.pool, &rid, &user.user.id, host).await?;
    let rows = sqlx::query("SELECT r.id,r.requested_role,r.status,r.created_at,d.label FROM cloud_remote_requests r JOIN cloud_remote_devices d ON d.id=r.device_id WHERE r.host_id=$1 ORDER BY (r.status='pending') DESC, r.created_at DESC, r.id LIMIT $2")
        .bind(host)
        .bind(MAX_LIST)
        .fetch_all(&state.pool)
        .await
        .map_err(db(&rid))?;
    let items: Vec<Value> = rows
        .iter()
        .map(|row| {
            json!({
                "id": row.get::<Uuid, _>("id"),
                "requestedRole": row.get::<String, _>("requested_role"),
                "status": row.get::<String, _>("status"),
                "deviceLabel": row.get::<String, _>("label"),
                "createdAt": row.get::<chrono::DateTime<chrono::Utc>, _>("created_at").to_rfc3339(),
            })
        })
        .collect();
    Ok(Json(json!({"items": items})))
}

/// Approves or denies a join request. Approval creates a grant awaiting the
/// host's confirmation; nothing is operational until the host confirms it.
pub async fn decide_request(
    Authenticated(user): Authenticated,
    Extension(rid): Extension<RequestId>,
    State(state): State<AppState>,
    Path((host_id, request_id)): Path<(String, String)>,
    body: Bytes,
) -> Result<Json<Value>, V3Error> {
    let host = parse_uuid(&rid, &host_id)?;
    let request = parse_uuid(&rid, &request_id)?;
    let decision: Decision = parse_body(&rid, &body, SMALL_BODY)?;
    let owner = &user.user.id;
    let mut tx = state.pool.begin().await.map_err(db(&rid))?;
    let found = sqlx::query("SELECT r.status,r.requested_role,r.device_id,r.requester_user_id,h.owner_user_id,h.status AS host_status FROM cloud_remote_requests r JOIN cloud_remote_hosts h ON h.id=r.host_id WHERE r.id=$1 AND r.host_id=$2 FOR UPDATE OF r")
        .bind(request)
        .bind(host)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db(&rid))?
        .ok_or(V3Error::new(&rid, Kind::NotFound))?;
    if found.get::<String, _>("owner_user_id") != *owner {
        return Err(V3Error::new(&rid, Kind::NotFound));
    }
    if found.get::<String, _>("status") != "pending" {
        return Err(V3Error::new(&rid, Kind::Conflict));
    }
    if found.get::<String, _>("host_status") != "active" {
        return Err(V3Error::new(&rid, Kind::Conflict));
    }
    match decision.decision.as_str() {
        "deny" => {
            sqlx::query("UPDATE cloud_remote_requests SET status='denied', decided_by=$2, decided_at=now() WHERE id=$1")
                .bind(request)
                .bind(owner)
                .execute(&mut *tx)
                .await
                .map_err(db(&rid))?;
            audit_from(&mut tx, &rid, owner, Some(host), "request.denied", request).await?;
            tx.commit().await.map_err(db(&rid))?;
            Ok(Json(json!({"requestId": request, "status": "denied"})))
        }
        "approve" => {
            let role = decision
                .role
                .clone()
                .unwrap_or_else(|| found.get::<String, _>("requested_role"));
            if !PRESET_ROLES.contains(&role.as_str()) {
                return Err(V3Error::new(&rid, Kind::InvalidRequest));
            }
            let device: Uuid = found.get("device_id");
            let requester: String = found.get("requester_user_id");
            let grant = Uuid::new_v4();
            let inserted = sqlx::query("INSERT INTO cloud_remote_grants(id,host_id,device_id,requester_user_id,request_id,role,policy,status,revision) VALUES($1,$2,$3,$4,$5,$6,'{}','pendingHost',1) ON CONFLICT DO NOTHING")
                .bind(grant)
                .bind(host)
                .bind(device)
                .bind(&requester)
                .bind(request)
                .bind(&role)
                .execute(&mut *tx)
                .await
                .map_err(db(&rid))?;
            if inserted.rows_affected() == 0 {
                return Err(V3Error::new(&rid, Kind::Conflict));
            }
            sqlx::query("UPDATE cloud_remote_requests SET status='approved', decided_by=$2, decided_at=now() WHERE id=$1")
                .bind(request)
                .bind(owner)
                .execute(&mut *tx)
                .await
                .map_err(db(&rid))?;
            audit_from(&mut tx, &rid, owner, Some(host), "request.approved", grant).await?;
            tx.commit().await.map_err(db(&rid))?;
            state.remote.notify_grants_changed(&state.pool, host).await;
            Ok(Json(json!({
                "requestId": request,
                "status": "approved",
                "grant": {"id": grant, "role": role, "status": "pendingHost", "revision": 1},
            })))
        }
        _ => Err(V3Error::new(&rid, Kind::InvalidRequest)),
    }
}

pub async fn list_grants(
    Authenticated(user): Authenticated,
    Extension(rid): Extension<RequestId>,
    State(state): State<AppState>,
    Path(host_id): Path<String>,
) -> Result<Json<Value>, V3Error> {
    let host = parse_uuid(&rid, &host_id)?;
    owned_host(&state.pool, &rid, &user.user.id, host).await?;
    let rows = sqlx::query("SELECT g.id,g.role,g.policy,g.status,g.revision,g.confirmed_revision,g.device_id,d.label FROM cloud_remote_grants g JOIN cloud_remote_devices d ON d.id=g.device_id WHERE g.host_id=$1 ORDER BY g.id LIMIT $2")
        .bind(host)
        .bind(MAX_LIST * 4)
        .fetch_all(&state.pool)
        .await
        .map_err(db(&rid))?;
    let items: Vec<Value> = rows.iter().map(grant_view).collect();
    Ok(Json(json!({"items": items})))
}

fn grant_view(row: &sqlx::postgres::PgRow) -> Value {
    json!({
        "id": row.get::<Uuid, _>("id"),
        "deviceId": row.get::<Uuid, _>("device_id"),
        "deviceLabel": row.get::<String, _>("label"),
        "role": row.get::<String, _>("role"),
        "policy": row.get::<Value, _>("policy"),
        "status": row.get::<String, _>("status"),
        "revision": row.get::<i64, _>("revision"),
        "confirmedRevision": row.get::<Option<i64>, _>("confirmed_revision"),
    })
}

/// Changes a grant with an expected revision. The change needs a fresh host
/// confirmation, and sessions opened under the previous revision end now.
pub async fn update_grant(
    Authenticated(user): Authenticated,
    Extension(rid): Extension<RequestId>,
    State(state): State<AppState>,
    Path((host_id, grant_id)): Path<(String, String)>,
    body: Bytes,
) -> Result<Json<Value>, V3Error> {
    let host = parse_uuid(&rid, &host_id)?;
    let grant = parse_uuid(&rid, &grant_id)?;
    let request: UpdateGrant = parse_body(&rid, &body, POLICY_BODY)?;
    let owner = &user.user.id;
    owned_host(&state.pool, &rid, owner, host).await?;
    let mut tx = state.pool.begin().await.map_err(db(&rid))?;
    let current = sqlx::query("SELECT role,policy,status,revision FROM cloud_remote_grants WHERE id=$1 AND host_id=$2 FOR UPDATE")
        .bind(grant)
        .bind(host)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db(&rid))?
        .ok_or(V3Error::new(&rid, Kind::NotFound))?;
    if current.get::<String, _>("status") == "revoked" {
        return Err(V3Error::new(&rid, Kind::Conflict));
    }
    if current.get::<i64, _>("revision") != request.expected_revision {
        return Err(V3Error::new(&rid, Kind::Conflict));
    }
    let role = request.role.clone().unwrap_or_else(|| current.get("role"));
    let policy = match &request.policy {
        Some(policy) => policy.clone(),
        None => {
            // Keeping the stored policy is only valid when the role does not change its meaning.
            current.get::<Value, _>("policy")
        }
    };
    valid_policy(&rid, &role, &policy)?;
    let revision: i64 = sqlx::query_scalar("UPDATE cloud_remote_grants SET role=$2, policy=$3, status='pendingHost', revision=revision+1, confirmed_revision=NULL, updated_at=now() WHERE id=$1 RETURNING revision")
        .bind(grant)
        .bind(&role)
        .bind(&policy)
        .fetch_one(&mut *tx)
        .await
        .map_err(db(&rid))?;
    let closed = sqlx::query("UPDATE cloud_remote_sessions SET status='closed', closed_at=now() WHERE grant_id=$1 AND status='active' RETURNING id")
        .bind(grant)
        .fetch_all(&mut *tx)
        .await
        .map_err(db(&rid))?;
    sqlx::query("UPDATE cloud_remote_tickets SET consumed_at=now() WHERE grant_id=$1 AND consumed_at IS NULL")
        .bind(grant)
        .execute(&mut *tx)
        .await
        .map_err(db(&rid))?;
    audit_from(&mut tx, &rid, owner, Some(host), "grant.updated", grant).await?;
    tx.commit().await.map_err(db(&rid))?;
    for row in closed {
        state
            .remote
            .close_session(&state.pool, row.get("id"), "grantChanged")
            .await;
    }
    state.remote.notify_grants_changed(&state.pool, host).await;
    Ok(Json(
        json!({"id": grant, "role": role, "status": "pendingHost", "revision": revision}),
    ))
}

pub async fn revoke_grant(
    Authenticated(user): Authenticated,
    Extension(rid): Extension<RequestId>,
    State(state): State<AppState>,
    Path((host_id, grant_id)): Path<(String, String)>,
    body: Bytes,
) -> Result<Json<Value>, V3Error> {
    let host = parse_uuid(&rid, &host_id)?;
    let grant = parse_uuid(&rid, &grant_id)?;
    let request: ExpectRevision = parse_body(&rid, &body, SMALL_BODY)?;
    let owner = &user.user.id;
    owned_host(&state.pool, &rid, owner, host).await?;
    let mut tx = state.pool.begin().await.map_err(db(&rid))?;
    let current = sqlx::query(
        "SELECT status,revision FROM cloud_remote_grants WHERE id=$1 AND host_id=$2 FOR UPDATE",
    )
    .bind(grant)
    .bind(host)
    .fetch_optional(&mut *tx)
    .await
    .map_err(db(&rid))?
    .ok_or(V3Error::new(&rid, Kind::NotFound))?;
    if current.get::<String, _>("status") == "revoked" {
        tx.commit().await.map_err(db(&rid))?;
        return Ok(Json(json!({"id": grant, "status": "revoked"})));
    }
    if current.get::<i64, _>("revision") != request.expected_revision {
        return Err(V3Error::new(&rid, Kind::Conflict));
    }
    sqlx::query("UPDATE cloud_remote_grants SET status='revoked', revision=revision+1, confirmed_revision=NULL, updated_at=now() WHERE id=$1")
        .bind(grant)
        .execute(&mut *tx)
        .await
        .map_err(db(&rid))?;
    let closed = sqlx::query("UPDATE cloud_remote_sessions SET status='revoked', closed_at=now() WHERE grant_id=$1 AND status='active' RETURNING id")
        .bind(grant)
        .fetch_all(&mut *tx)
        .await
        .map_err(db(&rid))?;
    sqlx::query("UPDATE cloud_remote_tickets SET consumed_at=now() WHERE grant_id=$1 AND consumed_at IS NULL")
        .bind(grant)
        .execute(&mut *tx)
        .await
        .map_err(db(&rid))?;
    audit_from(&mut tx, &rid, owner, Some(host), "grant.revoked", grant).await?;
    tx.commit().await.map_err(db(&rid))?;
    for row in closed {
        state
            .remote
            .close_session(&state.pool, row.get("id"), "grantRevoked")
            .await;
    }
    state.remote.notify_grants_changed(&state.pool, host).await;
    Ok(Json(json!({"id": grant, "status": "revoked"})))
}

pub async fn list_sessions(
    Authenticated(user): Authenticated,
    Extension(rid): Extension<RequestId>,
    State(state): State<AppState>,
    Path(host_id): Path<String>,
) -> Result<Json<Value>, V3Error> {
    let host = parse_uuid(&rid, &host_id)?;
    owned_host(&state.pool, &rid, &user.user.id, host).await?;
    let rows = sqlx::query("SELECT id,grant_id,device_id,grant_revision,opened_at FROM cloud_remote_sessions WHERE host_id=$1 AND status='active' ORDER BY opened_at DESC, id LIMIT $2")
        .bind(host)
        .bind(MAX_LIST)
        .fetch_all(&state.pool)
        .await
        .map_err(db(&rid))?;
    let items: Vec<Value> = rows
        .iter()
        .map(|row| {
            json!({
                "id": row.get::<Uuid, _>("id"),
                "grantId": row.get::<Uuid, _>("grant_id"),
                "deviceId": row.get::<Uuid, _>("device_id"),
                "grantRevision": row.get::<i64, _>("grant_revision"),
                "openedAt": row.get::<chrono::DateTime<chrono::Utc>, _>("opened_at").to_rfc3339(),
            })
        })
        .collect();
    Ok(Json(json!({"items": items})))
}

pub async fn revoke_session(
    Authenticated(user): Authenticated,
    Extension(rid): Extension<RequestId>,
    State(state): State<AppState>,
    Path((host_id, session_id)): Path<(String, String)>,
) -> Result<Json<Value>, V3Error> {
    let host = parse_uuid(&rid, &host_id)?;
    let session = parse_uuid(&rid, &session_id)?;
    owned_host(&state.pool, &rid, &user.user.id, host).await?;
    let updated = sqlx::query("UPDATE cloud_remote_sessions SET status='revoked', closed_at=now() WHERE id=$1 AND host_id=$2 AND status='active'")
        .bind(session)
        .bind(host)
        .execute(&state.pool)
        .await
        .map_err(db(&rid))?;
    if updated.rows_affected() == 0 {
        return Err(V3Error::new(&rid, Kind::NotFound));
    }
    state
        .remote
        .close_session(&state.pool, session, "sessionRevoked")
        .await;
    Ok(Json(json!({"id": session, "status": "revoked"})))
}

#[cfg(test)]
mod policy_tests {
    use super::*;

    #[test]
    fn native_only_custom_policy_never_grants_legacy_relay_operations() {
        let rid = RequestId::fresh();
        let policy =
            json!({"nativeGrant":{"capabilities":["live.read"]},"nativeClientKind":"desktop"});
        assert!(valid_policy(&rid, "custom", &policy).is_ok());
        for (operation, _) in OPERATIONS {
            assert!(!super::super::role_admits("custom", &policy, operation));
        }
        assert!(valid_policy(&rid, "custom", &json!({})).is_err());
        assert!(valid_policy(&rid, "moderator", &policy).is_err());
        assert!(valid_policy(
            &rid,
            "custom",
            &json!({"nativeGrant":{},"nativeClientKind":"browser"})
        )
        .is_err());
        assert!(valid_policy(
            &rid,
            "custom",
            &json!({"nativeGrant":{},"allowedOperations":["unknown"]})
        )
        .is_err());
    }
}
