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
use better_auth_core::error::AuthError;
use serde::{Deserialize, Serialize};
use std::{net::SocketAddr, time::Duration};

const CLIENT_ID: &str = "tiktools-desktop";
const TTL: Duration = Duration::from_secs(60);

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
    Ok(Json(RedeemOutput {
        session_token: session.session_token,
        expires_at: session.session.expires_at,
    }))
}

#[cfg(test)]
mod tests {
    use super::rate_limit_identity;
    use axum::http::HeaderMap;
    use std::net::SocketAddr;

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
