//! User-owned backup and restore of every cloud domain.
//!
//! Unlike `/api/v2/me/export` (a privacy report), a backup manifest is meant
//! to be restored on another device. Restore is explicit, bounded by the
//! caller's plan and always reported as a summary before it is applied.

use crate::{dual_auth::DualUser, library, portable, workspace, AppState};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::Row;

const BACKUP_SCHEMA_VERSION: u32 = 1;
const MAX_BACKUP_BYTES: usize = 64 * 1024 * 1024;
const MAX_BACKUP_RECORDS: i64 = 5_000;

fn db(_: sqlx::Error) -> StatusCode {
    tracing::error!("cloud backup database operation failed");
    StatusCode::INTERNAL_SERVER_ERROR
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum RestoreMode {
    Replace,
    Merge,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RestoreRequest {
    manifest: Value,
    #[serde(default = "default_mode")]
    mode: RestoreMode,
    #[serde(default)]
    dry_run: bool,
}

fn default_mode() -> RestoreMode {
    RestoreMode::Replace
}

fn array<'a>(value: &'a Value, key: &str) -> Result<&'a Vec<Value>, StatusCode> {
    value
        .get(key)
        .and_then(Value::as_array)
        .ok_or(StatusCode::BAD_REQUEST)
}

/// Build a portable, versioned manifest for one account.
async fn build_manifest(pool: &sqlx::PgPool, owner: &str) -> Result<Value, StatusCode> {
    let mut tx = pool.begin().await.map_err(db)?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
        .execute(&mut *tx)
        .await
        .map_err(db)?;

    let legacy_rows = sqlx::query(
        "SELECT collection,key,revision,deleted,value FROM cloud_sync_records WHERE user_id=$1 ORDER BY collection,key LIMIT $2",
    )
    .bind(owner)
    .bind(MAX_BACKUP_RECORDS + 1)
    .fetch_all(&mut *tx)
    .await
    .map_err(db)?;
    if legacy_rows.len() > MAX_BACKUP_RECORDS as usize {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }
    let legacy: Vec<Value> = legacy_rows
        .iter()
        .map(|row| {
            json!({
                "collection": row.get::<String, _>("collection"),
                "key": row.get::<String, _>("key"),
                "revision": row.get::<i64, _>("revision"),
                "deleted": row.get::<bool, _>("deleted"),
                "value": row.get::<Option<Value>, _>("value"),
            })
        })
        .collect();

    let workspace = workspace::snapshot(pool, owner).await?;
    let library = library::export(pool, owner).await?;

    // Overlays and packages are stored in the library tables but excluded from
    // library::export, so back them up explicitly with all their versions.
    let media_rows = sqlx::query(
        "SELECT d.id,d.kind,d.slug,d.revision,d.published_revision,d.fork_of_document,d.fork_of_revision,v.revision AS v_revision,v.title,v.document,v.sha256,v.status \
         FROM cloud_library_documents d JOIN cloud_library_versions v ON v.document_id=d.id \
         WHERE d.owner_user_id=$1 AND d.kind IN ('overlay','package') ORDER BY d.id,v.revision LIMIT $2",
    )
    .bind(owner)
    .bind(MAX_BACKUP_RECORDS + 1)
    .fetch_all(&mut *tx)
    .await
    .map_err(db)?;
    if media_rows.len() > MAX_BACKUP_RECORDS as usize {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }
    let mut media: Vec<Value> = Vec::new();
    for row in &media_rows {
        media.push(json!({
            "id": row.get::<uuid::Uuid, _>("id").to_string(),
            "kind": row.get::<String, _>("kind"),
            "slug": row.get::<String, _>("slug"),
            "revision": row.get::<i64, _>("revision"),
            "publishedRevision": row.get::<Option<i64>, _>("published_revision"),
            "forkOfDocument": row.get::<Option<uuid::Uuid>, _>("fork_of_document").map(|id| id.to_string()),
            "forkOfRevision": row.get::<Option<i64>, _>("fork_of_revision"),
            "version": {
                "revision": row.get::<i64, _>("v_revision"),
                "title": row.get::<String, _>("title"),
                "document": row.get::<Value, _>("document"),
                "sha256": row.get::<String, _>("sha256"),
                "status": row.get::<String, _>("status"),
            },
        }));
    }

    let asset_rows = sqlx::query(
        "SELECT id,sha256,mime,width,height,size,label,created_at,content FROM cloud_assets WHERE owner_user_id=$1 ORDER BY created_at,id LIMIT $2",
    )
    .bind(owner)
    .bind(MAX_BACKUP_RECORDS + 1)
    .fetch_all(&mut *tx)
    .await
    .map_err(db)?;
    if asset_rows.len() > MAX_BACKUP_RECORDS as usize {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }
    let assets: Vec<Value> = asset_rows
        .iter()
        .map(|row| {
            let bytes: Vec<u8> = row.get("content");
            json!({
                "id": row.get::<uuid::Uuid, _>("id").to_string(),
                "sha256": row.get::<String, _>("sha256"),
                "mime": row.get::<String, _>("mime"),
                "width": row.get::<i32, _>("width"),
                "height": row.get::<i32, _>("height"),
                "size": row.get::<i32, _>("size"),
                "label": row.get::<Option<String>, _>("label"),
                "createdAt": row.get::<chrono::DateTime<chrono::Utc>, _>("created_at").to_rfc3339(),
                "contentBase64": base64::engine::general_purpose::STANDARD.encode(bytes),
            })
        })
        .collect();

    tx.commit().await.map_err(db)?;
    let manifest = json!({
        "schemaVersion": BACKUP_SCHEMA_VERSION,
        "createdAt": chrono::Utc::now().to_rfc3339(),
        "legacySync": { "records": legacy },
        "workspaceSync": workspace,
        "library": library,
        "mediaLibrary": { "documents": media },
        "assets": { "items": assets },
    });
    if serde_json::to_vec(&manifest)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .len()
        > MAX_BACKUP_BYTES
    {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }
    Ok(manifest)
}

