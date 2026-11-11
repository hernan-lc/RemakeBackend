//! Product API for one operator-configured cloud instance.
use crate::{dual_auth::DualUser, library, portable, workspace, AppState};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/v2/capabilities", get(capabilities))
        .route("/api/v2/catalog/plugins", get(plugins))
        .route("/api/v2/catalog/plugins/{id}", get(plugin))
        .route(
            "/api/v2/catalog/pagination-capabilities",
            get(crate::catalog_pages::capabilities),
        )
        .route(
            "/api/v2/catalog/plugin-summaries",
            get(crate::catalog_pages::summaries),
        )
        .route(
            "/api/v2/catalog/plugins/{id}/version-pages",
            get(crate::catalog_pages::versions),
        )
        .route("/api/v2/catalog/documents", get(library::catalog))
        .route("/api/v2/catalog/documents/{id}", get(library::published))
        .route(
            "/api/v2/catalog/documents/{id}/versions/{revision}",
            get(library::published_version),
        )
        .route(
            "/api/v2/library/documents",
            get(library::list).post(library::create),
        )
        .route(
            "/api/v2/library/documents/{id}",
            get(library::get)
                .put(library::update)
                .delete(library::delete),
        )
        .route(
            "/api/v2/library/documents/{id}/versions",
            get(library::versions),
        )
        .route(
            "/api/v2/library/documents/{id}/versions/{revision}",
            get(library::version),
        )
        .route(
            "/api/v2/library/documents/{id}/versions/{revision}/submit",
            post(library::submit),
        )
        .route(
            "/api/v2/library/documents/{id}/versions/{revision}/withdraw",
            post(library::withdraw),
        )
        .route("/api/v2/moderation/documents", get(library::queue))
        .route(
            "/api/v2/moderation/documents/{id}/versions/{revision}/decision",
            post(library::decide),
        )
        .route(
            "/api/v2/moderation/documents/{id}/versions/{revision}/withdraw",
            post(library::moderate_withdraw),
        )
        .route(
            "/api/v2/sync/collections/{collection}",
            get(workspace::list),
        )
        .route(
            "/api/v2/sync/collections/{collection}/{key}",
            get(workspace::get)
                .put(workspace::put)
                .delete(workspace::delete),
        )
        .route("/api/v2/sync/head", get(workspace::head))
        .route("/api/v2/sync/write-epoch", get(workspace::write_epoch))
        .route("/api/v2/sync/write-barrier", post(workspace::write_barrier))
        .route(
            "/api/v2/sync/require-write-epoch",
            post(workspace::require_write_epoch),
        )
        .route("/api/v2/sync/changes", get(workspace::changes))
        .route("/api/v2/sync/export", get(workspace::export))
        .route("/api/v2/me/export", get(export))
        .layer(axum::extract::DefaultBodyLimit::max(
            portable::MAX_DOCUMENT_BYTES + 1024,
        ))
        .layer(axum::middleware::map_response(crate::sync::sync_response))
}
async fn capabilities() -> Json<Value> {
    Json(json!({
        "apiVersion":2,"instanceMode":"single","userConfigurableRegistries":false,
        "catalog":{"plugins":"/api/v2/catalog/plugins","documents":"/api/v2/catalog/documents"},
        "library":{"documents":"/api/v2/library/documents","kinds":["template","profile"],"independentReview":true},
        "sync":{"basePath":"/api/v2/sync","schemaVersion":2,"collections":portable::COLLECTIONS,
            "maxRecords":workspace::MAX_RECORDS,"maxBytes":workspace::MAX_BYTES,"maxDocumentBytes":portable::MAX_DOCUMENT_BYTES,
            "conflictStatus":409,"expiredCursorStatus":410,"preconditionsRequired":true,
            "writeEpochBarrier":{"epochPath":"/api/v2/sync/write-epoch","barrierPath":"/api/v2/sync/write-barrier","header":"X-Workspace-Write-Epoch","staleStatus":412,
                "requireEpochPath":"/api/v2/sync/require-write-epoch","missingEpochStatus":428}},
        "registry":{"current":"/api/v1/registry/current","files":"/api/v1/registry/generations/{generation}/files/{path}","independentTrustPinsRequired":true},
        "authentication":{"providers":"/api/v1/auth/providers","nativeHandoff":"/api/v1/desktop/handoff","nativeRedeem":"/api/v1/native/handoff/redeem"},
        // v3 additions are additive: v2 clients ignore them and keep their semantics.
        "presentationCatalog":{"apiVersion":3,"itemsPath":"/api/v3/catalog/items","resolvePath":"/api/v3/catalog/items/resolve","kinds":["plugin","template","profile"],"available":true},
        "remoteControl":{"protocolVersion":crate::remote::PROTOCOL_VERSION,"supported":true,"available":true,"multiReplica":true,"hostsPath":"/api/v3/remote/hosts","devicesPath":"/api/v3/remote/devices","challengesPath":"/api/v3/remote/challenges","invitationsRedeemPath":"/api/v3/remote/invitations/redeem","ticketPath":"/api/v3/remote/tickets","socketPath":"/api/v3/remote/socket"},
        "integrations":{"providersPath":"/api/v3/integrations/providers","providers":crate::integrations::published_providers().len()},
        "media":{"assetsPath":"/api/v3/assets","formats":["image/png","image/jpeg","image/webp"],"maxBytes":crate::media::MAX_BYTES,"available":true},
        "overlays":{"apiVersion":3,"schemaVersion":crate::overlays::SCHEMA_VERSION,"kinds":crate::overlays::OVERLAY_KINDS,"path":"/api/v3/overlays","available":true},
        "packages":{"apiVersion":3,"packageVersion":crate::packages::PACKAGE_VERSION,"path":"/api/v3/packages","preflightSuffix":"/preflight","available":true}
    }))
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogQuery {
    q: Option<String>,
    after: Option<String>,
    generation: Option<i64>,
    target: Option<String>,
}
fn validate_query(q: &CatalogQuery) -> Result<(), StatusCode> {
    if q.q.as_ref().is_some_and(|s| s.len() > 100)
        || q.after
            .as_ref()
            .is_some_and(|s| !tiktools_plugin_api::manifest::is_valid_plugin_id(s))
        || q.generation.is_some_and(|n| n < 1)
        || q.target.as_ref().is_some_and(|s| {
            s.is_empty()
                || s.len() > 128
                || !s
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
        })
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    Ok(())
}
async fn catalog_data(
    state: &AppState,
    query: CatalogQuery,
    id: Option<String>,
) -> Result<Value, StatusCode> {
    validate_query(&query)?;
    if id
        .as_ref()
        .is_some_and(|id| !tiktools_plugin_api::manifest::is_valid_plugin_id(id))
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    let mut tx = state.pool.begin().await.map_err(workspace::db)?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
        .execute(&mut *tx)
        .await
        .map_err(workspace::db)?;
    let generation: Option<i64> = if let Some(generation) = query.generation {
        sqlx::query_scalar("SELECT generation FROM cloud_registry_generations WHERE generation=$1 AND status IN ('active','superseded','rolled_back')").bind(generation).fetch_optional(&mut *tx).await.map_err(workspace::db)?
    } else {
        sqlx::query_scalar(
            "SELECT generation FROM cloud_registry_generations WHERE status='active'",
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(workspace::db)?
    };
    let Some(generation) = generation else {
        if id.is_some() || query.generation.is_some() {
            return Err(StatusCode::NOT_FOUND);
        }
        return Ok(json!({"generation":null,"items":[],"nextCursor":null}));
    };
    let ids: Vec<String>=sqlx::query_scalar(r#"SELECT DISTINCT plugin_id COLLATE "C" AS plugin_id FROM cloud_registry_generation_releases WHERE generation=$1 AND ($2::text IS NULL OR plugin_id=$2) AND ($3::text IS NULL OR plugin_id COLLATE "C">$3) AND ($4::text IS NULL OR strpos(lower(plugin_id),lower($4))>0) AND ($5::text IS NULL OR target=$5 OR target='any') ORDER BY plugin_id COLLATE "C" LIMIT 51"#)
        .bind(generation).bind(&id).bind(query.after).bind(query.q).bind(&query.target).fetch_all(&mut *tx).await.map_err(workspace::db)?;
    if id.is_some() && ids.is_empty() {
        return Err(StatusCode::NOT_FOUND);
    }
    let more = ids.len() > 50;
    let ids: Vec<_> = ids.into_iter().take(50).collect();
    let rows=sqlx::query("SELECT g.plugin_id,g.version,g.target,g.sha256,g.size,(y.release_id IS NOT NULL) AS yanked,k.status AS key_status,p.status AS publisher_status,a.severity,a.message,a.url FROM cloud_registry_generation_releases g JOIN cloud_plugin_releases r ON r.id=g.release_id JOIN cloud_publishers p ON p.id=r.publisher_id JOIN cloud_publisher_keys k ON k.id=r.publisher_key_id LEFT JOIN cloud_release_yanks y ON y.release_id=r.id LEFT JOIN cloud_release_advisories a ON a.release_id=r.id WHERE g.generation=$1 AND g.plugin_id=ANY($2) AND ($3::text IS NULL OR g.target=$3 OR g.target='any') ORDER BY g.plugin_id,g.version,g.target")
        .bind(generation).bind(&ids).bind(query.target).fetch_all(&mut *tx).await.map_err(workspace::db)?;
    let mut items = Vec::new();
    for plugin_id in &ids {
        let mut versions: Vec<Value>=rows.iter().filter(|r|r.get::<String,_>("plugin_id")==*plugin_id).map(|r|{
            let severity: Option<String>=r.get("severity");
            json!({"version":r.get::<String,_>("version"),"target":r.get::<String,_>("target"),"sha256":r.get::<String,_>("sha256"),"size":r.get::<i64,_>("size"),"yanked":r.get::<bool,_>("yanked"),"keyRevoked":r.get::<String,_>("key_status")!="approved","publisherApproved":r.get::<String,_>("publisher_status")=="approved","advisory":severity.map(|severity|json!({"severity":severity,"message":r.get::<Option<String>,_>("message"),"url":r.get::<Option<String>,_>("url")}))})
        }).collect();
        versions.sort_by(|a, b| {
            semver::Version::parse(b["version"].as_str().unwrap())
                .ok()
                .cmp(&semver::Version::parse(a["version"].as_str().unwrap()).ok())
        });
        let latest = versions
            .iter()
            .find(|v| {
                v["yanked"] == false
                    && v["keyRevoked"] == false
                    && v["publisherApproved"] == true
                    && semver::Version::parse(v["version"].as_str().unwrap())
                        .is_ok_and(|v| v.pre.is_empty())
            })
            .map(|v| v["version"].clone());
        items.push(json!({"id":plugin_id,"latestStableVersion":latest,"metadataPath":format!("/api/v1/registry/generations/{generation}/files/{}",tiktools_plugin_registry::shard_path(plugin_id)),"versions":versions}));
    }
    tx.commit().await.map_err(workspace::db)?;
    if id.is_some() {
        return Ok(
            json!({"generation":generation,"plugin":items.pop().ok_or(StatusCode::NOT_FOUND)?}),
        );
    }
    Ok(json!({"generation":generation,"items":items,"nextCursor":if more {ids.last()} else {None}}))
}
async fn plugins(
    State(state): State<AppState>,
    Query(query): Query<CatalogQuery>,
) -> Result<Json<Value>, StatusCode> {
    Ok(Json(catalog_data(&state, query, None).await?))
}
async fn plugin(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<CatalogQuery>,
) -> Result<Json<Value>, StatusCode> {
    Ok(Json(catalog_data(&state, query, Some(id)).await?))
}

async fn export(
    DualUser(user): DualUser,
    State(state): State<AppState>,
) -> Result<Json<Value>, StatusCode> {
    workspace::limit(&state, &user.user.id, "export", 10).await?;
    // Each section is transactionally consistent; sections may reflect different
    // instants. This is a privacy export, not a cross-domain restore transaction.
    let legacy = crate::account::export_inner(user.clone(), state.clone())
        .await?
        .0;
    let workspace = workspace::snapshot(&state.pool, &user.user.id).await?;
    let library = library::export(&state.pool, &user.user.id).await?;
    let value =
        json!({"schemaVersion":2,"account":legacy,"workspaceSync":workspace,"library":library});
    if serde_json::to_vec(&value)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .len()
        > 32 * 1024 * 1024
    {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }
    Ok(Json(value))
}
