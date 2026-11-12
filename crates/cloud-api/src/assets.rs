//! Managed media assets: validated raster uploads owned by one account.
//!
//! The owner can list (newest first, keyset pages), upload with an optional
//! label, rename and delete. An asset an overlay revision still references
//! cannot be deleted, so no stored overlay ever points at a missing image.

use crate::{
    api_error::{Authenticated, Kind, RequestId, V3Error},
    catalog_v3::query_map,
    media::{self, Rejection},
    paging::{self, Position},
    remote::{db, parse_body, parse_uuid, throttle},
    AppState,
};
use axum::{
    body::Bytes,
    extract::{Path, RawQuery, State},
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, patch},
    Extension, Json, Router,
};
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;

/// Quotas per account. They bound storage in the shared database.
pub const MAX_ASSETS_PER_OWNER: i64 = 200;
pub const MAX_BYTES_PER_OWNER: i64 = 32 * 1024 * 1024;
pub const MAX_LABEL_CHARS: usize = 120;
const SCOPE: &str = "assets";
const COLUMNS: &str = "id,sha256,mime,width,height,size,label,created_at";

pub fn routes() -> Router<AppState> {
    let no_store = || axum::middleware::map_response(crate::sync::no_store);
    Router::new()
        .route("/api/v3/assets", get(list).layer(no_store()).post(upload))
        .route(
            "/api/v3/assets/{id}",
            get(read).merge(patch(rename).delete(remove).layer(no_store())),
        )
}

/// One per-owner lock serializes every change that creates or drops a
/// reference between assets, overlays and packages, and every quota check.
pub(crate) async fn lock_owner(
    tx: &mut Transaction<'_, Postgres>,
    rid: &RequestId,
    owner: &str,
) -> Result<(), V3Error> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))")
        .bind(format!("cloud-assets:{owner}"))
        .execute(&mut **tx)
        .await
        .map_err(db(rid))?;
    Ok(())
}

/// A label is 1..=120 characters with no control or line-separator
/// characters. It is trimmed; the stored value is the trimmed text.
pub fn check_label(label: &str) -> Option<String> {
    let label = label.trim();
    let valid = (1..=MAX_LABEL_CHARS).contains(&label.chars().count())
        && !label
            .chars()
            .any(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}'));
    valid.then(|| label.to_owned())
}

fn view(row: &sqlx::postgres::PgRow) -> Value {
    let mut item = json!({
        "id": row.get::<Uuid, _>("id"),
        "sha256": row.get::<String, _>("sha256"),
        "mime": row.get::<String, _>("mime"),
        "width": row.get::<i32, _>("width"),
        "height": row.get::<i32, _>("height"),
        "size": row.get::<i32, _>("size"),
        "createdAt": row.get::<DateTime<Utc>, _>("created_at").to_rfc3339(),
    });
    if let Some(label) = row.get::<Option<String>, _>("label") {
        item["label"] = json!(label);
    }
    item
}

pub async fn upload(
    Authenticated(user): Authenticated,
    Extension(rid): Extension<RequestId>,
    State(state): State<AppState>,
    RawQuery(raw): RawQuery,
    body: Bytes,
) -> Result<(StatusCode, Json<Value>), V3Error> {
    let owner = &user.user.id;
    throttle(&state, &rid, owner, "asset-upload", 30).await?;
    // The label travels in the query string (`?label=`, percent-encoded UTF-8)
    // because the body is the raw image. Unknown or repeated keys are refused.
    let params = query_map(&rid, raw, &["label"])?;
    let label = match params.get("label") {
        None => None,
        Some(raw) => Some(check_label(raw).ok_or(V3Error::new(&rid, Kind::InvalidRequest))?),
    };
    // Any refusal is a payload problem: too large, empty, unsupported or damaged.
    let image =
        media::inspect(&body).map_err(|_: Rejection| V3Error::new(&rid, Kind::InvalidRequest))?;
    let digest = hex::encode(Sha256::digest(&body));
    let mut tx = state.pool.begin().await.map_err(db(&rid))?;
    // Serialize quota checks per owner so concurrent uploads cannot both pass the limit.
    lock_owner(&mut tx, &rid, owner).await?;
    // Identical bytes keep their id and their existing label: a repeated upload
    // never renames silently. Use PATCH to change the label.
    if let Some(existing) = sqlx::query(&format!(
        "SELECT {COLUMNS} FROM cloud_assets WHERE owner_user_id=$1 AND sha256=$2"
    ))
    .bind(owner)
    .bind(&digest)
    .fetch_optional(&mut *tx)
    .await
    .map_err(db(&rid))?
    {
        tx.commit().await.map_err(db(&rid))?;
        return Ok((StatusCode::OK, Json(view(&existing))));
    }
    let totals = sqlx::query("SELECT count(*) AS n, COALESCE(sum(size),0)::bigint AS bytes FROM cloud_assets WHERE owner_user_id=$1")
        .bind(owner)
        .fetch_one(&mut *tx)
        .await
        .map_err(db(&rid))?;
    let count: i64 = totals.get("n");
    let bytes: i64 = totals.get("bytes");
    if count >= MAX_ASSETS_PER_OWNER || bytes + body.len() as i64 > MAX_BYTES_PER_OWNER {
        return Err(V3Error::new(&rid, Kind::QuotaExceeded));
    }
    let id = Uuid::new_v4();
    let created = sqlx::query(&format!("INSERT INTO cloud_assets(id,owner_user_id,sha256,mime,width,height,size,content,label) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9) RETURNING {COLUMNS}"))
        .bind(id)
        .bind(owner)
        .bind(&digest)
        .bind(image.format.mime())
        .bind(image.width as i32)
        .bind(image.height as i32)
        .bind(body.len() as i32)
        .bind(body.as_ref())
        .bind(&label)
        .fetch_one(&mut *tx)
        .await
        .map_err(db(&rid))?;
    tx.commit().await.map_err(db(&rid))?;
    Ok((StatusCode::CREATED, Json(view(&created))))
}

