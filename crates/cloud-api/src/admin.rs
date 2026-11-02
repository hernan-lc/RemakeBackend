//! Cloud roles are managed out-of-band; publisher approval never delegates signing keys.
use crate::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
};
use better_auth::axum_integration::AuthUser;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

fn internal(_: sqlx::Error) -> StatusCode {
    tracing::error!("cloud authorization database operation failed");
    StatusCode::INTERNAL_SERVER_ERROR
}

pub async fn roles(pool: &PgPool, user: &str) -> Result<Vec<String>, StatusCode> {
    sqlx::query_scalar("SELECT role FROM cloud_account_roles WHERE user_id = $1 ORDER BY role")
        .bind(user)
        .fetch_all(pool)
        .await
        .map_err(internal)
}

pub async fn approve(
    AuthUser(principal): AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, StatusCode> {
    let mut tx: Transaction<'_, Postgres> = state.pool.begin().await.map_err(internal)?;
    // Locking the role row makes a concurrent role revocation serialize with this decision.
    let role: Option<String> = sqlx::query_scalar(
        "SELECT role FROM cloud_account_roles WHERE user_id=$1 AND role='admin' FOR SHARE",
    )
    .bind(&principal.user.id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(internal)?;
    if role.is_none() {
        return Err(StatusCode::FORBIDDEN);
    }
    let status: Option<String> =
        sqlx::query_scalar("SELECT status FROM cloud_publishers WHERE id=$1 FOR UPDATE")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(internal)?;
    let Some(status) = status else {
        return Err(StatusCode::NOT_FOUND);
    };
    let member: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM cloud_publisher_memberships WHERE publisher_id=$1 AND user_id=$2)")
        .bind(id).bind(&principal.user.id).fetch_one(&mut *tx).await.map_err(internal)?;
    if member {
        return Err(StatusCode::FORBIDDEN);
    }
    if status != "pending" {
        return Err(StatusCode::CONFLICT);
    }
    sqlx::query("UPDATE cloud_publishers SET status='approved' WHERE id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    sqlx::query("INSERT INTO cloud_audit_events (id,actor_user_id,publisher_id,action) VALUES ($1,$2,$3,'publisher.approved')")
        .bind(Uuid::new_v4()).bind(&principal.user.id).bind(id).execute(&mut *tx).await.map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Explicit operator command. The operator label is recorded as a claim, not an authenticated identity.
pub async fn change_role(
    pool: &PgPool,
    grant: bool,
    user: &str,
    role: &str,
    operator: &str,
) -> Result<bool, Box<dyn std::error::Error>> {
    if user.is_empty()
        || user.len() > 256
        || !matches!(role, "admin" | "moderator")
        || operator.is_empty()
        || operator.len() > 80
        || !operator
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_.@".contains(&c))
    {
        return Err("invalid user, role, or operator label".into());
    }
    let mut tx = pool.begin().await?;
    // Serialize out-of-band role changes, including the last-admin check.
    sqlx::query("SELECT pg_advisory_xact_lock(714928001)")
        .execute(&mut *tx)
        .await?;
    let user_exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM \"user\" WHERE id=$1)")
        .bind(user)
        .fetch_one(&mut *tx)
        .await?;
    if !user_exists {
        return Err("user does not exist".into());
    }
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM cloud_account_roles WHERE user_id=$1 AND role=$2)",
    )
    .bind(user)
    .bind(role)
    .fetch_one(&mut *tx)
    .await?;
    if exists == grant {
        tx.commit().await?;
        return Ok(false);
    }
    if !grant && role == "admin" {
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM cloud_account_roles WHERE role='admin'")
                .fetch_one(&mut *tx)
                .await?;
        if count <= 1 {
            return Err("cannot revoke the last admin role".into());
        }
    }
    if grant {
        sqlx::query("INSERT INTO cloud_account_roles(user_id,role) VALUES($1,$2)")
            .bind(user)
            .bind(role)
            .execute(&mut *tx)
            .await?;
    } else {
        sqlx::query("DELETE FROM cloud_account_roles WHERE user_id=$1 AND role=$2")
            .bind(user)
            .bind(role)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("INSERT INTO cloud_audit_events(id,actor_user_id,action,target_user_id,old_role,new_role) VALUES($1,$2,$3,$4,$5,$6)")
        .bind(Uuid::new_v4()).bind(format!("operator:{operator}")).bind(if grant { "account.role_granted" } else { "account.role_revoked" }).bind(user).bind(if grant { None } else { Some(role) }).bind(if grant { Some(role) } else { None }).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(true)
}
