//! Error envelope and request correlation for the v3 product API.
//!
//! Every v3 response that fails carries the same JSON object: a stable `code`,
//! the backend-generated `requestId`, a `messageKey` for the client's copy, and
//! whether a retry is meaningful. `retryable` never authorizes repeating a
//! non-idempotent mutation; callers must consult the receipt first.

use axum::{
    extract::Request,
    http::{HeaderValue, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;

pub const REQUEST_ID_HEADER: &str = "x-request-id";

/// Identifier generated at the backend edge. Client-supplied identifiers are
/// ignored so a caller cannot forge or collide correlation values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestId(pub String);

impl RequestId {
    pub fn fresh() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }
}

/// Stable, documented failure classes for v3 endpoints. `Offline`,
/// `Misconfigured` and `RateLimited` form part of the published vocabulary even
/// where no handler emits them yet, so clients can branch on them safely.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    InvalidRequest,
    Unauthorized,
    Forbidden,
    NotFound,
    Conflict,
    Expired,
    Unsupported,
    Offline,
    Misconfigured,
    RateLimited,
    QuotaExceeded,
    TemporaryFailure,
}

impl Kind {
    pub fn code(self) -> &'static str {
        match self {
            Kind::InvalidRequest => "invalidRequest",
            Kind::Unauthorized => "unauthorized",
            Kind::Forbidden => "forbidden",
            Kind::NotFound => "notFound",
            Kind::Conflict => "conflict",
            Kind::Expired => "expired",
            Kind::Unsupported => "unsupported",
            Kind::Offline => "offline",
            Kind::Misconfigured => "misconfigured",
            Kind::RateLimited => "rateLimited",
            Kind::QuotaExceeded => "quotaExceeded",
            Kind::TemporaryFailure => "temporaryFailure",
        }
    }

    pub fn status(self) -> StatusCode {
        match self {
            Kind::InvalidRequest => StatusCode::BAD_REQUEST,
            Kind::Unauthorized => StatusCode::UNAUTHORIZED,
            Kind::Forbidden => StatusCode::FORBIDDEN,
            Kind::NotFound => StatusCode::NOT_FOUND,
            Kind::Conflict => StatusCode::CONFLICT,
            Kind::Expired => StatusCode::GONE,
            Kind::Unsupported => StatusCode::NOT_IMPLEMENTED,
            Kind::Offline => StatusCode::SERVICE_UNAVAILABLE,
            Kind::Misconfigured => StatusCode::SERVICE_UNAVAILABLE,
            Kind::RateLimited => StatusCode::TOO_MANY_REQUESTS,
            Kind::QuotaExceeded => StatusCode::INSUFFICIENT_STORAGE,
            Kind::TemporaryFailure => StatusCode::SERVICE_UNAVAILABLE,
        }
    }

    /// Whether repeating the same request may succeed without operator action.
    /// Mutations still require checking their receipts first.
    pub fn retryable(self) -> bool {
        matches!(
            self,
            Kind::Offline | Kind::RateLimited | Kind::TemporaryFailure
        )
    }

    /// Message identifier the clients translate; the English copy lives in the
    /// client catalogs, never in the server response.
    pub fn message_key(self) -> &'static str {
        match self {
            Kind::InvalidRequest => "api.invalidRequest",
            Kind::Unauthorized => "api.unauthorized",
            Kind::Forbidden => "api.forbidden",
            Kind::NotFound => "api.notFound",
            Kind::Conflict => "api.conflict",
            Kind::Expired => "api.expired",
            Kind::Unsupported => "api.unsupported",
            Kind::Offline => "api.offline",
            Kind::Misconfigured => "api.misconfigured",
            Kind::RateLimited => "api.rateLimited",
            Kind::QuotaExceeded => "api.quotaExceeded",
            Kind::TemporaryFailure => "api.temporaryFailure",
        }
    }
}

/// Wire shape of a failure. Field names are part of the v3 contract.
#[derive(Serialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ApiError {
    pub code: &'static str,
    pub request_id: String,
    pub message_key: &'static str,
    pub retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
}

