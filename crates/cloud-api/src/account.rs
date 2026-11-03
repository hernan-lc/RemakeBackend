//! Read-only visibility into Cloud-owned account deletion blockers.
use crate::AppState;
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, Method, StatusCode, Uri},
    Json,
};
use better_auth::{
    axum_integration::{AuthUser, NativeAuthUser},
    check_csrf,
    context::{RequestContext, RequestMetadata},
};
use serde::{Deserialize, Serialize};
use sqlx::Row;
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
