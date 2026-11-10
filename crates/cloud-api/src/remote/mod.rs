//! Remote control through the Cloud relay.
//!
//! A host opens an outbound WSS connection; controllers never reach the PC
//! directly. The Cloud authenticates identities and routes bounded commands,
//! while the host re-evaluates every permission at execution time. Grants here
//! are the admission record the host confirmed, not a second rules engine.

pub mod access;
pub mod hosts;
pub mod relay;
pub mod tickets;

use crate::api_error::{Kind, RequestId, V3Error};
use axum::{
    routing::{get, post},
    Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use ed25519_dalek::{Signature, VerifyingKey};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::time::Duration;

pub const PROTOCOL_VERSION: i32 = 1;
pub const DEVICE_PROOF_DOMAIN: &str = "tiktools.remote.v1";
pub const CHALLENGE_TTL: Duration = Duration::from_secs(60);
pub const TICKET_TTL: Duration = Duration::from_secs(60);
pub const INVITATION_MAX_TTL_SECONDS: i64 = 900;
pub const INVITATION_DEFAULT_TTL_SECONDS: i64 = 600;
pub const MAX_COMMAND_LIFETIME_MS: i64 = 60_000;
pub const MAX_COMMAND_ARGS_BYTES: usize = 16 * 1024;
pub const MAX_RESULT_BYTES: usize = 64 * 1024;
pub const MAX_POLICY_BYTES: usize = 16 * 1024;
pub const MAX_LIST: i64 = 50;

pub fn routes() -> Router<crate::AppState> {
    Router::new()
        .route("/api/v3/remote/challenges", post(hosts::create_challenge))
        .route(
            "/api/v3/remote/devices",
            get(hosts::list_devices).post(hosts::enroll_device),
        )
        .route(
            "/api/v3/remote/devices/{device_id}",
            axum::routing::delete(hosts::revoke_device),
        )
        .route(
            "/api/v3/remote/hosts",
            get(hosts::list_hosts).post(hosts::create_host),
        )
        .route(
            "/api/v3/remote/hosts/{host_id}",
            get(hosts::get_host).patch(hosts::update_host),
        )
        .route(
            "/api/v3/remote/hosts/{host_id}/invitations",
            get(access::list_invitations).post(access::create_invitation),
        )
        .route(
            "/api/v3/remote/hosts/{host_id}/invitations/{invitation_id}/revoke",
            post(access::revoke_invitation),
        )
        .route(
            "/api/v3/remote/invitations/redeem",
            post(access::redeem_invitation),
        )
        .route(
            "/api/v3/remote/hosts/{host_id}/requests",
            get(access::list_requests),
        )
        .route(
            "/api/v3/remote/hosts/{host_id}/requests/{request_id}/decision",
            post(access::decide_request),
        )
        .route(
            "/api/v3/remote/hosts/{host_id}/grants",
            get(access::list_grants),
        )
        .route(
            "/api/v3/remote/hosts/{host_id}/grants/{grant_id}",
            axum::routing::patch(access::update_grant),
        )
        .route(
            "/api/v3/remote/hosts/{host_id}/grants/{grant_id}/revoke",
            post(access::revoke_grant),
        )
        .route(
            "/api/v3/remote/hosts/{host_id}/sessions",
            get(access::list_sessions),
        )
        .route(
            "/api/v3/remote/hosts/{host_id}/sessions/{session_id}/revoke",
            post(access::revoke_session),
        )
        .route("/api/v3/remote/tickets", post(tickets::issue))
        .route("/api/v3/remote/socket", get(relay::upgrade))
        .route(
            "/api/v3/remote/hosts/{host_id}/commands/{command_id}",
            get(relay::command_status),
        )
}

/// Operations the relay admits, grouped by the authority class that gates them.
/// The host still checks every operation against its own grant and limits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationClass {
    Read,
    Moderation,
    AutomationDraft,
}