/// The caller's assets, newest first (`created_at`, then `id`), as keyset pages.
pub async fn list(
    Authenticated(user): Authenticated,
    Extension(rid): Extension<RequestId>,
    State(state): State<AppState>,
    RawQuery(raw): RawQuery,
) -> Result<Json<Value>, V3Error> {
    let owner = &user.user.id;
    let (limit, cursor) = paging::request(&rid, raw)?;
    let (after_at, after_id) = match cursor {
        None => (None, None),
        Some(raw) => match paging::decode(&rid, &raw, SCOPE, owner, false)? {
            Position::Time(at, id) => (Some(at), Some(id)),
            Position::Number(_) => return Err(V3Error::new(&rid, Kind::InvalidRequest)),
        },
    };
    let rows = sqlx::query(&format!("SELECT {COLUMNS} FROM cloud_assets WHERE owner_user_id=$1 AND ($2::timestamptz IS NULL OR created_at < $2::timestamptz OR (created_at = $2::timestamptz AND id < $3::uuid)) ORDER BY created_at DESC, id DESC LIMIT $4"))
        .bind(owner)
        .bind(after_at)
        .bind(after_id)
        .bind(limit + 1)
        .fetch_all(&state.pool)
        .await
        .map_err(db(&rid))?;
    let more = rows.len() as i64 > limit;
    let page = &rows[..rows.len().min(limit as usize)];
    let next = match (more, page.last()) {
        (true, Some(last)) => Some(paging::encode(
            SCOPE,
            owner,
            &Position::Time(last.get("created_at"), last.get("id")),
        )),
        _ => None,
    };
    let items: Vec<Value> = page.iter().map(view).collect();
    Ok(Json(json!({"items": items, "nextCursor": next})))
}

/// Serves the stored bytes to the owner only. The content type is the verified
/// one, and `nosniff` stops browsers from reinterpreting the bytes.
pub async fn read(
    Authenticated(user): Authenticated,
    Extension(rid): Extension<RequestId>,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, V3Error> {
    let asset = parse_uuid(&rid, &id)?;
    let row = sqlx::query("SELECT mime,content FROM cloud_assets WHERE id=$1 AND owner_user_id=$2")
        .bind(asset)
        .bind(&user.user.id)
        .fetch_optional(&state.pool)
        .await
        .map_err(db(&rid))?
        .ok_or(V3Error::new(&rid, Kind::NotFound))?;
    let mime: String = row.get("mime");
    let content: Vec<u8> = row.get("content");
    let mut response = (StatusCode::OK, content).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&mime)
            .unwrap_or(HeaderValue::from_static("application/octet-stream")),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=300"),
    );
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    Ok(response)
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Rename {
    /// Required: a string sets the label, `null` clears it.
    label: Value,
}

