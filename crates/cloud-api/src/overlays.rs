//! Versioned overlay documents.
//!
//! An overlay is a fixed-schema layout: a canvas, a bounded list of typed
//! elements and the assets they reference. Elements are chosen from the set the
//! renderer knows; there are no free-form nodes, scripts or URLs. Dynamic values
//! may come only from the allow-listed event fields.

use crate::{
    api_error::{Authenticated, Kind, RequestId, V3Error},
    remote::{binding_digest, db, parse_body, parse_uuid, throttle},
    AppState,
};
use axum::{
    body::Bytes,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Extension, Json, Router,
};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::collections::{BTreeSet, HashSet};
use uuid::Uuid;

pub const SCHEMA_VERSION: i64 = 1;
pub const OVERLAY_KINDS: &[&str] = &[
    "follow",
    "gift",
    "chat",
    "share",
    "subscribe",
    "likes",
    "ranking",
];
const ELEMENT_TYPES: &[&str] = &["text", "avatar", "art"];
const FIELDS: &[&str] = &[
    "viewer.name",
    "viewer.handle",
    "viewer.avatar",
    "gift.name",
    "gift.count",
    "gift.diamonds",
    "chat.message",
    "points.total",
];
const MAX_ELEMENTS: usize = 64;
const MAX_DOCUMENT_BYTES: usize = 64 * 1024;
const TOP_KEYS: &[&str] = &[
    "overlayVersion",
    "kind",
    "title",
    "canvas",
    "elements",
    "capabilities",
];
const ELEMENT_KEYS: &[&str] = &[
    "id", "type", "x", "y", "width", "height", "opacity", "source", "assetId", "text",
];

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/v3/overlays", post(create))
        .route("/api/v3/overlays/{id}", get(read))
        .route("/api/v3/overlays/{id}/revisions", post(revise))
}

/// Result of a successful structural check: the asset ids the overlay needs
/// and the capabilities it must declare.
#[derive(Debug, PartialEq, Eq)]
pub struct Checked {
    pub assets: BTreeSet<Uuid>,
    pub capabilities: BTreeSet<&'static str>,
}

fn int(value: &Value) -> Option<i64> {
    value.as_i64()
}