pub const OPERATIONS: &[(&str, OperationClass)] = &[
    ("live.status", OperationClass::Read),
    ("workflow.list", OperationClass::Read),
    ("competition.list", OperationClass::Read),
    ("points.read", OperationClass::Read),
    ("automation.list", OperationClass::Read),
    ("command.outcome", OperationClass::Read),
    ("workflow.disable", OperationClass::Moderation),
    ("workflow.delete", OperationClass::Moderation),
    ("points.adjust", OperationClass::Moderation),
    ("automation.toggle", OperationClass::Moderation),
    ("automation.delete", OperationClass::Moderation),
    ("competition.control", OperationClass::Moderation),
    ("automation.draft.submit", OperationClass::AutomationDraft),
];

pub fn operation_class(operation: &str) -> Option<OperationClass> {
    OPERATIONS
        .iter()
        .find(|(name, _)| *name == operation)
        .map(|(_, class)| *class)
}

/// Preset roles map to operation classes. `custom` uses the explicit list stored
/// in the grant policy, validated against [`OPERATIONS`] when it is written.
pub fn role_admits(role: &str, policy: &Value, operation: &str) -> bool {
    let Some(class) = operation_class(operation) else {
        return false;
    };
    match role {
        "observer" => class == OperationClass::Read,
        "moderator" => matches!(class, OperationClass::Read | OperationClass::Moderation),
        "automationEditor" => matches!(
            class,
            OperationClass::Read | OperationClass::AutomationDraft
        ),
        "custom" => policy["allowedOperations"]
            .as_array()
            .is_some_and(|ops| ops.iter().any(|op| op.as_str() == Some(operation))),
        _ => false,
    }
}

pub const PRESET_ROLES: &[&str] = &["observer", "moderator", "automationEditor"];

/// Serializes JSON with sorted object keys so digests do not depend on map order.
pub fn canonical_json(value: &Value) -> String {
    fn write(value: &Value, out: &mut String) {
        match value {
            Value::Object(map) => {
                out.push('{');
                let mut keys: Vec<&String> = map.keys().collect();
                keys.sort();
                for (i, key) in keys.into_iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push_str(&serde_json::to_string(key).unwrap_or_default());
                    out.push(':');
                    write(&map[key], out);
                }
                out.push('}');
            }
            Value::Array(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write(item, out);
                }
                out.push(']');
            }
            other => out.push_str(&other.to_string()),
        }
    }
    let mut out = String::new();
    write(value, &mut out);
    out
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Fresh 256-bit bearer secret, returned to the caller once in base64url form.
pub fn new_secret() -> String {
    let bytes: [u8; 32] = rand::random();
    URL_SAFE_NO_PAD.encode(bytes)
}

pub fn secret_hash(secret: &str) -> String {
    sha256_hex(secret.as_bytes())
}

/// Digest of the request context a challenge is bound to.
pub fn binding_digest(binding: &Value) -> String {
    sha256_hex(canonical_json(binding).as_bytes())
}

/// Message a device signs for a challenge. Every field comes from the stored
/// challenge row, so a signature cannot be replayed across purposes or contexts.
pub fn proof_message(purpose: &str, nonce: &[u8], binding_sha256: &str) -> Vec<u8> {
    format!(
        "{DEVICE_PROOF_DOMAIN}\n{purpose}\n{}\n{binding_sha256}",
        URL_SAFE_NO_PAD.encode(nonce)
    )
    .into_bytes()
}

pub fn verify_proof(public_key: &[u8], message: &[u8], signature: &str) -> bool {
    let Ok(key_bytes) = <[u8; 32]>::try_from(public_key) else {
        return false;
    };
    let Ok(key) = VerifyingKey::from_bytes(&key_bytes) else {
        return false;
    };
    let Ok(sig_bytes) = URL_SAFE_NO_PAD.decode(signature) else {
        return false;
    };
    let Ok(sig) = Signature::from_slice(&sig_bytes) else {
        return false;
    };
    key.verify_strict(message, &sig).is_ok()
}

pub fn decode_public_key(raw: &str) -> Option<Vec<u8>> {
    let bytes = URL_SAFE_NO_PAD.decode(raw).ok()?;
    (bytes.len() == 32).then_some(bytes)
}

pub fn is_command_id(value: &str) -> bool {
    (8..=128).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.:".contains(&b))
}

pub fn parse_uuid(rid: &RequestId, raw: &str) -> Result<uuid::Uuid, V3Error> {
    uuid::Uuid::parse_str(raw).map_err(|_| V3Error::new(rid, Kind::InvalidRequest))
}

