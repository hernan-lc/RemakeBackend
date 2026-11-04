//! Additive security response headers.
//!
//! The API is designed to sit behind a TLS-terminating reverse proxy (see
//! `deploy/cloud-api-reverse-proxy.example.conf`), which owns HSTS, but the
//! application still sets its own baseline on every response. HSTS is only
//! emitted when the configured public origin uses HTTPS so loopback HTTP
//! development never tells a browser to require TLS.
use axum::{
    extract::{Request, State},
    http::HeaderValue,
    middleware::Next,
    response::Response,
};

/// Apply with `axum::middleware::from_fn_with_state(hsts, apply)`.
/// `hsts` must be true only when the public origin is HTTPS.
pub async fn apply(State(hsts): State<bool>, request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    headers.insert("x-frame-options", HeaderValue::from_static("DENY"));
    headers.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    headers.insert(
        "permissions-policy",
        HeaderValue::from_static("camera=(), microphone=(), geolocation=(), payment=()"),
    );
    if hsts {
        headers.insert(
            "strict-transport-security",
            HeaderValue::from_static("max-age=63072000; includeSubDomains"),
        );
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::{middleware, routing::get, Router};
    use tower::ServiceExt;

    async fn headers_for(hsts: bool) -> axum::http::HeaderMap {
        let app = Router::new()
            .route("/", get(|| async { "ok" }))
            .layer(middleware::from_fn_with_state(hsts, apply));
        let response = app
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        response.headers().clone()
    }

    #[tokio::test]
    async fn baseline_headers_are_always_present() {
        let headers = headers_for(false).await;
        assert_eq!(headers["x-content-type-options"], "nosniff");
        assert_eq!(headers["x-frame-options"], "DENY");
        assert_eq!(headers["referrer-policy"], "no-referrer");
        assert!(headers.contains_key("permissions-policy"));
        assert!(!headers.contains_key("strict-transport-security"));
    }

    #[tokio::test]
    async fn hsts_is_emitted_only_for_https_origins() {
        let headers = headers_for(true).await;
        assert_eq!(
            headers["strict-transport-security"],
            "max-age=63072000; includeSubDomains"
        );
    }
}
