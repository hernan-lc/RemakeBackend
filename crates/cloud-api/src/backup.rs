//! User-owned backup and restore of every cloud domain.
//!
//! Unlike `/api/v2/me/export` (a privacy report), a backup manifest is meant
//! to be restored on another device. Restore is explicit, bounded by the
//! caller's plan and always reported as a summary before it is applied.

use crate::{dual_auth::DualUser, library, portable, workspace, AppState};
use axum::{extract::State, http::StatusCode, Json};
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

/// Build a portable, versioned manifest for the authenticated user.
pub async fn backup(
    DualUser(user): DualUser,
    State(state): State<AppState>,
) -> Result<Json<Value>, StatusCode> {
    workspace::limit(&state, &user.user.id, "export", 10).await?;
    let owner = &user.user.id;
    let mut tx = state.pool.begin().await.map_err(db)?;
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

    let workspace = workspace::snapshot(&state.pool, owner).await?;
    let library = library::export(&state.pool, owner).await?;

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
    Ok(Json(manifest))
}

/// Report (and optionally apply) a restore. `dryRun` powers the one-click
/// summary shown on a fresh device; `replace` skips conflict review there.
pub async fn restore(
    DualUser(user): DualUser,
    State(state): State<AppState>,
    Json(request): Json<RestoreRequest>,
) -> Result<Json<Value>, StatusCode> {
    workspace::limit(&state, &user.user.id, "write", 30).await?;
    let owner = user.user.id.clone();
    let manifest = &request.manifest;
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
    let limits = crate::plans::limits_for(&state.pool, &owner).await;
    if assets.len() as i64 > limits.max_assets
        || workspace_records.len() as i64 > limits.max_records
    {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }

    let summary = json!({
        "mode": request.mode,
        "dryRun": request.dry_run,
        "legacySync": legacy.len(),
        "workspaceSync": workspace_records.len(),
        "library": library_items.len(),
        "mediaLibrary": media_items.len(),
        "assets": assets.len(),
        "applied": !request.dry_run,
    });
    if request.dry_run {
        return Ok(Json(summary));
    }

    let mut tx = state.pool.begin().await.map_err(db)?;
    sqlx::query("INSERT INTO cloud_workspace_cursors(user_id) VALUES($1) ON CONFLICT DO NOTHING")
        .bind(&owner)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    if request.mode == RestoreMode::Replace {
        sqlx::query("DELETE FROM cloud_workspace_records WHERE user_id=$1")
            .bind(&owner)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        sqlx::query("DELETE FROM cloud_sync_records WHERE user_id=$1")
            .bind(&owner)
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
            .bind(&owner).bind(collection).bind(key).bind(revision).bind(value).bind(deleted)
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
            .bind(&owner).bind(collection).bind(key).bind(revision).bind(value).bind(deleted)
            .execute(&mut *tx).await.map_err(db)?;
    }
    let cursor = workspace_snapshot
        .get("cursor")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    sqlx::query("UPDATE cloud_workspace_cursors SET cursor=GREATEST(cursor,$2) WHERE user_id=$1")
        .bind(&owner)
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
            .bind(id).bind(&owner).bind(kind).bind(slug).bind(revision)
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
            .bind(id).bind(&owner).bind(kind).bind(slug).bind(revision)
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
            .bind(id).bind(&owner).bind(sha256).bind(mime).bind(width).bind(height).bind(size).bind(content).bind(label)
            .execute(&mut *tx).await.map_err(db)?;
    }
    tx.commit().await.map_err(db)?;
    Ok(Json(summary))
}
