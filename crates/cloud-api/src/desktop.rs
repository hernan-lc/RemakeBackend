//! Backend-mediated desktop handoff; OAuth/session validation stays in Better Auth.
use crate::AppState;
use axum::{
    extract::{ConnectInfo, State},
    http::StatusCode,
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

pub async fn redeem(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    State(state): State<AppState>,
    Json(input): Json<RedeemInput>,
) -> Result<Json<RedeemOutput>, StatusCode> {
    // The socket peer is authoritative. Forwarded headers are deliberately ignored.
    // Behind a reverse proxy all requests share its limit unless deployment provides
    // an independently verified rate-limiting layer at the edge.
    limit(&state, "redeem", &peer.ip().to_string(), 60).await?;
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