pub fn parse_body<T: serde::de::DeserializeOwned>(
    rid: &RequestId,
    bytes: &[u8],
    limit: usize,
) -> Result<T, V3Error> {
    if bytes.len() > limit {
        return Err(V3Error::new(rid, Kind::InvalidRequest));
    }
    serde_json::from_slice(bytes).map_err(|_| V3Error::new(rid, Kind::InvalidRequest))
}

/// Per-account admission for write-heavy remote operations. It reuses the Cloud
/// limiter; if the limiter cannot answer, the request fails closed.
pub async fn throttle(
    state: &crate::AppState,
    rid: &RequestId,
    user: &str,
    action: &str,
    per_minute: i64,
) -> Result<(), V3Error> {
    crate::workspace::limit(state, user, action, per_minute)
        .await
        .map_err(|status| match status {
            axum::http::StatusCode::TOO_MANY_REQUESTS => V3Error::new(rid, Kind::RateLimited),
            _ => V3Error::new(rid, Kind::TemporaryFailure),
        })
}

pub fn db(rid: &RequestId) -> impl Fn(sqlx::Error) -> V3Error + '_ {
    move |error| crate::api_error::database(rid, error)
}

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use serde_json::json;

    #[test]
    fn canonical_json_sorts_keys_recursively() {
        let a = json!({"b": 1, "a": {"d": [2, {"z": true, "y": null}], "c": "x"}});
        let b = json!({"a": {"c": "x", "d": [2, {"y": null, "z": true}]}, "b": 1});
        assert_eq!(canonical_json(&a), canonical_json(&b));
        assert_eq!(
            canonical_json(&a),
            r#"{"a":{"c":"x","d":[2,{"y":null,"z":true}]},"b":1}"#
        );
    }

    #[test]
    fn proof_is_bound_to_domain_purpose_nonce_and_binding() {
        let key = SigningKey::from_bytes(&[7; 32]);
        let nonce = [9u8; 32];
        let binding = json!({"deviceId": "d1", "hostId": "h1", "role": "host"});
        let digest = binding_digest(&binding);
        let message = proof_message("ticket", &nonce, &digest);
        let sig = URL_SAFE_NO_PAD.encode(key.sign(&message).to_bytes());
        let public = key.verifying_key().to_bytes();
        assert!(verify_proof(&public, &message, &sig));
        // The same signature must not authorize another purpose or binding.
        let other_purpose = proof_message("enroll", &nonce, &digest);
        assert!(!verify_proof(&public, &other_purpose, &sig));
        let other_binding = proof_message(
            "ticket",
            &nonce,
            &binding_digest(&json!({"deviceId": "d2"})),
        );
        assert!(!verify_proof(&public, &other_binding, &sig));
    }

    #[test]
    fn roles_admit_only_their_operation_classes() {
        let none = json!({});
        assert!(role_admits("observer", &none, "live.status"));
        assert!(!role_admits("observer", &none, "points.adjust"));
        assert!(role_admits("moderator", &none, "points.adjust"));
        assert!(!role_admits("moderator", &none, "automation.draft.submit"));
        assert!(role_admits(
            "automationEditor",
            &none,
            "automation.draft.submit"
        ));
        assert!(!role_admits("automationEditor", &none, "automation.toggle"));
        assert!(!role_admits("unknown", &none, "live.status"));
        assert!(!role_admits("observer", &none, "not.an.operation"));
    }

    #[test]
    fn custom_roles_use_only_listed_operations() {
        let policy = json!({"allowedOperations": ["points.read"]});
        assert!(role_admits("custom", &policy, "points.read"));
        assert!(!role_admits("custom", &policy, "points.adjust"));
    }

    #[test]
    fn command_ids_are_bounded_and_restricted() {
        assert!(is_command_id("cmd-000001"));
        assert!(!is_command_id("short"));
        assert!(!is_command_id("has space in it"));
        assert!(!is_command_id(&"a".repeat(129)));
    }

    #[test]
    fn secrets_are_random_and_hashed_not_stored_raw() {
        let a = new_secret();
        let b = new_secret();
        assert_ne!(a, b);
        assert_eq!(a.len(), 43);
        assert_ne!(secret_hash(&a), a);
        assert_eq!(secret_hash(&a).len(), 64);
    }
}