/// Validates the document without touching the database. Ownership of the
/// referenced assets is checked separately.
pub fn check(document: &Value) -> Result<Checked, &'static str> {
    let text = serde_json::to_vec(document).map_err(|_| "invalidDocument")?;
    if text.len() > MAX_DOCUMENT_BYTES {
        return Err("documentTooLarge");
    }
    let object = document.as_object().ok_or("invalidDocument")?;
    if object.keys().any(|k| !TOP_KEYS.contains(&k.as_str())) {
        return Err("unknownKey");
    }
    if int(&document["overlayVersion"]) != Some(SCHEMA_VERSION) {
        return Err("unsupportedSchema");
    }
    let kind = document["kind"].as_str().ok_or("invalidKind")?;
    if !OVERLAY_KINDS.contains(&kind) {
        return Err("invalidKind");
    }
    let title = document["title"].as_str().ok_or("invalidTitle")?;
    if title.trim().is_empty() || title.chars().count() > 120 || title.chars().any(char::is_control)
    {
        return Err("invalidTitle");
    }
    let canvas = &document["canvas"];
    let (cw, ch) = match (int(&canvas["width"]), int(&canvas["height"])) {
        (Some(w), Some(h))
            if (320..=3840).contains(&w)
                && (180..=2160).contains(&h)
                && canvas.as_object().map(|o| o.len()) == Some(2) =>
        {
            (w, h)
        }
        _ => return Err("invalidCanvas"),
    };
    let elements = document["elements"].as_array().ok_or("invalidElements")?;
    if elements.len() > MAX_ELEMENTS {
        return Err("tooManyElements");
    }
    let mut ids = HashSet::new();
    let mut assets = BTreeSet::new();
    let mut capabilities = BTreeSet::new();
    for element in elements {
        let obj = element.as_object().ok_or("invalidElement")?;
        if obj.keys().any(|k| !ELEMENT_KEYS.contains(&k.as_str())) {
            return Err("unknownElementKey");
        }
        let id = element["id"].as_str().ok_or("invalidElement")?;
        if id.is_empty()
            || id.len() > 64
            || !id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
        {
            return Err("invalidElementId");
        }
        if !ids.insert(id.to_owned()) {
            return Err("duplicateElementId");
        }
        let kind = element["type"].as_str().ok_or("invalidElementType")?;
        if !ELEMENT_TYPES.contains(&kind) {
            return Err("invalidElementType");
        }
        let fits = match (
            int(&element["x"]),
            int(&element["y"]),
            int(&element["width"]),
            int(&element["height"]),
        ) {
            (Some(x), Some(y), Some(w), Some(h)) => {
                x >= 0 && y >= 0 && w >= 1 && h >= 1 && x + w <= cw && y + h <= ch
            }
            _ => false,
        };
        if !fits {
            return Err("invalidGeometry");
        }
        match element.get("opacity") {
            None => {}
            Some(value) => match value.as_f64() {
                Some(o) if (0.0..=1.0).contains(&o) => {}
                _ => return Err("invalidOpacity"),
            },
        }
        match kind {
            "text" => {
                if element.get("assetId").is_some() {
                    return Err("unexpectedAsset");
                }
                match element["source"]["kind"].as_str() {
                    Some("static") => {
                        if element["source"].as_object().map(|o| o.len()) != Some(2) {
                            return Err("invalidSource");
                        }
                        let text = element["source"]["text"].as_str().ok_or("invalidSource")?;
                        if text.chars().count() > 200
                            || text.chars().any(|c| c.is_control() && c != '\n')
                        {
                            return Err("invalidSource");
                        }
                    }
                    Some("field") => {
                        if element["source"].as_object().map(|o| o.len()) != Some(2) {
                            return Err("invalidSource");
                        }
                        let field = element["source"]["field"].as_str().ok_or("invalidSource")?;
                        if !FIELDS.contains(&field) || field == "viewer.avatar" {
                            return Err("invalidSource");
                        }
                        capabilities.insert("live.read");
                    }
                    _ => return Err("invalidSource"),
                }
                if element.get("text").is_some() {
                    return Err("unexpectedText");
                }
            }
            "avatar" => {
                let ok = element["source"] == json!({"kind": "field", "field": "viewer.avatar"});
                if !ok || element.get("assetId").is_some() || element.get("text").is_some() {
                    return Err("invalidSource");
                }
                capabilities.insert("live.read");
            }
            _ => {
                let asset = element["assetId"].as_str().ok_or("missingAsset")?;
                let asset = Uuid::parse_str(asset).map_err(|_| "missingAsset")?;
                if element.get("text").is_some() {
                    return Err("unexpectedText");
                }
                if element.get("source").is_some() {
                    return Err("unexpectedSource");
                }
                assets.insert(asset);
            }
        }
    }
    let declared: BTreeSet<&str> = match document.get("capabilities") {
        None => BTreeSet::new(),
        Some(value) => value
            .as_array()
            .ok_or("invalidCapabilities")?
            .iter()
            .map(|c| {
                c.as_str()
                    .and_then(|c| ["live.read"].iter().find(|known| **known == c).copied())
                    .ok_or("invalidCapabilities")
            })
            .collect::<Result<_, _>>()?,
    };
    if !capabilities.iter().all(|c| declared.contains(c)) {
        return Err("missingCapability");
    }
    Ok(Checked {
        assets,
        capabilities,
    })
}

