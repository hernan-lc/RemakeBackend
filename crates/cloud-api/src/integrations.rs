//! Managed integrations surface.
//!
//! Only adapters that are implemented and configured are published. The real
//! connectors audited in TikTools-app (OBS, RCON, audio, files) run on the host
//! PC, so they are reached through remote control rather than a backend adapter.
//! Until a backend adapter exists, the provider list is empty and every
//! connection or operation answers `unsupported` instead of pretending to work.

use crate::api_error::{Authenticated, Kind, RequestId, V3Error};
use crate::AppState;
use axum::{
    extract::{Path, State},
    routing::{delete, get, post},
    Extension, Json, Router,
};
use serde_json::{json, Value};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/v3/integrations/providers", get(providers))
        .route("/api/v3/integrations/connections", get(connections))
        .route("/api/v3/integrations/{provider_id}/connect", post(connect))
        .route(
            "/api/v3/integrations/connections/{connection_id}",
            delete(disconnect),
        )
        .route(
            "/api/v3/integrations/connections/{connection_id}/operations/{operation_id}",
            post(operation),
        )
}

/// Implemented, configured backend adapters. Empty until one exists.
pub const PUBLISHED_PROVIDERS: &[&str] = &[];

async fn providers() -> Json<Value> {
    Json(json!({"apiVersion": 3, "providers": []}))
}

async fn connections(Authenticated(_user): Authenticated) -> Json<Value> {
    Json(json!({"apiVersion": 3, "items": []}))
}

async fn connect(
    Extension(rid): Extension<RequestId>,
    Path(provider_id): Path<String>,
) -> Result<Json<Value>, V3Error> {
    let _ = provider_id;
    // Connecting requires a published adapter; none is configured for this build.
    Err(V3Error::new(&rid, Kind::Unsupported))
}

async fn disconnect(
    Authenticated(_user): Authenticated,
    Extension(rid): Extension<RequestId>,
    Path(_connection_id): Path<String>,
) -> Result<Json<Value>, V3Error> {
    Err(V3Error::new(&rid, Kind::NotFound))
}

async fn operation(
    Authenticated(_user): Authenticated,
    Extension(rid): Extension<RequestId>,
    State(_state): State<AppState>,
    Path((_connection_id, _operation_id)): Path<(String, String)>,
) -> Result<Json<Value>, V3Error> {
    Err(V3Error::new(&rid, Kind::Unsupported))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_provider_is_published_without_a_configured_adapter() {
        assert!(PUBLISHED_PROVIDERS.is_empty());
    }
}
