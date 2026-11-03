//! Read-only visibility into Cloud-owned account deletion blockers.
use crate::AppState;
use axum::{
    extract::{Query, State},
    http::StatusCode,
    Json,
};
use better_auth::axum_integration::{AuthUser, NativeAuthUser};
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
