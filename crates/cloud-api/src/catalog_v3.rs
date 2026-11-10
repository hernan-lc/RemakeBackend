//! Visual product catalog (v3).
//!
//! Projects existing verified metadata: plugins come from the active registry
//! generation and templates/profiles from their published revisions. The v3
//! presentation never replaces installation metadata; the signed package and
//! its generation remain the only source of install decisions.

use crate::api_error::{Authenticated, Kind, RequestId, V3Error};
use crate::{remote::db, AppState};
use axum::{
    body::Bytes,
    extract::{Path, RawQuery, State},
    Extension, Json,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{PgPool, Row};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

pub const MAX_LIMIT: i64 = 50;
pub const MAX_QUERY_CHARS: usize = 100;
pub const MAX_RESOLVE: usize = 50;
const RANK_PLUGIN: u8 = 0;
const RANK_PROFILE: u8 = 1;
const RANK_TEMPLATE: u8 = 2;

pub fn routes() -> axum::Router<AppState> {
    use axum::routing::{get, post};
    axum::Router::new()
        .route("/api/v3/catalog/items", get(list_items))
        .route("/api/v3/catalog/items/{kind}/{id}", get(get_item))
        .route("/api/v3/catalog/items/resolve", post(resolve_items))
        .route("/api/v3/library/items", get(list_library))
        .route("/api/v3/library/items/{id}", get(get_library_item))
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
struct Cursor {
    v: u8,
    gen: Option<i64>,
    q: Option<String>,
    kind: Option<String>,
    target: Option<String>,
    rank: u8,
    id: String,
}

fn encode_cursor(cursor: &Cursor) -> String {
    URL_SAFE_NO_PAD.encode(serde_json::to_vec(cursor).unwrap_or_default())
}

fn decode_cursor(rid: &RequestId, raw: &str) -> Result<Cursor, V3Error> {
    let bytes = URL_SAFE_NO_PAD
        .decode(raw)
        .map_err(|_| V3Error::new(rid, Kind::InvalidRequest))?;
    let cursor: Cursor =
        serde_json::from_slice(&bytes).map_err(|_| V3Error::new(rid, Kind::InvalidRequest))?;
    if cursor.v != 1 {
        return Err(V3Error::new(rid, Kind::InvalidRequest));
    }
    Ok(cursor)
}

/// Parses the query string with a closed set of keys. Unknown keys are refused
/// so a typo cannot silently widen a search.
fn query_map(
    rid: &RequestId,
    raw: Option<String>,
    allowed: &[&str],
) -> Result<BTreeMap<String, String>, V3Error> {
    let mut map = BTreeMap::new();
    let Some(raw) = raw else {
        return Ok(map);
    };
    for pair in raw.split('&').filter(|p| !p.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let key = percent_decode(key).ok_or(V3Error::new(rid, Kind::InvalidRequest))?;
        let value = percent_decode(value).ok_or(V3Error::new(rid, Kind::InvalidRequest))?;
        if !allowed.contains(&key.as_str()) || map.insert(key, value).is_some() {
            return Err(V3Error::new(rid, Kind::InvalidRequest));
        }
    }
    Ok(map)
}

fn percent_decode(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' => {
                let hex = raw.get(i + 1..i + 3)?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                i += 3;
            }
            other => {
                out.push(other);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

fn parse_limit(rid: &RequestId, raw: Option<&String>) -> Result<i64, V3Error> {
    match raw {
        None => Ok(MAX_LIMIT),
        Some(value) => value
            .parse::<i64>()
            .ok()
            .filter(|n| (1..=MAX_LIMIT).contains(n))
            .ok_or(V3Error::new(rid, Kind::InvalidRequest)),
    }
}

fn parse_search(rid: &RequestId, raw: Option<&String>) -> Result<Option<String>, V3Error> {
    match raw.map(|s| s.trim()) {
        None | Some("") => Ok(None),
        Some(value)
            if value.chars().count() <= MAX_QUERY_CHARS && !value.chars().any(char::is_control) =>
        {
            Ok(Some(value.to_owned()))
        }
        Some(_) => Err(V3Error::new(rid, Kind::InvalidRequest)),
    }
}

fn parse_kinds(rid: &RequestId, raw: Option<&String>) -> Result<Vec<&'static str>, V3Error> {
    match raw.map(String::as_str) {
        None | Some("") => Ok(vec!["plugin", "profile", "template"]),
        Some("plugin") => Ok(vec!["plugin"]),
        Some("profile") => Ok(vec!["profile"]),
        Some("template") => Ok(vec!["template"]),
        Some(_) => Err(V3Error::new(rid, Kind::InvalidRequest)),
    }
}

fn kind_of(rank: u8) -> &'static str {
    match rank {
        RANK_PLUGIN => "plugin",
        RANK_PROFILE => "profile",
        _ => "template",
    }
}

async fn active_generation(pool: &PgPool, rid: &RequestId) -> Result<Option<i64>, V3Error> {
    sqlx::query_scalar("SELECT generation FROM cloud_registry_generations WHERE status='active'")
        .fetch_optional(pool)
        .await
        .map_err(db(rid))
}

fn localized_text(value: &Value, max: usize) -> Option<String> {
    let text = match value {
        Value::String(s) => Some(s.as_str()),
        Value::Object(map) => map.get("en").and_then(Value::as_str).or_else(|| {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            keys.into_iter().find_map(|k| map[k].as_str())
        }),
        _ => None,
    }?;
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.chars().take(max).collect())
}

