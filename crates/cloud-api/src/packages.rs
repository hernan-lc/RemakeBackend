//! Package documents: pinned references to plugins, templates, profiles and
//! overlays, plus a read-only preflight.
//!
//! A package is not an install script. It names what it needs at an exact
//! version or revision. Applying it stays in the client, which runs the existing
//! inspect/consent/job flow for plugins and the typed import flow for documents.
//! Imported automations always arrive disabled.

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
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;
use std::collections::HashSet;
use uuid::Uuid;

pub const PACKAGE_VERSION: i64 = 1;
pub const MAX_ITEMS: usize = 50;
const MAX_DOCUMENT_BYTES: usize = 32 * 1024;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/v3/packages", post(create))
        .route(
            "/api/v3/packages/{id}",
            get(read).merge(
                axum::routing::delete(remove)
                    .layer(axum::middleware::map_response(crate::sync::no_store)),
            ),
        )
        .route("/api/v3/packages/{id}/preflight", get(preflight))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreatePackage {
    title: String,
    items: Vec<Item>,
}

#[derive(Deserialize, serde::Serialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Item {
    kind: String,
    id: String,
    /// Plugins pin a semantic version; documents and overlays pin a revision.
    version: Option<String>,
    revision: Option<i64>,
    required: bool,
}

/// Structural checks that need no database. Returns the first refusal reason.
pub fn check(title: &str, items: &[Item]) -> Result<(), &'static str> {
    if title.trim().is_empty() || title.chars().count() > 120 || title.chars().any(char::is_control)
    {
        return Err("invalidTitle");
    }
    if items.is_empty() || items.len() > MAX_ITEMS {
        return Err("invalidItems");
    }
    let mut seen = HashSet::new();
    for item in items {
        if !seen.insert((item.kind.clone(), item.id.clone())) {
            return Err("duplicateItem");
        }
        match item.kind.as_str() {
            "plugin" => {
                if !tiktools_plugin_api::manifest::is_valid_plugin_id(&item.id) {
                    return Err("invalidItem");
                }
                let version = item.version.as_deref().ok_or("missingVersion")?;
                if item.revision.is_some()
                    || tiktools_plugin_api::manifest::parse_version(version).is_none()
                {
                    return Err("invalidVersion");
                }
            }
            "template" | "profile" | "overlay" => {
                if Uuid::parse_str(&item.id).is_err() {
                    return Err("invalidItem");
                }
                if item.version.is_some() || item.revision.filter(|r| *r >= 1).is_none() {
                    return Err("missingRevision");
                }
            }
            _ => return Err("invalidItem"),
        }
    }
    Ok(())
}

pub async fn create(
    Authenticated(user): Authenticated,
    Extension(rid): Extension<RequestId>,
    State(state): State<AppState>,
    body: Bytes,
) -> Result<(StatusCode, Json<Value>), V3Error> {
    let owner = &user.user.id;
    throttle(&state, &rid, owner, "package-write", 30).await?;
    let request: CreatePackage = parse_body(&rid, &body, MAX_DOCUMENT_BYTES)?;
    check(&request.title, &request.items).map_err(|_| V3Error::new(&rid, Kind::InvalidRequest))?;
    // Every pinned reference must exist at the pinned version now; drift later is a preflight result.
    for item in &request.items {
        let status = evaluate(&state, &rid, owner, item).await?;
        if status == "missing" || status == "noActiveGeneration" {
            return Err(V3Error::new(&rid, Kind::NotFound));
        }
        if status == "revisionChanged" {
            return Err(V3Error::new(&rid, Kind::Conflict));
        }
    }
    let id = Uuid::new_v4();
    let document = json!({
        "packageVersion": PACKAGE_VERSION,
        "title": request.title.trim(),
        "items": request.items,
    });
    let digest = binding_digest(&document);
    let mut tx = state.pool.begin().await.map_err(db(&rid))?;
    // Same per-owner lock as overlay/asset deletion: a pinned overlay cannot be
    // deleted between the check above and this insert.
    crate::assets::lock_owner(&mut tx, &rid, owner).await?;
    let pinned: HashSet<Uuid> = request
        .items
        .iter()
        .filter(|item| item.kind == "overlay")
        .filter_map(|item| Uuid::parse_str(&item.id).ok())
        .collect();
    if !pinned.is_empty() {
        let ids: Vec<Uuid> = pinned.iter().copied().collect();
        let found: i64 = sqlx::query_scalar("SELECT count(*) FROM cloud_library_documents WHERE owner_user_id=$1 AND kind='overlay' AND id = ANY($2)")
            .bind(owner)
            .bind(&ids)
            .fetch_one(&mut *tx)
            .await
            .map_err(db(&rid))?;
        if found as usize != ids.len() {
            return Err(V3Error::new(&rid, Kind::NotFound));
        }
    }
    sqlx::query("INSERT INTO cloud_library_documents(id,owner_user_id,kind,slug) VALUES($1,$2,'package',$3)")
        .bind(id)
        .bind(owner)
        .bind(format!("pkg-{}", id.simple()))
        .execute(&mut *tx)
        .await
        .map_err(db(&rid))?;
    sqlx::query("INSERT INTO cloud_library_versions(document_id,revision,title,document,sha256,status) VALUES($1,1,$2,$3,$4,'draft')")
        .bind(id)
        .bind(request.title.trim())
        .bind(&document)
        .bind(&digest)
        .execute(&mut *tx)
        .await
        .map_err(db(&rid))?;
    tx.commit().await.map_err(db(&rid))?;
    Ok((
        StatusCode::CREATED,
        Json(
            json!({"id": id, "revision": 1, "status": "draft", "packageVersion": PACKAGE_VERSION}),
        ),
    ))
}

