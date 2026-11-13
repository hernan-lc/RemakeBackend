//! Device-bound opaque TLS records over the existing WSS endpoint. This layer
//! authorizes delivery, never decrypts RPC or grants business authority.
use super::*;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::{Deserialize, Serialize};
use sqlx::{Postgres, Transaction};
mod routing;
pub(super) use routing::deliver_local;

const DOMAIN: &[u8] = b"tiktools.relay.tls.v1\0";
const MAX_DATA: usize = 16 * 1024;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Context {
    host_id: Uuid,
    session_id: Uuid,
    host_device_id: Uuid,
    from_device_id: Uuid,
    negotiation_id: Uuid,
    grant_revision: i64,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Announcement {
    context: Context,
    certificate: String,
    signature: String,
}
#[derive(Deserialize)]
#[serde(tag = "type", deny_unknown_fields)]
enum Input {
    #[serde(rename = "relay.open")]
    Open { announcement: Announcement },
    #[serde(rename = "relay.data", rename_all = "camelCase")]
    Data {
        tunnel_id: Uuid,
        session_id: Uuid,
        sequence: i64,
        data: String,
    },
    #[serde(rename = "relay.close", rename_all = "camelCase")]
    Close {
        tunnel_id: Uuid,
        session_id: Uuid,
        sequence: i64,
    },
}
impl Input {
    fn ids(&self) -> (Uuid, Uuid, Option<i64>) {
        match self {
            Self::Open { announcement } => (
                announcement.context.negotiation_id,
                announcement.context.session_id,
                None,
            ),
            Self::Data {
                tunnel_id,
                session_id,
                sequence,
                ..
            }
            | Self::Close {
                tunnel_id,
                session_id,
                sequence,
            } => (*tunnel_id, *session_id, Some(*sequence)),
        }
    }
}

pub(super) async fn handle(
    state: &AppState,
    actor: &Authorized,
    raw: Value,
    fencing: i64,
) -> String {
    let Ok(input) = serde_json::from_value::<Input>(raw) else {
        return frame("error", json!({"code":"invalidFrame"}));
    };
    let (id, _, sequence) = input.ids();
    match forward(state, actor, input, fencing).await {
        Ok(()) => frame("relay.accepted", json!({"tunnelId":id,"sequence":sequence})),
        Err(code) => frame("error", json!({"code":code,"tunnelId":id})),
    }
}

struct Scope {
    host_device: Uuid,
    revision: i64,
    controller_endpoint: String,
    host_endpoint: String,
    controller_node: String,
    host_node: String,
    controller_fencing: i64,
    host_fencing: i64,
}

async fn scope(
    tx: &mut Transaction<'_, Postgres>,
    actor: &Authorized,
    session: Uuid,
    host_device: Uuid,
    fencing: i64,
    node: &str,
) -> Result<Scope, &'static str> {
    if matches!(actor.role, Role::Controller) && actor.session != Some(session)
        || matches!(actor.role, Role::Host) && actor.device != host_device
    {
        return Err("forbidden");
    }
    // Serialize streams within a session and order delivery against revocation.
    // Both live lease generations are pinned, including across backend nodes.
    let row = sqlx::query("SELECT s.device_id,g.revision,cp.endpoint_id AS controller_endpoint,hp.endpoint_id AS host_endpoint,cl.node_id AS controller_node,cl.fencing_token AS controller_fencing FROM cloud_remote_sessions s JOIN cloud_remote_grants g ON g.id=s.grant_id JOIN cloud_remote_hosts h ON h.id=s.host_id JOIN cloud_remote_devices cd ON cd.id=s.device_id JOIN cloud_remote_devices hd ON hd.id=$3 JOIN cloud_remote_peer_endpoints cp ON cp.device_id=cd.id JOIN cloud_remote_peer_endpoints hp ON hp.device_id=hd.id JOIN cloud_relay_leases cl ON cl.resource_kind='session' AND cl.resource_id=s.id AND cl.device_id=cd.id WHERE s.id=$1 AND s.host_id=$2 AND s.status='active' AND g.status='active' AND g.revision=s.grant_revision AND g.confirmed_revision=g.revision AND h.status='active' AND cd.status='active' AND hd.status='active' AND hd.kind='host' AND hd.host_id=h.id AND hd.owner_user_id=h.owner_user_id AND cp.expires_at>now() AND hp.expires_at>now() AND cl.expires_at>now() FOR UPDATE OF s FOR SHARE OF g,h,cd,hd,cl")
        .bind(session).bind(actor.host).bind(host_device).fetch_optional(&mut **tx).await
        .map_err(|_| "temporaryFailure")?.ok_or("revoked")?;
    let controller: Uuid = row.get("device_id");
    let revision: i64 = row.get("revision");
    let controller_node: String = row.get("controller_node");
    let controller_fencing: i64 = row.get("controller_fencing");
    if matches!(actor.role, Role::Controller)
        && (actor.device != controller
            || actor.grant_revision != revision
            || controller_node != node
            || controller_fencing != fencing)
    {
        return Err("revoked");
    }
    let host = sqlx::query("SELECT node_id,fencing_token FROM cloud_relay_leases WHERE resource_kind='host' AND resource_id=$1 AND device_id=$2 AND expires_at>now() FOR SHARE")
        .bind(actor.host).bind(host_device).fetch_optional(&mut **tx).await.map_err(|_| "temporaryFailure")?;
    // Missing target ownership is availability for a valid controller. A
    // missing or replaced sender lease never authorizes an old host socket.
    let host = host.ok_or(match actor.role {
        Role::Host => "revoked",
        Role::Controller => "peerOffline",
    })?;
    let host_node: String = host.get("node_id");
    let host_fencing: i64 = host.get("fencing_token");
    if matches!(actor.role, Role::Host) && (host_node != node || host_fencing != fencing) {
        return Err("revoked");
    }
    let scope = Scope {
        host_device,
        revision,
        controller_node,
        controller_fencing,
        host_node,
        host_fencing,
        controller_endpoint: row.get("controller_endpoint"),
        host_endpoint: row.get("host_endpoint"),
    };
    Ok(scope)
}

