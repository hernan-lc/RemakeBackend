//! Proven endpoint-key bindings and scoped discovery. Never proxies RPC effects.
use super::{binding_digest, db, parse_body, proof_message, throttle, verify_proof};
use crate::{
    api_error::{Authenticated, Kind, RequestId, V3Error},
    AppState,
};
use axum::{
    body::Bytes,
    extract::{Path, State},
    Extension, Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;
use std::net::SocketAddr;
use uuid::Uuid;

#[derive(Clone, Default)]
pub struct PeerConfig {
    pub relay_urls: Vec<String>,
    pub stun_urls: Vec<String>,
    pub relay_only: bool,
}

impl PeerConfig {
    pub fn with_relay_only(mut self, raw: Option<String>) -> Result<Self, String> {
        self.relay_only = match raw.as_deref().unwrap_or("false") {
            "true" => true,
            "false" => false,
            _ => return Err("RTC_RELAY_ONLY must be true or false".into()),
        };
        Ok(self)
    }
    pub fn with_stun_urls(mut self, raw: Option<String>) -> Result<Self, String> {
        self.stun_urls = super::ice::parse(raw)?;
        Ok(self)
    }
    pub fn parse(raw: Option<String>) -> Result<Self, String> {
        let mut relay_urls = Vec::new();
        for value in raw
            .as_deref()
            .unwrap_or("")
            .split(',')
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            let url =
                url::Url::parse(value).map_err(|_| "PEER_RELAY_URLS contains an invalid URL")?;
            if value.len() > 2048
                || url.scheme() != "https"
                || url.host().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
                || url.path() != "/"
            {
                return Err(
                    "PEER_RELAY_URLS must contain HTTPS origins without credentials".into(),
                );
            }
            let origin = url.origin().ascii_serialization();
            if !relay_urls.contains(&origin) {
                relay_urls.push(origin);
            }
        }
        if relay_urls.len() > 4 {
            return Err("PEER_RELAY_URLS supports at most four relays".into());
        }
        Ok(Self {
            relay_urls,
            stun_urls: vec![],
            relay_only: false,
        })
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Binding {
    device_id: Uuid,
    endpoint_id: String,
    addresses: Vec<String>,
    #[serde(default)]
    relay_urls: Vec<String>,
    expected_revision: i64,
}

fn binding(rid: &RequestId, value: &Value) -> Result<(Binding, [u8; 32]), V3Error> {
    let parsed: Binding = serde_json::from_value(value.clone())
        .map_err(|_| V3Error::new(rid, Kind::InvalidRequest))?;
    if parsed.expected_revision < 0
        || parsed.expected_revision == i64::MAX
        || parsed.addresses.len() > 8
        || parsed.endpoint_id.len() != 64
        || !parsed
            .endpoint_id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(V3Error::new(rid, Kind::InvalidRequest));
    }
    for address in &parsed.addresses {
        let socket: SocketAddr = address
            .parse()
            .map_err(|_| V3Error::new(rid, Kind::InvalidRequest))?;
        if socket.port() == 0 || socket.ip().is_unspecified() || socket.ip().is_multicast() {
            return Err(V3Error::new(rid, Kind::InvalidRequest));
        }
    }
    let mut key = [0; 32];
    for (index, byte) in key.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&parsed.endpoint_id[index * 2..index * 2 + 2], 16)
            .map_err(|_| V3Error::new(rid, Kind::InvalidRequest))?;
    }
    if ed25519_dalek::VerifyingKey::from_bytes(&key).is_err() {
        return Err(V3Error::new(rid, Kind::InvalidRequest));
    }
    Ok((parsed, key))
}

pub async fn check_binding(
    state: &AppState,
    rid: &RequestId,
    owner: &str,
    value: &Value,
) -> Result<(), V3Error> {
    let (request, _) = binding(rid, value)?;
    check_relays(rid, &request, &state.peer)?;
    let found: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM cloud_remote_devices d LEFT JOIN cloud_remote_hosts h ON h.id=d.host_id WHERE d.id=$1 AND d.owner_user_id=$2 AND d.status='active' AND (d.kind='controller' OR h.status='active'))")
        .bind(request.device_id).bind(owner).fetch_one(&state.pool).await.map_err(db(rid))?;
    if !found {
        return Err(V3Error::new(rid, Kind::NotFound));
    }
    Ok(())
}

fn check_relays(rid: &RequestId, binding: &Binding, config: &PeerConfig) -> Result<(), V3Error> {
    if binding.relay_urls.len() > 4 {
        return Err(V3Error::new(rid, Kind::InvalidRequest));
    }
    for raw in &binding.relay_urls {
        let url = url::Url::parse(raw).map_err(|_| V3Error::new(rid, Kind::InvalidRequest))?;
        if raw.len() > 2048
            || url.scheme() != "https"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
            || !config
                .relay_urls
                .contains(&url.origin().ascii_serialization())
        {
            return Err(V3Error::new(rid, Kind::InvalidRequest));
        }
    }
    Ok(())
}

