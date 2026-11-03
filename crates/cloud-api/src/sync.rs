//! Revisioned per-user sync. Only explicit, credential-free schemas may be added.
use crate::AppState;
use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    Json,
};
use better_auth::{
    axum_integration::{AuthUser, NativeAuthUser},
    rate_limit::{RateLimitPolicy, RateLimiter},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{Postgres, Row, Transaction};
use std::time::Duration;

pub async fn no_store(mut response: axum::response::Response) -> axum::response::Response {
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    response
}

/// Sync rate policies all use a 60-second window. Tell clients a conservative
/// retry delay without adding rate headers to unrelated endpoints.
pub async fn sync_response(response: axum::response::Response) -> axum::response::Response {
    let mut response = no_store(response).await;
    if response.status() == StatusCode::TOO_MANY_REQUESTS {
        response
            .headers_mut()
            .insert(header::RETRY_AFTER, header::HeaderValue::from_static("60"));
    }
    response
}

const COLLECTION: &str = "plugin-selections";
const PAGE_LIMIT: i64 = 100;
const MAX_RECORDS: i64 = 1000;
const MAX_CHANGES: i64 = 10_000;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginSelection {
    schema_version: u8,
    plugin_id: String,
    version: String,
    enabled: bool,
}

fn valid_plugin_id(id: &str) -> bool {
    id.len() <= 128
        && id.split_once('.').is_some()
        && id.split('.').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-' || c == b'_')
        })
}
fn validate(name: &str, key: &str, value: &Value) -> bool {
    if name != COLLECTION || !valid_plugin_id(key) {
        return false;
    }
    let Ok(input) = serde_json::from_value::<PluginSelection>(value.clone()) else {
        return false;
    };
    let _ = input.enabled;
    input.schema_version == 1
        && input.plugin_id == key
        && input.version.len() <= 64
        && semver::Version::parse(&input.version).is_ok()
}
fn internal(_: sqlx::Error) -> StatusCode {
    tracing::error!("sync database operation failed");
    StatusCode::INTERNAL_SERVER_ERROR
}

async fn limit(state: &AppState, user: &str, action: &str, count: i64) -> Result<(), StatusCode> {
    let backend = state
        .auth
        .secondary_storage
        .clone()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let policy = RateLimitPolicy::new(action, count, Duration::from_secs(60))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    RateLimiter::new(backend, "cloud:sync")
        .enforce(&policy, user)
        .await
        .map_err(|error| match error {
            better_auth_core::error::AuthError::RateLimited { .. } => StatusCode::TOO_MANY_REQUESTS,
            _ => StatusCode::SERVICE_UNAVAILABLE,
        })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    collection: String,
    key: String,
    revision: i64,
    deleted: bool,
    value: Option<Value>,
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

/// A bounded, consistent export of the Cloud-owned sync dataset only.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncExport {
    schema_version: u8,
    cursor: i64,
    records: Vec<Record>,
}

pub async fn export(
    AuthUser(user): AuthUser,
    State(state): State<AppState>,
) -> Result<Json<SyncExport>, StatusCode> {
    limit(&state, &user.user.id, "export", 30).await?;
    let mut tx = state.pool.begin().await.map_err(internal)?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    let cursor: i64 = sqlx::query_scalar(
        "SELECT COALESCE((SELECT cursor FROM cloud_sync_cursors WHERE user_id=$1), 0)",
    )
    .bind(&user.user.id)
    .fetch_one(&mut *tx)
    .await
    .map_err(internal)?;
    let rows = sqlx::query("SELECT collection,key,revision,deleted,value FROM cloud_sync_records WHERE user_id=$1 ORDER BY collection,key LIMIT $2").bind(&user.user.id).bind(MAX_RECORDS + 1).fetch_all(&mut *tx).await.map_err(internal)?;
    if rows.len() > MAX_RECORDS as usize {
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }
    let records: Vec<_> = rows.into_iter().map(record).collect();
    if records.iter().any(|r| {
        r.collection != COLLECTION
            || !valid_plugin_id(&r.key)
            || r.revision <= 0
            || r.revision > cursor
            || if r.deleted {
                r.value.is_some()
            } else {
                r.value
                    .as_ref()
                    .is_none_or(|value| !validate(&r.collection, &r.key, value))
            }
    }) {
        tracing::error!("sync export contains invalid or unsupported data");
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }
    tx.commit().await.map_err(internal)?;
    Ok(Json(SyncExport {
        schema_version: 1,
        cursor,
        records,
    }))
}

