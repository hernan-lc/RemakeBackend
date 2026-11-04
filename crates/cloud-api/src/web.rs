use axum::{
    http::{header, HeaderMap, HeaderValue},
    response::{Html, IntoResponse, Response},
};

fn security(mut response: Response) -> Response {
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static("default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'"));
    response
}
pub async fn page() -> Response {
    security(Html(include_str!("../web/dist/index.html")).into_response())
}
pub async fn script() -> Response {
    asset(
        "text/javascript; charset=utf-8",
        include_str!("../web/dist/login.js"),
    )
}
pub async fn style() -> Response {
    asset(
        "text/css; charset=utf-8",
        include_str!("../web/dist/login.css"),
    )
}
fn asset(content_type: &'static str, body: &'static str) -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    security((headers, body).into_response())
}