async fn forward(
    state: &AppState,
    actor: &Authorized,
    input: Input,
    fencing: i64,
) -> Result<(), &'static str> {
    let (id, session, _) = input.ids();
    if matches!(actor.role, Role::Controller) && actor.session != Some(session) {
        return Err("forbidden");
    }
    super::super::throttle(
        state,
        &RequestId::fresh(),
        &actor.device.to_string(),
        if matches!(&input, Input::Open { .. }) {
            "relay-open"
        } else {
            "relay-record"
        },
        if matches!(&input, Input::Open { .. }) {
            30
        } else {
            6000
        },
    )
    .await
    .map_err(|_| "rateLimited")?;
    let mut tx = state.pool.begin().await.map_err(|_| "temporaryFailure")?;
    let host_device = match &input {
        Input::Open { announcement } => announcement.context.host_device_id,
        _ => sqlx::query_scalar(
            "SELECT host_device_id FROM cloud_remote_relay_tunnels WHERE id=$1 AND session_id=$2",
        )
        .bind(id)
        .bind(session)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|_| "temporaryFailure")?
        .ok_or("stale")?,
    };
    let scope = scope(
        &mut tx,
        actor,
        session,
        host_device,
        fencing,
        state.remote.node(),
    )
    .await?;
    let forwarded = match input {
        Input::Open { announcement } => open(&mut tx, actor, &scope, announcement).await?,
        Input::Data { sequence, data, .. } => {
            if sequence < 0 || sequence == i64::MAX || data.len() > MAX_DATA * 4 / 3 + 4 {
                return Err("invalidFrame");
            }
            let decoded = URL_SAFE_NO_PAD.decode(&data).map_err(|_| "invalidFrame")?;
            if decoded.is_empty() || decoded.len() > MAX_DATA {
                return Err("invalidFrame");
            }
            advance(&mut tx, actor, &scope, id, session, sequence, false).await?;
            json!({"type":"relay.data","tunnelId":id,"sessionId":session,
                "fromDeviceId":actor.device,"sequence":sequence,"data":data})
        }
        Input::Close { sequence, .. } => {
            if sequence < 0 || sequence == i64::MAX {
                return Err("invalidFrame");
            }
            advance(&mut tx, actor, &scope, id, session, sequence, true).await?;
            json!({"type":"relay.close","tunnelId":id,"sessionId":session,
                "fromDeviceId":actor.device,"sequence":sequence})
        }
    };
    let (kind, target, node, generation) = match actor.role {
        Role::Controller => (
            cluster::Kind::Host,
            actor.host,
            scope.host_node.as_str(),
            scope.host_fencing,
        ),
        Role::Host => (
            cluster::Kind::Session,
            session,
            scope.controller_node.as_str(),
            scope.controller_fencing,
        ),
    };
    let delivery = routing::prepare(
        &state.remote,
        &mut tx,
        kind,
        target,
        node,
        generation,
        forwarded,
    )
    .await?;
    tx.commit().await.map_err(|_| "temporaryFailure")?;
    delivery.send();
    Ok(())
}

