//! Backend-mediated desktop handoff; OAuth/session validation stays in Better Auth.
use crate::AppState;
use axum::{
    extract::{ConnectInfo, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use better_auth::{
    axum_integration::AuthUser,
    desktop::{DesktopHandoffService, HandoffRequest, RedeemRequest},
    rate_limit::{RateLimitPolicy, RateLimiter},
};
use better_auth_core::{error::AuthError, StorageValue};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{net::SocketAddr, time::Duration};

const CLIENT_ID: &str = "tiktools-desktop";
const TTL: Duration = Duration::from_secs(60);
/// A per-user generation counter. Signing out bumps it, so every handoff code
/// issued before the sign-out becomes unusable even though Better Auth keeps
/// the code itself in its own secondary-storage key space.
const HANDOFF_GENERATION_TTL: Duration = Duration::from_secs(30 * 24 * 3600);

fn handoff_index_key(code: &str) -> String {
    format!(
        "cloud:handoff-index:{}",
        hex::encode(Sha256::digest(code.as_bytes()))
    )
}

fn handoff_generation_key(user_id: &str) -> String {
    format!("cloud:handoff-gen:{user_id}")
}

fn storage(
    state: &AppState,
) -> Result<&std::sync::Arc<dyn better_auth_core::SecondaryStorage>, StatusCode> {
    state
        .auth
        .secondary_storage
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)
}

async fn current_generation(state: &AppState, user_id: &str) -> Result<i64, StatusCode> {
    stored_generation(storage(state)?.as_ref(), user_id).await
}

/// Bump the user's handoff generation. Called on browser and native sign-out.
pub async fn invalidate_handoffs(state: &AppState, user_id: &str) -> Result<(), StatusCode> {
    bump_generation(storage(state)?.as_ref(), user_id).await
}

/// Shared accessors for native login-code endpoints. They live here so the
/// generation counter stays the single invalidation mechanism for both the
/// PKCE handoff and the device-relayed login code.
pub(crate) fn kv_storage(
    state: &AppState,
) -> Result<&std::sync::Arc<dyn better_auth_core::SecondaryStorage>, StatusCode> {
    storage(state)
}

pub(crate) async fn generation_of(state: &AppState, user_id: &str) -> Result<i64, StatusCode> {
    current_generation(state, user_id).await
}

pub(crate) fn generation_key(user_id: &str) -> String {
    handoff_generation_key(user_id)
}

/// Storage-level helper, kept separate so the generation semantics can be
/// tested without a database or an AppState.
async fn bump_generation(
    storage: &dyn better_auth_core::SecondaryStorage,
    user_id: &str,
) -> Result<(), StatusCode> {
    storage
        .increment(&handoff_generation_key(user_id), 1, HANDOFF_GENERATION_TTL)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(())
}

/// Read a generation from storage; a missing key means generation zero.
async fn stored_generation(
    storage: &dyn better_auth_core::SecondaryStorage,
    user_id: &str,
) -> Result<i64, StatusCode> {
    let value = storage
        .get(&handoff_generation_key(user_id))
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(value.and_then(|entry| entry.value.as_i64()).unwrap_or(0))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IssueInput {
    redirect_uri: String,
    code_challenge: String,
    state: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueOutput {
    callback_url: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RedeemInput {
    code: String,
    redirect_uri: String,
    code_verifier: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RedeemOutput {
    session_token: String,
    expires_at: u64,
}

fn reject(error: AuthError) -> StatusCode {
    match error {
        AuthError::InvalidRequest(_) => StatusCode::BAD_REQUEST,
        AuthError::Unauthorized => StatusCode::UNAUTHORIZED,
        AuthError::Forbidden(_) => StatusCode::FORBIDDEN,
        AuthError::RateLimited { .. } => StatusCode::TOO_MANY_REQUESTS,
        _ => {
            tracing::error!("desktop handoff failed");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}
async fn limit(
    state: &AppState,
    action: &str,
    identity: &str,
    count: i64,
) -> Result<(), StatusCode> {
    let storage = state
        .auth
        .secondary_storage
        .clone()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let policy = RateLimitPolicy::new(action, count, Duration::from_secs(60)).map_err(reject)?;
    RateLimiter::new(storage, "cloud:desktop")
        .enforce(&policy, identity)
        .await
        .map_err(reject)
}

pub async fn issue(
    AuthUser(user): AuthUser,
    State(state): State<AppState>,
    Json(input): Json<IssueInput>,
) -> Result<Json<IssueOutput>, StatusCode> {
    limit(&state, "issue", &user.user.id, 10).await?;
    if !(16..=256).contains(&input.state.len())
        || !input
            .state
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_.~".contains(&c))
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    let request = HandoffRequest {
        client_id: CLIENT_ID.into(),
        redirect_uri: input.redirect_uri.clone(),
        code_challenge: input.code_challenge,
        ttl: TTL,
    };
    let service = DesktopHandoffService::new(state.auth.clone()).map_err(reject)?;
    let code = service
        .issue(&user.user.id, &request)
        .await
        .map_err(reject)?;
    // Index the (hashed) code to its owner and generation so a sign-out can
    // invalidate it without enumerating Better Auth's private handoff keys.
    let generation = current_generation(&state, &user.user.id).await?;
    storage(&state)?
        .set(
            &handoff_index_key(&code),
            StorageValue {
                value: serde_json::json!({
                    "userId": user.user.id,
                    "generation": generation,
                }),
                expires_in: Some(TTL),
            },
        )
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let callback_url = better_auth::loopback_callback_url(&input.redirect_uri, &code, &input.state)
        .map_err(reject)?;
    Ok(Json(IssueOutput { callback_url }))
}

/// Rate-limit identity for redemption. The socket peer is authoritative: forwarded
/// headers are only present when the proxy boundary kept them, which happens
/// solely for peers inside `TRUSTED_PROXY_CIDRS` (untrusted peers have them
/// stripped). A parseable client IP then keys the limit per client instead of
/// per proxy; anything else falls back to the socket peer. Deployment must
/// still configure the edge proxy to overwrite forwarded headers and independently
/// verify edge rate limiting.
pub(crate) fn rate_limit_identity(headers: &HeaderMap, peer: SocketAddr) -> String {
    // Only the leftmost forwarded entry names the edge-observed client; entries
    // to its right are appended by downstream hops.
    let first_forwarded = headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .and_then(|list| list.split(',').next())
        .map(str::trim);
    let real_ip = headers
        .get("x-real-ip")
        .and_then(|value| value.to_str().ok())
        .map(str::trim);
    let forwarded = [first_forwarded, real_ip]
        .into_iter()
        .flatten()
        .find_map(|candidate| candidate.parse::<std::net::IpAddr>().ok());
    forwarded.unwrap_or_else(|| peer.ip()).to_string()
}

pub async fn redeem(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<RedeemInput>,
) -> Result<Json<RedeemOutput>, StatusCode> {
    limit(&state, "redeem", &rate_limit_identity(&headers, peer), 60).await?;
    if input.code.len() > 128 || input.redirect_uri.len() > 2048 || input.code_verifier.len() > 128
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    let index_key = handoff_index_key(&input.code);
    let index = storage(&state)?
        .get(&index_key)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        .ok_or(StatusCode::UNAUTHORIZED)?;
    let user_id = index
        .value
        .get("userId")
        .and_then(serde_json::Value::as_str)
        .ok_or(StatusCode::UNAUTHORIZED)?;
    let issued_generation = index
        .value
        .get("generation")
        .and_then(serde_json::Value::as_i64)
        .ok_or(StatusCode::UNAUTHORIZED)?;
    if current_generation(&state, user_id).await? != issued_generation {
        // The user signed out after the code was issued.
        return Err(StatusCode::UNAUTHORIZED);
    }
    let request = RedeemRequest {
        code: input.code,
        client_id: CLIENT_ID.into(),
        redirect_uri: input.redirect_uri,
        code_verifier: input.code_verifier,
    };
    let session = DesktopHandoffService::new(state.auth.clone())
        .map_err(reject)?
        .redeem(&request, true)
        .await
        .map_err(reject)?;
    let _ = storage(&state)?.delete(&index_key).await;
    Ok(Json(RedeemOutput {
        session_token: session.session_token,
        expires_at: session.session.expires_at,
    }))
}

#[cfg(test)]
mod tests {
    use super::{bump_generation, handoff_index_key, rate_limit_identity, stored_generation};
    use axum::http::HeaderMap;
    use better_auth_core::adapter::memory::MemorySecondaryStorage;
    use std::net::SocketAddr;

    #[tokio::test]
    async fn sign_out_bumps_only_that_users_handoff_generation() {
        let storage = MemorySecondaryStorage::default();
        assert_eq!(stored_generation(&storage, "alice").await.unwrap(), 0);
        bump_generation(&storage, "alice").await.unwrap();
        assert_eq!(stored_generation(&storage, "alice").await.unwrap(), 1);
        bump_generation(&storage, "alice").await.unwrap();
        assert_eq!(stored_generation(&storage, "alice").await.unwrap(), 2);
        assert_eq!(stored_generation(&storage, "bob").await.unwrap(), 0);
    }

    #[test]
    fn handoff_index_keys_never_contain_the_raw_code() {
        let key = handoff_index_key("secret-code-123");
        assert!(key.starts_with("cloud:handoff-index:"));
        assert!(!key.contains("secret-code-123"));
        assert_eq!(key, handoff_index_key("secret-code-123"));
        assert_ne!(key, handoff_index_key("another-code"));
    }

    fn peer() -> SocketAddr {
        "127.0.0.1:43210".parse().unwrap()
    }

    #[test]
    fn redeem_limit_identity_prefers_forwarded_client_ip() {
        let headers = HeaderMap::new();
        assert_eq!(rate_limit_identity(&headers, peer()), "127.0.0.1");
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "203.0.113.7".parse().unwrap());
        assert_eq!(rate_limit_identity(&headers, peer()), "203.0.113.7");
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            "203.0.113.7, 198.51.100.9".parse().unwrap(),
        );
        assert_eq!(rate_limit_identity(&headers, peer()), "203.0.113.7");
        let mut headers = HeaderMap::new();
        headers.insert("x-real-ip", "2001:db8::1".parse().unwrap());
        assert_eq!(rate_limit_identity(&headers, peer()), "2001:db8::1");
        // Spoofed or malformed values fall back to the socket peer.
        for value in ["", "not-an-ip", "203.0.113.7:1234", "unknown"] {
            let mut headers = HeaderMap::new();
            headers.insert("x-forwarded-for", value.parse().unwrap());
            assert_eq!(rate_limit_identity(&headers, peer()), "127.0.0.1");
        }
        // Only the leftmost entry counts; a garbage first entry falls through
        // to X-Real-IP, then to the socket peer.
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "unknown, 203.0.113.7".parse().unwrap());
        headers.insert("x-real-ip", "198.51.100.9".parse().unwrap());
        assert_eq!(rate_limit_identity(&headers, peer()), "198.51.100.9");
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "unknown, 203.0.113.7".parse().unwrap());
        assert_eq!(rate_limit_identity(&headers, peer()), "127.0.0.1");
    }
}
