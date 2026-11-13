//! Device-signed SDP/ICE over the existing authenticated, fenced WSS relay.
//! Negotiation conveys no RPC authority and never creates business commands.
use super::*;
use serde::{Deserialize, Serialize};

const DOMAIN: &[u8] = b"tiktools.webrtc.signal.v1\0";

pub(crate) fn capabilities() -> Value {
    json!({"protocolVersion":1,"supported":true,"frameType":"rtc.signal",
        "signatureDomain":"tiktools.webrtc.signal.v1","iceConfigurationPath":"/api/v3/remote/rtc-config","sdpBytes":65536,
        "candidateBytes":2048,"signalsPerSide":65,"negotiationSeconds":60,
        "parallelNegotiationsPerSession":4,
        "applicationRelay":{"supported":true,"protocolVersion":1,
            "frameTypes":["relay.open","relay.data","relay.close"],"recordBytes":16384,
            "encryption":"mutual-tls-1.3","signatureDomain":"tiktools.relay.tls.v1",
            "clientKinds":["desktop","agent"]}})
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SignalFrame {
    r#type: String,
    negotiation_id: Uuid,
    session_id: Uuid,
    host_device_id: Uuid,
    sequence: u32,
    signal: Signal,
    signature: String,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
enum Signal {
    Offer {
        sdp: String,
    },
    Answer {
        sdp: String,
    },
    Candidate {
        candidate: String,
        #[serde(rename = "sdpMid")]
        sdp_mid: Option<String>,
        #[serde(rename = "sdpMLineIndex")]
        sdp_m_line_index: Option<u16>,
        #[serde(rename = "usernameFragment")]
        username_fragment: Option<String>,
    },
    EndOfCandidates,
}
impl Signal {
    fn valid(&self) -> bool {
        let field = |s: &Option<String>, limit| {
            s.as_ref()
                .is_none_or(|s| s.len() <= limit && !s.chars().any(char::is_control))
        };
        match self {
            Self::Offer { sdp } | Self::Answer { sdp } => {
                sdp.len() <= 64 * 1024
                    && !sdp.contains('\0')
                    && (sdp.starts_with("v=0\r\n") || sdp.starts_with("v=0\n"))
                    && sdp.lines().any(|line| line.starts_with("a=fingerprint:"))
            }
            Self::Candidate {
                candidate,
                sdp_mid,
                sdp_m_line_index,
                username_fragment,
            } => {
                candidate.starts_with("candidate:")
                    && candidate.len() <= 2048
                    && !candidate.chars().any(char::is_control)
                    && field(sdp_mid, 64)
                    && sdp_m_line_index.is_none_or(|index| index < 32)
                    && field(username_fragment, 256)
            }
            Self::EndOfCandidates => true,
        }
    }
}

pub(super) async fn handle(
    state: &AppState,
    actor: &Authorized,
    raw: Value,
    fencing: i64,
) -> String {
    let Ok(input) = serde_json::from_value::<SignalFrame>(raw) else {
        return frame("error", json!({"code":"invalidFrame"}));
    };
    let id = input.negotiation_id;
    let seq = input.sequence;
    if state.peer.relay_only {
        return frame(
            "error",
            json!({"code":"directUnavailable","negotiationId":id}),
        );
    }
    match forward(state, actor, input, fencing).await {
        Ok(()) => frame("rtc.accepted", json!({"negotiationId":id,"sequence":seq})),
        Err(code) => frame("error", json!({"code":code,"negotiationId":id})),
    }
}

async fn forward(
    state: &AppState,
    actor: &Authorized,
    input: SignalFrame,
    fencing: i64,
) -> Result<(), &'static str> {
    if input.r#type != "rtc.signal"
        || !input.signal.valid()
        || input.sequence > 64
        || input.signature.len() != 86
    {
        return Err("invalidFrame");
    }
    if matches!(actor.role, Role::Controller) && actor.session != Some(input.session_id)
        || matches!(actor.role, Role::Host) && actor.device != input.host_device_id
    {
        return Err("forbidden");
    }
    let offer = matches!(input.signal, Signal::Offer { .. });
    let answer = matches!(input.signal, Signal::Answer { .. });
    if offer && (!matches!(actor.role, Role::Controller) || input.sequence != 0)
        || answer && (!matches!(actor.role, Role::Host) || input.sequence != 0)
    {
        return Err("invalidFrame");
    }
    super::super::throttle(
        state,
        &RequestId::fresh(),
        &actor.device.to_string(),
        "rtc-signal",
        240,
    )
    .await
    .map_err(|_| "rateLimited")?;
    let mut tx = state.pool.begin().await.map_err(|_| "temporaryFailure")?;
    // Lock the session while counting offers. Permission writers are ordered
    // against this check; a cached socket ticket alone is insufficient.
    let (resource_kind, resource) = match actor.role {
        Role::Host => ("host", actor.host),
        Role::Controller => ("session", input.session_id),
    };
    let row = sqlx::query("SELECT s.device_id,g.revision,cp.endpoint_id AS controller_endpoint,hp.endpoint_id AS host_endpoint FROM cloud_remote_sessions s JOIN cloud_remote_grants g ON g.id=s.grant_id JOIN cloud_remote_hosts h ON h.id=s.host_id JOIN cloud_remote_devices cd ON cd.id=s.device_id JOIN cloud_remote_devices hd ON hd.id=$3 JOIN cloud_remote_peer_endpoints cp ON cp.device_id=cd.id JOIN cloud_remote_peer_endpoints hp ON hp.device_id=hd.id WHERE s.id=$1 AND s.host_id=$2 AND s.status='active' AND g.status='active' AND g.revision=s.grant_revision AND g.confirmed_revision=g.revision AND h.status='active' AND cd.status='active' AND hd.status='active' AND hd.kind='host' AND hd.host_id=h.id AND hd.owner_user_id=h.owner_user_id AND cp.expires_at>now() AND hp.expires_at>now() AND EXISTS(SELECT 1 FROM cloud_relay_leases l WHERE l.resource_kind=$4 AND l.resource_id=$5 AND l.node_id=$6 AND l.fencing_token=$7 AND l.expires_at>now()) FOR UPDATE OF s FOR SHARE OF g,h,cd,hd")
        .bind(input.session_id).bind(actor.host).bind(input.host_device_id)
        .bind(resource_kind).bind(resource).bind(state.remote.node()).bind(fencing)
        .fetch_optional(&mut *tx).await.map_err(|_| "temporaryFailure")?.ok_or("revoked")?;
    let device: Uuid = row.get("device_id");
    let revision: i64 = row.get("revision");
    if matches!(actor.role, Role::Controller)
        && (actor.device != device || actor.grant_revision != revision)
    {
        return Err("revoked");
    }
    let online = sqlx::query("SELECT 1 FROM cloud_relay_leases WHERE resource_kind='host' AND resource_id=$1 AND device_id=$2 AND expires_at>now() FOR SHARE")
        .bind(actor.host).bind(input.host_device_id).fetch_optional(&mut *tx).await.map_err(|_| "temporaryFailure")?;
    if online.is_none() {
        return Err("peerOffline");
    }
    let controller_endpoint: String = row.get("controller_endpoint");
    let host_endpoint: String = row.get("host_endpoint");
    let endpoint = if matches!(actor.role, Role::Controller) {
        &controller_endpoint
    } else {
        &host_endpoint
    };
    let signal = serde_json::to_value(&input.signal).map_err(|_| "invalidFrame")?;
    let signed = json!({"hostId":actor.host,"sessionId":input.session_id,
        "hostDeviceId":input.host_device_id,"fromDeviceId":actor.device,
        "negotiationId":input.negotiation_id,"grantRevision":revision,
        "sequence":input.sequence,"signal":signal});
    let mut message = DOMAIN.to_vec();
    message.extend_from_slice(super::super::canonical_json(&signed).as_bytes());
    let key = hex::decode(endpoint).map_err(|_| "forbidden")?;
    if !super::super::verify_proof(&key, &message, &input.signature) {
        return Err("forbidden");
    }

    let expires_at = if offer {
        sqlx::query(
            "DELETE FROM cloud_remote_rtc_negotiations WHERE session_id=$1 AND expires_at<=now()",
        )
        .bind(input.session_id)
        .execute(&mut *tx)
        .await
        .map_err(|_| "temporaryFailure")?;
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM cloud_remote_rtc_negotiations WHERE session_id=$1",
        )
        .bind(input.session_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| "temporaryFailure")?;
        if count >= 4 {
            return Err("busy");
        }
        sqlx::query_scalar::<_,chrono::DateTime<chrono::Utc>>("INSERT INTO cloud_remote_rtc_negotiations(id,session_id,host_device_id,controller_endpoint,host_endpoint,grant_revision,controller_sequence) VALUES($1,$2,$3,$4,$5,$6,1) ON CONFLICT(id) DO NOTHING RETURNING expires_at")
            .bind(input.negotiation_id).bind(input.session_id).bind(input.host_device_id)
            .bind(&controller_endpoint).bind(&host_endpoint).bind(revision)
            .fetch_optional(&mut *tx).await.map_err(|_| "temporaryFailure")?.ok_or("stale")?
    } else {
        let query = match actor.role {
            Role::Controller => "UPDATE cloud_remote_rtc_negotiations SET controller_sequence=controller_sequence+1 WHERE id=$1 AND session_id=$2 AND host_device_id=$3 AND grant_revision=$4 AND controller_endpoint=$5 AND host_endpoint=$6 AND expires_at>now() AND controller_sequence=$7 RETURNING expires_at",
            Role::Host if answer => "UPDATE cloud_remote_rtc_negotiations SET host_sequence=1,answered=TRUE WHERE id=$1 AND session_id=$2 AND host_device_id=$3 AND grant_revision=$4 AND controller_endpoint=$5 AND host_endpoint=$6 AND expires_at>now() AND host_sequence=$7 AND answered=FALSE RETURNING expires_at",
            Role::Host => "UPDATE cloud_remote_rtc_negotiations SET host_sequence=host_sequence+1 WHERE id=$1 AND session_id=$2 AND host_device_id=$3 AND grant_revision=$4 AND controller_endpoint=$5 AND host_endpoint=$6 AND expires_at>now() AND host_sequence=$7 AND answered=TRUE RETURNING expires_at",
        };
        sqlx::query_scalar::<_, chrono::DateTime<chrono::Utc>>(query)
            .bind(input.negotiation_id)
            .bind(input.session_id)
            .bind(input.host_device_id)
            .bind(revision)
            .bind(&controller_endpoint)
            .bind(&host_endpoint)
            .bind(input.sequence as i32)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|_| "temporaryFailure")?
            .ok_or("stale")?
    };
    tx.commit().await.map_err(|_| "temporaryFailure")?;
    let mut forwarded = signed;
    forwarded["type"] = json!("rtc.signal");
    forwarded["fromEndpointId"] = json!(endpoint);
    forwarded["signature"] = json!(input.signature);
    forwarded["expiresAt"] = json!(expires_at.timestamp_millis());
    let sent = match actor.role {
        Role::Controller => {
            state
                .remote
                .send_host(&state.pool, actor.host, &forwarded)
                .await
        }
        Role::Host => {
            state
                .remote
                .route_to_session(&state.pool, input.session_id, &forwarded)
                .await
        }
    };
    if sent {
        Ok(())
    } else {
        Err("peerOffline")
    }
}

#[cfg(test)]
mod tests;
