//! Account plans and their quota ceilings.
//!
//! Every account is `free` unless an operator assigns a plan in
//! `cloud_account_plan`. The Free ceilings are exactly the historical constants
//! so deploying this module cannot tighten or loosen any existing account.
//! Plans are enforced in the backend only: a client cannot raise its own quota.

use axum::{extract::State, http::StatusCode, Json};
use better_auth::axum_integration::{AuthUser, NativeAuthUser};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::{admin::require_admin, AppState};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Plan {
    #[default]
    Free,
    Pro,
    Max,
}

impl Plan {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Free => "free",
            Self::Pro => "pro",
            Self::Max => "max",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "free" => Some(Self::Free),
            "pro" => Some(Self::Pro),
            "max" => Some(Self::Max),
            _ => None,
        }
    }

    pub fn limits(self) -> PlanLimits {
        match self {
            // Historical values: workspace.rs, assets.rs, overlays.rs.
            Self::Free => PlanLimits {
                max_records: 1_000,
                max_bytes: 8 * 1024 * 1024,
                max_changes: 10_000,
                max_change_bytes: 16 * 1024 * 1024,
                max_assets: 200,
                max_assets_bytes: 32 * 1024 * 1024,
                max_overlays: 50,
                max_revisions_per_overlay: 100,
                max_overlay_bytes: 16 * 1024 * 1024,
            },
            Self::Pro => PlanLimits {
                max_records: 10_000,
                max_bytes: 64 * 1024 * 1024,
                max_changes: 100_000,
                max_change_bytes: 128 * 1024 * 1024,
                max_assets: 1_000,
                max_assets_bytes: 256 * 1024 * 1024,
                max_overlays: 250,
                max_revisions_per_overlay: 500,
                max_overlay_bytes: 64 * 1024 * 1024,
            },
            Self::Max => PlanLimits {
                max_records: 50_000,
                max_bytes: 256 * 1024 * 1024,
                max_changes: 500_000,
                max_change_bytes: 512 * 1024 * 1024,
                max_assets: 5_000,
                max_assets_bytes: 1_024 * 1024 * 1024,
                max_overlays: 1_000,
                max_revisions_per_overlay: 2_000,
                max_overlay_bytes: 256 * 1024 * 1024,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanLimits {
    pub max_records: i64,
    pub max_bytes: i64,
    pub max_changes: i64,
    pub max_change_bytes: i64,
    pub max_assets: i64,
    pub max_assets_bytes: i64,
    pub max_overlays: i64,
    pub max_revisions_per_overlay: i64,
    pub max_overlay_bytes: i64,
}

/// Missing row or database failure falls back to Free: fail closed, and never
/// let a transient error temporarily grant a higher quota.
pub async fn plan_for(pool: &PgPool, user_id: &str) -> Plan {
    let row: Option<String> =
        sqlx::query_scalar("SELECT plan FROM cloud_account_plan WHERE user_id = $1")
            .bind(user_id)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
    row.as_deref().and_then(Plan::parse).unwrap_or(Plan::Free)
}

pub async fn limits_for(pool: &PgPool, user_id: &str) -> PlanLimits {
    plan_for(pool, user_id).await.limits()
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanView {
    pub plan: &'static str,
    pub limits: PlanLimits,
}

/// Read-only view of the caller's effective plan and limits.
pub async fn view(
    NativeAuthUser(user): NativeAuthUser,
    State(state): State<AppState>,
) -> Result<Json<PlanView>, StatusCode> {
    let plan = plan_for(&state.pool, &user.user.id).await;
    Ok(Json(PlanView {
        plan: plan.as_str(),
        limits: plan.limits(),
    }))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetPlanInput {
    pub user_id: String,
    pub plan: String,
}

/// Operator-only plan assignment. The plan is never accepted from the client
/// being rate-limited.
pub async fn set_plan(
    AuthUser(principal): AuthUser,
    State(state): State<AppState>,
    Json(input): Json<SetPlanInput>,
) -> Result<StatusCode, StatusCode> {
    let plan = Plan::parse(&input.plan).ok_or(StatusCode::BAD_REQUEST)?;
    let mut tx = state.pool.begin().await.map_err(internal)?;
    require_admin(&mut tx, &principal.user.id).await?;
    sqlx::query(
        "INSERT INTO cloud_account_plan (user_id, plan, updated_at) \
         VALUES ($1, $2, (extract(epoch from now()) * 1000)::bigint) \
         ON CONFLICT (user_id) DO UPDATE SET plan = excluded.plan, \
         updated_at = excluded.updated_at",
    )
    .bind(&input.user_id)
    .bind(plan.as_str())
    .execute(&mut *tx)
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

fn internal(_: impl std::fmt::Display) -> StatusCode {
    StatusCode::INTERNAL_SERVER_ERROR
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_plan_is_free_with_historical_limits() {
        let free = Plan::Free.limits();
        assert_eq!(free.max_records, 1_000);
        assert_eq!(free.max_bytes, 8 * 1024 * 1024);
        assert_eq!(free.max_changes, 10_000);
        assert_eq!(free.max_change_bytes, 16 * 1024 * 1024);
        assert_eq!(free.max_assets, 200);
        assert_eq!(free.max_assets_bytes, 32 * 1024 * 1024);
        assert_eq!(free.max_overlays, 50);
        assert_eq!(free.max_revisions_per_overlay, 100);
        assert_eq!(free.max_overlay_bytes, 16 * 1024 * 1024);
    }

    #[test]
    fn plans_parse_and_are_monotonically_more_permissive() {
        assert_eq!(Plan::parse("free"), Some(Plan::Free));
        assert_eq!(Plan::parse("pro"), Some(Plan::Pro));
        assert_eq!(Plan::parse("max"), Some(Plan::Max));
        assert_eq!(Plan::parse("team"), None);
        assert_eq!(Plan::parse("enterprise"), None);
        let free = Plan::Free.limits();
        let pro = Plan::Pro.limits();
        let max = Plan::Max.limits();
        assert!(free.max_records < pro.max_records);
        assert!(pro.max_records < max.max_records);
        assert!(free.max_bytes < pro.max_bytes);
        assert!(pro.max_bytes < max.max_bytes);
    }

    #[test]
    fn plan_view_never_exposes_admin_fields() {
        let view = PlanView {
            plan: Plan::Free.as_str(),
            limits: Plan::Free.limits(),
        };
        let json = serde_json::to_value(&view).unwrap();
        assert!(json.get("updatedAt").is_none());
        assert!(json.get("userId").is_none());
    }
}