async fn open(
    tx: &mut Transaction<'_, Postgres>,
    actor: &Authorized,
    scope: &Scope,
    announcement: Announcement,
) -> Result<Value, &'static str> {
    let c = &announcement.context;
    if c.host_id != actor.host
        || c.from_device_id != actor.device
        || c.grant_revision != scope.revision
        || c.negotiation_id.is_nil()
        || announcement.signature.len() != 86
        || announcement.certificate.len() > 4096 * 4 / 3 + 4
    {
        return Err("invalidFrame");
    }
    let certificate = URL_SAFE_NO_PAD
        .decode(&announcement.certificate)
        .map_err(|_| "invalidFrame")?;
    if certificate.is_empty() || certificate.len() > 4096 {
        return Err("invalidFrame");
    }
    let endpoint = match actor.role {
        Role::Host => &scope.host_endpoint,
        Role::Controller => &scope.controller_endpoint,
    };
    let mut signed = serde_json::to_value(c).map_err(|_| "invalidFrame")?;
    signed["certificate"] = json!(announcement.certificate);
    let mut message = DOMAIN.to_vec();
    message.extend_from_slice(super::super::canonical_json(&signed).as_bytes());
    if !hex::decode(endpoint)
        .is_ok_and(|key| super::super::verify_proof(&key, &message, &announcement.signature))
    {
        return Err("forbidden");
    }
    let expiry: chrono::DateTime<chrono::Utc> = match actor.role {
        Role::Controller => {
            let count: i64 = sqlx::query_scalar("SELECT count(*) FROM cloud_remote_relay_tunnels WHERE session_id=$1 AND expires_at>now() AND NOT closed")
                .bind(c.session_id).fetch_one(&mut **tx).await.map_err(|_| "temporaryFailure")?;
            if count >= 2 {
                return Err("busy");
            }
            sqlx::query_scalar("INSERT INTO cloud_remote_relay_tunnels(id,session_id,host_device_id,controller_endpoint,host_endpoint,grant_revision,host_node,controller_node,host_fencing,controller_fencing) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10) ON CONFLICT(id) DO NOTHING RETURNING expires_at")
                .bind(c.negotiation_id).bind(c.session_id).bind(scope.host_device)
                .bind(&scope.controller_endpoint).bind(&scope.host_endpoint).bind(scope.revision)
                .bind(&scope.host_node).bind(&scope.controller_node).bind(scope.host_fencing).bind(scope.controller_fencing)
                .fetch_optional(&mut **tx).await.map_err(|_| "temporaryFailure")?.ok_or("stale")?
        }
        Role::Host => {
            let row = tunnel(tx, scope, c.negotiation_id, c.session_id).await?;
            if row.get::<bool, _>("answered") {
                return Err("stale");
            }
            sqlx::query_scalar("UPDATE cloud_remote_relay_tunnels SET answered=TRUE,expires_at=created_at+interval '8 hours' WHERE id=$1 RETURNING expires_at")
                .bind(c.negotiation_id).fetch_one(&mut **tx).await.map_err(|_| "temporaryFailure")?
        }
    };
    Ok(
        json!({"type":"relay.open","announcement":announcement,"fromEndpointId":endpoint,
        "expiresAt":expiry.timestamp_millis()}),
    )
}