/// Build and return a manifest without storing it on the server.
pub async fn backup(
    DualUser(user): DualUser,
    State(state): State<AppState>,
) -> Result<Json<Value>, StatusCode> {
    workspace::limit(&state, &user.user.id, "export", 10).await?;
    Ok(Json(build_manifest(&state.pool, &user.user.id).await?))
}

/// Report (and optionally apply) a restore. `dryRun` powers the one-click
/// summary shown on a fresh device; `replace` skips conflict review there.
async fn apply_restore(
    pool: &sqlx::PgPool,
    owner: &str,
    manifest: &Value,
    mode: RestoreMode,
    dry_run: bool,
) -> Result<Value, StatusCode> {
    if manifest
        .get("schemaVersion")
        .and_then(|value| value.as_u64())
        != Some(BACKUP_SCHEMA_VERSION as u64)
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    let legacy_snapshot = manifest.get("legacySync").ok_or(StatusCode::BAD_REQUEST)?;
    let legacy = array(legacy_snapshot, "records")?;
    let workspace_snapshot = manifest
        .get("workspaceSync")
        .ok_or(StatusCode::BAD_REQUEST)?;
    let workspace_records = array(workspace_snapshot, "records")?;
    let library_items = array(
        manifest.get("library").ok_or(StatusCode::BAD_REQUEST)?,
        "items",
    )?;
    let media_items = array(
        manifest
            .get("mediaLibrary")
            .ok_or(StatusCode::BAD_REQUEST)?,
        "documents",
    )?;
    let assets = array(
        manifest.get("assets").ok_or(StatusCode::BAD_REQUEST)?,
        "items",
    )?;
    if workspace_records.len()
        + legacy.len()
        + library_items.len()
        + media_items.len()
        + assets.len()
        > MAX_BACKUP_RECORDS as usize * 2
    {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }
    let limits = crate::plans::limits_for(pool, owner).await;
    if assets.len() as i64 > limits.max_assets
        || workspace_records.len() as i64 > limits.max_records
    {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }

    let summary = json!({
        "mode": mode,
        "dryRun": dry_run,
        "legacySync": legacy.len(),
        "workspaceSync": workspace_records.len(),
        "library": library_items.len(),
        "mediaLibrary": media_items.len(),
        "assets": assets.len(),
        "applied": !dry_run,
    });
    if dry_run {
        return Ok(summary);
    }

    let mut tx = pool.begin().await.map_err(db)?;
    sqlx::query("INSERT INTO cloud_workspace_cursors(user_id) VALUES($1) ON CONFLICT DO NOTHING")
        .bind(owner)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    if mode == RestoreMode::Replace {
        sqlx::query("DELETE FROM cloud_workspace_records WHERE user_id=$1")
            .bind(owner)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        sqlx::query("DELETE FROM cloud_sync_records WHERE user_id=$1")
            .bind(owner)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
    }
    for record in workspace_records {
        let collection = record
            .get("collection")
            .and_then(Value::as_str)
            .ok_or(StatusCode::BAD_REQUEST)?;
        let key = record
            .get("key")
            .and_then(Value::as_str)
            .ok_or(StatusCode::BAD_REQUEST)?;
        let revision = record
            .get("revision")
            .and_then(Value::as_i64)
            .ok_or(StatusCode::BAD_REQUEST)?;
        let deleted = record
            .get("deleted")
            .and_then(Value::as_bool)
            .ok_or(StatusCode::BAD_REQUEST)?;
        let value = record.get("value").cloned();
        if !portable::validate(collection, key, value.as_ref().unwrap_or(&Value::Null)) && !deleted
        {
            return Err(StatusCode::BAD_REQUEST);
        }
        sqlx::query("INSERT INTO cloud_workspace_records(user_id,collection,key,revision,value,deleted) VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT(user_id,collection,key) DO UPDATE SET revision=EXCLUDED.revision,value=EXCLUDED.value,deleted=EXCLUDED.deleted")
            .bind(owner).bind(collection).bind(key).bind(revision).bind(value).bind(deleted)
            .execute(&mut *tx).await.map_err(db)?;
    }
    for record in legacy {
        let collection = record
            .get("collection")
            .and_then(Value::as_str)
            .ok_or(StatusCode::BAD_REQUEST)?;
        let key = record
            .get("key")
            .and_then(Value::as_str)
            .ok_or(StatusCode::BAD_REQUEST)?;
        let revision = record
            .get("revision")
            .and_then(Value::as_i64)
            .ok_or(StatusCode::BAD_REQUEST)?;
        let deleted = record
            .get("deleted")
            .and_then(Value::as_bool)
            .ok_or(StatusCode::BAD_REQUEST)?;
        let value = record.get("value").cloned();
        sqlx::query("INSERT INTO cloud_sync_records(user_id,collection,key,revision,value,deleted) VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT(user_id,collection,key) DO UPDATE SET revision=EXCLUDED.revision,value=EXCLUDED.value,deleted=EXCLUDED.deleted")
            .bind(owner).bind(collection).bind(key).bind(revision).bind(value).bind(deleted)
            .execute(&mut *tx).await.map_err(db)?;
    }
    let cursor = workspace_snapshot
        .get("cursor")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    sqlx::query("UPDATE cloud_workspace_cursors SET cursor=GREATEST(cursor,$2) WHERE user_id=$1")
        .bind(owner)
        .bind(cursor)
        .execute(&mut *tx)
        .await
        .map_err(db)?;

    for item in library_items {
        let id = item
            .get("id")
            .and_then(Value::as_str)
            .ok_or(StatusCode::BAD_REQUEST)?;
        let id = uuid::Uuid::parse_str(id).map_err(|_| StatusCode::BAD_REQUEST)?;
        let kind = item
            .get("kind")
            .and_then(Value::as_str)
            .ok_or(StatusCode::BAD_REQUEST)?;
        let slug = item
            .get("slug")
            .and_then(Value::as_str)
            .ok_or(StatusCode::BAD_REQUEST)?;
        let revision = item
            .get("revision")
            .and_then(Value::as_i64)
            .ok_or(StatusCode::BAD_REQUEST)?;
        sqlx::query("INSERT INTO cloud_library_documents(id,owner_user_id,kind,slug,revision) VALUES($1,$2,$3,$4,$5) ON CONFLICT(id) DO NOTHING")
            .bind(id).bind(owner).bind(kind).bind(slug).bind(revision)
            .execute(&mut *tx).await.map_err(db)?;
        if let Some(versions) = item.get("versions").and_then(Value::as_array) {
            for version in versions {
                let v_revision = version
                    .get("revision")
                    .and_then(Value::as_i64)
                    .ok_or(StatusCode::BAD_REQUEST)?;
                let title = version
                    .get("title")
                    .and_then(Value::as_str)
                    .ok_or(StatusCode::BAD_REQUEST)?;
                let document = version
                    .get("document")
                    .cloned()
                    .ok_or(StatusCode::BAD_REQUEST)?;
                let sha256 = version
                    .get("sha256")
                    .and_then(Value::as_str)
                    .ok_or(StatusCode::BAD_REQUEST)?;
                let status = version
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("draft");
                sqlx::query("INSERT INTO cloud_library_versions(document_id,revision,title,document,sha256,status) VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT(document_id,revision) DO NOTHING")
                    .bind(id).bind(v_revision).bind(title).bind(document).bind(sha256).bind(status)
                    .execute(&mut *tx).await.map_err(db)?;
            }
        }
    }
    for item in media_items {
        let id = uuid::Uuid::parse_str(
            item.get("id")
                .and_then(Value::as_str)
                .ok_or(StatusCode::BAD_REQUEST)?,
        )
        .map_err(|_| StatusCode::BAD_REQUEST)?;
        let kind = item
            .get("kind")
            .and_then(Value::as_str)
            .ok_or(StatusCode::BAD_REQUEST)?;
        let slug = item
            .get("slug")
            .and_then(Value::as_str)
            .ok_or(StatusCode::BAD_REQUEST)?;
        let revision = item
            .get("revision")
            .and_then(Value::as_i64)
            .ok_or(StatusCode::BAD_REQUEST)?;
        sqlx::query("INSERT INTO cloud_library_documents(id,owner_user_id,kind,slug,revision) VALUES($1,$2,$3,$4,$5) ON CONFLICT(id) DO NOTHING")
            .bind(id).bind(owner).bind(kind).bind(slug).bind(revision)
            .execute(&mut *tx).await.map_err(db)?;
        let version = item.get("version").ok_or(StatusCode::BAD_REQUEST)?;
        let v_revision = version
            .get("revision")
            .and_then(Value::as_i64)
            .ok_or(StatusCode::BAD_REQUEST)?;
        let title = version
            .get("title")
            .and_then(Value::as_str)
            .ok_or(StatusCode::BAD_REQUEST)?;
        let document = version
            .get("document")
            .cloned()
            .ok_or(StatusCode::BAD_REQUEST)?;
        let sha256 = version
            .get("sha256")
            .and_then(Value::as_str)
            .ok_or(StatusCode::BAD_REQUEST)?;
        sqlx::query("INSERT INTO cloud_library_versions(document_id,revision,title,document,sha256,status) VALUES($1,$2,$3,$4,$5,'draft') ON CONFLICT(document_id,revision) DO NOTHING")
            .bind(id).bind(v_revision).bind(title).bind(document).bind(sha256)
            .execute(&mut *tx).await.map_err(db)?;
    }
    for item in assets {
        let id = uuid::Uuid::parse_str(
            item.get("id")
                .and_then(Value::as_str)
                .ok_or(StatusCode::BAD_REQUEST)?,
        )
        .map_err(|_| StatusCode::BAD_REQUEST)?;
        let encoded = item
            .get("contentBase64")
            .and_then(Value::as_str)
            .ok_or(StatusCode::BAD_REQUEST)?;
        let content = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|_| StatusCode::BAD_REQUEST)?;
        let sha256 = item
            .get("sha256")
            .and_then(Value::as_str)
            .ok_or(StatusCode::BAD_REQUEST)?;
        let digest = hex::encode(Sha256::digest(&content));
        if digest != sha256
            || content.len() != item.get("size").and_then(Value::as_i64).unwrap_or(-1) as usize
        {
            return Err(StatusCode::BAD_REQUEST);
        }
        let mime = item
            .get("mime")
            .and_then(Value::as_str)
            .ok_or(StatusCode::BAD_REQUEST)?;
        let width = item
            .get("width")
            .and_then(Value::as_i64)
            .ok_or(StatusCode::BAD_REQUEST)? as i32;
        let height = item
            .get("height")
            .and_then(Value::as_i64)
            .ok_or(StatusCode::BAD_REQUEST)? as i32;
        let size = content.len() as i32;
        let label = item.get("label").and_then(Value::as_str);
        sqlx::query("INSERT INTO cloud_assets(id,owner_user_id,sha256,mime,width,height,size,content,label) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9) ON CONFLICT(id) DO NOTHING")
            .bind(id).bind(owner).bind(sha256).bind(mime).bind(width).bind(height).bind(size).bind(content).bind(label)
            .execute(&mut *tx).await.map_err(db)?;
    }
    tx.commit().await.map_err(db)?;
    Ok(summary)
}

