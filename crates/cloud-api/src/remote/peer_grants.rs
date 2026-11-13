//! Confirms an owner-approved revision using the host device's key over HTTP.
//! Native P2P hosts do not need a legacy command-relay socket just for approval.
use super::{binding_digest, db, parse_body, proof_message, throttle, verify_proof};
use crate::{
    api_error::{Authenticated, Kind, RequestId, V3Error},
    AppState,
};
use axum::{body::Bytes, extract::State, Extension, Json};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Binding {
    host_id: Uuid,
    device_id: Uuid,
    grant_id: Uuid,
    revision: i64,
}

fn parse(rid: &RequestId, value: &Value) -> Result<Binding, V3Error> {
    let binding: Binding = serde_json::from_value(value.clone())
        .map_err(|_| V3Error::new(rid, Kind::InvalidRequest))?;
    if binding.revision <= 0 {
        return Err(V3Error::new(rid, Kind::InvalidRequest));
    }
    Ok(binding)
}

pub async fn check_binding(
    state: &AppState,
    rid: &RequestId,
    owner: &str,
    value: &Value,
) -> Result<(), V3Error> {
    let binding = parse(rid, value)?;
    let found: Option<Uuid> = sqlx::query_scalar("SELECT g.id FROM cloud_remote_grants g JOIN cloud_remote_hosts h ON h.id=g.host_id JOIN cloud_remote_devices d ON d.host_id=h.id WHERE g.id=$1 AND h.id=$2 AND h.owner_user_id=$3 AND h.status='active' AND d.id=$4 AND d.owner_user_id=$3 AND d.kind='host' AND d.status='active' AND g.revision=$5 AND g.status IN ('pendingHost','active')")
        .bind(binding.grant_id).bind(binding.host_id).bind(owner).bind(binding.device_id).bind(binding.revision)
        .fetch_optional(&state.pool).await.map_err(db(rid))?;
    if found.is_none() {
        return Err(V3Error::new(rid, Kind::NotFound));
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Confirm {
    challenge_id: Uuid,
    binding: Value,
    signature: String,
}

pub async fn confirm(
    Authenticated(user): Authenticated,
    Extension(rid): Extension<RequestId>,
    State(state): State<AppState>,
    body: Bytes,
) -> Result<Json<Value>, V3Error> {
    let request: Confirm = parse_body(&rid, &body, 4 * 1024)?;
    let binding = parse(&rid, &request.binding)?;
    let owner = &user.user.id;
    throttle(&state, &rid, owner, "peer-grant-confirm", 120).await?;
    let mut tx = state.pool.begin().await.map_err(db(&rid))?;
    let device = sqlx::query("SELECT d.public_key FROM cloud_remote_devices d JOIN cloud_remote_hosts h ON h.id=d.host_id WHERE h.id=$1 AND h.owner_user_id=$2 AND h.status='active' AND d.id=$3 AND d.owner_user_id=$2 AND d.kind='host' AND d.status='active' FOR UPDATE")
        .bind(binding.host_id).bind(owner).bind(binding.device_id).fetch_optional(&mut *tx).await.map_err(db(&rid))?
        .ok_or(V3Error::new(&rid, Kind::NotFound))?;
    let challenge = sqlx::query("SELECT nonce,binding_sha256 FROM cloud_remote_challenges WHERE id=$1 AND owner_user_id=$2 AND purpose='peer-grant' AND consumed_at IS NULL AND expires_at>now() FOR UPDATE")
        .bind(request.challenge_id).bind(owner).fetch_optional(&mut *tx).await.map_err(db(&rid))?
        .ok_or(V3Error::new(&rid, Kind::Expired))?;
    let digest: String = challenge.get("binding_sha256");
    if digest != binding_digest(&request.binding) {
        return Err(V3Error::new(&rid, Kind::InvalidRequest));
    }
    let message = proof_message("peer-grant", &challenge.get::<Vec<u8>, _>("nonce"), &digest);
    if !verify_proof(
        &device.get::<Vec<u8>, _>("public_key"),
        &message,
        &request.signature,
    ) {
        return Err(V3Error::new(&rid, Kind::Unauthorized));
    }
    let updated = sqlx::query("UPDATE cloud_remote_grants SET status='active',confirmed_revision=revision,updated_at=now() WHERE id=$1 AND host_id=$2 AND revision=$3 AND status IN ('pendingHost','active')")
        .bind(binding.grant_id).bind(binding.host_id).bind(binding.revision).execute(&mut *tx).await.map_err(db(&rid))?;
    if updated.rows_affected() != 1 {
        return Err(V3Error::new(&rid, Kind::Conflict));
    }
    sqlx::query("UPDATE cloud_remote_challenges SET consumed_at=now() WHERE id=$1")
        .bind(request.challenge_id)
        .execute(&mut *tx)
        .await
        .map_err(db(&rid))?;
    tx.commit().await.map_err(db(&rid))?;
    Ok(Json(
        json!({"grantId":binding.grant_id,"revision":binding.revision,"status":"active"}),
    ))
}