pub async fn read(
    Authenticated(user): Authenticated,
    Extension(rid): Extension<RequestId>,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, V3Error> {
    let package = parse_uuid(&rid, &id)?;
    let row = sqlx::query("SELECT d.revision,v.document FROM cloud_library_documents d JOIN cloud_library_versions v ON v.document_id=d.id AND v.revision=d.revision WHERE d.id=$1 AND d.owner_user_id=$2 AND d.kind='package'")
        .bind(package)
        .bind(&user.user.id)
        .fetch_optional(&state.pool)
        .await
        .map_err(db(&rid))?
        .ok_or(V3Error::new(&rid, Kind::NotFound))?;
    Ok(Json(json!({
        "id": package,
        "revision": row.get::<i64, _>("revision"),
        "document": row.get::<Value, _>("document"),
    })))
}

/// `DELETE /api/v3/packages/{id}`: owner only; `notFound` for others and for a
/// repeated delete. A package is only a list of pins, so deleting it releases the
/// overlays it pinned. Shares the `package-write` limit.
pub async fn remove(
    Authenticated(user): Authenticated,
    Extension(rid): Extension<RequestId>,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, V3Error> {
    let owner = &user.user.id;
    throttle(&state, &rid, owner, "package-write", 30).await?;
    let package = parse_uuid(&rid, &id)?;
    let mut tx = state.pool.begin().await.map_err(db(&rid))?;
    crate::assets::lock_owner(&mut tx, &rid, owner).await?;
    let deleted = sqlx::query(
        "DELETE FROM cloud_library_documents WHERE id=$1 AND owner_user_id=$2 AND kind='package'",
    )
    .bind(package)
    .bind(owner)
    .execute(&mut *tx)
    .await
    .map_err(db(&rid))?;
    if deleted.rows_affected() == 0 {
        return Err(V3Error::new(&rid, Kind::NotFound));
    }
    tx.commit().await.map_err(db(&rid))?;
    Ok(Json(json!({"id": package, "deleted": true})))
}

/// Evaluates one pinned reference against current state.
async fn evaluate(
    state: &AppState,
    rid: &RequestId,
    owner: &str,
    item: &Item,
) -> Result<&'static str, V3Error> {
    match item.kind.as_str() {
        "plugin" => {
            let generation: Option<i64> = sqlx::query_scalar(
                "SELECT generation FROM cloud_registry_generations WHERE status='active'",
            )
            .fetch_optional(&state.pool)
            .await
            .map_err(db(rid))?;
            let Some(generation) = generation else {
                return Ok("noActiveGeneration");
            };
            let version = item.version.clone().unwrap_or_default();
            let row = sqlx::query("SELECT (y.release_id IS NOT NULL) AS yanked,k.status AS key_status,p.status AS publisher_status FROM cloud_registry_generation_releases g JOIN cloud_plugin_releases r ON r.id=g.release_id JOIN cloud_publishers p ON p.id=r.publisher_id JOIN cloud_publisher_keys k ON k.id=r.publisher_key_id LEFT JOIN cloud_release_yanks y ON y.release_id=r.id WHERE g.generation=$1 AND g.plugin_id=$2 AND g.version=$3 LIMIT 1")
                .bind(generation)
                .bind(&item.id)
                .bind(&version)
                .fetch_optional(&state.pool)
                .await
                .map_err(db(rid))?;
            Ok(match row {
                None => "missing",
                Some(row) => {
                    let installable = !row.get::<bool, _>("yanked")
                        && row.get::<String, _>("key_status") == "approved"
                        && row.get::<String, _>("publisher_status") == "approved";
                    if installable {
                        "ready"
                    } else {
                        "notInstallable"
                    }
                }
            })
        }
        "template" | "profile" => {
            let uuid =
                Uuid::parse_str(&item.id).map_err(|_| V3Error::new(rid, Kind::InvalidRequest))?;
            let published: Option<i64> = sqlx::query_scalar("SELECT d.published_revision FROM cloud_library_documents d JOIN cloud_library_versions v ON v.document_id=d.id AND v.revision=d.published_revision WHERE d.id=$1 AND d.kind=$2 AND v.status='published'")
                .bind(uuid)
                .bind(&item.kind)
                .fetch_optional(&state.pool)
                .await
                .map_err(db(rid))?
                .flatten();
            Ok(match published {
                None => "missing",
                Some(revision) if Some(revision) == item.revision => "ready",
                Some(_) => "revisionChanged",
            })
        }
        _ => {
            // Overlays and packages are private: only the owner's own copy can satisfy them.
            let uuid =
                Uuid::parse_str(&item.id).map_err(|_| V3Error::new(rid, Kind::InvalidRequest))?;
            let current: Option<i64> = sqlx::query_scalar("SELECT revision FROM cloud_library_documents WHERE id=$1 AND owner_user_id=$2 AND kind=$3")
                .bind(uuid)
                .bind(owner)
                .bind(&item.kind)
                .fetch_optional(&state.pool)
                .await
                .map_err(db(rid))?;
            Ok(match current {
                None => "missing",
                Some(revision) if Some(revision) == item.revision => "ready",
                Some(_) => "revisionChanged",
            })
        }
    }
}

/// Read-only preflight: what the package needs, what is ready, and what blocks
/// application. Applying happens in the client and is never implied by this call.
pub async fn preflight(
    Authenticated(user): Authenticated,
    Extension(rid): Extension<RequestId>,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, V3Error> {
    let owner = &user.user.id;
    let package = parse_uuid(&rid, &id)?;
    let row = sqlx::query("SELECT v.document FROM cloud_library_documents d JOIN cloud_library_versions v ON v.document_id=d.id AND v.revision=d.revision WHERE d.id=$1 AND d.owner_user_id=$2 AND d.kind='package'")
        .bind(package)
        .bind(owner)
        .fetch_optional(&state.pool)
        .await
        .map_err(db(&rid))?
        .ok_or(V3Error::new(&rid, Kind::NotFound))?;
    let document: Value = row.get("document");
    let items: Vec<Item> = serde_json::from_value(document["items"].clone())
        .map_err(|_| V3Error::new(&rid, Kind::TemporaryFailure))?;
    let mut report = Vec::new();
    let mut blocking = Vec::new();
    for item in &items {
        let status = evaluate(&state, &rid, owner, item).await?;
        let ready = status == "ready";
        if item.required && !ready {
            blocking.push(json!({"kind": item.kind, "id": item.id, "status": status}));
        }
        report.push(json!({
            "kind": item.kind,
            "id": item.id,
            "version": item.version,
            "revision": item.revision,
            "required": item.required,
            "status": status,
            // Plugin permissions are shown by the host's consent step; the backend does not store them.
            "permissions": if item.kind == "plugin" { "reviewRequired" } else { "none" },
            "activation": if item.kind == "plugin" { "notApplicable" } else { "disabledUntilExplicit" },
        }));
    }
    Ok(Json(json!({
        "packageVersion": PACKAGE_VERSION,
        "items": report,
        "blocking": blocking,
        "canApply": blocking.is_empty(),
        "applyMode": "clientJournal",
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plugin(id: &str, version: &str) -> Item {
        Item {
            kind: "plugin".into(),
            id: id.into(),
            version: Some(version.into()),
            revision: None,
            required: true,
        }
    }

    #[test]
    fn plugins_pin_versions_and_documents_pin_revisions() {
        assert_eq!(check("Pack", &[plugin("acme.alert", "1.0.0")]), Ok(()));
        assert_eq!(
            check(
                "Pack",
                &[Item {
                    kind: "plugin".into(),
                    id: "acme.alert".into(),
                    version: None,
                    revision: None,
                    required: true
                }]
            ),
            Err("missingVersion")
        );
        assert_eq!(
            check(
                "Pack",
                &[Item {
                    kind: "template".into(),
                    id: Uuid::new_v4().to_string(),
                    version: None,
                    revision: None,
                    required: true
                }]
            ),
            Err("missingRevision")
        );
    }

    #[test]
    fn duplicates_and_unknown_kinds_are_refused() {
        assert_eq!(
            check(
                "Pack",
                &[plugin("acme.alert", "1.0.0"), plugin("acme.alert", "2.0.0")]
            ),
            Err("duplicateItem")
        );
        assert_eq!(
            check(
                "Pack",
                &[Item {
                    kind: "script".into(),
                    id: "x".into(),
                    version: None,
                    revision: Some(1),
                    required: true
                }]
            ),
            Err("invalidItem")
        );
    }

    #[test]
    fn bounds_apply_to_titles_and_item_counts() {
        assert_eq!(
            check("   ", &[plugin("acme.alert", "1.0.0")]),
            Err("invalidTitle")
        );
        assert_eq!(check("Pack", &[]), Err("invalidItems"));
        let many: Vec<Item> = (0..=MAX_ITEMS)
            .map(|i| plugin(&format!("acme.p{i}"), "1.0.0"))
            .collect();
        assert_eq!(check("Pack", &many), Err("invalidItems"));
    }
}