/// Public restore entry point: accepts an uploaded manifest.
pub async fn restore(
    DualUser(user): DualUser,
    State(state): State<AppState>,
    Json(request): Json<RestoreRequest>,
) -> Result<Json<Value>, StatusCode> {
    workspace::limit(&state, &user.user.id, "write", 30).await?;
    Ok(Json(
        apply_restore(
            &state.pool,
            &user.user.id,
            &request.manifest,
            request.mode,
            request.dry_run,
        )
        .await?,
    ))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateBackupRequest {
    name: String,
    /// A manifest exported on the client. When absent the server snapshots the
    /// account's current cloud data instead.
    #[serde(default)]
    manifest: Option<Value>,
}

/// Keep only the sections a restore understands and check every asset digest,
/// so an uploaded file cannot smuggle arbitrary data into storage or fail
/// later, at restore time.
fn normalize_uploaded(manifest: &Value) -> Result<Value, StatusCode> {
    let object = manifest.as_object().ok_or(StatusCode::BAD_REQUEST)?;
    let assets = array(
        manifest.get("assets").ok_or(StatusCode::BAD_REQUEST)?,
        "items",
    )?;
    for item in assets {
        let content = base64::engine::general_purpose::STANDARD
            .decode(
                item.get("contentBase64")
                    .and_then(Value::as_str)
                    .ok_or(StatusCode::BAD_REQUEST)?,
            )
            .map_err(|_| StatusCode::BAD_REQUEST)?;
        let sha256 = item
            .get("sha256")
            .and_then(Value::as_str)
            .ok_or(StatusCode::BAD_REQUEST)?;
        if hex::encode(Sha256::digest(&content)) != sha256
            || item.get("size").and_then(Value::as_u64) != Some(content.len() as u64)
        {
            return Err(StatusCode::BAD_REQUEST);
        }
    }
    let mut kept = serde_json::Map::new();
    for key in [
        "schemaVersion",
        "createdAt",
        "legacySync",
        "workspaceSync",
        "library",
        "mediaLibrary",
        "assets",
    ] {
        if let Some(value) = object.get(key) {
            kept.insert(key.to_owned(), value.clone());
        }
    }
    Ok(Value::Object(kept))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StoredRestoreRequest {
    #[serde(default = "default_mode")]
    mode: RestoreMode,
    #[serde(default)]
    dry_run: bool,
}

fn backup_metadata(row: &sqlx::postgres::PgRow) -> Value {
    json!({
        "id": row.get::<uuid::Uuid, _>("id").to_string(),
        "name": row.get::<String, _>("name"),
        "createdAt": row.get::<chrono::DateTime<chrono::Utc>, _>("created_at").to_rfc3339(),
        "sizeBytes": row.get::<i64, _>("size_bytes"),
        "workspaceSync": row.get::<i32, _>("workspace_records"),
        "library": row.get::<i32, _>("library_documents"),
        "mediaLibrary": row.get::<i32, _>("media_documents"),
        "assets": row.get::<i32, _>("assets"),
    })
}

/// List stored backups for the signed-in account, newest first.
pub async fn backup_list(
    DualUser(user): DualUser,
    State(state): State<AppState>,
) -> Result<Json<Value>, StatusCode> {
    workspace::limit(&state, &user.user.id, "read", 60).await?;
    let rows = sqlx::query(
        "SELECT id,name,created_at,size_bytes,workspace_records,library_documents,media_documents,assets \
         FROM cloud_backups WHERE owner_user_id=$1 ORDER BY created_at DESC,id DESC LIMIT 100",
    )
    .bind(&user.user.id)
    .fetch_all(&state.pool)
    .await
    .map_err(db)?;
    Ok(Json(
        json!({"items": rows.iter().map(backup_metadata).collect::<Vec<_>>()}),
    ))
}

/// Store a backup under a name and return its metadata. The manifest is either
/// uploaded by the client (a local file) or snapshotted from the account.
pub async fn backup_create_named(
    DualUser(user): DualUser,
    State(state): State<AppState>,
    Json(input): Json<CreateBackupRequest>,
) -> Result<Json<Value>, StatusCode> {
    workspace::limit(&state, &user.user.id, "write", 10).await?;
    let name = input.name.trim();
    if name.is_empty() || name.chars().count() > 120 {
        return Err(StatusCode::BAD_REQUEST);
    }
    let limits = crate::plans::limits_for(&state.pool, &user.user.id).await;
    let manifest = match input.manifest.as_ref() {
        Some(uploaded) => {
            // Validates the schema version, structure and plan record limits.
            apply_restore(
                &state.pool,
                &user.user.id,
                uploaded,
                RestoreMode::Replace,
                true,
            )
            .await?;
            normalize_uploaded(uploaded)?
        }
        None => build_manifest(&state.pool, &user.user.id).await?,
    };
    let bytes = serde_json::to_vec(&manifest).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let existing: i64 =
        sqlx::query_scalar("SELECT count(*) FROM cloud_backups WHERE owner_user_id=$1")
            .bind(&user.user.id)
            .fetch_one(&state.pool)
            .await
            .map_err(db)?;
    if existing >= limits.max_backups {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }
    let total_bytes: i64 = sqlx::query_scalar(
        "SELECT COALESCE(sum(size_bytes),0)::bigint FROM cloud_backups WHERE owner_user_id=$1",
    )
    .bind(&user.user.id)
    .fetch_one(&state.pool)
    .await
    .map_err(db)?;
    if total_bytes + bytes.len() as i64 > limits.max_backup_bytes {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }
    if bytes.len() > MAX_BACKUP_BYTES {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }
    let count = |value: Option<&Value>, key: &str| -> i32 {
        value
            .and_then(|v| v.get(key))
            .and_then(Value::as_array)
            .map_or(0, |items| items.len() as i32)
    };
    let workspace = manifest.get("workspaceSync");
    let library = manifest.get("library");
    let media = manifest.get("mediaLibrary");
    let assets = manifest.get("assets");
    let id = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO cloud_backups(id,owner_user_id,name,manifest,size_bytes,workspace_records,library_documents,media_documents,assets) \
         VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)",
    )
    .bind(id)
    .bind(&user.user.id)
    .bind(name)
    .bind(&manifest)
    .bind(bytes.len() as i64)
    .bind(count(workspace, "records"))
    .bind(count(library, "items"))
    .bind(count(media, "documents"))
    .bind(count(assets, "items"))
    .execute(&state.pool)
    .await
    .map_err(db)?;
    let row = sqlx::query(
        "SELECT id,name,created_at,size_bytes,workspace_records,library_documents,media_documents,assets FROM cloud_backups WHERE id=$1 AND owner_user_id=$2",
    )
    .bind(id)
    .bind(&user.user.id)
    .fetch_one(&state.pool)
    .await
    .map_err(db)?;
    Ok(Json(backup_metadata(&row)))
}