/// Confirms every referenced asset is owned by the same account.
async fn assets_owned(
    pool: &PgPool,
    owner: &str,
    assets: &BTreeSet<Uuid>,
) -> Result<bool, sqlx::Error> {
    if assets.is_empty() {
        return Ok(true);
    }
    let ids: Vec<Uuid> = assets.iter().copied().collect();
    let found: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM cloud_assets WHERE owner_user_id=$1 AND id = ANY($2)",
    )
    .bind(owner)
    .bind(&ids)
    .fetch_one(pool)
    .await?;
    Ok(found as usize == ids.len())
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateOverlay {
    document: Value,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ReviseOverlay {
    document: Value,
    #[serde(rename = "expectedRevision")]
    expected_revision: i64,
}

fn refusal(rid: &RequestId, _reason: &'static str) -> V3Error {
    // The reason is stable for clients but never echoes document content.
    V3Error::new(rid, Kind::InvalidRequest)
}

pub async fn create(
    Authenticated(user): Authenticated,
    Extension(rid): Extension<RequestId>,
    State(state): State<AppState>,
    body: Bytes,
) -> Result<(StatusCode, Json<Value>), V3Error> {
    let owner = &user.user.id;
    throttle(&state, &rid, owner, "overlay-write", 60).await?;
    let request: CreateOverlay = parse_body(&rid, &body, MAX_DOCUMENT_BYTES + 1024)?;
    let checked = check(&request.document).map_err(|r| refusal(&rid, r))?;
    if !assets_owned(&state.pool, owner, &checked.assets)
        .await
        .map_err(db(&rid))?
    {
        return Err(V3Error::new(&rid, Kind::NotFound));
    }
    let id = Uuid::new_v4();
    let slug = format!("ov-{}", id.simple());
    let title = request.document["title"]
        .as_str()
        .unwrap_or_default()
        .trim()
        .to_owned();
    let digest = binding_digest(&request.document);
    let mut tx = state.pool.begin().await.map_err(db(&rid))?;
    sqlx::query("INSERT INTO cloud_library_documents(id,owner_user_id,kind,slug) VALUES($1,$2,'overlay',$3)")
        .bind(id)
        .bind(owner)
        .bind(&slug)
        .execute(&mut *tx)
        .await
        .map_err(db(&rid))?;
    sqlx::query("INSERT INTO cloud_library_versions(document_id,revision,title,document,sha256,status) VALUES($1,1,$2,$3,$4,'draft')")
        .bind(id)
        .bind(&title)
        .bind(&request.document)
        .bind(&digest)
        .execute(&mut *tx)
        .await
        .map_err(db(&rid))?;
    tx.commit().await.map_err(db(&rid))?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"id": id, "revision": 1, "status": "draft", "schemaVersion": SCHEMA_VERSION})),
    ))
}

