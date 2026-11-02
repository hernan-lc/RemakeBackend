//! Publisher namespace requests. A pending namespace does not grant signing trust.
use crate::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use better_auth::axum_integration::AuthUser;
use serde::{Deserialize, Serialize};
use sqlx::{Postgres, Row, Transaction};
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
    AuthUser(principal): AuthUser,
    State(state): State<AppState>,
    Json(input): Json<CreatePublisher>,
) -> Result<(StatusCode, Json<Publisher>), StatusCode> {
    if !valid_namespace(&input.namespace) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let id = Uuid::new_v4();
    let mut tx = state.pool.begin().await.map_err(internal)?;
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
    AuthUser(principal): AuthUser,
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
