mod account;
mod admin;
mod config;
mod desktop;
mod moderation;
mod publication;
mod publication_worker;
mod publisher_keys;
mod publishers;
mod readiness;
mod recovery;
mod releases;
mod scanner;
mod snapshot_export;
mod snapshot_verify;
mod snapshot_worker;
mod sync;
mod uploads;
mod web;
mod worker;

use axum::{
    extract::{FromRef, State},
    http::StatusCode,
    routing::get,
    Json, Router,
};
use better_auth::{
    axum_integration::{auth_routes, AuthUser, NativeAuthUser},
    context::AuthContext,
    database::{SqlxDbAdapter, SqlxSecondaryStorage},
    http_oauth::HttpOAuthProvider,
    router::AuthRouter,
    JwksRefreshPolicy,
};
use better_auth_core::{
    migration::SqlDialect,
    options::{AuthOptions, BaseUrl},
};
use config::Config;
use sqlx::{postgres::PgPoolOptions, PgPool};
use std::{error::Error, sync::Arc};

#[derive(Clone)]
struct AppState {
    auth: AuthContext,
    pool: PgPool,
    object_storage: Option<cloud_storage::QuarantineStore>,
    key_enrollment_enabled: bool,
}
impl FromRef<AppState> for AuthContext {
    fn from_ref(state: &AppState) -> Self {
        state.auth.clone()
    }
}

async fn me(
    AuthUser(principal): AuthUser,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let roles = admin::roles(&state.pool, &principal.user.id).await?;
    Ok(Json(
        serde_json::json!({ "id": principal.user.id, "email": principal.user.email, "name": principal.user.name, "roles": roles }),
    ))
}
async fn native_me(
    NativeAuthUser(principal): NativeAuthUser,
    state: State<AppState>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    me(AuthUser(principal), state).await
}