fn semver_stable(version: &str) -> Option<semver::Version> {
    semver::Version::parse(version)
        .ok()
        .filter(|v| v.pre.is_empty())
}

fn installable(r: &sqlx::postgres::PgRow) -> bool {
    !r.get::<bool, _>("yanked")
        && r.get::<String, _>("key_status") == "approved"
        && r.get::<String, _>("publisher_status") == "approved"
}

/// One plugin, projected from the rows of its generation. The representative
/// release is the newest installable stable version, else the newest installable
/// version; a plugin with none is listed but not offered for installation.
fn plugin_item(plugin_id: &str, generation: i64, rows: &[&sqlx::postgres::PgRow]) -> Value {
    let mut versions: Vec<&sqlx::postgres::PgRow> = rows.to_vec();
    versions.sort_by(|a, b| {
        let av = semver::Version::parse(&a.get::<String, _>("version")).ok();
        let bv = semver::Version::parse(&b.get::<String, _>("version")).ok();
        bv.cmp(&av)
    });
    let representative = versions
        .iter()
        .copied()
        .find(|r| installable(r) && semver_stable(&r.get::<String, _>("version")).is_some())
        .or_else(|| versions.iter().copied().find(|r| installable(r)));
    let targets: BTreeSet<String> = rows.iter().map(|r| r.get::<String, _>("target")).collect();
    let (title, summary, publisher, authors, categories, engine, homepage, dependencies) =
        match representative {
            Some(r) => (
                r.get::<Option<String>, _>("display_name"),
                r.get::<Option<String>, _>("summary"),
                Some(r.get::<String, _>("namespace")),
                r.get::<Vec<String>, _>("authors"),
                r.get::<Vec<String>, _>("categories"),
                r.get::<Option<String>, _>("engine"),
                r.get::<Option<String>, _>("homepage"),
                r.get::<Value, _>("dependencies"),
            ),
            None => (
                None,
                None,
                None,
                Vec::new(),
                Vec::new(),
                None,
                None,
                json!({}),
            ),
        };
    let publishable = representative;
    let advisory = representative.and_then(|r| {
        r.get::<Option<String>, _>("severity").map(|severity| {
            json!({
                "severity": severity,
                "message": r.get::<Option<String>, _>("message"),
                "url": r.get::<Option<String>, _>("url"),
            })
        })
    });
    let release_ref = publishable.map(|r| {
        json!({
            "type": "pluginRelease",
            "generation": generation,
            "version": r.get::<String, _>("version"),
            "target": r.get::<String, _>("target"),
            "sha256": r.get::<String, _>("sha256"),
            "size": r.get::<i64, _>("size"),
        })
    });
    json!({
        "identity": {"kind": "plugin", "id": plugin_id},
        "presentation": {
            "title": title.unwrap_or_else(|| plugin_id.to_owned()),
            "summary": summary,
            "publisher": publisher,
            "authors": authors,
            "categories": categories,
            "tags": [],
        },
        "media": {"iconAssetId": null},
        "compatibility": {"targets": targets, "engine": engine, "homepage": homepage},
        "releaseRef": release_ref,
        "publication": {
            "status": if publishable.is_some() { "published" } else { "notInstallable" },
            "yanked": representative.map(|r| r.get::<bool, _>("yanked")).unwrap_or(false),
            "keyRevoked": representative.map(|r| r.get::<String, _>("key_status") != "approved").unwrap_or(false),
            "publisherApproved": representative.map(|r| r.get::<String, _>("publisher_status") == "approved").unwrap_or(false),
            "advisory": advisory,
        },
        "requirements": {"dependencies": dependencies},
    })
}

