//! Versioned workspace synchronization, isolated from the plugin-only v1 feed.
use crate::{dual_auth::DualUser, portable, AppState};
use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    Json,
};
pub use cloud_contracts::WorkspaceRecord as Record;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{PgPool, Row};

pub const MAX_RECORDS: i64 = 1000;
pub const MAX_BYTES: i64 = 8 * 1024 * 1024;
const MAX_CHANGES: i64 = 10_000;
const MAX_CHANGE_BYTES: i64 = 16 * 1024 * 1024;
const PAGE: i64 = 50;

pub(crate) fn db(_: sqlx::Error) -> StatusCode {
    tracing::error!("cloud workspace database operation failed");
    StatusCode::INTERNAL_SERVER_ERROR
}
pub(crate) async fn limit(
    state: &AppState,
    user: &str,
    action: &str,
    count: i64,
) -> Result<(), StatusCode> {
    use better_auth::rate_limit::{RateLimitPolicy, RateLimiter};
    let storage = state
        .auth
        .secondary_storage
        .clone()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let policy = RateLimitPolicy::new(action, count, std::time::Duration::from_secs(60))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    RateLimiter::new(storage, "cloud:workspace")
        .enforce(&policy, user)
        .await
        .map_err(|e| match e {
            better_auth_core::error::AuthError::RateLimited { .. } => StatusCode::TOO_MANY_REQUESTS,
            _ => StatusCode::SERVICE_UNAVAILABLE,
        })
}

