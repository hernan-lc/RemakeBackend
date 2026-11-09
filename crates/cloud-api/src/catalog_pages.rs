//! Bounded catalog traversal. The existing catalog endpoints remain unchanged.
use crate::{workspace, AppState};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{Postgres, Row, Transaction};

const PAGE_ITEMS: usize = 50;
const PAGE_BYTES: usize = 512 * 1024;
const CURSOR_BYTES: usize = 2048;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SummaryQuery {
    q: Option<String>,
    target: Option<String>,
    generation: Option<i64>,
    cursor: Option<String>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VersionQuery {
    target: Option<String>,
    generation: Option<i64>,
    cursor: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
enum Cursor {
    Summary {
        generation: i64,
        q: Option<String>,
        target: Option<String>,
        last: String,
    },
    Versions {
        generation: i64,
        id: String,
        target: Option<String>,
        version: String,
        last_target: String,
    },
}
fn encode(cursor: &Cursor) -> Result<String, StatusCode> {
    let bytes = serde_json::to_vec(cursor).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let value = URL_SAFE_NO_PAD.encode(bytes);
    if value.len() > CURSOR_BYTES {
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }
    Ok(value)
}
fn decode(value: &str) -> Result<Cursor, StatusCode> {
    if value.len() > CURSOR_BYTES {
        return Err(StatusCode::BAD_REQUEST);
    }
    let bytes = URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    serde_json::from_slice(&bytes).map_err(|_| StatusCode::BAD_REQUEST)
}
fn valid_target(target: &str) -> bool {
    !target.is_empty()
        && target.len() <= 128
        && target
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
}
fn validate(generation: Option<i64>, target: Option<&str>) -> Result<(), StatusCode> {
    if generation.is_some_and(|n| n < 1) || target.is_some_and(|v| !valid_target(v)) {
        return Err(StatusCode::BAD_REQUEST);
    }
    Ok(())
}
async fn generation(
    tx: &mut Transaction<'_, Postgres>,
    requested: Option<i64>,
) -> Result<Option<i64>, StatusCode> {
    let result = if let Some(id) = requested {
        sqlx::query_scalar("SELECT generation FROM cloud_registry_generations WHERE generation=$1 AND status IN ('active','superseded','rolled_back')")
            .bind(id).fetch_optional(&mut **tx).await.map_err(workspace::db)?
    } else {
        sqlx::query_scalar(
            "SELECT generation FROM cloud_registry_generations WHERE status='active'",
        )
        .fetch_optional(&mut **tx)
        .await
        .map_err(workspace::db)?
    };
    if result.is_none() && requested.is_some() {
        return Err(StatusCode::NOT_FOUND);
    }
    Ok(result)
}
async fn transaction(state: &AppState) -> Result<Transaction<'_, Postgres>, StatusCode> {
    let mut tx = state.pool.begin().await.map_err(workspace::db)?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
        .execute(&mut *tx)
        .await
        .map_err(workspace::db)?;
    Ok(tx)
}
fn metadata_path(generation: i64, id: &str) -> Result<String, StatusCode> {
    if !tiktools_plugin_api::manifest::is_valid_plugin_id(id) {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    let path = tiktools_plugin_registry::shard_path(id);
    if path
        .split('/')
        .any(|part| part == ".." || part == "." || part.is_empty())
    {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    Ok(format!(
        "/api/v1/registry/generations/{generation}/files/{path}"
    ))
}
fn response_size(value: &Value) -> Result<usize, StatusCode> {
    serde_json::to_vec(value)
        .map(|v| v.len())
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}
pub async fn capabilities() -> Json<Value> {
    Json(
        json!({"schemaVersion":1,"summaryPath":"/api/v2/catalog/plugin-summaries","versionPagePath":"/api/v2/catalog/plugins/{id}/version-pages","maxItems":PAGE_ITEMS,"maxBytes":PAGE_BYTES,"versionOrder":"ascii-version-target"}),
    )
}

pub async fn summaries(
    State(state): State<AppState>,
    Query(query): Query<SummaryQuery>,
) -> Result<Json<Value>, StatusCode> {
    validate(query.generation, query.target.as_deref())?;
    if query.q.as_ref().is_some_and(|s| s.len() > 100) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let last = if let Some(raw) = &query.cursor {
        let Cursor::Summary {
            generation,
            q,
            target,
            last,
        } = decode(raw)?
        else {
            return Err(StatusCode::BAD_REQUEST);
        };
        if Some(generation) != query.generation
            || q != query.q
            || target != query.target
            || !tiktools_plugin_api::manifest::is_valid_plugin_id(&last)
        {
            return Err(StatusCode::BAD_REQUEST);
        }
        Some(last)
    } else {
        None
    };
    let mut tx = transaction(&state).await?;
    let Some(generation) = generation(&mut tx, query.generation).await? else {
        return Ok(Json(
            json!({"generation":null,"items":[],"nextCursor":null}),
        ));
    };
    let ids: Vec<String> = sqlx::query_scalar(r#"SELECT DISTINCT plugin_id COLLATE "C" AS plugin_id FROM cloud_registry_generation_releases WHERE generation=$1 AND ($2::text IS NULL OR plugin_id COLLATE "C">$2) AND ($3::text IS NULL OR strpos(lower(plugin_id),lower($3))>0) AND ($4::text IS NULL OR target=$4 OR target='any') ORDER BY plugin_id COLLATE "C" LIMIT 51"#)
        .bind(generation).bind(last).bind(&query.q).bind(&query.target).fetch_all(&mut *tx).await.map_err(workspace::db)?;
    let more = ids.len() > PAGE_ITEMS;
    let ids: Vec<_> = ids.into_iter().take(PAGE_ITEMS).collect();
    // Bound work even if a corrupt database violates the published inventory limit.
    let rows = sqlx::query("SELECT g.plugin_id,g.version,(y.release_id IS NOT NULL) AS yanked,k.status AS key_status,p.status AS publisher_status FROM cloud_registry_generation_releases g JOIN cloud_plugin_releases r ON r.id=g.release_id JOIN cloud_publishers p ON p.id=r.publisher_id JOIN cloud_publisher_keys k ON k.id=r.publisher_key_id LEFT JOIN cloud_release_yanks y ON y.release_id=r.id WHERE g.generation=$1 AND g.plugin_id=ANY($2) AND ($3::text IS NULL OR g.target=$3 OR g.target='any') LIMIT 10001")
        .bind(generation).bind(&ids).bind(&query.target).fetch_all(&mut *tx).await.map_err(workspace::db)?;
    if rows.len() > 10000 {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    let mut items = Vec::with_capacity(ids.len());
    for id in &ids {
        let mut latest: Option<semver::Version> = None;
        for row in rows
            .iter()
            .filter(|r| r.get::<String, _>("plugin_id") == *id)
        {
            if row.get::<bool, _>("yanked")
                || row.get::<String, _>("key_status") != "approved"
                || row.get::<String, _>("publisher_status") != "approved"
            {
                continue;
            }
            let version = semver::Version::parse(&row.get::<String, _>("version"))
                .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
            if version.pre.is_empty() && latest.as_ref().is_none_or(|old| version > *old) {
                latest = Some(version);
            }
        }
        items.push(json!({"id":id,"metadataPath":metadata_path(generation,id)?,"latestStableVersion":latest.map(|v|v.to_string())}));
    }
    let next = if more {
        Some(encode(&Cursor::Summary {
            generation,
            q: query.q,
            target: query.target,
            last: ids.last().expect("nonempty page").clone(),
        })?)
    } else {
        None
    };
    let page = json!({"generation":generation.to_string(),"items":items,"nextCursor":next});
    if response_size(&page)? > PAGE_BYTES {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }
    tx.commit().await.map_err(workspace::db)?;
    Ok(Json(page))
}

pub async fn versions(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<VersionQuery>,
) -> Result<Json<Value>, StatusCode> {
    validate(query.generation, query.target.as_deref())?;
    if !tiktools_plugin_api::manifest::is_valid_plugin_id(&id) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let (last_version, last_target) = if let Some(raw) = &query.cursor {
        let Cursor::Versions {
            generation,
            id: old_id,
            target,
            version,
            last_target,
        } = decode(raw)?
        else {
            return Err(StatusCode::BAD_REQUEST);
        };
        if Some(generation) != query.generation
            || old_id != id
            || target != query.target
            || version.len() > 128
            || semver::Version::parse(&version).is_err()
            || !valid_target(&last_target)
        {
            return Err(StatusCode::BAD_REQUEST);
        }
        (Some(version), Some(last_target))
    } else {
        (None, None)
    };
    let mut tx = transaction(&state).await?;
    let generation = generation(&mut tx, query.generation)
        .await?
        .ok_or(StatusCode::NOT_FOUND)?;
    let path = metadata_path(generation, &id)?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM cloud_registry_generation_releases WHERE generation=$1 AND plugin_id=$2 AND ($3::text IS NULL OR target=$3 OR target='any'))")
        .bind(generation).bind(&id).bind(&query.target).fetch_one(&mut *tx).await.map_err(workspace::db)?;
    if !exists {
        return Err(StatusCode::NOT_FOUND);
    }
    // Never pull arbitrarily large stored advisory strings into memory.
    let rows = sqlx::query(r#"SELECT g.version,g.target,g.sha256,g.size,(y.release_id IS NOT NULL) AS yanked,k.status AS key_status,p.status AS publisher_status,a.severity,CASE WHEN coalesce(octet_length(a.message),0)+coalesce(octet_length(a.url),0)<=524288 THEN a.message END AS message,CASE WHEN coalesce(octet_length(a.message),0)+coalesce(octet_length(a.url),0)<=524288 THEN a.url END AS url,coalesce(octet_length(a.message),0) AS message_bytes,coalesce(octet_length(a.url),0) AS url_bytes FROM cloud_registry_generation_releases g JOIN cloud_plugin_releases r ON r.id=g.release_id JOIN cloud_publishers p ON p.id=r.publisher_id JOIN cloud_publisher_keys k ON k.id=r.publisher_key_id LEFT JOIN cloud_release_yanks y ON y.release_id=r.id LEFT JOIN cloud_release_advisories a ON a.release_id=r.id WHERE g.generation=$1 AND g.plugin_id=$2 AND ($3::text IS NULL OR g.target=$3 OR g.target='any') AND ($4::text IS NULL OR (g.version COLLATE "C",g.target COLLATE "C")>($4::text COLLATE "C",$5::text COLLATE "C")) ORDER BY g.version COLLATE "C",g.target COLLATE "C" LIMIT 51"#)
        .bind(generation).bind(&id).bind(&query.target).bind(last_version).bind(last_target).fetch_all(&mut *tx).await.map_err(workspace::db)?;
    let mut items = Vec::new();
    let mut last = None;
    let mut more = false;
    for row in &rows {
        if items.len() == PAGE_ITEMS {
            more = true;
            break;
        }
        if i64::from(row.get::<i32, _>("message_bytes")) + i64::from(row.get::<i32, _>("url_bytes"))
            > PAGE_BYTES as i64
        {
            if items.is_empty() {
                return Err(StatusCode::PAYLOAD_TOO_LARGE);
            }
            more = true;
            break;
        }
        let version: String = row.get("version");
        let target: String = row.get("target");
        let severity: Option<String> = row.get("severity");
        let item = json!({"version":version,"target":target,"sha256":row.get::<String,_>("sha256"),"size":row.get::<i64,_>("size").to_string(),"yanked":row.get::<bool,_>("yanked"),"keyRevoked":row.get::<String,_>("key_status") != "approved","publisherApproved":row.get::<String,_>("publisher_status") == "approved","advisory":severity.map(|severity|json!({"severity":severity,"message":row.get::<Option<String>,_>("message"),"url":row.get::<Option<String>,_>("url")}))});
        let candidate_cursor = encode(&Cursor::Versions {
            generation,
            id: id.clone(),
            target: query.target.clone(),
            version: version.clone(),
            last_target: target.clone(),
        })?;
        items.push(item);
        let tentative = json!({"generation":generation.to_string(),"id":id,"metadataPath":path,"items":items,"nextCursor":candidate_cursor});
        if response_size(&tentative)? > PAGE_BYTES {
            items.pop();
            if items.is_empty() {
                return Err(StatusCode::PAYLOAD_TOO_LARGE);
            }
            more = true;
            break;
        }
        last = Some((version, target));
    }
    let next = if more {
        let (version, last_target) = last.ok_or(StatusCode::PAYLOAD_TOO_LARGE)?;
        Some(encode(&Cursor::Versions {
            generation,
            id: id.clone(),
            target: query.target,
            version,
            last_target,
        })?)
    } else {
        None
    };
    let page = json!({"generation":generation.to_string(),"id":id,"metadataPath":path,"items":items,"nextCursor":next});
    tx.commit().await.map_err(workspace::db)?;
    Ok(Json(page))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_roundtrips_with_its_filter_and_rejects_unbounded_or_unknown_fields() {
        let cursor = Cursor::Summary {
            generation: 9007199254740993,
            q: Some("a.b".into()),
            target: Some("any".into()),
            last: "hotkeys".into(),
        };
        let value = encode(&cursor).unwrap();
        let decoded = decode(&value).unwrap();
        match decoded {
            Cursor::Summary {
                generation,
                q,
                target,
                last,
            } => {
                assert_eq!(generation, 9007199254740993);
                assert_eq!(q.as_deref(), Some("a.b"));
                assert_eq!(target.as_deref(), Some("any"));
                assert_eq!(last, "hotkeys");
            }
            _ => panic!("unexpected cursor kind"),
        }
        assert!(decode(&"x".repeat(CURSOR_BYTES + 1)).is_err());
        let unknown = URL_SAFE_NO_PAD.encode(br#"{"kind":"summary","generation":1,"q":null,"target":null,"last":"hotkeys","extra":true}"#);
        assert!(decode(&unknown).is_err());
    }

    #[test]
    fn metadata_paths_fail_closed_for_dot_shard_and_preserve_safe_paths() {
        assert_eq!(
            metadata_path(5, "hotkeys").unwrap(),
            "/api/v1/registry/generations/5/files/index/ho/tk/hotkeys.json"
        );
        assert_eq!(
            metadata_path(5, "ab..foo"),
            Err(StatusCode::SERVICE_UNAVAILABLE)
        );
        assert_eq!(
            metadata_path(5, "1bad"),
            Err(StatusCode::SERVICE_UNAVAILABLE)
        );
    }
}