/// A failure bound to the request that produced it.
#[derive(Debug)]
pub struct V3Error {
    kind: Kind,
    request_id: String,
    retry_after_ms: Option<u64>,
}

impl V3Error {
    pub fn new(request_id: &RequestId, kind: Kind) -> Self {
        Self {
            kind,
            request_id: request_id.0.clone(),
            retry_after_ms: None,
        }
    }

    pub fn body(&self) -> ApiError {
        ApiError {
            code: self.kind.code(),
            request_id: self.request_id.clone(),
            message_key: self.kind.message_key(),
            retryable: self.kind.retryable(),
            retry_after_ms: self.retry_after_ms,
        }
    }
}

impl IntoResponse for V3Error {
    fn into_response(self) -> Response {
        let status = self.kind.status();
        let body = self.body();
        let mut response = (status, Json(body)).into_response();
        if let Ok(value) = HeaderValue::from_str(&self.request_id) {
            response.headers_mut().insert(REQUEST_ID_HEADER, value);
        }
        response
    }
}

/// Assigns a fresh request id to every v3 request and echoes it in the header.
pub async fn request_id(mut request: Request, next: Next) -> Response {
    let id = RequestId::fresh();
    let header = HeaderValue::from_str(&id.0).ok();
    request.extensions_mut().insert(id);
    let mut response = next.run(request).await;
    if let Some(header) = header {
        response.headers_mut().insert(REQUEST_ID_HEADER, header);
    }
    response
}

/// Logs a database failure without its text, which may contain query fragments.
pub fn database(request_id: &RequestId, error: sqlx::Error) -> V3Error {
    tracing::error!(request_id = %request_id.0, kind = %error_kind(&error), "v3 database operation failed");
    V3Error::new(request_id, Kind::TemporaryFailure)
}

fn error_kind(error: &sqlx::Error) -> &'static str {
    match error {
        sqlx::Error::RowNotFound => "row_not_found",
        sqlx::Error::PoolTimedOut => "pool_timeout",
        sqlx::Error::Database(_) => "database",
        _ => "other",
    }
}

/// Authenticated principal for v3 routes. Failures use the v3 envelope and the
/// request id generated by [`request_id`], so every rejection is correlatable.
pub struct Authenticated(pub better_auth::session::AuthPrincipal);

impl<S> axum::extract::FromRequestParts<S> for Authenticated
where
    S: Send + Sync,
    better_auth::AuthContext: axum::extract::FromRef<S>,
{
    type Rejection = V3Error;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &S,
    ) -> Result<Self, Self::Rejection> {
        let request_id = parts
            .extensions
            .get::<RequestId>()
            .cloned()
            .unwrap_or_else(RequestId::fresh);
        crate::dual_auth::DualUser::from_request_parts(parts, state)
            .await
            .map(|user| Self(user.0))
            .map_err(|_| V3Error::new(&request_id, Kind::Unauthorized))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_uses_documented_names_and_omits_absent_retry_hint() {
        let id = RequestId("req-1".into());
        let body = V3Error::new(&id, Kind::Offline).body();
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["code"], "offline");
        assert_eq!(json["requestId"], "req-1");
        assert_eq!(json["messageKey"], "api.offline");
        assert_eq!(json["retryable"], true);
        assert!(json.get("retryAfterMs").is_none());
    }

    #[test]
    fn permanent_failures_are_not_retryable_and_map_to_distinct_statuses() {
        assert!(!Kind::Unsupported.retryable());
        assert!(!Kind::Conflict.retryable());
        assert!(!Kind::Unauthorized.retryable());
        assert_eq!(Kind::Unsupported.status(), StatusCode::NOT_IMPLEMENTED);
        assert_eq!(Kind::Expired.status(), StatusCode::GONE);
        assert_eq!(Kind::Conflict.status(), StatusCode::CONFLICT);
    }

    #[test]
    fn request_ids_are_generated_and_unique() {
        assert_ne!(RequestId::fresh(), RequestId::fresh());
    }
}
