//! Optional same-origin hosting of the built administration interface.
use axum::{
    extract::Request,
    http::{header, HeaderValue},
    middleware::{self, Next},
    response::Response,
    Router,
};
use std::{io, path::Path};
use tower_http::services::{ServeDir, ServeFile};

pub fn mount(router: Router, dist: &Path, hsts: bool) -> io::Result<Router> {
    let index = dist.join("index.html");
    if !index.is_file() || !dist.join("assets").is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "ADMIN_DIST_DIR must contain index.html and assets/",
        ));
    }
    // Register only UI paths: unknown API and auth endpoints must remain 404.
    let ui = Router::new()
        .route_service("/app", ServeFile::new(&index))
        .route_service("/app/{*path}", ServeFile::new(&index))
        .route_service("/login", ServeFile::new(&index))
        .route_service("/logout", ServeFile::new(&index))
        .route_service("/403", ServeFile::new(&index))
        .route_service("/auth/callback", ServeFile::new(&index))
        .nest_service("/assets", ServeDir::new(dist.join("assets")))
        .layer(middleware::from_fn(no_store))
        .layer(middleware::from_fn_with_state(
            hsts,
            crate::security_headers::apply,
        ));
    Ok(router.merge(ui))
}

async fn no_store(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    // These hashes cover the theme bootstrap script and style in index.html.
    // Keep the CSP in sync with that file when modifying the bootstrap.
    response.headers_mut().insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static("default-src 'none'; script-src 'self' 'sha256-s183EwDi/ZvUIL8bQcxVOuNRoXsPqlj8tuSYCYP5tgk='; style-src 'self' 'sha256-xkdVNFlc5CUuULQ3WqPWX1DyExQKr2xLsmTWkVJ3Pmg='; img-src 'self' data:; connect-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'"));
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    #[test]
    fn csp_hashes_match_admin_bootstrap() {
        use base64::{engine::general_purpose::STANDARD, Engine};
        use sha2::{Digest, Sha256};
        let html = include_str!("../../../web/admin/index.html");
        for (tag, expected) in [
            ("script", "s183EwDi/ZvUIL8bQcxVOuNRoXsPqlj8tuSYCYP5tgk="),
            ("style", "xkdVNFlc5CUuULQ3WqPWX1DyExQKr2xLsmTWkVJ3Pmg="),
        ] {
            let start = format!("<{tag}>");
            let end = format!("</{tag}>");
            let body = html
                .split_once(&start)
                .unwrap()
                .1
                .split_once(&end)
                .unwrap()
                .0;
            assert_eq!(STANDARD.encode(Sha256::digest(body.as_bytes())), expected);
        }
    }

    #[tokio::test]
    async fn serves_ui_without_capturing_unknown_api_routes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("assets")).unwrap();
        std::fs::write(dir.path().join("index.html"), "admin").unwrap();
        std::fs::write(dir.path().join("assets/app.js"), "script").unwrap();
        let app = mount(Router::new(), dir.path(), false).unwrap();
        for (path, expected) in [
            ("/app/portal", StatusCode::OK),
            ("/assets/app.js", StatusCode::OK),
            ("/api/v1/missing", StatusCode::NOT_FOUND),
            ("/assets/missing.js", StatusCode::NOT_FOUND),
        ] {
            let response = app
                .clone()
                .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), expected, "{path}");
        }
    }
}
