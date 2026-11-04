//! Dual native-or-browser authentication for the publisher API surface.
//!
//! Native clients (the TikTools host) hold a Bearer session token and no
//! cookies; browser dashboard calls carry the session cookie. This extractor
//! accepts the Bearer token when an `Authorization` header is present and
//! otherwise falls back to the cookie flow with its existing CSRF
//! protections. It yields the same principal either way, so handlers stay
//! unchanged apart from the extractor type.
//!
//! Approval and administration routes intentionally stay cookie-only:
//! independent-admin approval remains a browser-bound dashboard action.

use axum::extract::{FromRef, FromRequestParts};
use axum::http::{header, request::Parts};
use better_auth::axum_integration::{AuthRejection, AuthUser, NativeAuthUser};
use better_auth::session::AuthPrincipal;
use better_auth::AuthContext;

/// Authenticated user through a native Bearer token or the browser cookie.
pub struct DualUser(pub AuthPrincipal);

impl<S> FromRequestParts<S> for DualUser
where
    S: Send + Sync,
    AuthContext: FromRef<S>,
{
    type Rejection = AuthRejection;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        if parts.headers.contains_key(header::AUTHORIZATION) {
            NativeAuthUser::from_request_parts(parts, state)
                .await
                .map(|native| Self(native.0))
        } else {
            AuthUser::from_request_parts(parts, state)
                .await
                .map(|cookie| Self(cookie.0))
        }
    }
}
