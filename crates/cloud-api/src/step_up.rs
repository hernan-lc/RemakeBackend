//! Session-bound step-up authentication for publisher signing-key operations.
//!
//! A signing-key enrollment or approval needs proof that the caller recently
//! reauthenticated, bound to the exact session, action, and resource. Two
//! verification methods mint short-lived, single-use, server-side grants:
//!
//! - password: re-verified through Better Auth's
//!   [`EmailPasswordService::verify_credentials`](better_auth::email_password::EmailPasswordService),
//!   which compares in constant time, answers unknown accounts with a dummy
//!   verification, and returns a generic error either way;
//! - Google OAuth: a fresh Authorization Code + PKCE flow that requests
//!   `max_age`, with signature, issuer, audience, expiry, and nonce checks
//!   through the library's [`JwksIdTokenVerifier`](better_auth::jwks::JwksIdTokenVerifier)
//!   plus an explicit `auth_time` freshness check the pinned library does not
//!   perform. A missing or stale `auth_time`, or a subject that is not the
//!   caller's linked Google account, fails closed.
//!
//! Better Auth audit (Task 2.1): the pinned revision (`c694051`) provides
//! reusable password re-verification (`verify_credentials`), one-time OAuth
//! state consumption (`OAuthStateManager::consume`), PKCE generation
//! (`PkcePair`), code exchange (`OAuthProvider::exchange_code`), and JWKS
//! ID-token validation (`JwksIdTokenVerifier`, `OidcIdTokenValidator`). It
//! does not provide `max_age` authorization parameters, `auth_time` claim
//! validation, or a session-bound grant primitive, so those live here,
//! against the same guarantees, instead of in a forked authentication
//! system. A future upstream step-up API could replace the grant tables and
//! the `auth_time` extraction below; the endpoint contract would not change.
//!
//! Grants bind user ID, session ID, action, and target resource, live at most
//! five minutes, and are consumed atomically with the protected mutation in
//! the same database transaction. A grant never leaves the server except as
//! an opaque identifier the client echoes back; nothing is kept in
//! localStorage or sessionStorage, there is no privileged-session flag, and
//! there is no environment bypass. Passwords are never logged and no
//! password-derived material is stored outside Better Auth's own tables.
use crate::AppState;
use axum::{
    extract::{Query, State},
    http::{header::HeaderName, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use better_auth::{
    email_password::{EmailPasswordService, SignInInput},
    jwks::JwksIdTokenVerifier,
    oauth::OAuthProvider,
    rate_limit::{RateLimitPolicy, RateLimiter},
    HttpOAuthProvider,
};
use better_auth_core::error::AuthError;
use serde::{Deserialize, Serialize};
use sqlx::{Postgres, Row, Transaction};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use url::Url;
use uuid::Uuid;

pub const ACTION_ENROLL: &str = "publisher.key.enroll";
pub const ACTION_APPROVE: &str = "publisher.key.approve";
/// Maximum lifetime of a minted grant. Verification must be this recent when
/// the protected mutation executes.
pub const GRANT_TTL_SECS: i64 = 300;
/// Lifetime of a pending OAuth step-up flow (state, nonce, PKCE verifier).
pub const OAUTH_STATE_TTL_SECS: i64 = 600;
/// Requested (and enforced) maximum age of the provider authentication, in
/// seconds. Sent as OIDC `max_age` and enforced against `auth_time`.
pub const MAX_AUTH_AGE_SECS: u64 = 300;
/// Clock-skew allowance for `auth_time` values slightly in the future.
const AUTH_TIME_SKEW_SECS: u64 = 60;
/// Verification attempts per user per minute for each step-up entry point.
const VERIFY_LIMIT_PER_MINUTE: i64 = 10;
/// Grant presentations per user per minute on the protected key endpoints.
const USE_LIMIT_PER_MINUTE: i64 = 30;
/// Upper bound on a submitted password; matches account deletion's bound so a
/// single request cannot force unbounded hashing work.
const MAX_PASSWORD_LEN: usize = 1024;

fn db_error() -> (StatusCode, Json<serde_json::Value>) {
    tracing::error!("step-up operation failed");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({ "error": "internal_error" })),
    )
}

