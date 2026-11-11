//! Managed media assets: validated raster uploads owned by one account.

use crate::{
    api_error::{Authenticated, Kind, RequestId, V3Error},
    media::{self, Rejection},
    remote::{db, parse_uuid, throttle},
    AppState,
};
use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Extension, Json, Router,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::Row;
use uuid::Uuid;

/// Quotas per account. They bound storage in the shared database.
pub const MAX_ASSETS_PER_OWNER: i64 = 200;
pub const MAX_BYTES_PER_OWNER: i64 = 32 * 1024 * 1024;
const LIST_LIMIT: i64 = 50;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/v3/assets", get(list).post(upload))
        .route("/api/v3/assets/{id}", get(read))
}

fn view(row: &sqlx::postgres::PgRow) -> Value {
    json!({
        "id": row.get::<Uuid, _>("id"),
        "sha256": row.get::<String, _>("sha256"),
        "mime": row.get::<String, _>("mime"),
        "width": row.get::<i32, _>("width"),
        "height": row.get::<i32, _>("height"),
        "size": row.get::<i32, _>("size"),
    })
}

pub async fn upload(
    Authenticated(user): Authenticated,
    Extension(rid): Extension<RequestId>,
    State(state): State<AppState>,
    body: Bytes,
) -> Result<(StatusCode, Json<Value>), V3Error> {
    let owner = &user.user.id;
    throttle(&state, &rid, owner, "asset-upload", 30).await?;
    // Any refusal is a payload problem: too large, empty, unsupported or damaged.
    let image =
        media::inspect(&body).map_err(|_: Rejection| V3Error::new(&rid, Kind::InvalidRequest))?;
    let digest = hex::encode(Sha256::digest(&body));
    let mut tx = state.pool.begin().await.map_err(db(&rid))?;
    // Serialize quota checks per owner so concurrent uploads cannot both pass the limit.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))")
        .bind(format!("cloud-assets:{owner}"))
        .execute(&mut *tx)
        .await
        .map_err(db(&rid))?;
    if let Some(existing) = sqlx::query("SELECT id,sha256,mime,width,height,size FROM cloud_assets WHERE owner_user_id=$1 AND sha256=$2")
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
    sqlx::query("INSERT INTO cloud_assets(id,owner_user_id,sha256,mime,width,height,size,content) VALUES($1,$2,$3,$4,$5,$6,$7,$8)")
        .bind(id)
        .bind(owner)
        .bind(&digest)
        .bind(image.format.mime())
        .bind(image.width as i32)
        .bind(image.height as i32)
        .bind(body.len() as i32)
        .bind(body.as_ref())
        .execute(&mut *tx)
        .await
        .map_err(db(&rid))?;
    tx.commit().await.map_err(db(&rid))?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": id,
            "sha256": digest,
            "mime": image.format.mime(),
            "width": image.width,
            "height": image.height,
            "size": body.len(),
        })),
    ))
}

pub async fn list(
    Authenticated(user): Authenticated,
    Extension(rid): Extension<RequestId>,
    State(state): State<AppState>,
) -> Result<Json<Value>, V3Error> {
    let rows = sqlx::query("SELECT id,sha256,mime,width,height,size FROM cloud_assets WHERE owner_user_id=$1 ORDER BY id LIMIT $2")
        .bind(&user.user.id)
        .bind(LIST_LIMIT)
        .fetch_all(&state.pool)
        .await
        .map_err(db(&rid))?;
    let items: Vec<Value> = rows.iter().map(view).collect();
    Ok(Json(json!({"items": items})))
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