pub async fn read(
    Authenticated(user): Authenticated,
    Extension(rid): Extension<RequestId>,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, V3Error> {
    let overlay = parse_uuid(&rid, &id)?;
    let row = sqlx::query("SELECT d.revision,v.status,v.document FROM cloud_library_documents d JOIN cloud_library_versions v ON v.document_id=d.id AND v.revision=d.revision WHERE d.id=$1 AND d.owner_user_id=$2 AND d.kind='overlay'")
        .bind(overlay)
        .bind(&user.user.id)
        .fetch_optional(&state.pool)
        .await
        .map_err(db(&rid))?
        .ok_or(V3Error::new(&rid, Kind::NotFound))?;
    use sqlx::Row;
    Ok(Json(json!({
        "id": overlay,
        "revision": row.get::<i64, _>("revision"),
        "status": row.get::<String, _>("status"),
        "schemaVersion": SCHEMA_VERSION,
        "document": row.get::<Value, _>("document"),
    })))
}

/// Adds a revision. The expected revision guards against overwriting a newer edit.
pub async fn revise(
    Authenticated(user): Authenticated,
    Extension(rid): Extension<RequestId>,
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Bytes,
) -> Result<(StatusCode, Json<Value>), V3Error> {
    let owner = &user.user.id;
    throttle(&state, &rid, owner, "overlay-write", 60).await?;
    let overlay = parse_uuid(&rid, &id)?;
    let request: ReviseOverlay = parse_body(&rid, &body, MAX_DOCUMENT_BYTES + 1024)?;
    let checked = check(&request.document).map_err(|r| refusal(&rid, r))?;
    if !assets_owned(&state.pool, owner, &checked.assets)
        .await
        .map_err(db(&rid))?
    {
        return Err(V3Error::new(&rid, Kind::NotFound));
    }
    let mut tx = state.pool.begin().await.map_err(db(&rid))?;
    let current: i64 = sqlx::query_scalar("SELECT revision FROM cloud_library_documents WHERE id=$1 AND owner_user_id=$2 AND kind='overlay' FOR UPDATE")
        .bind(overlay)
        .bind(owner)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db(&rid))?
        .ok_or(V3Error::new(&rid, Kind::NotFound))?;
    if current != request.expected_revision {
        return Err(V3Error::new(&rid, Kind::Conflict));
    }
    let next = current + 1;
    let title = request.document["title"]
        .as_str()
        .unwrap_or_default()
        .trim()
        .to_owned();
    sqlx::query("INSERT INTO cloud_library_versions(document_id,revision,title,document,sha256,status) VALUES($1,$2,$3,$4,$5,'draft')")
        .bind(overlay)
        .bind(next)
        .bind(&title)
        .bind(&request.document)
        .bind(binding_digest(&request.document))
        .execute(&mut *tx)
        .await
        .map_err(db(&rid))?;
    sqlx::query("UPDATE cloud_library_documents SET revision=$2 WHERE id=$1")
        .bind(overlay)
        .bind(next)
        .execute(&mut *tx)
        .await
        .map_err(db(&rid))?;
    tx.commit().await.map_err(db(&rid))?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"id": overlay, "revision": next, "status": "draft"})),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn overlay() -> Value {
        json!({
            "overlayVersion": 1,
            "kind": "chat",
            "title": "Chat",
            "canvas": {"width": 1920, "height": 1080},
            "elements": [
                {"id": "title", "type": "text", "x": 0, "y": 0, "width": 400, "height": 60, "source": {"kind": "static", "text": "Live chat"}},
                {"id": "who", "type": "text", "x": 0, "y": 60, "width": 400, "height": 40, "opacity": 0.8, "source": {"kind": "field", "field": "viewer.name"}},
                {"id": "avatar", "type": "avatar", "x": 0, "y": 100, "width": 64, "height": 64, "source": {"kind": "field", "field": "viewer.avatar"}}
            ],
            "capabilities": ["live.read"]
        })
    }

    #[test]
    fn accepts_a_typed_layout_and_reports_needed_capabilities() {
        let checked = check(&overlay()).unwrap();
        assert!(checked.capabilities.contains("live.read"));
        assert!(checked.assets.is_empty());
    }

    #[test]
    fn refuses_unknown_keys_types_and_out_of_canvas_geometry() {
        let mut doc = overlay();
        doc["script"] = json!("alert(1)");
        assert_eq!(check(&doc), Err("unknownKey"));
        let mut doc = overlay();
        doc["elements"][0]["type"] = json!("html");
        assert_eq!(check(&doc), Err("invalidElementType"));
        let mut doc = overlay();
        doc["elements"][0]["x"] = json!(1900);
        assert_eq!(check(&doc), Err("invalidGeometry"));
        let mut doc = overlay();
        doc["elements"][1]["source"] = json!({"kind": "field", "field": "secret.token"});
        assert_eq!(check(&doc), Err("invalidSource"));
    }

    #[test]
    fn avatar_and_field_sources_require_the_live_capability_to_be_declared() {
        let mut doc = overlay();
        doc.as_object_mut().unwrap().remove("capabilities");
        assert_eq!(check(&doc), Err("missingCapability"));
    }

    #[test]
    fn art_elements_name_an_asset_and_carry_no_free_text() {
        let mut doc = overlay();
        doc["elements"] = json!([
            {"id": "logo", "type": "art", "x": 0, "y": 0, "width": 128, "height": 128, "assetId": Uuid::new_v4().to_string()}
        ]);
        doc["capabilities"] = json!([]);
        let checked = check(&doc).unwrap();
        assert_eq!(checked.assets.len(), 1);
        doc["elements"][0]["text"] = json!("<img src=x>");
        assert_eq!(check(&doc), Err("unexpectedText"));
    }

    #[test]
    fn element_ids_are_unique_and_bounded() {
        let mut doc = overlay();
        doc["elements"][1]["id"] = json!("title");
        assert_eq!(check(&doc), Err("duplicateElementId"));
        let mut doc = overlay();
        doc["elements"] = json!((0..65).map(|i| json!({"id": format!("e{i}"), "type": "text", "x": 0, "y": 0, "width": 1, "height": 1, "source": {"kind": "static", "text": "x"}})).collect::<Vec<_>>());
        assert_eq!(check(&doc), Err("tooManyElements"));
    }

    #[test]
    fn schema_version_and_canvas_are_fixed() {
        let mut doc = overlay();
        doc["overlayVersion"] = json!(2);
        assert_eq!(check(&doc), Err("unsupportedSchema"));
        let mut doc = overlay();
        doc["canvas"] = json!({"width": 100, "height": 100});
        assert_eq!(check(&doc), Err("invalidCanvas"));
    }
}
