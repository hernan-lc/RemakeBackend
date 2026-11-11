//! Cross-node coordination for the relay.
//!
//! A connection lives on one node. Its lease names that node and carries a
//! fencing token that only grows when ownership moves. Frames addressed to a
//! connection on another node are queued in PostgreSQL and drained by the owner.
//! A node that cannot renew its lease must stop serving that connection.

use serde_json::Value;
use sqlx::{PgPool, Row};
use std::time::Duration;
use uuid::Uuid;

/// A lease lapses this long after its last renewal, so a crashed node's
/// connections are reclaimed without manual intervention.
pub const LEASE_TTL: Duration = Duration::from_secs(30);
pub const RENEW_EVERY: Duration = Duration::from_secs(10);
pub const DRAIN_EVERY: Duration = Duration::from_millis(200);
pub const MAX_QUEUED_FRAME_BYTES: usize = 128 * 1024;
const FRAME_RETENTION_SECONDS: i64 = 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Host,
    Session,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Host => "host",
            Kind::Session => "session",
        }
    }
}

/// Records this node as alive. Called at start and on every renewal.
pub async fn heartbeat(pool: &PgPool, node: &str) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO cloud_relay_nodes(node_id,heartbeat_at) VALUES($1,now()) ON CONFLICT (node_id) DO UPDATE SET heartbeat_at=now()")
        .bind(node)
        .execute(pool)
        .await?;
    Ok(())
}

/// Takes ownership of a connection. A host connection may move from a lapsed or
/// different node; a session never moves. Returns the new fencing token.
pub async fn acquire(
    pool: &PgPool,
    node: &str,
    kind: Kind,
    id: Uuid,
) -> Result<Option<i64>, sqlx::Error> {
    let ttl = LEASE_TTL.as_secs_f64();
    let row = sqlx::query(
        "INSERT INTO cloud_relay_leases(resource_kind,resource_id,node_id,fencing_token,expires_at) \
         VALUES($1,$2,$3,1,now() + make_interval(secs => $4)) \
         ON CONFLICT (resource_kind,resource_id) DO UPDATE \
         SET node_id=EXCLUDED.node_id, fencing_token=cloud_relay_leases.fencing_token+1, expires_at=EXCLUDED.expires_at \
         WHERE cloud_relay_leases.expires_at < now() OR (cloud_relay_leases.node_id=EXCLUDED.node_id AND $1='host') \
         RETURNING fencing_token",
    )
    .bind(kind.as_str())
    .bind(id)
    .bind(node)
    .bind(ttl)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| r.get::<i64, _>("fencing_token")))
}

/// Extends a lease only while this node still holds the same fencing token.
/// `false` means ownership was lost and the connection must be closed locally.
pub async fn renew(
    pool: &PgPool,
    node: &str,
    kind: Kind,
    id: Uuid,
    fencing: i64,
) -> Result<bool, sqlx::Error> {
    let ttl = LEASE_TTL.as_secs_f64();
    let result = sqlx::query("UPDATE cloud_relay_leases SET expires_at=now() + make_interval(secs => $5) WHERE resource_kind=$1 AND resource_id=$2 AND node_id=$3 AND fencing_token=$4 AND expires_at > now()")
        .bind(kind.as_str())
        .bind(id)
        .bind(node)
        .bind(fencing)
        .bind(ttl)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() == 1)
}

pub async fn release(
    pool: &PgPool,
    node: &str,
    kind: Kind,
    id: Uuid,
    fencing: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM cloud_relay_leases WHERE resource_kind=$1 AND resource_id=$2 AND node_id=$3 AND fencing_token=$4")
        .bind(kind.as_str())
        .bind(id)
        .bind(node)
        .bind(fencing)
        .execute(pool)
        .await?;
    Ok(())
}

/// The node that owns a live lease, if any.
pub async fn owner(pool: &PgPool, kind: Kind, id: Uuid) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT node_id FROM cloud_relay_leases WHERE resource_kind=$1 AND resource_id=$2 AND expires_at > now()")
        .bind(kind.as_str())
        .bind(id)
        .fetch_optional(pool)
        .await
}

/// Queues a frame for the node that owns `id`. Oversized frames are refused.
pub async fn enqueue(
    pool: &PgPool,
    target: &str,
    kind: Kind,
    id: Uuid,
    frame: &Value,
) -> Result<bool, sqlx::Error> {
    let payload = frame.to_string();
    if payload.len() > MAX_QUEUED_FRAME_BYTES {
        return Ok(false);
    }
    sqlx::query("INSERT INTO cloud_relay_frames(target_node,resource_kind,resource_id,payload) VALUES($1,$2,$3,$4)")
        .bind(target)
        .bind(kind.as_str())
        .bind(id)
        .bind(payload)
        .execute(pool)
        .await?;
    Ok(true)
}

/// Removes and returns frames addressed to `node`. Concurrent drains never
/// deliver the same frame twice.
pub async fn drain(
    pool: &PgPool,
    node: &str,
    limit: i64,
) -> Result<Vec<(Kind, Uuid, Value)>, sqlx::Error> {
    let rows = sqlx::query(
        "DELETE FROM cloud_relay_frames WHERE id IN (SELECT id FROM cloud_relay_frames WHERE target_node=$1 ORDER BY id LIMIT $2 FOR UPDATE SKIP LOCKED) \
         RETURNING resource_kind,resource_id,payload",
    )
    .bind(node)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    let mut frames = Vec::with_capacity(rows.len());
    for row in rows {
        let kind = if row.get::<String, _>("resource_kind") == "host" {
            Kind::Host
        } else {
            Kind::Session
        };
        let payload: String = row.get("payload");
        if let Ok(value) = serde_json::from_str::<Value>(&payload) {
            frames.push((kind, row.get("resource_id"), value));
        }
    }
    Ok(frames)
}

/// Drops expired leases, dead nodes' leases, and old frames nobody drained.
pub async fn reap(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query(
        "DELETE FROM cloud_relay_nodes WHERE heartbeat_at < now() - make_interval(secs => $1)",
    )
    .bind(LEASE_TTL.as_secs_f64() * 2.0)
    .execute(pool)
    .await?;
    sqlx::query(
        "DELETE FROM cloud_relay_leases WHERE expires_at < now() - make_interval(secs => $1)",
    )
    .bind(LEASE_TTL.as_secs_f64())
    .execute(pool)
    .await?;
    sqlx::query(
        "DELETE FROM cloud_relay_frames WHERE created_at < now() - make_interval(secs => $1)",
    )
    .bind(FRAME_RETENTION_SECONDS as f64)
    .execute(pool)
    .await?;
    Ok(())
}