/// Download one stored manifest (no metadata-only listing payload).
pub async fn backup_get(
    DualUser(user): DualUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, StatusCode> {
    let id = uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?;
    let row = sqlx::query("SELECT manifest FROM cloud_backups WHERE id=$1 AND owner_user_id=$2")
        .bind(id)
        .bind(&user.user.id)
        .fetch_optional(&state.pool)
        .await
        .map_err(db)?
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(row.get::<Value, _>("manifest")))
}

/// Delete one stored backup.
pub async fn backup_delete(
    DualUser(user): DualUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, StatusCode> {
    workspace::limit(&state, &user.user.id, "write", 30).await?;
    let id = uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?;
    let result = sqlx::query("DELETE FROM cloud_backups WHERE id=$1 AND owner_user_id=$2")
        .bind(id)
        .bind(&user.user.id)
        .execute(&state.pool)
        .await
        .map_err(db)?;
    if result.rows_affected() == 0 {
        return Err(StatusCode::NOT_FOUND);
    }
    Ok(Json(json!({"deleted": true})))
}

/// Preview or apply a stored backup by id.
pub async fn backup_restore_stored(
    DualUser(user): DualUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(input): Json<StoredRestoreRequest>,
) -> Result<Json<Value>, StatusCode> {
    workspace::limit(&state, &user.user.id, "write", 30).await?;
    let id = uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?;
    let row = sqlx::query("SELECT manifest FROM cloud_backups WHERE id=$1 AND owner_user_id=$2")
        .bind(id)
        .bind(&user.user.id)
        .fetch_optional(&state.pool)
        .await
        .map_err(db)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let manifest = row.get::<Value, _>("manifest");
    Ok(Json(
        apply_restore(
            &state.pool,
            &user.user.id,
            &manifest,
            input.mode,
            input.dry_run,
        )
        .await?,
    ))
}