pub async fn native_export(
    NativeAuthUser(user): NativeAuthUser,
    state: State<AppState>,
) -> Result<Json<SyncExport>, StatusCode> {
    export(AuthUser(user), state).await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListQuery {
    after: Option<String>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListResponse {
    records: Vec<Record>,
    next_after: Option<String>,
}
pub async fn list(
    AuthUser(user): AuthUser,
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(query): Query<ListQuery>,
) -> Result<Json<ListResponse>, StatusCode> {
    if name != COLLECTION || query.after.as_ref().is_some_and(|s| !valid_plugin_id(s)) {
        return Err(StatusCode::BAD_REQUEST);
    }
    limit(&state, &user.user.id, "read", 300).await?;
    let rows = sqlx::query("SELECT collection, key, revision, deleted, value FROM cloud_sync_records WHERE user_id = $1 AND collection = $2 AND key > $3 ORDER BY key LIMIT $4")
        .bind(&user.user.id).bind(&name).bind(query.after.unwrap_or_default()).bind(PAGE_LIMIT + 1).fetch_all(&state.pool).await.map_err(internal)?;
    let has_more = rows.len() > PAGE_LIMIT as usize;
    let records: Vec<_> = rows
        .into_iter()
        .take(PAGE_LIMIT as usize)
        .map(record)
        .collect();
    let next_after = if has_more {
        records.last().map(|r| r.key.clone())
    } else {
        None
    };
    Ok(Json(ListResponse {
        records,
        next_after,
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChangesQuery {
    cursor: Option<i64>,
}
#[derive(Serialize)]
pub struct ChangesResponse {
    changes: Vec<Record>,
    cursor: i64,
    has_more: bool,
}
#[derive(Serialize)]
pub struct SyncHead {
    cursor: i64,
}
pub async fn head(
    AuthUser(user): AuthUser,
    State(state): State<AppState>,
) -> Result<Json<SyncHead>, StatusCode> {
    limit(&state, &user.user.id, "read", 300).await?;
    let cursor: i64 = sqlx::query_scalar(
        "SELECT COALESCE((SELECT cursor FROM cloud_sync_cursors WHERE user_id=$1), 0)",
    )
    .bind(&user.user.id)
    .fetch_one(&state.pool)
    .await
    .map_err(internal)?;
    Ok(Json(SyncHead { cursor }))
}
pub async fn changes(
    AuthUser(user): AuthUser,
    State(state): State<AppState>,
    Query(query): Query<ChangesQuery>,
) -> Result<Json<ChangesResponse>, StatusCode> {
    let cursor = query.cursor.unwrap_or(0);
    if cursor < 0 {
        return Err(StatusCode::BAD_REQUEST);
    }
    limit(&state, &user.user.id, "read", 300).await?;
    // Hold a shared cursor lock while checking the retention boundary and reading.
    // A concurrent writer cannot prune rows between those two operations.
    let mut tx = state.pool.begin().await.map_err(internal)?;
    let current: Option<i64> =
        sqlx::query_scalar("SELECT cursor FROM cloud_sync_cursors WHERE user_id = $1 FOR SHARE")
            .bind(&user.user.id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(internal)?;
    let Some(current) = current else {
        if cursor != 0 {
            return Err(StatusCode::BAD_REQUEST);
        }
        // There was no cursor row to lock. Return immediately so a concurrent
        // first write cannot race retention and make us skip its early changes.
        return Ok(Json(ChangesResponse {
            changes: Vec::new(),
            cursor: 0,
            has_more: false,
        }));
    };
    if cursor > current {
        return Err(StatusCode::BAD_REQUEST);
    }
    if cursor < current.saturating_sub(MAX_CHANGES) {
        return Err(StatusCode::GONE);
    }
    let rows = sqlx::query("SELECT collection, key, cursor AS revision, deleted, value FROM cloud_sync_changes WHERE user_id = $1 AND cursor > $2 ORDER BY cursor LIMIT $3")
        .bind(&user.user.id).bind(cursor).bind(PAGE_LIMIT + 1).fetch_all(&mut *tx).await.map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    let has_more = rows.len() > PAGE_LIMIT as usize;
    let changes: Vec<_> = rows
        .into_iter()
        .take(PAGE_LIMIT as usize)
        .map(record)
        .collect();
    let cursor = changes.last().map_or(cursor, |r| r.revision);
    Ok(Json(ChangesResponse {
        changes,
        cursor,
        has_more,
    }))
}

// Create is explicit; update/delete must supply exactly one strong revision ETag.
enum Precondition {
    Create,
    Revision(i64),
}
fn precondition(headers: &HeaderMap) -> Result<Precondition, StatusCode> {
    let if_match: Vec<_> = headers.get_all(header::IF_MATCH).iter().collect();
    let if_none: Vec<_> = headers.get_all(header::IF_NONE_MATCH).iter().collect();
    match (if_match.as_slice(), if_none.as_slice()) {
        ([], [value]) if value.as_bytes() == b"*" => Ok(Precondition::Create),
        ([value], []) => {
            let s = value.to_str().map_err(|_| StatusCode::BAD_REQUEST)?;
            let raw = s
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
                .ok_or(StatusCode::BAD_REQUEST)?;
            let n = raw
                .parse::<i64>()
                .ok()
                .filter(|n| *n > 0 && n.to_string() == raw)
                .ok_or(StatusCode::BAD_REQUEST)?;
            Ok(Precondition::Revision(n))
        }
        ([], []) => Err(StatusCode::PRECONDITION_REQUIRED),
        _ => Err(StatusCode::BAD_REQUEST),
    }
}

pub async fn put(
    AuthUser(user): AuthUser,
    State(state): State<AppState>,
    Path((name, key)): Path<(String, String)>,
    headers: HeaderMap,
    Json(value): Json<Value>,
) -> Result<(StatusCode, HeaderMap, Json<Record>), StatusCode> {
    if !validate(&name, &key, &value) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let condition = precondition(&headers)?;
    mutate(&state, &user.user.id, name, key, Some(value), condition).await
}
pub async fn delete(
    AuthUser(user): AuthUser,
    State(state): State<AppState>,
    Path((name, key)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<(StatusCode, HeaderMap, Json<Record>), StatusCode> {
    if name != COLLECTION || !valid_plugin_id(&key) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let condition = precondition(&headers)?;
    if matches!(condition, Precondition::Create) {
        return Err(StatusCode::BAD_REQUEST);
    }
    mutate(&state, &user.user.id, name, key, None, condition).await
}
async fn mutate(
    state: &AppState,
    user: &str,
    collection: String,
    key: String,
    value: Option<Value>,
    condition: Precondition,
) -> Result<(StatusCode, HeaderMap, Json<Record>), StatusCode> {
    limit(state, user, "write", 120).await?;
    let mut tx: Transaction<'_, Postgres> = state.pool.begin().await.map_err(internal)?;
    sqlx::query("INSERT INTO cloud_sync_cursors (user_id) VALUES ($1) ON CONFLICT DO NOTHING")
        .bind(user)
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    sqlx::query("SELECT cursor FROM cloud_sync_cursors WHERE user_id = $1 FOR UPDATE")
        .bind(user)
        .fetch_one(&mut *tx)
        .await
        .map_err(internal)?;
    let existing: Option<i64> = sqlx::query_scalar(
        "SELECT revision FROM cloud_sync_records WHERE user_id=$1 AND collection=$2 AND key=$3",
    )
    .bind(user)
    .bind(&collection)
    .bind(&key)
    .fetch_optional(&mut *tx)
    .await
    .map_err(internal)?;
    match condition {
        Precondition::Create if existing.is_none() => (),
        Precondition::Revision(n) if existing == Some(n) => (),
        _ => return Err(StatusCode::CONFLICT),
    }
    if existing.is_none() {
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM cloud_sync_records WHERE user_id = $1")
                .bind(user)
                .fetch_one(&mut *tx)
                .await
                .map_err(internal)?;
        if count >= MAX_RECORDS {
            return Err(StatusCode::INSUFFICIENT_STORAGE);
        }
    }
    let revision: i64 = sqlx::query_scalar(
        "UPDATE cloud_sync_cursors SET cursor = cursor + 1 WHERE user_id=$1 RETURNING cursor",
    )
    .bind(user)
    .fetch_one(&mut *tx)
    .await
    .map_err(internal)?;
    let deleted = value.is_none();
    sqlx::query("INSERT INTO cloud_sync_records(user_id,collection,key,revision,value,deleted) VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT (user_id,collection,key) DO UPDATE SET revision=EXCLUDED.revision,value=EXCLUDED.value,deleted=EXCLUDED.deleted")
        .bind(user).bind(&collection).bind(&key).bind(revision).bind(&value).bind(deleted).execute(&mut *tx).await.map_err(internal)?;
    sqlx::query("INSERT INTO cloud_sync_changes(user_id,cursor,collection,key,value,deleted) VALUES($1,$2,$3,$4,$5,$6)")
        .bind(user).bind(revision).bind(&collection).bind(&key).bind(&value).bind(deleted).execute(&mut *tx).await.map_err(internal)?;
    // Prune in the same transaction as the cursor advance. Clients below the
    // retained boundary get 410 and must rebuild from the collection listing.
    sqlx::query("DELETE FROM cloud_sync_changes WHERE user_id = $1 AND cursor <= $2")
        .bind(user)
        .bind(revision.saturating_sub(MAX_CHANGES))
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    let mut headers = HeaderMap::new();
    headers.insert(
        header::ETAG,
        format!("\"{revision}\"")
            .parse()
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
    );
    Ok((
        if existing.is_none() {
            StatusCode::CREATED
        } else {
            StatusCode::OK
        },
        headers,
        Json(Record {
            collection,
            key,
            revision,
            deleted,
            value,
        }),
    ))
}

// Native routes opt in explicitly to bearer-only authentication. These adapters
// share authorization and storage logic with the cookie routes; they never read cookies.
pub async fn native_list(
    NativeAuthUser(user): NativeAuthUser,
    state: State<AppState>,
    path: Path<String>,
    query: Query<ListQuery>,
) -> Result<Json<ListResponse>, StatusCode> {
    list(AuthUser(user), state, path, query).await
}
pub async fn native_head(
    NativeAuthUser(user): NativeAuthUser,
    state: State<AppState>,
) -> Result<Json<SyncHead>, StatusCode> {
    head(AuthUser(user), state).await
}
pub async fn native_changes(
    NativeAuthUser(user): NativeAuthUser,
    state: State<AppState>,
    query: Query<ChangesQuery>,
) -> Result<Json<ChangesResponse>, StatusCode> {
    changes(AuthUser(user), state, query).await
}
pub async fn native_put(
    NativeAuthUser(user): NativeAuthUser,
    state: State<AppState>,
    path: Path<(String, String)>,
    headers: HeaderMap,
    body: Json<Value>,
) -> Result<(StatusCode, HeaderMap, Json<Record>), StatusCode> {
    put(AuthUser(user), state, path, headers, body).await
}
pub async fn native_delete(
    NativeAuthUser(user): NativeAuthUser,
    state: State<AppState>,
    path: Path<(String, String)>,
    headers: HeaderMap,
) -> Result<(StatusCode, HeaderMap, Json<Record>), StatusCode> {
    delete(AuthUser(user), state, path, headers).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_credentials_and_unreviewed_fields() {
        let good = serde_json::json!({"schemaVersion":1,"pluginId":"acme.plugin","version":"1.2.3","enabled":true});
        assert!(validate(COLLECTION, "acme.plugin", &good));
        let mut bad = good.clone();
        bad["token"] = serde_json::json!("secret");
        assert!(!validate(COLLECTION, "acme.plugin", &bad));
        assert!(!validate("profiles", "acme.plugin", &good));
        assert!(!validate(COLLECTION, "other.plugin", &good));
    }
}