fn deny(status: StatusCode, code: &str) -> (StatusCode, Json<serde_json::Value>) {
    (status, Json(serde_json::json!({ "error": code })))
}

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Google (or fixture) OAuth material for the step-up flow. The exchange
/// provider and the ID-token verifier share the same client ID and issuer so
/// a token minted for another client or issuer never validates.
#[derive(Clone)]
pub struct StepUpOAuth {
    provider_id: String,
    client_id: String,
    authorization_endpoint: Url,
    redirect_uri: String,
    exchange: std::sync::Arc<HttpOAuthProvider>,
    verifier: JwksIdTokenVerifier,
}

impl StepUpOAuth {
    /// Production constructor for Google. Fetches Google's signing keys now so
    /// a broken or unreachable JWKS endpoint fails startup closed instead of
    /// failing the first verification open.
    pub async fn google(
        client_id: String,
        client_secret: String,
        origin: &str,
    ) -> Result<Self, String> {
        let exchange = HttpOAuthProvider::google_with_jwks(&client_id, &client_secret)
            .await
            .map_err(|e| format!("google step-up provider: {e}"))?;
        let verifier = JwksIdTokenVerifier::google(&client_id)
            .map_err(|e| format!("google step-up verifier: {e}"))?;
        let loaded = verifier
            .load()
            .await
            .map_err(|e| format!("google step-up JWKS load: {e}"))?;
        if loaded == 0 {
            return Err("google step-up JWKS load returned no keys".into());
        }
        Ok(Self {
            provider_id: "google".into(),
            client_id,
            authorization_endpoint: Url::parse("https://accounts.google.com/o/oauth2/v2/auth")
                .map_err(|e| e.to_string())?,
            redirect_uri: format!("{origin}/api/v1/step-up/oauth/callback"),
            exchange: std::sync::Arc::new(exchange),
            verifier,
        })
    }

