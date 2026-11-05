//! Publisher namespace requests. A pending namespace does not grant signing trust.
use crate::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use better_auth::{
    axum_integration::AuthUser,
    rate_limit::{RateLimitPolicy, RateLimiter},
};

use crate::dual_auth::DualUser;
use serde::{Deserialize, Serialize};
use sqlx::{Postgres, Row, Transaction};
use std::time::Duration;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreatePublisher {
    namespace: String,
}

#[derive(Serialize)]
pub struct Publisher {
    id: Uuid,
    namespace: String,
    status: String,
    role: String,
}

fn valid_namespace(s: &str) -> bool {
    let b = s.as_bytes();
    (2..=63).contains(&b.len())
        && b[0].is_ascii_lowercase()
        && b[b.len() - 1] != b'-'
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

// Do not send database errors to clients: they can include query details.
fn internal(error: sqlx::Error) -> StatusCode {
    tracing::error!(
        error_class = "database",
        "publisher operation failed: {}",
        error
            .as_database_error()
            .map(|e| e.code().unwrap_or_default().into_owned())
            .unwrap_or_default()
    );
    StatusCode::INTERNAL_SERVER_ERROR
}

pub async fn create(
    DualUser(principal): DualUser,
    State(state): State<AppState>,
    Json(input): Json<CreatePublisher>,
) -> Result<(StatusCode, Json<Publisher>), StatusCode> {
    if !valid_namespace(&input.namespace) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let backend = state
        .auth
        .secondary_storage
        .clone()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let policy = RateLimitPolicy::new("create", 20, Duration::from_secs(60))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    RateLimiter::new(backend, "cloud:publishers")
        .enforce(&policy, &principal.user.id)
        .await
        .map_err(|error| match error {
            better_auth_core::error::AuthError::RateLimited { .. } => StatusCode::TOO_MANY_REQUESTS,
            _ => StatusCode::SERVICE_UNAVAILABLE,
        })?;
    let id = Uuid::new_v4();
    let mut tx = state.pool.begin().await.map_err(internal)?;
    // Serialize creation by this user, including the count check. FK locking also
    // makes a concurrent account deletion wait for the transaction.
    let exists: Option<String> =
        sqlx::query_scalar("SELECT id FROM \"user\" WHERE id=$1 FOR UPDATE")
            .bind(&principal.user.id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(internal)?;
    if exists.is_none() {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let pending: i64 = sqlx::query_scalar("SELECT count(*) FROM cloud_publisher_memberships m JOIN cloud_publishers p ON p.id=m.publisher_id WHERE m.user_id=$1 AND m.role='owner' AND p.status='pending'").bind(&principal.user.id).fetch_one(&mut *tx).await.map_err(internal)?;
    if pending >= 10 {
        return Err(StatusCode::TOO_MANY_REQUESTS);
    }

    let inserted = sqlx::query(
        "INSERT INTO cloud_publishers (id, namespace, status) VALUES ($1, $2, 'pending')",
    )
    .bind(id)
    .bind(&input.namespace)
    .execute(&mut *tx)
    .await;
    if let Err(error) = inserted {
        if error
            .as_database_error()
            .is_some_and(|e| e.is_unique_violation())
        {
            return Err(StatusCode::CONFLICT);
        }
        return Err(internal(error));
    }
    sqlx::query("INSERT INTO cloud_publisher_memberships (publisher_id, user_id, role) VALUES ($1, $2, 'owner')")
        .bind(id).bind(&principal.user.id).execute(&mut *tx).await.map_err(internal)?;
    sqlx::query("INSERT INTO cloud_audit_events (id, actor_user_id, publisher_id, action) VALUES ($1, $2, $3, 'publisher.requested')")
        .bind(Uuid::new_v4()).bind(&principal.user.id).bind(id).execute(&mut *tx).await.map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok((
        StatusCode::CREATED,
        Json(Publisher {
            id,
            namespace: input.namespace,
            status: "pending".into(),
            role: "owner".into(),
        }),
    ))
}

pub async fn list(
    DualUser(principal): DualUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<Publisher>>, StatusCode> {
    let rows = sqlx::query("SELECT p.id, p.namespace, p.status, m.role FROM cloud_publishers p JOIN cloud_publisher_memberships m ON m.publisher_id = p.id WHERE m.user_id = $1 ORDER BY p.namespace")
        .bind(&principal.user.id).fetch_all(&state.pool).await.map_err(internal)?;
    Ok(Json(
        rows.into_iter()
            .map(|r| Publisher {
                id: r.get("id"),
                namespace: r.get("namespace"),
                status: r.get("status"),
                role: r.get("role"),
            })
            .collect(),
    ))
}

#[derive(Serialize)]
pub struct Member {
    user_id: String,
    role: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetMember {
    role: String,
}

// Serialize all membership changes for a publisher, including permission checks.
// This prevents an authorization decision based on a membership concurrently revoked.
async fn lock_publisher<'a>(
    tx: &mut Transaction<'a, Postgres>,
    id: Uuid,
    actor: &str,
    owner_only: bool,
) -> Result<(), StatusCode> {
    let exists: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM cloud_publishers WHERE id = $1 FOR UPDATE")
            .bind(id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(internal)?;
    if exists.is_none() {
        return Err(StatusCode::NOT_FOUND);
    }
    let role: Option<String> = sqlx::query_scalar(
        "SELECT role FROM cloud_publisher_memberships WHERE publisher_id = $1 AND user_id = $2",
    )
    .bind(id)
    .bind(actor)
    .fetch_optional(&mut **tx)
    .await
    .map_err(internal)?;
    match role.as_deref() {
        None => Err(StatusCode::NOT_FOUND),
        Some("owner") => Ok(()),
        Some(_) if !owner_only => Ok(()),
        Some(_) => Err(StatusCode::FORBIDDEN),
    }
}

/// Remove an unused namespace while retaining its audit history.
pub async fn delete(
    DualUser(principal): DualUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, StatusCode> {
    let mut tx = state.pool.begin().await.map_err(internal)?;
    lock_publisher(&mut tx, id, &principal.user.id, true).await?;
    let blocked: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM cloud_publisher_memberships WHERE publisher_id=$1 AND user_id<>$2)
          OR EXISTS(SELECT 1 FROM cloud_publisher_keys WHERE publisher_id=$1)
          OR EXISTS(SELECT 1 FROM cloud_publisher_key_challenges WHERE publisher_id=$1)
          OR EXISTS(SELECT 1 FROM cloud_upload_sessions WHERE publisher_id=$1)
          OR EXISTS(SELECT 1 FROM cloud_plugin_releases WHERE publisher_id=$1)
          OR EXISTS(SELECT 1 FROM cloud_publisher_transfers WHERE publisher_id=$1)
          OR EXISTS(SELECT 1 FROM cloud_step_up_grants WHERE publisher_id=$1)
          OR EXISTS(SELECT 1 FROM cloud_step_up_oauth_states WHERE publisher_id=$1)",
    )
    .bind(id)
    .bind(&principal.user.id)
    .fetch_one(&mut *tx)
    .await
    .map_err(internal)?;
    if blocked {
        return Err(StatusCode::CONFLICT);
    }
    sqlx::query("UPDATE cloud_audit_events SET deleted_publisher_id=publisher_id, publisher_id=NULL WHERE publisher_id=$1")
        .bind(id).execute(&mut *tx).await.map_err(internal)?;
    sqlx::query("INSERT INTO cloud_audit_events(id,actor_user_id,deleted_publisher_id,action) VALUES($1,$2,$3,'publisher.deleted')")
        .bind(Uuid::new_v4()).bind(&principal.user.id).bind(id).execute(&mut *tx).await.map_err(internal)?;
    sqlx::query("DELETE FROM cloud_publisher_memberships WHERE publisher_id=$1 AND user_id=$2")
        .bind(id)
        .bind(&principal.user.id)
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    sqlx::query("DELETE FROM cloud_publishers WHERE id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn members(
    AuthUser(principal): AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<Member>>, StatusCode> {
    let mut tx = state.pool.begin().await.map_err(internal)?;
    lock_publisher(&mut tx, id, &principal.user.id, false).await?;
    let rows = sqlx::query("SELECT user_id, role FROM cloud_publisher_memberships WHERE publisher_id = $1 ORDER BY user_id")
        .bind(id).fetch_all(&mut *tx).await.map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(Json(
        rows.into_iter()
            .map(|row| Member {
                user_id: row.get("user_id"),
                role: row.get("role"),
            })
            .collect(),
    ))
}

pub async fn set_member(
    AuthUser(principal): AuthUser,
    State(state): State<AppState>,
    Path((id, user_id)): Path<(Uuid, String)>,
    Json(input): Json<SetMember>,
) -> Result<StatusCode, StatusCode> {
    if !matches!(input.role.as_str(), "admin" | "publisher" | "viewer")
        || user_id.is_empty()
        || user_id.len() > 256
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    let mut tx = state.pool.begin().await.map_err(internal)?;
    lock_publisher(&mut tx, id, &principal.user.id, true).await?;
    let old: Option<String> = sqlx::query_scalar(
        "SELECT role FROM cloud_publisher_memberships WHERE publisher_id = $1 AND user_id = $2",
    )
    .bind(id)
    .bind(&user_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(internal)?;
    if old.as_deref() == Some("owner") {
        return Err(StatusCode::CONFLICT);
    }
    if old.is_none() {
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM cloud_publisher_memberships WHERE publisher_id=$1",
        )
        .bind(id)
        .fetch_one(&mut *tx)
        .await
        .map_err(internal)?;
        if count >= 1000 {
            return Err(StatusCode::INSUFFICIENT_STORAGE);
        }
    }
    // Better Auth owns identities. Never create a membership for an unknown user.
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM \"user\" WHERE id = $1)")
        .bind(&user_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(internal)?;
    if !exists {
        return Err(StatusCode::NOT_FOUND);
    }
    if old.as_deref() != Some(input.role.as_str()) {
        sqlx::query("INSERT INTO cloud_publisher_memberships (publisher_id, user_id, role) VALUES ($1, $2, $3) ON CONFLICT (publisher_id, user_id) DO UPDATE SET role = EXCLUDED.role")
            .bind(id).bind(&user_id).bind(&input.role).execute(&mut *tx).await.map_err(internal)?;
        audit_member(
            &mut tx,
            id,
            &principal.user.id,
            &user_id,
            old.as_deref(),
            Some(&input.role),
        )
        .await?;
    }
    tx.commit().await.map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn remove_member(
    AuthUser(principal): AuthUser,
    State(state): State<AppState>,
    Path((id, user_id)): Path<(Uuid, String)>,
) -> Result<StatusCode, StatusCode> {
    let mut tx = state.pool.begin().await.map_err(internal)?;
    lock_publisher(&mut tx, id, &principal.user.id, true).await?;
    let old: Option<String> = sqlx::query_scalar(
        "SELECT role FROM cloud_publisher_memberships WHERE publisher_id = $1 AND user_id = $2",
    )
    .bind(id)
    .bind(&user_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(internal)?;
    if old.as_deref() == Some("owner") {
        return Err(StatusCode::CONFLICT);
    }
    if let Some(old) = old {
        sqlx::query(
            "DELETE FROM cloud_publisher_memberships WHERE publisher_id = $1 AND user_id = $2",
        )
        .bind(id)
        .bind(&user_id)
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
        audit_member(&mut tx, id, &principal.user.id, &user_id, Some(&old), None).await?;
    }
    tx.commit().await.map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

/// A non-owner may leave without cooperation from the publisher owner.
pub async fn leave(
    AuthUser(principal): AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, StatusCode> {
    let mut tx = state.pool.begin().await.map_err(internal)?;
    lock_publisher(&mut tx, id, &principal.user.id, false).await?;
    let role: String = sqlx::query_scalar(
        "SELECT role FROM cloud_publisher_memberships WHERE publisher_id=$1 AND user_id=$2",
    )
    .bind(id)
    .bind(&principal.user.id)
    .fetch_one(&mut *tx)
    .await
    .map_err(internal)?;
    if role == "owner" {
        return Err(StatusCode::CONFLICT);
    }
    sqlx::query("DELETE FROM cloud_publisher_memberships WHERE publisher_id=$1 AND user_id=$2")
        .bind(id)
        .bind(&principal.user.id)
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    audit_member(
        &mut tx,
        id,
        &principal.user.id,
        &principal.user.id,
        Some(&role),
        None,
    )
    .await?;
    tx.commit().await.map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TransferOffer {
    recipient_user_id: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Transfer {
    id: Uuid,
    offered_by: String,
    recipient_user_id: String,
}

pub async fn offer_transfer(
    AuthUser(principal): AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(input): Json<TransferOffer>,
) -> Result<(StatusCode, Json<Transfer>), StatusCode> {
    if input.recipient_user_id.is_empty()
        || input.recipient_user_id.len() > 256
        || input.recipient_user_id == principal.user.id
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    let mut tx = state.pool.begin().await.map_err(internal)?;
    lock_publisher(&mut tx, id, &principal.user.id, true).await?;
    let role: Option<String> = sqlx::query_scalar(
        "SELECT role FROM cloud_publisher_memberships WHERE publisher_id=$1 AND user_id=$2",
    )
    .bind(id)
    .bind(&input.recipient_user_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(internal)?;
    if role.is_none() {
        return Err(StatusCode::NOT_FOUND);
    }
    if role.as_deref() == Some("owner") {
        return Err(StatusCode::CONFLICT);
    }
    let transfer = Transfer {
        id: Uuid::new_v4(),
        offered_by: principal.user.id.clone(),
        recipient_user_id: input.recipient_user_id,
    };
    sqlx::query("INSERT INTO cloud_publisher_transfers(id,publisher_id,offered_by,recipient) VALUES($1,$2,$3,$4) ON CONFLICT (publisher_id) DO UPDATE SET id=EXCLUDED.id,offered_by=EXCLUDED.offered_by,recipient=EXCLUDED.recipient,created_at=now(),expires_at=now()+interval '24 hours'")
        .bind(transfer.id).bind(id).bind(&transfer.offered_by).bind(&transfer.recipient_user_id).execute(&mut *tx).await.map_err(internal)?;
    sqlx::query("INSERT INTO cloud_audit_events(id,actor_user_id,publisher_id,action,target_user_id,subject_id) VALUES($1,$2,$3,'publisher.transfer_offered',$4,$5)").bind(Uuid::new_v4()).bind(&transfer.offered_by).bind(id).bind(&transfer.recipient_user_id).bind(transfer.id).execute(&mut *tx).await.map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok((StatusCode::CREATED, Json(transfer)))
}

pub async fn current_transfer(
    AuthUser(principal): AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Transfer>, StatusCode> {
    let mut tx = state.pool.begin().await.map_err(internal)?;
    lock_publisher(&mut tx, id, &principal.user.id, false).await?;
    let row = sqlx::query("SELECT id,offered_by,recipient FROM cloud_publisher_transfers WHERE publisher_id=$1 AND expires_at>now() AND ($2=offered_by OR $2=recipient)").bind(id).bind(&principal.user.id).fetch_optional(&mut *tx).await.map_err(internal)?.ok_or(StatusCode::NOT_FOUND)?;
    let transfer = Transfer {
        id: row.get("id"),
        offered_by: row.get("offered_by"),
        recipient_user_id: row.get("recipient"),
    };
    tx.commit().await.map_err(internal)?;
    Ok(Json(transfer))
}

pub async fn cancel_transfer(
    AuthUser(principal): AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, StatusCode> {
    let mut tx = state.pool.begin().await.map_err(internal)?;
    lock_publisher(&mut tx, id, &principal.user.id, true).await?;
    let removed = sqlx::query(
        "DELETE FROM cloud_publisher_transfers WHERE publisher_id=$1 RETURNING id,recipient",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(internal)?;
    if let Some(row) = removed {
        sqlx::query("INSERT INTO cloud_audit_events(id,actor_user_id,publisher_id,action,target_user_id,subject_id) VALUES($1,$2,$3,'publisher.transfer_cancelled',$4,$5)").bind(Uuid::new_v4()).bind(&principal.user.id).bind(id).bind(row.get::<String,_>("recipient")).bind(row.get::<Uuid,_>("id")).execute(&mut *tx).await.map_err(internal)?;
    }
    tx.commit().await.map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn accept_transfer(
    AuthUser(principal): AuthUser,
    State(state): State<AppState>,
    Path((id, transfer_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, StatusCode> {
    let mut tx = state.pool.begin().await.map_err(internal)?;
    lock_publisher(&mut tx, id, &principal.user.id, false).await?;
    let offered_by: Option<String> = sqlx::query_scalar("SELECT offered_by FROM cloud_publisher_transfers WHERE publisher_id=$1 AND id=$2 AND recipient=$3 AND expires_at>now()").bind(id).bind(transfer_id).bind(&principal.user.id).fetch_optional(&mut *tx).await.map_err(internal)?;
    let offered_by = offered_by.ok_or(StatusCode::NOT_FOUND)?;
    let issuer_role: Option<String> = sqlx::query_scalar(
        "SELECT role FROM cloud_publisher_memberships WHERE publisher_id=$1 AND user_id=$2",
    )
    .bind(id)
    .bind(&offered_by)
    .fetch_optional(&mut *tx)
    .await
    .map_err(internal)?;
    let recipient_role: Option<String> = sqlx::query_scalar(
        "SELECT role FROM cloud_publisher_memberships WHERE publisher_id=$1 AND user_id=$2",
    )
    .bind(id)
    .bind(&principal.user.id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(internal)?;
    if issuer_role.as_deref() != Some("owner")
        || recipient_role.as_deref().is_none_or(|r| r == "owner")
    {
        return Err(StatusCode::CONFLICT);
    }
    sqlx::query(
        "UPDATE cloud_publisher_memberships SET role='admin' WHERE publisher_id=$1 AND user_id=$2",
    )
    .bind(id)
    .bind(&offered_by)
    .execute(&mut *tx)
    .await
    .map_err(internal)?;
    sqlx::query(
        "UPDATE cloud_publisher_memberships SET role='owner' WHERE publisher_id=$1 AND user_id=$2",
    )
    .bind(id)
    .bind(&principal.user.id)
    .execute(&mut *tx)
    .await
    .map_err(internal)?;
    sqlx::query("DELETE FROM cloud_publisher_transfers WHERE publisher_id=$1 AND id=$2")
        .bind(id)
        .bind(transfer_id)
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    audit_member(
        &mut tx,
        id,
        &principal.user.id,
        &offered_by,
        Some("owner"),
        Some("admin"),
    )
    .await?;
    audit_member(
        &mut tx,
        id,
        &principal.user.id,
        &principal.user.id,
        recipient_role.as_deref(),
        Some("owner"),
    )
    .await?;
    sqlx::query("INSERT INTO cloud_audit_events(id,actor_user_id,publisher_id,action,target_user_id,subject_id) VALUES($1,$2,$3,'publisher.transfer_accepted',$4,$5)").bind(Uuid::new_v4()).bind(&principal.user.id).bind(id).bind(&offered_by).bind(transfer_id).execute(&mut *tx).await.map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn audit_member(
    tx: &mut Transaction<'_, Postgres>,
    publisher: Uuid,
    actor: &str,
    target: &str,
    old: Option<&str>,
    new: Option<&str>,
) -> Result<(), StatusCode> {
    sqlx::query("INSERT INTO cloud_audit_events (id, actor_user_id, publisher_id, action, target_user_id, old_role, new_role) VALUES ($1, $2, $3, 'publisher.membership_changed', $4, $5, $6)")
        .bind(Uuid::new_v4()).bind(actor).bind(publisher).bind(target).bind(old).bind(new).execute(&mut **tx).await.map_err(internal)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::valid_namespace;
    #[test]
    fn namespace_rules_are_strict_ascii() {
        for good in ["ab", "a1", "example-plugin"] {
            assert!(valid_namespace(good));
        }
        for bad in ["a", "-ab", "ab-", "Ab", "a_b", "a.b", "áa", "a/b", ""] {
            assert!(!valid_namespace(bad));
        }
        assert!(!valid_namespace(&"a".repeat(64)));
    }
}