async fn native_logout(
    NativeAuthUser(principal): NativeAuthUser,
    State(state): State<AppState>,
) -> Result<StatusCode, StatusCode> {
    let sessions = better_auth::session::SessionService::new(state.auth)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    sessions
        .discard(&principal.session.id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(StatusCode::NO_CONTENT)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "cloud_api=info".into()),
        )
        .init();
    let config =
        Config::from_env().map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    let adapter = Arc::new(SqlxDbAdapter::postgres(&config.database_url).await?);
    let storage = Arc::new(SqlxSecondaryStorage::postgres(&config.database_url).await?);
    let context = AuthContext::new(
        AuthOptions {
            secret: config.auth_secret,
            base_url: Some(BaseUrl::Static(config.origin.clone())),
            trusted_origins: vec![config.origin],
            advanced: better_auth_core::options::AdvancedOptions {
                trusted_proxy_headers: !config.trusted_proxy_cidrs.is_empty(),
                ..Default::default()
            },
            ..AuthOptions::default()
        },
        adapter.clone(),
        Some(storage.clone()),
        vec![],
    )?;
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args == ["migrate"] {
        adapter
            .apply_migrations(&context.migration_plan(), SqlDialect::Postgres)
            .await?;
        storage.migrate().await?;
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .connect(&config.database_url)
            .await?;
        readiness::MIGRATOR.run(&pool).await?;
        tracing::info!("auth and cloud migrations applied");
        return Ok(());
    }
    if let [command, candidate] = args.as_slice() {
        if command == "verify-candidate" {
            let id = uuid::Uuid::parse_str(candidate)?;
            let store = cloud_storage::QuarantineStore::new(
                config
                    .object_storage
                    .ok_or("object storage must be configured")?,
            )?;
            let pool = PgPoolOptions::new()
                .max_connections(2)
                .connect(&config.database_url)
                .await?;
            let root: tiktools_plugin_package::TrustedKey =
                serde_json::from_slice(&snapshot_worker::read_file(
                    std::path::Path::new(&std::env::var("SNAPSHOT_ROOT_PIN_FILE")?),
                    false,
                    16384,
                )?)?;
            let index: tiktools_plugin_package::PublicKey =
                serde_json::from_slice(&snapshot_worker::read_file(
                    std::path::Path::new(&std::env::var("SNAPSHOT_INDEX_PUBLIC_KEY_FILE")?),
                    false,
                    16384,
                )?)?;
            let scanner = scanner::ClamAv::new(
                config
                    .clamav_addr
                    .ok_or("CLAMAV_ADDR must be configured for verification")?,
            );
            let seed = std::path::PathBuf::from(std::env::var("SNAPSHOT_SEED_DIR")?);
            snapshot_verify::verify(&pool, &store, &scanner, &seed, id, &root, &index).await?;
            tracing::info!(candidate_id=%id, "private candidate checks passed; not approved for deployment");
            return Ok(());
        }
    }
    if let [command, job, operator] = args.as_slice() {
        let kind = match command.as_str() {
            "retry-verification" => Some(recovery::Kind::Verification),
            "retry-publication" => Some(recovery::Kind::Publication),
            _ => None,
        };
        if let Some(kind) = kind {
            let pool = PgPoolOptions::new()
                .max_connections(1)
                .connect(&config.database_url)
                .await?;
            let changed =
                recovery::retry(&pool, kind, uuid::Uuid::parse_str(job)?, operator).await?;
            if !changed {
                return Err("job is not an eligible transient failure".into());
            }
            tracing::info!("transient job requeued");
            return Ok(());
        }
    }
    if let [command, candidate, destination] = args.as_slice() {
        if command == "export-candidate" {
            let id = uuid::Uuid::parse_str(candidate)?;
            let storage_config = config
                .object_storage
                .ok_or("object storage must be configured")?;
            let store = cloud_storage::QuarantineStore::new(storage_config)?;
            let pool = PgPoolOptions::new()
                .max_connections(2)
                .connect(&config.database_url)
                .await?;
            snapshot_export::export(&pool, &store, id, std::path::Path::new(destination)).await?;
            tracing::info!(candidate_id=%id,"private snapshot candidate exported");
            return Ok(());
        }
    }
    if let [command, user, role, operator] = args.as_slice() {
        if command == "grant-role" || command == "revoke-role" {
            let pool = PgPoolOptions::new()
                .max_connections(1)
                .connect(&config.database_url)
                .await?;
            let changed =
                admin::change_role(&pool, command == "grant-role", user, role, operator).await?;
            tracing::info!(changed, "role command completed");
            return Ok(());
        }
    }
    if args == ["verify-once"]
        || args == ["prepare-publication-once"]
        || args == ["build-initial-candidate"]
    {
        let storage_config = config
            .object_storage
            .ok_or("object storage must be configured")?;
        let store = cloud_storage::QuarantineStore::new(storage_config)?;
        let pool = PgPoolOptions::new()
            .max_connections(2)
            .connect(&config.database_url)
            .await?;
        let scanner = scanner::ClamAv::new(
            config
                .clamav_addr
                .ok_or("CLAMAV_ADDR must be configured for verification")?,
        );
        let worked = if args == ["verify-once"] {
            worker::once(&pool, &store, &scanner).await?
        } else if args == ["prepare-publication-once"] {
            publication_worker::once(&pool, &store, &scanner).await?
        } else {
            let inputs = snapshot_worker::inputs_from_env()?;
            let candidate = snapshot_worker::build(&pool, &store, &scanner, inputs).await?;
            if let Some(id) = candidate {
                tracing::info!(candidate_id=%id,"private snapshot candidate staged");
            }
            candidate.is_some()
        };
        tracing::info!(worked, "worker finished");
        return Ok(());
    }
    if !args.is_empty() {
        return Err("usage: cloud-api [migrate | verify-once | prepare-publication-once | build-initial-candidate | export-candidate ID OUTPUT_DIR | verify-candidate ID | retry-verification UPLOAD_ID OPERATOR | retry-publication JOB_ID OPERATOR | grant-role USER ROLE OPERATOR | revoke-role USER ROLE OPERATOR]".into());
    }
    let mut auth = AuthRouter::new(context.clone())?;
    if let Some((id, secret)) = config.google {
        let mut google = HttpOAuthProvider::google_with_jwks(id, secret).await?;
        if config.jwks_policy != JwksRefreshPolicy::default() {
            // Changing policy rebuilds the upstream cache. Reload immediately to
            // preserve fail-closed startup rather than starting with an empty cache.
            google = google.with_jwks_policy(config.jwks_policy)?;
            google
                .refresh_oidc_jwks(url::Url::parse(
                    "https://www.googleapis.com/oauth2/v3/certs",
                )?)
                .await?;
        }
        auth = auth.with_social_providers(vec![google])?;
    }
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&config.database_url)
        .await?;
    let object_storage = config
        .object_storage
        .map(cloud_storage::QuarantineStore::new)
        .transpose()?;
    let app = app_with_proxies(
        context,
        pool,
        auth,
        object_storage,
        config.trusted_proxy_cidrs,
    );
    let listener = tokio::net::TcpListener::bind(config.bind).await?;
    tracing::info!(address = %config.bind, "cloud API listening");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(async {
        let _ = tokio::signal::ctrl_c().await;
    })
    .await?;
    Ok(())
}