    /// Test/fixture constructor. The caller supplies an insecure fixture
    /// provider and verifier pointing at localhost fixtures; production code
    /// never builds this path.
    #[cfg(test)]
    pub fn for_testing(
        provider_id: &str,
        client_id: &str,
        authorization_endpoint: Url,
        redirect_uri: String,
        exchange: HttpOAuthProvider,
        verifier: JwksIdTokenVerifier,
    ) -> Self {
        Self {
            provider_id: provider_id.into(),
            client_id: client_id.into(),
            authorization_endpoint,
            redirect_uri,
            exchange: std::sync::Arc::new(exchange),
            verifier,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PasswordRequest {
    pub action: String,
    pub publisher_id: Uuid,
    #[serde(default)]
    pub key_id: Option<Uuid>,
    pub password: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrantResponse {
    pub grant_id: Uuid,
    pub expires_in: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OAuthStartRequest {
    pub action: String,
    pub publisher_id: Uuid,
    #[serde(default)]
    pub key_id: Option<Uuid>,
    pub return_to: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OAuthStartResponse {
    pub authorization_url: String,
    pub expires_in: u32,
}

/// Google appends its own parameters (`scope`, `authuser`, `prompt`, ...)
// to the callback, so unknown fields must be tolerated here.
#[derive(Deserialize)]
pub struct CallbackQuery {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GrantBody {
    pub step_up_grant: Uuid,
}

/// Validated (action, publisher, key) triple. Approval always names the key
/// record; enrollment never does, so a grant minted for one cannot authorize
/// the other.
pub(crate) struct Target {
    pub action: &'static str,
    pub publisher_id: Uuid,
    pub key_id: Option<Uuid>,
}

pub(crate) fn validate_target(
    action: &str,
    publisher_id: Uuid,
    key_id: Option<Uuid>,
) -> Result<Target, ()> {
    let canonical = match action {
        a if a == ACTION_ENROLL => ACTION_ENROLL,
        a if a == ACTION_APPROVE => ACTION_APPROVE,
        _ => return Err(()),
    };
    match (canonical, key_id) {
        (ACTION_ENROLL, None) => Ok(Target {
            action: canonical,
            publisher_id,
            key_id: None,
        }),
        (ACTION_APPROVE, Some(key)) => Ok(Target {
            action: canonical,
            publisher_id,
            key_id: Some(key),
        }),
        _ => Err(()),
    }
}

/// The frontend return destination after the OAuth callback. Only an
/// in-application path; anything else (external URLs, protocol-relative
/// URLs, fragments that could smuggle the grant) is refused.
fn validate_return_to(value: &str) -> Result<&str, ()> {
    if value != "/app" && !value.starts_with("/app/") && !value.starts_with("/app?") {
        return Err(());
    }
    if value.contains("://") || value.starts_with("//") || value.contains(['#', '\\', '\r', '\n']) {
        return Err(());
    }
    Ok(value)
}

/// Rate-limit grant presentations on the protected key endpoints.
pub(crate) async fn limit_grant_use(
    state: &AppState,
    user_id: &str,
) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    limit(state, "step-up-use", USE_LIMIT_PER_MINUTE, user_id).await
}

async fn limit(
    state: &AppState,
    policy_name: &str,
    limit: i64,
    key: &str,
) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    let backend = state.auth.secondary_storage.clone().ok_or_else(db_error)?;
    let policy = RateLimitPolicy::new(policy_name, limit, Duration::from_secs(60))
        .map_err(|_| db_error())?;
    RateLimiter::new(backend, "cloud:step-up")
        .enforce(&policy, key)
        .await
        .map_err(|error| match error {
            AuthError::RateLimited { .. } => deny(StatusCode::TOO_MANY_REQUESTS, "rate_limited"),
            _ => db_error(),
        })
}

async fn audit(
    tx: &mut Transaction<'_, Postgres>,
    actor: &str,
    publisher: Uuid,
    grant: Option<Uuid>,
    action: &str,
) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    sqlx::query("INSERT INTO cloud_audit_events (id,actor_user_id,publisher_id,subject_id,action) VALUES ($1,$2,$3,$4,$5)")
        .bind(Uuid::new_v4()).bind(actor).bind(publisher).bind(grant).bind(action).execute(&mut **tx).await.map_err(|_| db_error())?;
    Ok(())
}

async fn mint_grant(
    tx: &mut Transaction<'_, Postgres>,
    user_id: &str,
    session_id: &str,
    target: &Target,
    method: &str,
) -> Result<Uuid, (StatusCode, Json<serde_json::Value>)> {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO cloud_step_up_grants (id,user_id,session_id,action,publisher_id,key_id,method,expires_at) VALUES ($1,$2,$3,$4,$5,$6,$7,now()+make_interval(secs=>$8))")
        .bind(id).bind(user_id).bind(session_id).bind(target.action).bind(target.publisher_id).bind(target.key_id).bind(method).bind(GRANT_TTL_SECS as f64)
        .execute(&mut **tx).await.map_err(|_| db_error())?;
    Ok(id)
}

async fn publisher_exists(
    pool: &sqlx::PgPool,
    id: Uuid,
) -> Result<bool, (StatusCode, Json<serde_json::Value>)> {
    sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM cloud_publishers WHERE id=$1)")
        .bind(id)
        .fetch_one(pool)
        .await
        .map_err(|_| db_error())
}

/// Consume one grant atomically: user, session, action, and target must match
/// an unconsumed, unexpired grant, and the session must still be valid. The
/// session row is rechecked here (Better Auth resolved it for this request,
/// but a logout, expiry, or account deletion landing just before the mutation
/// must still revoke the grant). A concurrent second submission finds the row
/// already consumed and fails.
pub async fn consume_grant(
    tx: &mut Transaction<'_, Postgres>,
    grant_id: Uuid,
    user_id: &str,
    session_id: &str,
    target: &Target,
) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    let now = now_seconds() as i64;
    let consumed: Option<Uuid> = sqlx::query_scalar(
        "UPDATE cloud_step_up_grants SET consumed_at=now() WHERE id=$1 AND consumed_at IS NULL AND expires_at>now() AND user_id=$2 AND session_id=$3 AND action=$4 AND publisher_id=$5 AND ((key_id=$6) OR (key_id IS NULL AND $6 IS NULL)) AND EXISTS (SELECT 1 FROM session WHERE id=$3 AND user_id=$2 AND expires_at>$7) RETURNING id",
    )
    .bind(grant_id)
    .bind(user_id)
    .bind(session_id)
    .bind(target.action)
    .bind(target.publisher_id)
    .bind(target.key_id)
    .bind(now)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|_| db_error())?;
    if consumed.is_none() {
        return Err(grant_failure_reason(tx, grant_id, user_id).await?);
    }
    Ok(())
}

/// Map a failed consumption to a generic answer: expired or already-consumed
/// grants are distinguishable (the client must re-verify), while anything
/// else stays a plain requirement so one session cannot probe another's
/// grants.
async fn grant_failure_reason(
    tx: &mut Transaction<'_, Postgres>,
    grant_id: Uuid,
    user_id: &str,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, Json<serde_json::Value>)> {
    let row = sqlx::query("SELECT user_id, expires_at<=now() AS expired, consumed_at IS NOT NULL AS consumed FROM cloud_step_up_grants WHERE id=$1")
        .bind(grant_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|_| db_error())?;
    match row {
        Some(row)
            if row.get::<String, _>("user_id") == user_id
                && (row.get::<bool, _>("expired") || row.get::<bool, _>("consumed")) =>
        {
            Err(deny(StatusCode::FORBIDDEN, "step_up_expired"))
        }
        _ => Err(deny(StatusCode::FORBIDDEN, "step_up_required")),
    }
}

/// Verify the caller's password through Better Auth and mint an
/// action-scoped grant. Wrong passwords, unknown credentials, and missing
/// password accounts all fail without revealing which case applied beyond
/// the `password_unavailable` signal the caller's own client needs to offer
/// the Google path. The password value is never logged.
pub async fn password(
    better_auth::axum_integration::AuthUser(principal): better_auth::axum_integration::AuthUser,
    State(state): State<AppState>,
    Json(input): Json<PasswordRequest>,
) -> Result<(StatusCode, Json<GrantResponse>), (StatusCode, Json<serde_json::Value>)> {
    limit(
        &state,
        "step-up-password",
        VERIFY_LIMIT_PER_MINUTE,
        &principal.user.id,
    )
    .await?;
    let target = validate_target(&input.action, input.publisher_id, input.key_id)
        .map_err(|()| deny(StatusCode::BAD_REQUEST, "invalid_target"))?;
    if !publisher_exists(&state.pool, target.publisher_id).await? {
        return Err(deny(StatusCode::NOT_FOUND, "unknown_publisher"));
    }
    if input.password.is_empty() || input.password.len() > MAX_PASSWORD_LEN {
        return Err(deny(StatusCode::BAD_REQUEST, "invalid_credentials"));
    }
    // Social-only accounts have no password credential to re-check; they must
    // use the OAuth path. This only describes the caller's own account.
    let credential: Option<String> = sqlx::query_scalar("SELECT id FROM account WHERE id=$1")
        .bind(format!("{}:credential", principal.user.id))
        .fetch_optional(&state.pool)
        .await
        .map_err(|_| db_error())?;
    if credential.is_none() {
        let mut tx = state.pool.begin().await.map_err(|_| db_error())?;
        audit(
            &mut tx,
            &principal.user.id,
            target.publisher_id,
            None,
            "step_up.failed",
        )
        .await?;
        tx.commit().await.map_err(|_| db_error())?;
        return Err(deny(StatusCode::FORBIDDEN, "password_unavailable"));
    }
    let service = EmailPasswordService::new(state.auth.clone()).map_err(|_| db_error())?;
    let verified = service
        .verify_credentials(SignInInput {
            email: principal.user.email.clone(),
            password: input.password,
        })
        .await;
    // `input.password` is moved into the verification call above and is never
    // logged, stored, or included in an error or audit row.
    if verified.is_err() {
        let mut tx = state.pool.begin().await.map_err(|_| db_error())?;
        audit(
            &mut tx,
            &principal.user.id,
            target.publisher_id,
            None,
            "step_up.failed",
        )
        .await?;
        tx.commit().await.map_err(|_| db_error())?;
        return Err(deny(StatusCode::UNAUTHORIZED, "invalid_credentials"));
    }
    let mut tx = state.pool.begin().await.map_err(|_| db_error())?;
    let grant = mint_grant(
        &mut tx,
        &principal.user.id,
        &principal.session.id,
        &target,
        "password",
    )
    .await?;
    audit(
        &mut tx,
        &principal.user.id,
        target.publisher_id,
        Some(grant),
        "step_up.granted",
    )
    .await?;
    tx.commit().await.map_err(|_| db_error())?;
    Ok((
        StatusCode::OK,
        Json(GrantResponse {
            grant_id: grant,
            expires_in: GRANT_TTL_SECS as u32,
        }),
    ))
}

/// Begin a fresh Google step-up flow: store a one-time state/nonce/PKCE
/// record bound to the caller and session, and return the authorization URL.
/// The provider authentication must satisfy `max_age`; mere possession of a
/// session, a verified email, or a refreshed token proves nothing here.
pub async fn oauth_start(
    better_auth::axum_integration::AuthUser(principal): better_auth::axum_integration::AuthUser,
    State(state): State<AppState>,
    Json(input): Json<OAuthStartRequest>,
) -> Result<(StatusCode, Json<OAuthStartResponse>), (StatusCode, Json<serde_json::Value>)> {
    limit(
        &state,
        "step-up-oauth-start",
        VERIFY_LIMIT_PER_MINUTE,
        &principal.user.id,
    )
    .await?;
    let target = validate_target(&input.action, input.publisher_id, input.key_id)
        .map_err(|()| deny(StatusCode::BAD_REQUEST, "invalid_target"))?;
    let return_to = validate_return_to(&input.return_to)
        .map_err(|()| deny(StatusCode::BAD_REQUEST, "invalid_return_to"))?;
    if !publisher_exists(&state.pool, target.publisher_id).await? {
        return Err(deny(StatusCode::NOT_FOUND, "unknown_publisher"));
    }
    let oauth = state
        .step_up_oauth
        .clone()
        .ok_or_else(|| deny(StatusCode::SERVICE_UNAVAILABLE, "oauth_unavailable"))?;
    let pair = better_auth::oauth::PkcePair::generate();
    let flow_state = Uuid::new_v4().simple().to_string();
    let nonce = Uuid::new_v4().simple().to_string();
    sqlx::query("INSERT INTO cloud_step_up_oauth_states (state,user_id,session_id,action,publisher_id,key_id,nonce,code_verifier,redirect_uri,return_to,expires_at) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,now()+make_interval(secs=>$11))")
        .bind(&flow_state).bind(&principal.user.id).bind(&principal.session.id).bind(target.action).bind(target.publisher_id).bind(target.key_id).bind(&nonce).bind(&pair.verifier).bind(&oauth.redirect_uri).bind(return_to).bind(OAUTH_STATE_TTL_SECS as f64)
        .execute(&state.pool).await.map_err(|_| db_error())?;
    let mut url = oauth.authorization_endpoint.clone();
    {
        let mut query = url.query_pairs_mut();
        query
            .append_pair("response_type", "code")
            .append_pair("client_id", &oauth.client_id)
            .append_pair("redirect_uri", &oauth.redirect_uri)
            .append_pair("scope", "openid email")
            .append_pair("state", &flow_state)
            .append_pair("nonce", &nonce)
            .append_pair("code_challenge", &pair.challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("max_age", &MAX_AUTH_AGE_SECS.to_string());
    }
    Ok((
        StatusCode::OK,
        Json(OAuthStartResponse {
            authorization_url: url.as_str().to_owned(),
            expires_in: OAUTH_STATE_TTL_SECS as u32,
        }),
    ))
}

/// Validate a grant without consuming it: used when issuing the
/// proof-of-possession challenge, which the single enrollment grant spans but
/// does not itself spend. The grant is spent when the key is registered.
pub async fn require_grant(
    tx: &mut Transaction<'_, Postgres>,
    grant_id: Uuid,
    user_id: &str,
    session_id: &str,
    target: &Target,
) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    let now = now_seconds() as i64;
    let valid: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM cloud_step_up_grants WHERE id=$1 AND consumed_at IS NULL AND expires_at>now() AND user_id=$2 AND session_id=$3 AND action=$4 AND publisher_id=$5 AND ((key_id=$6) OR (key_id IS NULL AND $6 IS NULL)) AND EXISTS (SELECT 1 FROM session WHERE id=$3 AND user_id=$2 AND expires_at>$7))",
    )
    .bind(grant_id)
    .bind(user_id)
    .bind(session_id)
    .bind(target.action)
    .bind(target.publisher_id)
    .bind(target.key_id)
    .bind(now)
    .fetch_one(&mut **tx)
    .await
    .map_err(|_| db_error())?;
    if !valid {
        return Err(grant_failure_reason(tx, grant_id, user_id).await?);
    }
    Ok(())
}