pub async fn configuration(
    Authenticated(_user): Authenticated,
    State(state): State<AppState>,
) -> Json<Value> {
    Json(
        json!({"protocolVersion":1,"alpn":"tiktools/collaboration/1","relayUrls":state.peer.relay_urls,"maxOfflineSeconds":28800}),
    )
}

/// Owner recovery for an expired/lost-ack registration. This does not expose
/// another account's address and never resets the endpoint key or revision.
pub async fn own_endpoint(
    Authenticated(user): Authenticated,
    Extension(rid): Extension<RequestId>,
    State(state): State<AppState>,
    Path(device): Path<Uuid>,
) -> Result<Json<Value>, V3Error> {
    let row = sqlx::query("SELECT p.device_id,p.endpoint_id,p.addresses,p.relay_urls,p.revision,p.expires_at,d.kind FROM cloud_remote_peer_endpoints p JOIN cloud_remote_devices d ON d.id=p.device_id WHERE d.id=$1 AND d.owner_user_id=$2 AND d.status='active'")
        .bind(device).bind(&user.user.id).fetch_optional(&state.pool).await.map_err(db(&rid))?
        .ok_or(V3Error::new(&rid, Kind::NotFound))?;
    Ok(Json(
        json!({"deviceId":row.get::<Uuid,_>("device_id"),"endpointId":row.get::<String,_>("endpoint_id"),"addresses":row.get::<Value,_>("addresses"),"relayUrls":row.get::<Value,_>("relay_urls"),"revision":row.get::<i64,_>("revision"),"kind":row.get::<String,_>("kind"),"expiresAt":row.get::<chrono::DateTime<chrono::Utc>,_>("expires_at").to_rfc3339()}),
    ))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Publish {
    challenge_id: Uuid,
    binding: Value,
    signature: String,
    endpoint_signature: String,
}

pub async fn publish(
    Authenticated(user): Authenticated,
    Extension(rid): Extension<RequestId>,
    State(state): State<AppState>,
    body: Bytes,
) -> Result<Json<Value>, V3Error> {
    let request: Publish = parse_body(&rid, &body, 16 * 1024)?;
    let owner = &user.user.id;
    throttle(&state, &rid, owner, "peer-publish", 60).await?;
    let (value, endpoint_key) = binding(&rid, &request.binding)?;
    check_relays(&rid, &value, &state.peer)?;
    let mut tx = state.pool.begin().await.map_err(db(&rid))?;
    // Device lock serializes updates and revocation; no stale publication can
    // revive an endpoint after the registered device has been revoked.
    let device = sqlx::query("SELECT public_key,status FROM cloud_remote_devices WHERE id=$1 AND owner_user_id=$2 FOR UPDATE")
        .bind(value.device_id).bind(owner).fetch_optional(&mut *tx).await.map_err(db(&rid))?
        .ok_or(V3Error::new(&rid, Kind::NotFound))?;
    if device.get::<String, _>("status") != "active" {
        return Err(V3Error::new(&rid, Kind::Forbidden));
    }
    let challenge = sqlx::query("SELECT nonce,binding_sha256 FROM cloud_remote_challenges WHERE id=$1 AND owner_user_id=$2 AND purpose='peer' AND consumed_at IS NULL AND expires_at>now() FOR UPDATE")
        .bind(request.challenge_id).bind(owner).fetch_optional(&mut *tx).await.map_err(db(&rid))?
        .ok_or(V3Error::new(&rid, Kind::Expired))?;
    let digest: String = challenge.get("binding_sha256");
    if digest != binding_digest(&request.binding) {
        return Err(V3Error::new(&rid, Kind::InvalidRequest));
    }
    let message = proof_message("peer", &challenge.get::<Vec<u8>, _>("nonce"), &digest);
    if !verify_proof(
        &device.get::<Vec<u8>, _>("public_key"),
        &message,
        &request.signature,
    ) || !verify_proof(&endpoint_key, &message, &request.endpoint_signature)
    {
        return Err(V3Error::new(&rid, Kind::Unauthorized));
    }
    let revision: Option<i64> = sqlx::query_scalar("INSERT INTO cloud_remote_peer_endpoints(device_id,endpoint_id,addresses,relay_urls,revision,expires_at,relay_authorized_until) SELECT $1,$2,$3,$5,1,now()+interval '5 minutes',now()+interval '8 hours' WHERE $4=0 OR EXISTS(SELECT 1 FROM cloud_remote_peer_endpoints WHERE device_id=$1) ON CONFLICT(device_id) DO UPDATE SET addresses=EXCLUDED.addresses,relay_urls=EXCLUDED.relay_urls,revision=cloud_remote_peer_endpoints.revision+1,expires_at=EXCLUDED.expires_at,relay_authorized_until=EXCLUDED.relay_authorized_until,updated_at=now() WHERE cloud_remote_peer_endpoints.revision=$4 AND cloud_remote_peer_endpoints.endpoint_id=EXCLUDED.endpoint_id RETURNING revision")
        .bind(value.device_id).bind(&value.endpoint_id).bind(json!(value.addresses)).bind(value.expected_revision)
        .bind(json!(value.relay_urls))
        .fetch_optional(&mut *tx).await.map_err(|error| {
            if error.as_database_error().is_some_and(|error| error.is_unique_violation()) {
                V3Error::new(&rid, Kind::Conflict)
            } else { db(&rid)(error) }
        })?;
    let revision = revision.ok_or(V3Error::new(&rid, Kind::Conflict))?;
    sqlx::query("UPDATE cloud_remote_challenges SET consumed_at=now() WHERE id=$1")
        .bind(request.challenge_id)
        .execute(&mut *tx)
        .await
        .map_err(db(&rid))?;
    tx.commit().await.map_err(db(&rid))?;
    Ok(Json(
        json!({"deviceId":value.device_id,"endpointId":value.endpoint_id,"revision":revision,"ttlSeconds":300}),
    ))
}

pub async fn discover(
    Authenticated(user): Authenticated,
    Extension(rid): Extension<RequestId>,
    State(state): State<AppState>,
    Path(host): Path<Uuid>,
) -> Result<Json<Value>, V3Error> {
    let owner = &user.user.id;
    throttle(&state, &rid, owner, "peer-discover", 120).await?;
    // A consistent read transaction binds discovery to the same host/grant
    // snapshot. The peer protocol still reauthorizes actual operations on host.
    let mut tx = state.pool.begin().await.map_err(db(&rid))?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await
        .map_err(db(&rid))?;
    let host_owner: Option<String> = sqlx::query_scalar(
        "SELECT owner_user_id FROM cloud_remote_hosts WHERE id=$1 AND status='active'",
    )
    .bind(host)
    .fetch_optional(&mut *tx)
    .await
    .map_err(db(&rid))?;
    let is_owner = host_owner.as_deref() == Some(owner);
    let admitted: Option<Uuid> = if is_owner {
        Some(host)
    } else {
        sqlx::query_scalar("SELECT g.id FROM cloud_remote_grants g JOIN cloud_remote_devices d ON d.id=g.device_id WHERE g.host_id=$1 AND g.requester_user_id=$2 AND g.status='active' AND g.confirmed_revision=g.revision AND d.status='active' LIMIT 1")
            .bind(host).bind(owner).fetch_optional(&mut *tx).await.map_err(db(&rid))?
    };
    if host_owner.is_none() || admitted.is_none() {
        return Ok(Json(
            json!({"hostId":host,"items":[],"relayUrls":state.peer.relay_urls}),
        ));
    }
    let mut rows = sqlx::query("SELECT p.device_id,p.endpoint_id,p.addresses,p.relay_urls,p.revision,p.expires_at,d.kind FROM cloud_remote_peer_endpoints p JOIN cloud_remote_devices d ON d.id=p.device_id WHERE d.host_id=$1 AND d.kind='host' AND d.status='active' AND p.expires_at>now() AND (NOT EXISTS(SELECT 1 FROM cloud_relay_leases l WHERE l.resource_kind='host' AND l.resource_id=$1 AND l.expires_at>now() AND l.device_id IS NOT NULL) OR d.id=(SELECT l.device_id FROM cloud_relay_leases l WHERE l.resource_kind='host' AND l.resource_id=$1 AND l.expires_at>now())) ORDER BY d.id LIMIT 1")
        .bind(host).fetch_all(&mut *tx).await.map_err(db(&rid))?;
    let controllers = sqlx::query("SELECT p.device_id,p.endpoint_id,p.addresses,p.relay_urls,p.revision,p.expires_at,d.kind FROM cloud_remote_peer_endpoints p JOIN cloud_remote_devices d ON d.id=p.device_id JOIN cloud_remote_grants g ON g.device_id=d.id WHERE g.host_id=$1 AND d.kind='controller' AND d.status='active' AND g.status='active' AND g.confirmed_revision=g.revision AND p.expires_at>now() AND (d.owner_user_id=$2 OR $3) ORDER BY d.id LIMIT 63")
        .bind(host).bind(owner).bind(is_owner).fetch_all(&mut *tx).await.map_err(db(&rid))?;
    rows.extend(controllers);
    tx.commit().await.map_err(db(&rid))?;
    let items: Vec<_> = rows.into_iter().map(|row| json!({"deviceId":row.get::<Uuid,_>("device_id"),"endpointId":row.get::<String,_>("endpoint_id"),"addresses":row.get::<Value,_>("addresses"),"relayUrls":row.get::<Value,_>("relay_urls"),"revision":row.get::<i64,_>("revision"),"kind":row.get::<String,_>("kind"),"expiresAt":row.get::<chrono::DateTime<chrono::Utc>,_>("expires_at").to_rfc3339()})).collect();
    Ok(Json(
        json!({"hostId":host,"items":items,"relayUrls":state.peer.relay_urls}),
    ))
}