#[cfg(test)]
fn app(context: AuthContext, pool: PgPool, auth: AuthRouter) -> Router {
    app_with_storage(context, pool, auth, None)
}

#[cfg(test)]
fn app_with_storage(
    context: AuthContext,
    pool: PgPool,
    auth: AuthRouter,
    object_storage: Option<cloud_storage::QuarantineStore>,
) -> Router {
    app_inner(context, pool, auth, object_storage, Vec::new(), false)
}

fn app_with_proxies(
    context: AuthContext,
    pool: PgPool,
    auth: AuthRouter,
    object_storage: Option<cloud_storage::QuarantineStore>,
    trusted_proxy_cidrs: Vec<ipnet::IpNet>,
) -> Router {
    app_inner(
        context,
        pool,
        auth,
        object_storage,
        trusted_proxy_cidrs,
        false,
    )
}

// The true value is available only to the test harness; there is no production
// configuration override until session-bound step-up is implemented.
fn app_inner(
    context: AuthContext,
    pool: PgPool,
    auth: AuthRouter,
    object_storage: Option<cloud_storage::QuarantineStore>,
    trusted_proxy_cidrs: Vec<ipnet::IpNet>,
    key_enrollment_enabled: bool,
) -> Router {
    Router::new()
        .route("/", get(web::page))
        .route("/desktop/login", get(web::page))
        .route("/desktop/login.js", get(web::script))
        .route("/desktop/login.css", get(web::style))
        .route("/health/live", get(|| async { StatusCode::OK }))
        .route("/health/ready", get(readiness::ready))
        .route(
            "/api/v1/me/deletion-blockers",
            get(account::blockers).layer(axum::middleware::map_response(sync::no_store)),
        )
        .route(
            "/api/v1/me/export",
            get(account::export).layer(axum::middleware::map_response(sync::sync_response)),
        )
        .route(
            "/api/v1/me/roles/{role}",
            axum::routing::delete(account::leave_role)
                .layer(axum::middleware::map_response(sync::no_store)),
        )
        .route(
            "/api/v1/native/me/deletion-blockers",
            get(account::native_blockers).layer(axum::middleware::map_response(sync::no_store)),
        )
        .route(
            "/api/v1/native/me/export",
            get(account::native_export).layer(axum::middleware::map_response(sync::sync_response)),
        )
        .route(
            "/api/v1/me",
            get(me).layer(axum::middleware::map_response(sync::no_store)),
        )
        .route(
            "/api/v1/admin/publishers/{id}/approve",
            axum::routing::post(admin::approve)
                .layer(axum::middleware::map_response(sync::no_store)),
        )
        .route(
            "/api/v1/moderation/releases",
            get(moderation::queue).layer(axum::middleware::map_response(sync::no_store)),
        )
        .route(
            "/api/v1/moderation/releases/{id}/inspection",
            axum::routing::post(moderation::inspection)
                .layer(axum::middleware::map_response(sync::no_store)),
        )
        .route(
            "/api/v1/moderation/releases/{id}/decision",
            axum::routing::post(moderation::decide)
                .layer(axum::extract::DefaultBodyLimit::max(2048))
                .layer(axum::middleware::map_response(sync::no_store)),
        )
        .route(
            "/api/v1/plugins/releases",
            axum::routing::post(releases::submit)
                .layer(axum::extract::DefaultBodyLimit::max(4096))
                .layer(axum::middleware::map_response(sync::no_store)),
        )
        .route(
            "/api/v1/plugins/releases/{id}",
            get(releases::get).layer(axum::middleware::map_response(sync::no_store)),
        )
        .route(
            "/api/v1/plugins/releases/{id}/publish",
            axum::routing::post(publication::request)
                .layer(axum::middleware::map_response(sync::no_store)),
        )
        .route(
            "/api/v1/plugins/releases/{id}/review",
            get(releases::review).layer(axum::middleware::map_response(sync::no_store)),
        )
        .route(
            "/api/v1/plugins/uploads",
            axum::routing::post(uploads::create)
                .layer(axum::extract::DefaultBodyLimit::max(4096))
                .layer(axum::middleware::map_response(sync::no_store)),
        )
        .route(
            "/api/v1/plugins/uploads/{id}/complete",
            axum::routing::post(uploads::complete)
                .layer(axum::extract::DefaultBodyLimit::max(4096))
                .layer(axum::middleware::map_response(sync::no_store)),
        )
        .route(
            "/api/v1/publishers",
            get(publishers::list)
                .post(publishers::create)
                .layer(axum::extract::DefaultBodyLimit::max(4096))
                .layer(axum::middleware::map_response(sync::no_store)),
        )
        .route(
            "/api/v1/publishers/{id}/keys/challenges",
            axum::routing::post(publisher_keys::challenge)
                .layer(axum::middleware::map_response(sync::no_store)),
        )
        .route(
            "/api/v1/publishers/{id}/keys",
            get(publisher_keys::list)
                .post(publisher_keys::request_key)
                .layer(axum::extract::DefaultBodyLimit::max(4096))
                .layer(axum::middleware::map_response(sync::no_store)),
        )
        .route(
            "/api/v1/publishers/{id}/keys/{key_id}/revoke",
            axum::routing::post(publisher_keys::revoke)
                .layer(axum::middleware::map_response(sync::no_store)),
        )
        .route(
            "/api/v1/admin/publishers/{id}/keys",
            get(publisher_keys::admin_list).layer(axum::middleware::map_response(sync::no_store)),
        )
        .route(
            "/api/v1/admin/publishers/{id}/keys/{key_id}/approve",
            axum::routing::post(publisher_keys::approve)
                .layer(axum::middleware::map_response(sync::no_store)),
        )
        .route(
            "/api/v1/admin/publishers/{id}/keys/{key_id}/revoke",
            axum::routing::post(publisher_keys::admin_revoke)
                .layer(axum::middleware::map_response(sync::no_store)),
        )
        .route(
            "/api/v1/publishers/{id}/transfer",
            get(publishers::current_transfer)
                .post(publishers::offer_transfer)
                .delete(publishers::cancel_transfer)
                .layer(axum::extract::DefaultBodyLimit::max(4096))
                .layer(axum::middleware::map_response(sync::no_store)),
        )
        .route(
            "/api/v1/publishers/{id}/transfer/{transfer_id}/accept",
            axum::routing::post(publishers::accept_transfer)
                .layer(axum::middleware::map_response(sync::no_store)),
        )
        .route(
            "/api/v1/publishers/{id}/membership",
            axum::routing::delete(publishers::leave)
                .layer(axum::middleware::map_response(sync::no_store)),
        )
        .route("/api/v1/publishers/{id}/members", get(publishers::members))
        .route(
            "/api/v1/publishers/{id}/members/{user_id}",
            axum::routing::put(publishers::set_member).delete(publishers::remove_member),
        )
        .route(
            "/api/v1/sync/collections/{name}",
            get(sync::list).layer(axum::middleware::map_response(sync::sync_response)),
        )
        .route(
            "/api/v1/sync/collections/{name}/{key}",
            axum::routing::put(sync::put)
                .delete(sync::delete)
                .layer(axum::extract::DefaultBodyLimit::max(4096))
                .layer(axum::middleware::map_response(sync::sync_response)),
        )
        .route(
            "/api/v1/sync/changes",
            get(sync::changes).layer(axum::middleware::map_response(sync::sync_response)),
        )
        .route(
            "/api/v1/sync/export",
            get(sync::export).layer(axum::middleware::map_response(sync::sync_response)),
        )
        .route(
            "/api/v1/sync/head",
            get(sync::head).layer(axum::middleware::map_response(sync::sync_response)),
        )
        .route(
            "/api/v1/desktop/handoff",
            axum::routing::post(desktop::issue)
                .layer(axum::extract::DefaultBodyLimit::max(4096))
                .layer(axum::middleware::map_response(sync::no_store)),
        )
        .route(
            "/api/v1/native/handoff/redeem",
            axum::routing::post(desktop::redeem)
                .layer(axum::extract::DefaultBodyLimit::max(4096))
                .layer(axum::middleware::map_response(sync::no_store)),
        )
        .route(
            "/api/v1/native/logout",
            axum::routing::post(native_logout)
                .layer(axum::middleware::map_response(sync::no_store)),
        )
        .route(
            "/api/v1/native/me",
            get(native_me).layer(axum::middleware::map_response(sync::no_store)),
        )
        .route(
            "/api/v1/native/sync/collections/{name}",
            get(sync::native_list).layer(axum::middleware::map_response(sync::sync_response)),
        )
        .route(
            "/api/v1/native/sync/collections/{name}/{key}",
            axum::routing::put(sync::native_put)
                .delete(sync::native_delete)
                .layer(axum::extract::DefaultBodyLimit::max(4096))
                .layer(axum::middleware::map_response(sync::sync_response)),
        )
        .route(
            "/api/v1/native/sync/changes",
            get(sync::native_changes).layer(axum::middleware::map_response(sync::sync_response)),
        )
        .route(
            "/api/v1/native/sync/export",
            get(sync::native_export).layer(axum::middleware::map_response(sync::sync_response)),
        )
        .route(
            "/api/v1/native/sync/head",
            get(sync::native_head).layer(axum::middleware::map_response(sync::sync_response)),
        )
        .with_state(AppState {
            auth: context,
            pool,
            object_storage,
            key_enrollment_enabled,
        })
        .merge(auth_routes(auth))
        .layer(axum::middleware::from_fn_with_state(
            trusted_proxy_cidrs,
            proxy_boundary,
        ))
}

async fn proxy_boundary(
    State(trusted): State<Vec<ipnet::IpNet>>,
    mut request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let peer = request
        .extensions()
        .get::<axum::extract::connect_info::ConnectInfo<std::net::SocketAddr>>()
        .map(|connect| connect.0.ip());
    let trusted_peer =
        peer.is_some_and(|peer| trusted.iter().any(|network| network.contains(&peer)));
    if !trusted_peer {
        for name in [
            "forwarded",
            "x-forwarded-for",
            "x-forwarded-host",
            "x-forwarded-proto",
            "x-forwarded-port",
            "x-forwarded-prefix",
            "x-real-ip",
        ] {
            request.headers_mut().remove(name);
        }
    }
    next.run(request).await
}

#[cfg(test)]
mod auth_tests;