/// Read `auth_time` from an ID token whose signature, issuer, audience,
/// expiry, and nonce were already verified. The pinned library's claims type
/// does not carry `auth_time`, so it is extracted from the verified payload
/// here; anything unparsable fails closed.
fn extract_auth_time(id_token: &str) -> Option<u64> {
    let payload = id_token.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    value.get("auth_time")?.as_u64()
}

fn step_up_redirect(return_to: &str, param: &str, value: &str) -> Response {
    let separator = if return_to.contains('?') { '&' } else { '?' };
    let location = format!("{return_to}{separator}{param}={value}");
    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("location"),
        location
            .parse()
            .unwrap_or_else(|_| axum::http::HeaderValue::from_static("/app")),
    );
    headers.insert(
        HeaderName::from_static("cache-control"),
        axum::http::HeaderValue::from_static("no-store"),
    );
    (StatusCode::SEE_OTHER, headers, String::new()).into_response()
}

/// Complete the Google step-up flow. The pending state is consumed before any
/// network call so a captured callback URL cannot be replayed; the code is
/// then exchanged for the exact stored redirect URI, and the ID token must
/// prove a fresh provider authentication (`auth_time` within `max_age`) for
/// the caller's own linked account. Success redirects to the stored
/// application path with a grant; every failure redirects with an opaque
/// error code instead.
pub async fn oauth_callback(
    better_auth::axum_integration::AuthUser(principal): better_auth::axum_integration::AuthUser,
    State(state): State<AppState>,
    Query(query): Query<CallbackQuery>,
) -> Response {
    // Without a session there is nothing to bind a grant to.
    if limit(
        &state,
        "step-up-oauth-callback",
        VERIFY_LIMIT_PER_MINUTE,
        &principal.user.id,
    )
    .await
    .is_err()
    {
        return deny(StatusCode::TOO_MANY_REQUESTS, "rate_limited").into_response();
    }
    let Some(oauth) = state.step_up_oauth.clone() else {
        return deny(StatusCode::SERVICE_UNAVAILABLE, "oauth_unavailable").into_response();
    };
    let Some(flow_state) = query.state.clone() else {
        return deny(StatusCode::BAD_REQUEST, "invalid_callback").into_response();
    };
    // Load the pending flow first: provider errors only become redirects once
    // the stored application return path is known.
    let mut tx = match state.pool.begin().await {
        Ok(tx) => tx,
        Err(_) => return db_error().into_response(),
    };
    let pending = match sqlx::query("SELECT user_id,session_id,action,publisher_id,key_id,nonce,code_verifier,redirect_uri,return_to,expires_at<=now() AS expired,consumed_at IS NOT NULL AS consumed FROM cloud_step_up_oauth_states WHERE state=$1 FOR UPDATE")
        .bind(&flow_state).fetch_optional(&mut *tx).await
    {
        Ok(row) => row,
        Err(_) => return db_error().into_response(),
    };
    let Some(row) = pending else {
        let _ = tx.rollback().await;
        return deny(StatusCode::BAD_REQUEST, "invalid_callback").into_response();
    };
    let stored_user: String = row.get("user_id");
    let stored_session: String = row.get("session_id");
    let action: String = row.get("action");
    let publisher_id: Uuid = row.get("publisher_id");
    let key_id: Option<Uuid> = row.get("key_id");
    let nonce: String = row.get("nonce");
    let verifier: String = row.get("code_verifier");
    let redirect_uri: String = row.get("redirect_uri");
    let return_to: String = row.get("return_to");
    let expired: bool = row.get("expired");
    let consumed: bool = row.get("consumed");
    if query.error.is_some() {
        sqlx::query("UPDATE cloud_step_up_oauth_states SET consumed_at=now() WHERE state=$1")
            .bind(&flow_state)
            .execute(&mut *tx)
            .await
            .ok();
        let _ = audit(&mut tx, &stored_user, publisher_id, None, "step_up.failed").await;
        let _ = tx.commit().await;
        return step_up_redirect(&return_to, "stepUpError", "provider_denied");
    }
    let Some(code) = query.code.clone() else {
        let _ = tx.rollback().await;
        return step_up_redirect(&return_to, "stepUpError", "invalid_callback");
    };
    if stored_user != principal.user.id
        || stored_session != principal.session.id
        || expired
        || consumed
    {
        let _ = tx.rollback().await;
        let code = if consumed || expired {
            "step_up_expired"
        } else {
            "invalid_callback"
        };
        return step_up_redirect(&return_to, "stepUpError", code);
    }
    // One-time consumption commits before the exchange and validation
    // below, so only the first callback presentation can proceed; a replayed
    // callback lands on `consumed` above.
    if sqlx::query("UPDATE cloud_step_up_oauth_states SET consumed_at=now() WHERE state=$1 AND consumed_at IS NULL")
        .bind(&flow_state)
        .execute(&mut *tx)
        .await
        .is_err()
        || tx.commit().await.is_err()
    {
        return db_error().into_response();
    }
    let fail = |code: &'static str| {
        let state = &state;
        let stored_user = stored_user.clone();
        let return_to = return_to.clone();
        async move {
            let mut tx = state.pool.begin().await.map_err(|_| db_error())?;
            audit(&mut tx, &stored_user, publisher_id, None, "step_up.failed").await?;
            tx.commit().await.map_err(|_| db_error())?;
            Ok::<Response, (StatusCode, Json<serde_json::Value>)>(step_up_redirect(
                &return_to,
                "stepUpError",
                code,
            ))
        }
    };
    if code.is_empty() || code.len() > 4096 {
        return fail("invalid_callback")
            .await
            .unwrap_or_else(|e| e.into_response());
    }
    let tokens = match oauth
        .exchange
        .exchange_code(&code, &redirect_uri, &verifier)
        .await
    {
        Ok(tokens) => tokens,
        Err(_) => {
            return fail("token_exchange_failed")
                .await
                .unwrap_or_else(|e| e.into_response());
        }
    };
    let Some(id_token) = tokens.id_token.as_deref() else {
        return fail("invalid_token")
            .await
            .unwrap_or_else(|e| e.into_response());
    };
    let claims = match oauth.verifier.validate(id_token, Some(&nonce)).await {
        Ok(claims) => claims,
        Err(_) => {
            return fail("invalid_token")
                .await
                .unwrap_or_else(|e| e.into_response());
        }
    };
    // `auth_time` proves the provider actively authenticated the user within
    // `max_age`. Without it (or when stale) there is no recent
    // authentication, no matter what the email or session claims.
    let fresh = extract_auth_time(id_token).is_some_and(|auth_time| {
        let now = now_seconds();
        auth_time <= now + AUTH_TIME_SKEW_SECS && now.saturating_sub(auth_time) <= MAX_AUTH_AGE_SECS
    });
    if !fresh {
        return fail("stale_authentication")
            .await
            .unwrap_or_else(|e| e.into_response());
    }
    // The token subject must be the caller's own linked provider account, not
    // merely an account the caller can sign into at the provider.
    let linked: Vec<String> = match sqlx::query_scalar(
        "SELECT account_id FROM account WHERE user_id=$1 AND provider_id=$2",
    )
    .bind(&principal.user.id)
    .bind(&oauth.provider_id)
    .fetch_all(&state.pool)
    .await
    {
        Ok(rows) => rows,
        Err(_) => return db_error().into_response(),
    };
    if !linked.iter().any(|subject| subject == &claims.sub) {
        return fail("account_mismatch")
            .await
            .unwrap_or_else(|e| e.into_response());
    }
    let target = match validate_target(&action, publisher_id, key_id) {
        Ok(target) => target,
        Err(()) => {
            return fail("invalid_callback")
                .await
                .unwrap_or_else(|e| e.into_response())
        }
    };
    let mut tx = match state.pool.begin().await {
        Ok(tx) => tx,
        Err(_) => return db_error().into_response(),
    };
    let grant = match mint_grant(
        &mut tx,
        &principal.user.id,
        &principal.session.id,
        &target,
        "oauth",
    )
    .await
    {
        Ok(grant) => grant,
        Err(e) => {
            let _ = tx.rollback().await;
            return e.into_response();
        }
    };
    if audit(
        &mut tx,
        &principal.user.id,
        target.publisher_id,
        Some(grant),
        "step_up.granted",
    )
    .await
    .is_err()
    {
        let _ = tx.rollback().await;
        return db_error().into_response();
    }
    if tx.commit().await.is_err() {
        return db_error().into_response();
    }
    step_up_redirect(&return_to, "stepUpGrant", &grant.to_string())
}