const PLUGIN_ROWS: &str = "SELECT g.plugin_id,g.version,g.target,g.sha256,g.size,(y.release_id IS NOT NULL) AS yanked,k.status AS key_status,p.status AS publisher_status,p.namespace,r.display_name,r.summary,r.categories,r.authors,r.engine,r.homepage,r.dependencies,a.severity,a.message,a.url FROM cloud_registry_generation_releases g JOIN cloud_plugin_releases r ON r.id=g.release_id JOIN cloud_publishers p ON p.id=r.publisher_id JOIN cloud_publisher_keys k ON k.id=r.publisher_key_id LEFT JOIN cloud_release_yanks y ON y.release_id=r.id LEFT JOIN cloud_release_advisories a ON a.release_id=r.id";

async fn plugin_page(
    pool: &PgPool,
    rid: &RequestId,
    generation: i64,
    after: Option<&str>,
    q: Option<&str>,
    target: Option<&str>,
    limit: i64,
) -> Result<Vec<Value>, V3Error> {
    let ids: Vec<String> = sqlx::query_scalar(r#"SELECT DISTINCT g.plugin_id COLLATE "C" AS plugin_id FROM cloud_registry_generation_releases g JOIN cloud_plugin_releases r ON r.id=g.release_id JOIN cloud_publishers p ON p.id=r.publisher_id WHERE g.generation=$1 AND ($2::text IS NULL OR g.plugin_id COLLATE "C" > $2) AND ($3::text IS NULL OR strpos(lower(g.plugin_id),lower($3))>0 OR strpos(lower(coalesce(r.display_name,'')),lower($3))>0 OR strpos(lower(coalesce(r.summary,'')),lower($3))>0 OR strpos(lower(p.namespace),lower($3))>0 OR EXISTS (SELECT 1 FROM unnest(r.categories) c WHERE strpos(c,lower($3))>0)) AND ($4::text IS NULL OR g.target=$4 OR g.target='any') ORDER BY g.plugin_id COLLATE "C" LIMIT $5"#)
        .bind(generation)
        .bind(after)
        .bind(q)
        .bind(target)
        .bind(limit)
        .fetch_all(pool)
        .await
        .map_err(db(rid))?;
    plugin_items_for(pool, rid, generation, &ids, target).await
}

async fn plugin_items_for(
    pool: &PgPool,
    rid: &RequestId,
    generation: i64,
    ids: &[String],
    target: Option<&str>,
) -> Result<Vec<Value>, V3Error> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows = sqlx::query(&format!("{PLUGIN_ROWS} WHERE g.generation=$1 AND g.plugin_id = ANY($2) AND ($3::text IS NULL OR g.target=$3 OR g.target='any') ORDER BY g.plugin_id COLLATE \"C\", g.version, g.target"))
        .bind(generation)
        .bind(ids)
        .bind(target)
        .fetch_all(pool)
        .await
        .map_err(db(rid))?;
    let mut items = Vec::new();
    for id in ids {
        let mine: Vec<&sqlx::postgres::PgRow> = rows
            .iter()
            .filter(|r| r.get::<String, _>("plugin_id") == *id)
            .collect();
        if !mine.is_empty() {
            items.push(plugin_item(id, generation, &mine));
        }
    }
    Ok(items)
}

fn document_item(row: &sqlx::postgres::PgRow) -> Value {
    let document: Value = row.get("document");
    let title: String = row.get("title");
    json!({
        "identity": {"kind": row.get::<String, _>("kind"), "id": row.get::<Uuid, _>("id").to_string()},
        "presentation": {
            "title": title,
            "summary": document.get("description").and_then(|d| localized_text(d, 500)),
            "publisher": null,
            "authors": [],
            "categories": [],
            "tags": [],
        },
        "media": {"iconAssetId": null},
        "compatibility": {"targets": [], "engine": null, "homepage": null},
        "releaseRef": {
            "type": "documentRevision",
            "revision": row.get::<i64, _>("revision"),
            "sha256": row.get::<String, _>("sha256"),
        },
        "publication": {"status": row.get::<String, _>("status")},
        "requirements": {"dependencies": {}},
    })
}

