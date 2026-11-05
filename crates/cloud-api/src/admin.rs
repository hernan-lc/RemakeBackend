//! Cloud roles are managed out-of-band; publisher approval never delegates signing keys.
use crate::AppState;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use better_auth::axum_integration::AuthUser;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

fn internal(_: sqlx::Error) -> StatusCode {
    tracing::error!("cloud authorization database operation failed");
    StatusCode::INTERNAL_SERVER_ERROR
}

/// Shared Cloud-admin gate. Route guards in the web app are UX only; every
/// administrative call is authorized here against the live role table.
async fn require_admin(tx: &mut Transaction<'_, Postgres>, user: &str) -> Result<(), StatusCode> {
    // Locking the role row serializes administrative reads with a concurrent
    // out-of-band role revocation.
    let role: Option<String> = sqlx::query_scalar(
        "SELECT role FROM cloud_account_roles WHERE user_id=$1 AND role='admin' FOR SHARE",
    )
    .bind(user)
    .fetch_optional(&mut **tx)
    .await
    .map_err(internal)?;
    if role.is_none() {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(())
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

/// Contract version for the read-only admin DTOs below. Bump when a response
/// shape changes so the contract tests and the web client catch drift.
pub const ADMIN_READ_CONTRACT_VERSION: u32 = 1;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublisherCounts {
    pending: i64,
    approved: i64,
    suspended: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseCounts {
    awaiting_verification: i64,
    integrity_passed: i64,
    antivirus_passed: i64,
    review_approved: i64,
    review_rejected: i64,
    rejected: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    contract_version: u32,
    publishers: PublisherCounts,
    releases: ReleaseCounts,
    pending_keys: i64,
    audit_events: i64,
}

/// Aggregate counts derived from existing tables. The queries are bounded
/// `COUNT(*)` scans over indexed status columns; results are computed live on
/// every call because stale review workload would mislead an admin, and the
/// route carries `no-store` so browsers and proxies never cache them.
pub async fn overview(
    AuthUser(principal): AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Overview>, StatusCode> {
    let mut tx = state.pool.begin().await.map_err(internal)?;
    require_admin(&mut tx, &principal.user.id).await?;
    let publisher_rows: Vec<(String, i64)> =
        sqlx::query_as("SELECT status, count(*) FROM cloud_publishers GROUP BY status")
            .fetch_all(&mut *tx)
            .await
            .map_err(internal)?;
    let release_rows: Vec<(String, i64)> =
        sqlx::query_as("SELECT status, count(*) FROM cloud_plugin_releases GROUP BY status")
            .fetch_all(&mut *tx)
            .await
            .map_err(internal)?;
    let pending_keys: i64 =
        sqlx::query_scalar("SELECT count(*) FROM cloud_publisher_keys WHERE status='pending'")
            .fetch_one(&mut *tx)
            .await
            .map_err(internal)?;
    let audit_events: i64 = sqlx::query_scalar("SELECT count(*) FROM cloud_audit_events")
        .fetch_one(&mut *tx)
        .await
        .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    let count = |rows: &[(String, i64)], status: &str| {
        rows.iter()
            .find(|(s, _)| s == status)
            .map(|(_, n)| *n)
            .unwrap_or(0)
    };
    Ok(Json(Overview {
        contract_version: ADMIN_READ_CONTRACT_VERSION,
        publishers: PublisherCounts {
            pending: count(&publisher_rows, "pending"),
            approved: count(&publisher_rows, "approved"),
            suspended: count(&publisher_rows, "suspended"),
        },
        releases: ReleaseCounts {
            awaiting_verification: count(&release_rows, "awaiting_verification"),
            integrity_passed: count(&release_rows, "integrity_passed"),
            antivirus_passed: count(&release_rows, "antivirus_passed"),
            review_approved: count(&release_rows, "review_approved"),
            review_rejected: count(&release_rows, "review_rejected"),
            rejected: count(&release_rows, "rejected"),
        },
        pending_keys,
        audit_events,
    }))
}

const PAGE_DEFAULT: i64 = 50;
const PAGE_MAX: i64 = 100;

/// Shared pagination bound. Malicious `limit` values are rejected instead of
/// clamped so callers notice a broken client instead of silently paging wrong.
fn page_limit(limit: Option<i64>) -> Result<i64, StatusCode> {
    match limit {
        None => Ok(PAGE_DEFAULT),
        Some(n) if (1..=PAGE_MAX).contains(&n) => Ok(n),
        Some(_) => Err(StatusCode::BAD_REQUEST),
    }
}

fn valid_publisher_status(status: &str) -> bool {
    matches!(status, "pending" | "approved" | "suspended")
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PublisherReviewQuery {
    status: Option<String>,
    cursor: Option<Uuid>,
    limit: Option<i64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublisherReviewItem {
    id: Uuid,
    namespace: String,
    status: String,
    created_at: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublisherReviewPage {
    contract_version: u32,
    items: Vec<PublisherReviewItem>,
    next_cursor: Option<Uuid>,
}

/// Filtered, paginated publisher projection for review. The projection carries
/// no user secrets: namespaces and statuses only, never membership user IDs,
/// emails, keys, or upload details. Ordered oldest-first so the review queue
/// is worked in request order.
pub async fn publishers(
    AuthUser(principal): AuthUser,
    State(state): State<AppState>,
    Query(query): Query<PublisherReviewQuery>,
) -> Result<Json<PublisherReviewPage>, StatusCode> {
    if query
        .status
        .as_deref()
        .is_some_and(|s| !valid_publisher_status(s))
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    let limit = page_limit(query.limit)?;
    let mut tx = state.pool.begin().await.map_err(internal)?;
    require_admin(&mut tx, &principal.user.id).await?;
    // Resolve the cursor to a stable (created_at, id) position. An unknown
    // cursor is a broken or forged page reference, not an empty page.
    let cursor: Option<(chrono::DateTime<chrono::Utc>, Uuid)> = match query.cursor {
        None => None,
        Some(id) => {
            let row = sqlx::query("SELECT created_at, id FROM cloud_publishers WHERE id=$1")
                .bind(id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(internal)?
                .ok_or(StatusCode::BAD_REQUEST)?;
            Some((row.get("created_at"), row.get("id")))
        }
    };
    let (cursor_at, cursor_id) = match cursor {
        Some((at, id)) => (Some(at), Some(id)),
        None => (None, None),
    };
    let rows = sqlx::query(
        "SELECT id, namespace, status, created_at FROM cloud_publishers WHERE ($1::text IS NULL OR status=$1) AND (($2::timestamptz IS NULL AND $3::uuid IS NULL) OR (created_at, id) > ($2, $3)) ORDER BY created_at, id LIMIT $4",
    )
    .bind(query.status.as_deref())
    .bind(cursor_at)
    .bind(cursor_id)
    .bind(limit + 1)
    .fetch_all(&mut *tx)
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    let more = rows.len() as i64 > limit;
    let items: Vec<PublisherReviewItem> = rows
        .into_iter()
        .take(limit as usize)
        .map(|row| {
            let created: chrono::DateTime<chrono::Utc> = row.get("created_at");
            PublisherReviewItem {
                id: row.get("id"),
                namespace: row.get("namespace"),
                status: row.get("status"),
                created_at: created.to_rfc3339(),
            }
        })
        .collect();
    let next_cursor = if more {
        items.last().map(|i| i.id)
    } else {
        None
    };
    Ok(Json(PublisherReviewPage {
        contract_version: ADMIN_READ_CONTRACT_VERSION,
        items,
        next_cursor,
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct AuditQuery {
    cursor: Option<Uuid>,
    limit: Option<i64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditEventView {
    id: Uuid,
    action: String,
    actor_user_id: String,
    target_user_id: Option<String>,
    publisher_id: Option<Uuid>,
    subject_id: Option<Uuid>,
    old_role: Option<String>,
    new_role: Option<String>,
    created_at: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditPage {
    contract_version: u32,
    events: Vec<AuditEventView>,
    next_cursor: Option<Uuid>,
}

/// Time-ordered (newest first), redacted audit events. Only scalar audit
/// columns are projected: opaque user IDs, role names, actions, and links.
/// The table stores no emails, tokens, object keys, or version IDs, and this
/// projection must stay that way — the contract test asserts the exact key
/// set so a future column cannot leak through `SELECT *`.
pub async fn audit(
    AuthUser(principal): AuthUser,
    State(state): State<AppState>,
    Query(query): Query<AuditQuery>,
) -> Result<Json<AuditPage>, StatusCode> {
    let limit = page_limit(query.limit)?;
    let mut tx = state.pool.begin().await.map_err(internal)?;
    require_admin(&mut tx, &principal.user.id).await?;
    let cursor: Option<(chrono::DateTime<chrono::Utc>, Uuid)> = match query.cursor {
        None => None,
        Some(id) => {
            let row = sqlx::query("SELECT created_at, id FROM cloud_audit_events WHERE id=$1")
                .bind(id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(internal)?
                .ok_or(StatusCode::BAD_REQUEST)?;
            Some((row.get("created_at"), row.get("id")))
        }
    };
    let (cursor_at, cursor_id) = match cursor {
        Some((at, id)) => (Some(at), Some(id)),
        None => (None, None),
    };
    let rows = sqlx::query(
        "SELECT id, action, actor_user_id, target_user_id, COALESCE(publisher_id, deleted_publisher_id) AS publisher_id, subject_id, old_role, new_role, created_at FROM cloud_audit_events WHERE (($1::timestamptz IS NULL AND $2::uuid IS NULL) OR (created_at, id) < ($1, $2)) ORDER BY created_at DESC, id DESC LIMIT $3",
    )
    .bind(cursor_at)
    .bind(cursor_id)
    .bind(limit + 1)
    .fetch_all(&mut *tx)
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    let more = rows.len() as i64 > limit;
    let events: Vec<AuditEventView> = rows
        .into_iter()
        .take(limit as usize)
        .map(|row| {
            let created: chrono::DateTime<chrono::Utc> = row.get("created_at");
            AuditEventView {
                id: row.get("id"),
                action: row.get("action"),
                actor_user_id: row.get("actor_user_id"),
                target_user_id: row.get("target_user_id"),
                publisher_id: row.get("publisher_id"),
                subject_id: row.get("subject_id"),
                old_role: row.get("old_role"),
                new_role: row.get("new_role"),
                created_at: created.to_rfc3339(),
            }
        })
        .collect();
    let next_cursor = if more {
        events.last().map(|e| e.id)
    } else {
        None
    };
    Ok(Json(AuditPage {
        contract_version: ADMIN_READ_CONTRACT_VERSION,
        events,
        next_cursor,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admin_page_limits_accept_only_bounded_values() {
        assert_eq!(page_limit(None).unwrap(), 50);
        assert_eq!(page_limit(Some(1)).unwrap(), 1);
        assert_eq!(page_limit(Some(100)).unwrap(), 100);
        assert!(page_limit(Some(0)).is_err());
        assert!(page_limit(Some(-5)).is_err());
        assert!(page_limit(Some(101)).is_err());
        assert!(page_limit(Some(10_000)).is_err());
    }

    #[test]
    fn publisher_status_filter_rejects_unknown_values() {
        assert!(valid_publisher_status("pending"));
        assert!(valid_publisher_status("approved"));
        assert!(valid_publisher_status("suspended"));
        assert!(!valid_publisher_status("PENDING"));
        assert!(!valid_publisher_status(""));
        assert!(!valid_publisher_status("pending,approved"));
        assert!(!valid_publisher_status("pending' OR '1'='1"));
    }

    #[test]
    fn overview_response_uses_stable_camel_case_keys() {
        // Contract guard: the web client keys off these names. Changing them
        // requires bumping ADMIN_READ_CONTRACT_VERSION and updating web/admin.
        let body = serde_json::to_value(Overview {
            contract_version: ADMIN_READ_CONTRACT_VERSION,
            publishers: PublisherCounts {
                pending: 1,
                approved: 2,
                suspended: 3,
            },
            releases: ReleaseCounts {
                awaiting_verification: 0,
                integrity_passed: 0,
                antivirus_passed: 4,
                review_approved: 5,
                review_rejected: 6,
                rejected: 0,
            },
            pending_keys: 7,
            audit_events: 8,
        })
        .unwrap();
        assert_eq!(
            body,
            serde_json::json!({
                "contractVersion": 1,
                "publishers": {"pending": 1, "approved": 2, "suspended": 3},
                "releases": {
                    "awaitingVerification": 0,
                    "integrityPassed": 0,
                    "antivirusPassed": 4,
                    "reviewApproved": 5,
                    "reviewRejected": 6,
                    "rejected": 0
                },
                "pendingKeys": 7,
                "auditEvents": 8
            })
        );
    }

    #[test]
    fn audit_event_projection_contains_no_secret_bearing_keys() {
        // Redaction guard: only opaque IDs, role names, actions, and links.
        // Never emails, tokens, object keys, version IDs, or digests.
        let body = serde_json::to_value(AuditEventView {
            id: Uuid::nil(),
            action: "publisher.approved".into(),
            actor_user_id: "user-1".into(),
            target_user_id: None,
            publisher_id: None,
            subject_id: None,
            old_role: None,
            new_role: None,
            created_at: "2026-01-01T00:00:00Z".into(),
        })
        .unwrap();
        let keys: std::collections::BTreeSet<&str> = body
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            [
                "action",
                "actorUserId",
                "createdAt",
                "id",
                "newRole",
                "oldRole",
                "publisherId",
                "subjectId",
                "targetUserId"
            ]
            .into_iter()
            .collect()
        );
        let raw = body.to_string().to_lowercase();
        for leaked in [
            "email",
            "token",
            "secret",
            "password",
            "objectkey",
            "versionid",
            "digest",
            "cookie",
        ] {
            assert!(!raw.contains(leaked), "audit projection leaks {leaked}");
        }
    }
}