async fn tunnel(
    tx: &mut Transaction<'_, Postgres>,
    scope: &Scope,
    id: Uuid,
    session: Uuid,
) -> Result<sqlx::postgres::PgRow, &'static str> {
    sqlx::query("SELECT answered,controller_sequence,host_sequence FROM cloud_remote_relay_tunnels WHERE id=$1 AND session_id=$2 AND host_device_id=$3 AND grant_revision=$4 AND controller_endpoint=$5 AND host_endpoint=$6 AND host_node=$7 AND controller_node=$8 AND host_fencing=$9 AND controller_fencing=$10 AND expires_at>now() AND NOT closed FOR UPDATE")
        .bind(id).bind(session).bind(scope.host_device).bind(scope.revision)
        .bind(&scope.controller_endpoint).bind(&scope.host_endpoint).bind(&scope.host_node).bind(&scope.controller_node)
        .bind(scope.host_fencing).bind(scope.controller_fencing).fetch_optional(&mut **tx).await
        .map_err(|_| "temporaryFailure")?.ok_or("stale")
}

async fn advance(
    tx: &mut Transaction<'_, Postgres>,
    actor: &Authorized,
    scope: &Scope,
    id: Uuid,
    session: Uuid,
    sequence: i64,
    close: bool,
) -> Result<(), &'static str> {
    let row = tunnel(tx, scope, id, session).await?;
    let field = match actor.role {
        Role::Host => "host_sequence",
        Role::Controller => "controller_sequence",
    };
    if (!close && !row.get::<bool, _>("answered")) || sequence != row.get::<i64, _>(field) {
        return Err("stale");
    }
    let query = match actor.role {
        Role::Host => "UPDATE cloud_remote_relay_tunnels SET host_sequence=host_sequence+1,closed=$2 WHERE id=$1",
        Role::Controller => "UPDATE cloud_remote_relay_tunnels SET controller_sequence=controller_sequence+1,closed=$2 WHERE id=$1",
    };
    sqlx::query(query)
        .bind(id)
        .bind(close)
        .execute(&mut **tx)
        .await
        .map_err(|_| "temporaryFailure")?;
    Ok(())
}

#[cfg(test)]
pub(super) async fn assert_queue_bound(pool: &PgPool) {
    use futures_util::stream;
    let hub = Hub::new(format!("queue-source-{}", Uuid::new_v4()));
    let target = Uuid::new_v4();
    let destination = format!("queue-destination-{}", Uuid::new_v4());
    let outcomes = stream::iter(0..OUTBOX_CAPACITY + 16)
        .map(|sequence| {
            let hub = &hub;
            let destination = &destination;
            async move {
                let mut tx = pool.begin().await.unwrap();
                match routing::prepare(
                    hub,
                    &mut tx,
                    cluster::Kind::Host,
                    target,
                    destination,
                    7,
                    json!({"type":"relay.data","sequence":sequence}),
                )
                .await
                {
                    Ok(delivery) => {
                        tx.commit().await.unwrap();
                        delivery.send();
                        true
                    }
                    Err(code) => {
                        assert_eq!(code, "peerBusy");
                        false
                    }
                }
            }
        })
        .buffer_unordered(8)
        .collect::<Vec<_>>()
        .await;
    assert_eq!(
        outcomes.iter().filter(|accepted| **accepted).count(),
        OUTBOX_CAPACITY
    );
    let pending: i64 =
        sqlx::query_scalar("SELECT count(*) FROM cloud_relay_frames WHERE resource_id=$1")
            .bind(target)
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(pending, OUTBOX_CAPACITY as i64);
    sqlx::query("DELETE FROM cloud_relay_frames WHERE resource_id=$1")
        .bind(target)
        .execute(pool)
        .await
        .unwrap();
}