async fn document_page(
    pool: &PgPool,
    rid: &RequestId,
    kind: &str,
    after: Option<Uuid>,
    q: Option<&str>,
    limit: i64,
) -> Result<Vec<(Uuid, Value)>, V3Error> {
    let rows = sqlx::query("SELECT d.id,d.kind,d.slug,v.revision,v.title,v.sha256,v.status,v.document FROM cloud_library_documents d JOIN cloud_library_versions v ON v.document_id=d.id AND v.revision=d.published_revision WHERE v.status='published' AND d.kind=$1 AND ($2::uuid IS NULL OR d.id>$2) AND ($3::text IS NULL OR strpos(lower(v.title),lower($3))>0 OR strpos(lower(coalesce(v.document->>'description','')),lower($3))>0) ORDER BY d.id LIMIT $4")
        .bind(kind)
        .bind(after)
        .bind(q)
        .bind(limit)
        .fetch_all(pool)
        .await
        .map_err(db(rid))?;
    Ok(rows
        .iter()
        .map(|r| (r.get::<Uuid, _>("id"), document_item(r)))
        .collect())
}

/// Keyset page over the merged catalog. Each rank is a separate ordered stream;
/// the cursor records the rank and last identifier so pages never overlap.
async fn list_items(
    State(state): State<AppState>,
    Extension(rid): Extension<RequestId>,
    RawQuery(raw): RawQuery,
) -> Result<Json<Value>, V3Error> {
    let params = query_map(&rid, raw, &["q", "kind", "target", "cursor", "limit"])?;
    let q = parse_search(&rid, params.get("q"))?;
    let kinds = parse_kinds(&rid, params.get("kind"))?;
    let limit = parse_limit(&rid, params.get("limit"))?;
    let target = params.get("target").cloned();
    if let Some(t) = &target {
        if t.is_empty()
            || t.len() > 128
            || !t
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
        {
            return Err(V3Error::new(&rid, Kind::InvalidRequest));
        }
    }
    let generation = active_generation(&state.pool, &rid).await?;
    let mut start_rank = 0u8;
    let mut after_id: Option<String> = None;
    if let Some(raw_cursor) = params.get("cursor") {
        let cursor = decode_cursor(&rid, raw_cursor)?;
        if cursor.q != q || cursor.kind != params.get("kind").cloned() || cursor.target != target {
            return Err(V3Error::new(&rid, Kind::InvalidRequest));
        }
        if cursor.gen != generation {
            return Err(V3Error::new(&rid, Kind::Expired));
        }
        start_rank = cursor.rank;
        after_id = Some(cursor.id);
    }
    let mut collected: Vec<(u8, String, Value)> = Vec::new();
    let needed = limit + 1;
    for rank in [RANK_PLUGIN, RANK_PROFILE, RANK_TEMPLATE] {
        if rank < start_rank || !kinds.contains(&kind_of(rank)) || collected.len() as i64 >= needed
        {
            continue;
        }
        let after = if rank == start_rank {
            after_id.clone()
        } else {
            None
        };
        let remaining = needed - collected.len() as i64;
        if rank == RANK_PLUGIN {
            let Some(gen) = generation else { continue };
            for item in plugin_page(
                &state.pool,
                &rid,
                gen,
                after.as_deref(),
                q.as_deref(),
                target.as_deref(),
                remaining,
            )
            .await?
            {
                let id = item["identity"]["id"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned();
                collected.push((rank, id, item));
            }
        } else {
            let after_uuid = match after {
                Some(raw) => Some(
                    Uuid::parse_str(&raw).map_err(|_| V3Error::new(&rid, Kind::InvalidRequest))?,
                ),
                None => None,
            };
            for (id, item) in document_page(
                &state.pool,
                &rid,
                kind_of(rank),
                after_uuid,
                q.as_deref(),
                remaining,
            )
            .await?
            {
                collected.push((rank, id.to_string(), item));
            }
        }
    }
    let more = collected.len() as i64 > limit;
    collected.truncate(limit as usize);
    let next = if more {
        collected.last().map(|(rank, id, _)| {
            encode_cursor(&Cursor {
                v: 1,
                gen: generation,
                q: q.clone(),
                kind: params.get("kind").cloned(),
                target: target.clone(),
                rank: *rank,
                id: id.clone(),
            })
        })
    } else {
        None
    };
    let items: Vec<Value> = collected.into_iter().map(|(_, _, item)| item).collect();
    Ok(Json(json!({
        "apiVersion": 3,
        "dataState": if generation.is_some() { "ready" } else { "noActiveGeneration" },
        "generation": generation,
        "items": items,
        "nextCursor": next,
    })))
}

async fn item_for(
    state: &AppState,
    rid: &RequestId,
    kind: &str,
    id: &str,
    target: Option<&str>,
) -> Result<Option<Value>, V3Error> {
    match kind {
        "plugin" => {
            let Some(generation) = active_generation(&state.pool, rid).await? else {
                return Ok(None);
            };
            Ok(
                plugin_items_for(&state.pool, rid, generation, &[id.to_owned()], target)
                    .await?
                    .into_iter()
                    .next(),
            )
        }
        "template" | "profile" => {
            let uuid = Uuid::parse_str(id).map_err(|_| V3Error::new(rid, Kind::InvalidRequest))?;
            let row = sqlx::query("SELECT d.id,d.kind,d.slug,v.revision,v.title,v.sha256,v.status,v.document FROM cloud_library_documents d JOIN cloud_library_versions v ON v.document_id=d.id AND v.revision=d.published_revision WHERE d.id=$1 AND d.kind=$2 AND v.status='published'")
                .bind(uuid)
                .bind(kind)
                .fetch_optional(&state.pool)
                .await
                .map_err(db(rid))?;
            Ok(row.as_ref().map(document_item))
        }
        _ => Err(V3Error::new(rid, Kind::InvalidRequest)),
    }
}

async fn get_item(
    State(state): State<AppState>,
    Extension(rid): Extension<RequestId>,
    Path((kind, id)): Path<(String, String)>,
    RawQuery(raw): RawQuery,
) -> Result<Json<Value>, V3Error> {
    let params = query_map(&rid, raw, &["target"])?;
    let target = params.get("target").cloned();
    match item_for(&state, &rid, &kind, &id, target.as_deref()).await? {
        Some(item) => Ok(Json(json!({"apiVersion": 3, "item": item}))),
        None => Err(V3Error::new(&rid, Kind::NotFound)),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResolveRequest {
    items: Vec<ResolveRef>,
}

#[derive(Deserialize, Serialize, Clone)]
struct ResolveRef {
    kind: String,
    id: String,
}

/// Resolves a bounded batch of identities. Unknown items are reported, not failed,
/// so a stale reference cannot break the rest of the page.
async fn resolve_items(
    State(state): State<AppState>,
    Extension(rid): Extension<RequestId>,
    body: Bytes,
) -> Result<Json<Value>, V3Error> {
    let request: ResolveRequest = crate::remote::parse_body(&rid, &body, 16 * 1024)?;
    if request.items.is_empty() || request.items.len() > MAX_RESOLVE {
        return Err(V3Error::new(&rid, Kind::InvalidRequest));
    }
    let mut found = Vec::new();
    let mut missing = Vec::new();
    for reference in request.items {
        match item_for(&state, &rid, &reference.kind, &reference.id, None).await {
            Ok(Some(item)) => found.push(item),
            Ok(None) => missing.push(reference),
            Err(_) => missing.push(reference),
        }
    }
    Ok(Json(
        json!({"apiVersion": 3, "items": found, "missing": missing}),
    ))
}

/// The caller's own templates and profiles, any status. Requires a session.
async fn list_library(
    Authenticated(user): Authenticated,
    State(state): State<AppState>,
    Extension(rid): Extension<RequestId>,
    RawQuery(raw): RawQuery,
) -> Result<Json<Value>, V3Error> {
    let params = query_map(&rid, raw, &["q", "kind", "cursor", "limit"])?;
    let q = parse_search(&rid, params.get("q"))?;
    let limit = parse_limit(&rid, params.get("limit"))?;
    let kind = match params.get("kind").map(String::as_str) {
        None | Some("") => None,
        Some("template") => Some("template"),
        Some("profile") => Some("profile"),
        Some(_) => return Err(V3Error::new(&rid, Kind::InvalidRequest)),
    };
    let after = match params.get("cursor") {
        Some(raw) => {
            Some(Uuid::parse_str(raw).map_err(|_| V3Error::new(&rid, Kind::InvalidRequest))?)
        }
        None => None,
    };
    let rows = sqlx::query("SELECT d.id,d.kind,d.slug,v.revision,v.title,v.sha256,v.status,v.document FROM cloud_library_documents d JOIN cloud_library_versions v ON v.document_id=d.id AND v.revision=d.revision WHERE d.owner_user_id=$1 AND ($2::uuid IS NULL OR d.id>$2) AND ($3::text IS NULL OR d.kind=$3) AND ($4::text IS NULL OR strpos(lower(v.title),lower($4))>0) ORDER BY d.id LIMIT $5")
        .bind(&user.user.id)
        .bind(after)
        .bind(kind)
        .bind(q)
        .bind(limit + 1)
        .fetch_all(&state.pool)
        .await
        .map_err(db(&rid))?;
    let more = rows.len() as i64 > limit;
    let items: Vec<Value> = rows
        .iter()
        .take(limit as usize)
        .map(document_item)
        .collect();
    let next = if more {
        items
            .last()
            .and_then(|item| item["identity"]["id"].as_str().map(str::to_owned))
    } else {
        None
    };
    Ok(Json(
        json!({"apiVersion": 3, "items": items, "nextCursor": next}),
    ))
}

async fn get_library_item(
    Authenticated(user): Authenticated,
    State(state): State<AppState>,
    Extension(rid): Extension<RequestId>,
    Path(id): Path<String>,
) -> Result<Json<Value>, V3Error> {
    let uuid = Uuid::parse_str(&id).map_err(|_| V3Error::new(&rid, Kind::InvalidRequest))?;
    let row = sqlx::query("SELECT d.id,d.kind,d.slug,v.revision,v.title,v.sha256,v.status,v.document FROM cloud_library_documents d JOIN cloud_library_versions v ON v.document_id=d.id AND v.revision=d.revision WHERE d.id=$1 AND d.owner_user_id=$2")
        .bind(uuid)
        .bind(&user.user.id)
        .fetch_optional(&state.pool)
        .await
        .map_err(db(&rid))?
        .ok_or(V3Error::new(&rid, Kind::NotFound))?;
    Ok(Json(json!({"apiVersion": 3, "item": document_item(&row)})))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_roundtrips_and_rejects_garbage() {
        let cursor = Cursor {
            v: 1,
            gen: Some(7),
            q: Some("gift".into()),
            kind: None,
            target: None,
            rank: RANK_PROFILE,
            id: "abc".into(),
        };
        let raw = encode_cursor(&cursor);
        let rid = RequestId("r".into());
        assert_eq!(decode_cursor(&rid, &raw).unwrap(), cursor);
        assert!(decode_cursor(&rid, "not base64 !!").is_err());
        let wrong_version = URL_SAFE_NO_PAD
            .encode(br#"{"v":2,"gen":null,"q":null,"kind":null,"target":null,"rank":0,"id":""}"#);
        assert!(decode_cursor(&rid, &wrong_version).is_err());
    }

    #[test]
    fn query_keys_are_closed_and_duplicates_are_refused() {
        let rid = RequestId("r".into());
        assert!(query_map(&rid, Some("q=a&kind=plugin".into()), &["q", "kind"]).is_ok());
        assert!(query_map(&rid, Some("q=a&bogus=1".into()), &["q", "kind"]).is_err());
        assert!(query_map(&rid, Some("q=a&q=b".into()), &["q"]).is_err());
        let decoded = query_map(&rid, Some("q=gift%20alerts+x".into()), &["q"]).unwrap();
        assert_eq!(decoded.get("q").map(String::as_str), Some("gift alerts x"));
    }

    #[test]
    fn limits_are_bounded() {
        let rid = RequestId("r".into());
        assert_eq!(parse_limit(&rid, None).unwrap(), MAX_LIMIT);
        assert!(parse_limit(&rid, Some(&"0".to_string())).is_err());
        assert!(parse_limit(&rid, Some(&"51".to_string())).is_err());
        assert_eq!(parse_limit(&rid, Some(&"50".to_string())).unwrap(), 50);
    }

    #[test]
    fn localized_text_prefers_english_then_stable_order() {
        assert_eq!(
            localized_text(&json!("  Hello  "), 100).as_deref(),
            Some("Hello")
        );
        assert_eq!(
            localized_text(&json!({"es": "Hola", "en": "Hi"}), 100).as_deref(),
            Some("Hi")
        );
        assert_eq!(
            localized_text(&json!({"fr": "Bonjour", "de": "Hallo"}), 100).as_deref(),
            Some("Hallo")
        );
        assert_eq!(localized_text(&json!(7), 100), None);
    }

    #[test]
    fn kinds_default_to_all_and_reject_unknown() {
        let rid = RequestId("r".into());
        assert_eq!(parse_kinds(&rid, None).unwrap().len(), 3);
        assert!(parse_kinds(&rid, Some(&"overlay".to_string())).is_err());
    }
}
