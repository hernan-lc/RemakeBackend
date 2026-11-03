//! Read-only visibility into Cloud-owned account deletion blockers.
use crate::AppState;
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, Method, StatusCode, Uri},
    Json,
};
use better_auth::rate_limit::{RateLimitPolicy, RateLimiter};
use better_auth::{
    axum_integration::{AuthUser, NativeAuthUser},
    check_csrf,
    context::{RequestContext, RequestMetadata},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{Postgres, Row, Transaction};
use std::time::Duration;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueryPage {
    after: Option<Uuid>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Membership {
    publisher_id: Uuid,
    namespace: String,
    role: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Blockers {
    cloud_roles: Vec<String>,
    memberships: Vec<Membership>,
    next_after: Option<Uuid>,
}

async fn bounded_rows(
    tx: &mut Transaction<'_, Postgres>,
    query: &str,
    user_id: &str,
) -> Result<Vec<Value>, StatusCode> {
    let rows = sqlx::query_scalar::<_, Value>(query)
        .bind(user_id)
        .fetch_all(&mut **tx)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if rows.len() > 1000 {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }
    Ok(rows)
}

async fn export_inner(
    principal: better_auth::session::AuthPrincipal,
    state: AppState,
) -> Result<Json<Value>, StatusCode> {
    let backend = state
        .auth
        .secondary_storage
        .clone()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let policy = RateLimitPolicy::new("export", 10, Duration::from_secs(60))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    RateLimiter::new(backend, "cloud:account")
        .enforce(&policy, &principal.user.id)
        .await
        .map_err(|error| match error {
            better_auth_core::error::AuthError::RateLimited { .. } => StatusCode::TOO_MANY_REQUESTS,
            _ => StatusCode::SERVICE_UNAVAILABLE,
        })?;

    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
        .execute(&mut *tx)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let user_id = &principal.user.id;
    let roles: Vec<String> = sqlx::query_scalar(
        "SELECT role FROM cloud_account_roles WHERE user_id=$1 ORDER BY role LIMIT 1001",
    )
    .bind(user_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if roles.len() > 1000 {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }
    let memberships = bounded_rows(&mut tx, "SELECT to_jsonb(x) FROM (SELECT p.id AS publisher_id,p.namespace,p.status,m.role FROM cloud_publisher_memberships m JOIN cloud_publishers p ON p.id=m.publisher_id WHERE m.user_id=$1 ORDER BY p.id LIMIT 1001) x", user_id).await?;
    let keys = bounded_rows(&mut tx, "SELECT to_jsonb(x) FROM (SELECT p.namespace,k.key_id,k.fingerprint,k.status,k.created_at::text AS created_at FROM cloud_publisher_keys k JOIN cloud_publishers p ON p.id=k.publisher_id WHERE k.requested_by=$1 ORDER BY k.created_at,k.key_id LIMIT 1001) x", user_id).await?;
    let releases = bounded_rows(&mut tx, "SELECT to_jsonb(x) FROM (SELECT p.namespace,r.plugin_id,r.version,r.target,r.status,r.created_at::text AS created_at FROM cloud_plugin_releases r JOIN cloud_publishers p ON p.id=r.publisher_id WHERE r.requested_by=$1 ORDER BY r.created_at,r.id LIMIT 1001) x", user_id).await?;
    let uploads = bounded_rows(&mut tx, "SELECT to_jsonb(x) FROM (SELECT p.namespace,u.expected_size,u.status,u.expires_at::text AS expires_at,u.created_at::text AS created_at FROM cloud_upload_sessions u JOIN cloud_publishers p ON p.id=u.publisher_id WHERE u.requested_by=$1 ORDER BY u.created_at,u.id LIMIT 1001) x", user_id).await?;
    let reviews = bounded_rows(&mut tx, "SELECT to_jsonb(x) FROM (SELECT p.namespace,r.plugin_id,r.version,m.decision,m.reason_code,m.created_at::text AS created_at FROM cloud_moderation_events m JOIN cloud_plugin_releases r ON r.id=m.release_id JOIN cloud_publishers p ON p.id=r.publisher_id WHERE m.actor_user_id=$1 ORDER BY m.created_at,m.id LIMIT 1001) x", user_id).await?;
    let publication_requests = bounded_rows(&mut tx, "SELECT to_jsonb(x) FROM (SELECT p.namespace,r.plugin_id,r.version,j.status,j.created_at::text AS created_at FROM cloud_publication_jobs j JOIN cloud_plugin_releases r ON r.id=j.release_id JOIN cloud_publishers p ON p.id=r.publisher_id WHERE j.requested_by=$1 ORDER BY j.created_at,j.id LIMIT 1001) x", user_id).await?;
    let ownership_transfers = bounded_rows(&mut tx, "SELECT to_jsonb(x) FROM (SELECT p.namespace,t.offered_by=$1 AS outgoing,t.created_at::text AS created_at,t.expires_at::text AS expires_at FROM cloud_publisher_transfers t JOIN cloud_publishers p ON p.id=t.publisher_id WHERE t.offered_by=$1 OR t.recipient=$1 ORDER BY t.created_at,t.id LIMIT 1001) x", user_id).await?;
    let audit_events = bounded_rows(&mut tx, "SELECT to_jsonb(x) FROM (SELECT e.action,e.publisher_id,e.created_at::text AS created_at FROM cloud_audit_events e WHERE e.actor_user_id=$1 ORDER BY e.created_at,e.id LIMIT 1001) x", user_id).await?;
    let yanks_recorded = bounded_rows(&mut tx, "SELECT to_jsonb(x) FROM (SELECT p.namespace,r.plugin_id,r.version,y.reason,y.created_at::text AS created_at FROM cloud_release_yanks y JOIN cloud_plugin_releases r ON r.id=y.release_id JOIN cloud_publishers p ON p.id=r.publisher_id WHERE y.yanked_by=$1 ORDER BY y.created_at,y.release_id LIMIT 1001) x", user_id).await?;
    let advisories_recorded = bounded_rows(&mut tx, "SELECT to_jsonb(x) FROM (SELECT p.namespace,r.plugin_id,r.version,a.severity,a.message,a.url,a.created_at::text AS created_at FROM cloud_release_advisories a JOIN cloud_plugin_releases r ON r.id=a.release_id JOIN cloud_publishers p ON p.id=r.publisher_id WHERE a.created_by=$1 ORDER BY a.created_at,a.release_id LIMIT 1001) x", user_id).await?;
    let sync_records = bounded_rows(&mut tx, "SELECT to_jsonb(x) FROM (SELECT collection,key,revision,deleted,value FROM cloud_sync_records WHERE user_id=$1 ORDER BY collection,key LIMIT 1001) x", user_id).await?;
    let cursor: i64 = sqlx::query_scalar(
        "SELECT COALESCE((SELECT cursor FROM cloud_sync_cursors WHERE user_id=$1),0)",
    )
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if sync_records.iter().any(|record| {
        let collection = record["collection"].as_str().unwrap_or_default();
        let key = record["key"].as_str().unwrap_or_default();
        let revision = record["revision"].as_i64().unwrap_or_default();
        let deleted = record["deleted"].as_bool().unwrap_or(false);
        collection != "plugin-selections"
            || !crate::sync::valid_plugin_id(key)
            || revision <= 0
            || revision > cursor
            || (deleted && !record["value"].is_null())
            || (!deleted && !crate::sync::validate(collection, key, &record["value"]))
    }) {
        tracing::error!("account export found invalid sync data");
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }
    let result = json!({
        "schemaVersion": 1,
        "profile": {"id": user_id, "email": principal.user.email, "name": principal.user.name},
        "cloudRoles": roles,
        "publisherMemberships": memberships,
        "publisherKeysRequested": keys,
        "releasesSubmitted": releases,
        "uploadsRequested": uploads,
        "reviewsRecorded": reviews,
        "publicationRequests": publication_requests,
        "ownershipTransfers": ownership_transfers,
        "auditEventsByActor": audit_events,
        "yanksRecorded": yanks_recorded,
        "advisoriesRecorded": advisories_recorded,
        "sync": {"cursor": cursor, "records": sync_records}
    });
    tx.commit()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(result))
}

pub async fn export(
    AuthUser(principal): AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Value>, StatusCode> {
    export_inner(principal, state).await
}

pub async fn native_export(
    NativeAuthUser(principal): NativeAuthUser,
    State(state): State<AppState>,
) -> Result<Json<Value>, StatusCode> {
    export_inner(principal, state).await
}

pub async fn blockers(
    AuthUser(principal): AuthUser,
    State(state): State<AppState>,
    Query(page): Query<QueryPage>,
) -> Result<Json<Blockers>, StatusCode> {
    let error = |_: sqlx::Error| {
        tracing::error!("account blocker query failed");
        StatusCode::INTERNAL_SERVER_ERROR
    };
    let mut tx = state.pool.begin().await.map_err(error)?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
        .execute(&mut *tx)
        .await
        .map_err(error)?;
    let cloud_roles =
        sqlx::query_scalar("SELECT role FROM cloud_account_roles WHERE user_id=$1 ORDER BY role")
            .bind(&principal.user.id)
            .fetch_all(&mut *tx)
            .await
            .map_err(error)?;
    let rows=sqlx::query("SELECT p.id,p.namespace,m.role FROM cloud_publisher_memberships m JOIN cloud_publishers p ON p.id=m.publisher_id WHERE m.user_id=$1 AND ($2::uuid IS NULL OR p.id>$2) ORDER BY p.id LIMIT 101").bind(&principal.user.id).bind(page.after).fetch_all(&mut *tx).await.map_err(error)?;
    let more = rows.len() > 100;
    let memberships: Vec<_> = rows
        .into_iter()
        .take(100)
        .map(|row| Membership {
            publisher_id: row.get("id"),
            namespace: row.get("namespace"),
            role: row.get("role"),
        })
        .collect();
    let next_after = if more {
        memberships.last().map(|m| m.publisher_id)
    } else {
        None
    };
    tx.commit().await.map_err(error)?;
    Ok(Json(Blockers {
        cloud_roles,
        memberships,
        next_after,
    }))
}
pub async fn native_blockers(
    NativeAuthUser(principal): NativeAuthUser,
    state: State<AppState>,
    page: Query<QueryPage>,
) -> Result<Json<Blockers>, StatusCode> {
    blockers(AuthUser(principal), state, page).await
}

/// Let an authenticated user remove their own Cloud role before deleting the
/// Better Auth account. The final admin role is protected across concurrent
/// self-service and operator revocations.
pub async fn leave_role(
    AuthUser(principal): AuthUser,
    State(state): State<AppState>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    Path(role): Path<String>,
) -> Result<StatusCode, StatusCode> {
    if !matches!(role.as_str(), "admin" | "moderator") {
        return Err(StatusCode::NOT_FOUND);
    }
    let mut trusted_origins = state.auth.options.trusted_origins.clone();
    if let Some(origin) = state.auth.base_origin() {
        if !trusted_origins.contains(&origin) {
            trusted_origins.push(origin);
        }
    }
    let request = RequestMetadata::new(method, uri, headers);
    let context = RequestContext {
        auth: state.auth.clone(),
        base_url: state.auth.base_url.clone(),
        trusted_origins,
        trusted_providers: Vec::new(),
        client_ip: None,
    };
    check_csrf(&context, &request).map_err(|_| StatusCode::FORBIDDEN)?;

    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    sqlx::query("SELECT pg_advisory_xact_lock(714928001)")
        .execute(&mut *tx)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM cloud_account_roles WHERE user_id=$1 AND role=$2)",
    )
    .bind(&principal.user.id)
    .bind(&role)
    .fetch_one(&mut *tx)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !exists {
        return Err(StatusCode::NOT_FOUND);
    }
    if role == "admin" {
        let admins: i64 =
            sqlx::query_scalar("SELECT count(*) FROM cloud_account_roles WHERE role='admin'")
                .fetch_one(&mut *tx)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        if admins <= 1 {
            return Err(StatusCode::CONFLICT);
        }
    }
    sqlx::query("DELETE FROM cloud_account_roles WHERE user_id=$1 AND role=$2")
        .bind(&principal.user.id)
        .bind(&role)
        .execute(&mut *tx)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    sqlx::query("INSERT INTO cloud_audit_events(id,actor_user_id,action,target_user_id,old_role,new_role) VALUES($1,$2,'account.role_self_revoked',$2,$3,NULL)")
        .bind(Uuid::new_v4())
        .bind(&principal.user.id)
        .bind(&role)
        .execute(&mut *tx)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    tx.commit()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(StatusCode::NO_CONTENT)
}