fn record(row: sqlx::postgres::PgRow) -> Record {
    Record {
        collection: row.get("collection"),
        key: row.get("key"),
        revision: row.get("revision"),
        deleted: row.get("deleted"),
        value: row.get("value"),
    }
}
pub(crate) fn valid_record(record: &Record, cursor: i64) -> bool {
    record.valid(cursor)
}
pub(crate) fn etag(revision: i64) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::ETAG,
        format!("\"{revision}\"").parse().expect("integer ETag"),
    );
    headers
}
/// None means create-only; Some(n) means replace exactly revision n.
pub(crate) fn precondition(headers: &HeaderMap) -> Result<Option<i64>, StatusCode> {
    let matches: Vec<_> = headers.get_all(header::IF_MATCH).iter().collect();
    let absent: Vec<_> = headers.get_all(header::IF_NONE_MATCH).iter().collect();
    match (matches.as_slice(), absent.as_slice()) {
        ([], [v]) if v.as_bytes() == b"*" => Ok(None),
        ([v], []) => {
            let raw = v
                .to_str()
                .ok()
                .and_then(|s| s.strip_prefix('"'))
                .and_then(|s| s.strip_suffix('"'))
                .ok_or(StatusCode::BAD_REQUEST)?;
            raw.parse::<i64>()
                .ok()
                .filter(|n| *n > 0 && n.to_string() == raw)
                .map(Some)
                .ok_or(StatusCode::BAD_REQUEST)
        }
        ([], []) => Err(StatusCode::PRECONDITION_REQUIRED),
        _ => Err(StatusCode::BAD_REQUEST),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Page {
    after: Option<String>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Listing {
    records: Vec<Record>,
    next_after: Option<String>,
}
pub async fn list(
    DualUser(user): DualUser,
    State(state): State<AppState>,
    Path(collection): Path<String>,
    Query(page): Query<Page>,
) -> Result<Json<Listing>, StatusCode> {
    if !portable::COLLECTIONS.contains(&collection.as_str())
        || page
            .after
            .as_ref()
            .is_some_and(|key| !portable::valid_key(&collection, key))
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    limit(&state, &user.user.id, "read", 300).await?;
    let rows = sqlx::query(r#"SELECT collection,key,revision,deleted,value FROM cloud_workspace_records WHERE user_id=$1 AND collection=$2 AND key COLLATE "C">$3 ORDER BY key COLLATE "C" LIMIT $4"#)
        .bind(&user.user.id).bind(collection).bind(page.after.unwrap_or_default()).bind(PAGE + 1).fetch_all(&state.pool).await.map_err(db)?;
    let more = rows.len() > PAGE as usize;
    let records: Vec<_> = rows.into_iter().take(PAGE as usize).map(record).collect();
    let next_after = if more {
        records.last().map(|r| r.key.clone())
    } else {
        None
    };
    Ok(Json(Listing {
        records,
        next_after,
    }))
}
pub async fn get(
    DualUser(user): DualUser,
    State(state): State<AppState>,
    Path((collection, key)): Path<(String, String)>,
) -> Result<(HeaderMap, Json<Record>), StatusCode> {
    if !portable::valid_key(&collection, &key) {
        return Err(StatusCode::BAD_REQUEST);
    }
    limit(&state, &user.user.id, "read", 300).await?;
    let row = sqlx::query("SELECT collection,key,revision,deleted,value FROM cloud_workspace_records WHERE user_id=$1 AND collection=$2 AND key=$3")
        .bind(&user.user.id).bind(collection).bind(key).fetch_optional(&state.pool).await.map_err(db)?.ok_or(StatusCode::NOT_FOUND)?;
    let r = record(row);
    Ok((etag(r.revision), Json(r)))
}
pub async fn put(
    DualUser(user): DualUser,
    State(state): State<AppState>,
    Path((collection, key)): Path<(String, String)>,
    headers: HeaderMap,
    Json(value): Json<Value>,
) -> Result<(StatusCode, HeaderMap, Json<Record>), StatusCode> {
    if !portable::validate(&collection, &key, &value) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let expected = precondition(&headers)?;
    mutate(
        &state,
        &user.user.id,
        collection,
        key,
        Some(value),
        expected,
    )
    .await
}
pub async fn delete(
    DualUser(user): DualUser,
    State(state): State<AppState>,
    Path((collection, key)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<(StatusCode, HeaderMap, Json<Record>), StatusCode> {
    if !portable::valid_key(&collection, &key) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let expected = precondition(&headers)?.ok_or(StatusCode::BAD_REQUEST)?;
    mutate(&state, &user.user.id, collection, key, None, Some(expected)).await
}
async fn mutate(
    state: &AppState,
    user: &str,
    collection: String,
    key: String,
    value: Option<Value>,
    expected: Option<i64>,
) -> Result<(StatusCode, HeaderMap, Json<Record>), StatusCode> {
    limit(state, user, "write", 120).await?;
    let mut tx = state.pool.begin().await.map_err(db)?;
    // JSONB normalizes numeric representations. Validate and acknowledge the
    // exact typed value that subsequent GET/snapshot/feed reads will decode.
    let value = match value {
        Some(input) => {
            let stored: Value = sqlx::query_scalar("SELECT $1::jsonb")
                .bind(input)
                .fetch_one(&mut *tx)
                .await
                .map_err(db)?;
            if !portable::validate(&collection, &key, &stored) {
                return Err(StatusCode::BAD_REQUEST);
            }
            Some(stored)
        }
        None => None,
    };
    sqlx::query("INSERT INTO cloud_workspace_cursors(user_id) VALUES($1) ON CONFLICT DO NOTHING")
        .bind(user)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    sqlx::query("SELECT cursor FROM cloud_workspace_cursors WHERE user_id=$1 FOR UPDATE")
        .bind(user)
        .fetch_one(&mut *tx)
        .await
        .map_err(db)?;
    let previous = sqlx::query("SELECT revision,value,deleted FROM cloud_workspace_records WHERE user_id=$1 AND collection=$2 AND key=$3").bind(user).bind(&collection).bind(&key).fetch_optional(&mut *tx).await.map_err(db)?;
    let revision = previous.as_ref().map(|r| r.get::<i64, _>("revision"));
    if expected != revision {
        return Err(StatusCode::CONFLICT);
    }
    // A retried no-op preserves its revision and does not create a sync loop.
    if previous.as_ref().is_some_and(|r| {
        let same_value = match (r.get::<Option<Value>, _>("value").as_ref(), value.as_ref()) {
            (Some(previous), Some(next)) => portable::values_equivalent(previous, next),
            (None, None) => true,
            _ => false,
        };
        same_value && r.get::<bool, _>("deleted") == value.is_none()
    }) {
        let revision = revision.expect("existing revision");
        tx.commit().await.map_err(db)?;
        return Ok((
            StatusCode::OK,
            etag(revision),
            Json(Record {
                collection,
                key,
                revision,
                deleted: value.is_none(),
                value: previous.as_ref().expect("existing record").get("value"),
            }),
        ));
    }
    let quota = sqlx::query("SELECT count(*) AS records, COALESCE(sum(octet_length(value::text)),0)::bigint AS bytes FROM cloud_workspace_records WHERE user_id=$1").bind(user).fetch_one(&mut *tx).await.map_err(db)?;
    let previous_bytes: i64 = sqlx::query_scalar("SELECT COALESCE(octet_length(value::text),0)::bigint FROM cloud_workspace_records WHERE user_id=$1 AND collection=$2 AND key=$3").bind(user).bind(&collection).bind(&key).fetch_optional(&mut *tx).await.map_err(db)?.unwrap_or(0);
    let new_bytes: i64 =
        sqlx::query_scalar("SELECT COALESCE(octet_length($1::jsonb::text),0)::bigint")
            .bind(&value)
            .fetch_one(&mut *tx)
            .await
            .map_err(db)?;
    if (previous.is_none() && quota.get::<i64, _>("records") >= MAX_RECORDS)
        || quota.get::<i64, _>("bytes") - previous_bytes + new_bytes > MAX_BYTES
    {
        return Err(StatusCode::INSUFFICIENT_STORAGE);
    }
    let revision: i64 = sqlx::query_scalar(
        "UPDATE cloud_workspace_cursors SET cursor=cursor+1 WHERE user_id=$1 RETURNING cursor",
    )
    .bind(user)
    .fetch_one(&mut *tx)
    .await
    .map_err(db)?;
    let deleted = value.is_none();
    sqlx::query("INSERT INTO cloud_workspace_records(user_id,collection,key,revision,value,deleted) VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT(user_id,collection,key) DO UPDATE SET revision=EXCLUDED.revision,value=EXCLUDED.value,deleted=EXCLUDED.deleted").bind(user).bind(&collection).bind(&key).bind(revision).bind(&value).bind(deleted).execute(&mut *tx).await.map_err(db)?;
    sqlx::query("INSERT INTO cloud_workspace_changes(user_id,cursor,collection,key,value,deleted) VALUES($1,$2,$3,$4,$5,$6)").bind(user).bind(revision).bind(&collection).bind(&key).bind(&value).bind(deleted).execute(&mut *tx).await.map_err(db)?;
    sqlx::query("DELETE FROM cloud_workspace_changes WHERE user_id=$1 AND cursor IN (SELECT cursor FROM (SELECT cursor, row_number() OVER (ORDER BY cursor DESC) AS n, sum(COALESCE(octet_length(value::text),0)+256) OVER (ORDER BY cursor DESC) AS bytes FROM cloud_workspace_changes WHERE user_id=$1) retained WHERE n>$2 OR bytes>$3)").bind(user).bind(MAX_CHANGES).bind(MAX_CHANGE_BYTES).execute(&mut *tx).await.map_err(db)?;
    tx.commit().await.map_err(db)?;
    Ok((
        if previous.is_none() {
            StatusCode::CREATED
        } else {
            StatusCode::OK
        },
        etag(revision),
        Json(Record {
            collection,
            key,
            revision,
            deleted,
            value,
        }),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChangesQuery {
    cursor: Option<i64>,
}
pub async fn head(
    DualUser(user): DualUser,
    State(state): State<AppState>,
) -> Result<Json<Value>, StatusCode> {
    limit(&state, &user.user.id, "read", 300).await?;
    let cursor: i64 = sqlx::query_scalar(
        "SELECT COALESCE((SELECT cursor FROM cloud_workspace_cursors WHERE user_id=$1),0)",
    )
    .bind(&user.user.id)
    .fetch_one(&state.pool)
    .await
    .map_err(db)?;
    Ok(Json(serde_json::json!({"cursor":cursor})))
}
pub async fn changes(
    DualUser(user): DualUser,
    State(state): State<AppState>,
    Query(query): Query<ChangesQuery>,
) -> Result<Json<Value>, StatusCode> {
    let cursor = query.cursor.unwrap_or(0);
    if cursor < 0 {
        return Err(StatusCode::BAD_REQUEST);
    }
    limit(&state, &user.user.id, "read", 300).await?;
    let mut tx = state.pool.begin().await.map_err(db)?;
    let current: Option<i64> =
        sqlx::query_scalar("SELECT cursor FROM cloud_workspace_cursors WHERE user_id=$1 FOR SHARE")
            .bind(&user.user.id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(db)?;
    let Some(current) = current else {
        if cursor != 0 {
            return Err(StatusCode::BAD_REQUEST);
        }
        return Ok(Json(
            serde_json::json!({"changes":[],"cursor":0,"has_more":false}),
        ));
    };
    if cursor > current {
        return Err(StatusCode::BAD_REQUEST);
    }
    let earliest: Option<i64> =
        sqlx::query_scalar("SELECT min(cursor) FROM cloud_workspace_changes WHERE user_id=$1")
            .bind(&user.user.id)
            .fetch_one(&mut *tx)
            .await
            .map_err(db)?;
    if earliest.is_some_and(|first| cursor < first.saturating_sub(1))
        || (earliest.is_none() && cursor < current)
    {
        return Err(StatusCode::GONE);
    }
    let rows = sqlx::query("SELECT collection,key,cursor AS revision,deleted,value FROM cloud_workspace_changes WHERE user_id=$1 AND cursor>$2 ORDER BY cursor LIMIT $3").bind(&user.user.id).bind(cursor).bind(PAGE+1).fetch_all(&mut *tx).await.map_err(db)?;
    let more = rows.len() > PAGE as usize;
    let mut changes: Vec<_> = rows.into_iter().map(record).collect();
    if !crate::sync::complete_feed_page(cursor, current, changes.iter().map(|r| r.revision), more) {
        tracing::error!("workspace change history has a missing or invalid cursor");
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }
    changes.truncate(PAGE as usize);
    let cursor = changes.last().map_or(cursor, |r| r.revision);
    tx.commit().await.map_err(db)?;
    Ok(Json(
        serde_json::json!({"changes":changes,"cursor":cursor,"has_more":more}),
    ))
}
pub(crate) async fn snapshot(pool: &PgPool, user: &str) -> Result<Value, StatusCode> {
    let mut tx = pool.begin().await.map_err(db)?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    let cursor: i64 = sqlx::query_scalar(
        "SELECT COALESCE((SELECT cursor FROM cloud_workspace_cursors WHERE user_id=$1),0)",
    )
    .bind(user)
    .fetch_one(&mut *tx)
    .await
    .map_err(db)?;
    let rows = sqlx::query("SELECT collection,key,revision,deleted,value FROM cloud_workspace_records WHERE user_id=$1 ORDER BY collection,key LIMIT $2").bind(user).bind(MAX_RECORDS+1).fetch_all(&mut *tx).await.map_err(db)?;
    let records: Vec<_> = rows.into_iter().map(record).collect();
    if records.len() > MAX_RECORDS as usize || records.iter().any(|r| !valid_record(r, cursor)) {
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }
    tx.commit().await.map_err(db)?;
    Ok(serde_json::json!({"schemaVersion":2,"cursor":cursor,"records":records}))
}
pub async fn export(
    DualUser(user): DualUser,
    State(state): State<AppState>,
) -> Result<Json<Value>, StatusCode> {
    limit(&state, &user.user.id, "export", 10).await?;
    Ok(Json(snapshot(&state.pool, &user.user.id).await?))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_one_canonical_strong_precondition_is_accepted() {
        for value in ["1", "W/\"1\"", "\"01\"", "\"0\"", "*", "\"1\",\"2\""] {
            let mut headers = HeaderMap::new();
            headers.insert(header::IF_MATCH, value.parse().unwrap());
            assert!(precondition(&headers).is_err());
        }
        let mut headers = etag(42);
        headers.remove(header::ETAG);
        headers.insert(header::IF_MATCH, "\"42\"".parse().unwrap());
        assert_eq!(precondition(&headers), Ok(Some(42)));
        headers.append(header::IF_MATCH, "\"42\"".parse().unwrap());
        assert!(precondition(&headers).is_err());
        assert_eq!(
            precondition(&HeaderMap::new()),
            Err(StatusCode::PRECONDITION_REQUIRED)
        );
    }
}