/// `PATCH /api/v3/assets/{id}` `{"label": string | null}`. Idempotent.
pub async fn rename(
    Authenticated(user): Authenticated,
    Extension(rid): Extension<RequestId>,
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Bytes,
) -> Result<Json<Value>, V3Error> {
    let owner = &user.user.id;
    throttle(&state, &rid, owner, "asset-write", 60).await?;
    let asset = parse_uuid(&rid, &id)?;
    let request: Rename = parse_body(&rid, &body, 2048)?;
    let label = match &request.label {
        Value::Null => None,
        Value::String(text) => {
            Some(check_label(text).ok_or(V3Error::new(&rid, Kind::InvalidRequest))?)
        }
        _ => return Err(V3Error::new(&rid, Kind::InvalidRequest)),
    };
    let row = sqlx::query(&format!(
        "UPDATE cloud_assets SET label=$3 WHERE id=$1 AND owner_user_id=$2 RETURNING {COLUMNS}"
    ))
    .bind(asset)
    .bind(owner)
    .bind(&label)
    .fetch_optional(&state.pool)
    .await
    .map_err(db(&rid))?
    .ok_or(V3Error::new(&rid, Kind::NotFound))?;
    Ok(Json(view(&row)))
}

/// Normalizes a stored reference the way `Uuid::parse_str` reads it, so the
/// hyphenated, simple, braced and `urn:uuid:` spellings of one id all match.
pub(crate) fn sql_uuid_key(expression: &str) -> String {
    format!("replace(replace(replace(replace(lower({expression}),'urn:uuid:',''),'{{',''),'}}',''),'-','')")
}

/// Whether any overlay revision of this owner still names the asset.
async fn referenced(
    tx: &mut Transaction<'_, Postgres>,
    rid: &RequestId,
    owner: &str,
    asset: Uuid,
) -> Result<bool, V3Error> {
    let key = sql_uuid_key("e->>'assetId'");
    sqlx::query_scalar(&format!(
        "SELECT EXISTS (SELECT 1 FROM cloud_library_documents d JOIN cloud_library_versions v ON v.document_id=d.id CROSS JOIN LATERAL jsonb_array_elements(CASE WHEN jsonb_typeof(v.document->'elements')='array' THEN v.document->'elements' ELSE '[]'::jsonb END) e WHERE d.owner_user_id=$1 AND d.kind='overlay' AND {key}=$2)"
    ))
    .bind(owner)
    .bind(asset.simple().to_string())
    .fetch_one(&mut **tx)
    .await
    .map_err(db(rid))
}

/// `DELETE /api/v3/assets/{id}`. Owner only (`notFound` for anyone else, and for
/// an asset that is already gone, so a repeated delete is `notFound`). Refused
/// with `conflict` while any overlay revision of the owner references the asset.
/// Quota is computed from the rows, so deleting frees it immediately.
pub async fn remove(
    Authenticated(user): Authenticated,
    Extension(rid): Extension<RequestId>,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, V3Error> {
    let owner = &user.user.id;
    throttle(&state, &rid, owner, "asset-write", 60).await?;
    let asset = parse_uuid(&rid, &id)?;
    let mut tx = state.pool.begin().await.map_err(db(&rid))?;
    lock_owner(&mut tx, &rid, owner).await?;
    sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM cloud_assets WHERE id=$1 AND owner_user_id=$2 FOR UPDATE",
    )
    .bind(asset)
    .bind(owner)
    .fetch_optional(&mut *tx)
    .await
    .map_err(db(&rid))?
    .ok_or(V3Error::new(&rid, Kind::NotFound))?;
    if referenced(&mut tx, &rid, owner, asset).await? {
        return Err(V3Error::new(&rid, Kind::Conflict));
    }
    sqlx::query("DELETE FROM cloud_assets WHERE id=$1")
        .bind(asset)
        .execute(&mut *tx)
        .await
        .map_err(db(&rid))?;
    tx.commit().await.map_err(db(&rid))?;
    Ok(Json(json!({"id": asset, "deleted": true})))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_are_trimmed_bounded_and_free_of_control_characters() {
        assert_eq!(check_label("  Logo  ").as_deref(), Some("Logo"));
        assert_eq!(
            check_label("Logotipo ñ 🎉").as_deref(),
            Some("Logotipo ñ 🎉")
        );
        assert_eq!(check_label(&"a".repeat(120)).map(|l| l.len()), Some(120));
        for bad in [
            "",
            "   ",
            "a\nb",
            "tab\there",
            "nul\0",
            "esc\u{1b}",
            "c1\u{85}x",
            "se\u{2028}p",
            &"a".repeat(121),
        ] {
            assert_eq!(check_label(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn uuid_key_expression_strips_every_accepted_spelling() {
        let sql = sql_uuid_key("x");
        assert!(sql.contains("'urn:uuid:'") && sql.contains("'{'") && sql.contains("'}'"));
        assert!(sql.starts_with("replace(") && sql.contains("lower(x)"));
    }
}
