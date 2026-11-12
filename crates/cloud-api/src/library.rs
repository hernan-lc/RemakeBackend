//! Private drafts and independently reviewed, immutable public templates/profiles.
use crate::{
    dual_auth::DualUser,
    portable,
    workspace::{self, db},
    AppState,
};
use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use better_auth::axum_integration::AuthUser;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;

const MAX_DOCUMENTS: i64 = 100;
const MAX_VERSIONS: i64 = 100;
const MAX_BYTES: i64 = 8 * 1024 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Create {
    kind: String,
    document: Value,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Update {
    document: Value,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Page {
    after: Option<Uuid>,
    kind: Option<String>,
    q: Option<String>,
}
fn check_page(page: &Page) -> Result<(), StatusCode> {
    if page
        .kind
        .as_ref()
        .is_some_and(|k| !matches!(k.as_str(), "template" | "profile"))
        || page.q.as_ref().is_some_and(|q| q.len() > 100)
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    Ok(())
}
fn title(kind: &str, value: &Value) -> String {
    let value = if kind == "profile" {
        &value["name"]
    } else {
        &value["title"]
    };
    value
        .as_str()
        .or_else(|| value["default"].as_str())
        .unwrap_or_default()
        .into()
}
pub(crate) fn digest(doc: &Value) -> String {
    hex::encode(Sha256::digest(serde_json::to_vec(doc).expect("JSON value")))
}
fn check_identity(row: &sqlx::postgres::PgRow) -> Result<Value, StatusCode> {
    let document: Value = row.get("document");
    if digest(&document) != row.get::<String, _>("sha256")
        || !portable::document(&row.get::<String, _>("kind"), &document)
    {
        // Legacy hashes made before JSONB normalization require an explicit,
        // audited repair. Never advertise a false identity or rewrite on GET.
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    Ok(document)
}
fn view(row: &sqlx::postgres::PgRow, content: bool) -> Result<Value, StatusCode> {
    let document = check_identity(row)?;
    let mut result = json!({"id":row.get::<Uuid,_>("id"),"kind":row.get::<String,_>("kind"),"slug":row.get::<String,_>("slug"),"revision":row.get::<i64,_>("revision"),"title":row.get::<String,_>("title"),"sha256":row.get::<String,_>("sha256"),"status":row.get::<String,_>("status")});
    // Provenance is row metadata, selected only by the owner's own queries.
    if let (Ok(Some(source)), Ok(Some(revision))) = (
        row.try_get::<Option<Uuid>, _>("fork_of_document"),
        row.try_get::<Option<i64>, _>("fork_of_revision"),
    ) {
        result["forkOf"] = json!({"documentId":source,"revision":revision});
    }
    if content {
        result["document"] = document;
    }
    Ok(result)
}
async fn insert_version(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    revision: i64,
    kind: &str,
    document: &Value,
) -> Result<(), StatusCode> {
    // The input's number representation may change in JSONB (1e18 becomes an
    // integer; -0.0 becomes 0.0). Hash the actual returned, typed stored value.
    // The temporary digest is unobservable outside this transaction.
    let stored: Value = sqlx::query_scalar("INSERT INTO cloud_library_versions(document_id,revision,title,document,sha256) VALUES($1,$2,$3,$4,$5) RETURNING document")
        .bind(id).bind(revision).bind(title(kind,document)).bind(document)
        .bind("0".repeat(64)).fetch_one(&mut **tx).await.map_err(db)?;
    if !portable::document(kind, &stored) {
        return Err(StatusCode::BAD_REQUEST);
    }
    sqlx::query("UPDATE cloud_library_versions SET sha256=$3 WHERE document_id=$1 AND revision=$2")
        .bind(id)
        .bind(revision)
        .bind(digest(&stored))
        .execute(&mut **tx)
        .await
        .map_err(db)?;
    Ok(())
}
async fn owner_lock(tx: &mut Transaction<'_, Postgres>, user: &str) -> Result<(), StatusCode> {
    // Serialize owner quotas and document changes. The FK prevents orphan writes.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,840170008))")
        .bind(user)
        .execute(&mut **tx)
        .await
        .map_err(db)?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM \"user\" WHERE id=$1)")
        .bind(user)
        .fetch_one(&mut **tx)
        .await
        .map_err(db)?;
    if !exists {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(())
}
async fn quota(
    tx: &mut Transaction<'_, Postgres>,
    user: &str,
    document: &Value,
) -> Result<(), StatusCode> {
    let bytes: i64 = sqlx::query_scalar("SELECT COALESCE(sum(octet_length(v.document::text)),0)::bigint FROM cloud_library_versions v JOIN cloud_library_documents d ON d.id=v.document_id WHERE d.owner_user_id=$1 AND d.kind IN ('template','profile')").bind(user).fetch_one(&mut **tx).await.map_err(db)?;
    let added: i64 = sqlx::query_scalar("SELECT octet_length($1::jsonb::text)::bigint")
        .bind(document)
        .fetch_one(&mut **tx)
        .await
        .map_err(db)?;
    if bytes + added > MAX_BYTES {
        return Err(StatusCode::INSUFFICIENT_STORAGE);
    }
    Ok(())
}
async fn audit(
    tx: &mut Transaction<'_, Postgres>,
    actor: &str,
    id: Uuid,
    action: &str,
) -> Result<(), StatusCode> {
    sqlx::query(
        "INSERT INTO cloud_audit_events(id,actor_user_id,subject_id,action) VALUES($1,$2,$3,$4)",
    )
    .bind(Uuid::new_v4())
    .bind(actor)
    .bind(id)
    .bind(action)
    .execute(&mut **tx)
    .await
    .map_err(db)?;
    Ok(())
}
pub async fn create(
    DualUser(user): DualUser,
    State(state): State<AppState>,
    Json(input): Json<Create>,
) -> Result<(StatusCode, HeaderMap, Json<Value>), StatusCode> {
    if !portable::document(&input.kind, &input.document) {
        return Err(StatusCode::BAD_REQUEST);
    }
    workspace::limit(&state, &user.user.id, "library-write", 60).await?;
    let mut tx = state.pool.begin().await.map_err(db)?;
    owner_lock(&mut tx, &user.user.id).await?;
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM cloud_library_documents WHERE owner_user_id=$1 AND kind IN ('template','profile')")
            .bind(&user.user.id)
            .fetch_one(&mut *tx)
            .await
            .map_err(db)?;
    if count >= MAX_DOCUMENTS {
        return Err(StatusCode::INSUFFICIENT_STORAGE);
    }
    quota(&mut tx, &user.user.id, &input.document).await?;
    let id = Uuid::new_v4();
    let created = sqlx::query("INSERT INTO cloud_library_documents(id,owner_user_id,kind,slug) VALUES($1,$2,$3,$4) ON CONFLICT(owner_user_id,kind,slug) DO NOTHING").bind(id).bind(&user.user.id).bind(&input.kind).bind(input.document["id"].as_str().unwrap()).execute(&mut *tx).await.map_err(db)?;
    if created.rows_affected() == 0 {
        return Err(StatusCode::CONFLICT);
    }
    insert_version(&mut tx, id, 1, &input.kind, &input.document).await?;
    audit(&mut tx, &user.user.id, id, "library.created").await?;
    tx.commit().await.map_err(db)?;
    Ok((
        StatusCode::CREATED,
        workspace::etag(1),
        Json(json!({"id":id,"revision":1,"status":"draft"})),
    ))
}
pub async fn list(
    DualUser(user): DualUser,
    State(state): State<AppState>,
    Query(page): Query<Page>,
) -> Result<Json<Value>, StatusCode> {
    check_page(&page)?;
    workspace::limit(&state, &user.user.id, "read", 300).await?;
    let rows = sqlx::query("SELECT d.id,d.kind,d.slug,v.revision,v.title,v.sha256,v.status,v.document,d.fork_of_document,d.fork_of_revision FROM cloud_library_documents d JOIN cloud_library_versions v ON v.document_id=d.id AND v.revision=d.revision WHERE d.owner_user_id=$1 AND d.kind IN ('template','profile') AND ($2::uuid IS NULL OR d.id>$2) AND ($3::text IS NULL OR d.kind=$3) AND ($4::text IS NULL OR strpos(lower(v.title),lower($4))>0) ORDER BY d.id LIMIT 51").bind(&user.user.id).bind(page.after).bind(page.kind).bind(page.q).fetch_all(&state.pool).await.map_err(db)?;
    Ok(Json(listing(rows)?))
}
fn listing(rows: Vec<sqlx::postgres::PgRow>) -> Result<Value, StatusCode> {
    let more = rows.len() > 50;
    let items: Vec<_> = rows
        .iter()
        .take(50)
        .map(|r| view(r, false))
        .collect::<Result<_, _>>()?;
    let next = if more {
        items.last().map(|v| v["id"].clone())
    } else {
        None
    };
    Ok(json!({"items":items,"nextCursor":next}))
}
pub async fn get(
    DualUser(user): DualUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<(HeaderMap, Json<Value>), StatusCode> {
    workspace::limit(&state, &user.user.id, "read", 300).await?;
    let row = sqlx::query("SELECT d.id,d.kind,d.slug,v.revision,v.title,v.sha256,v.status,v.document,d.fork_of_document,d.fork_of_revision FROM cloud_library_documents d JOIN cloud_library_versions v ON v.document_id=d.id AND v.revision=d.revision WHERE d.id=$1 AND d.owner_user_id=$2 AND d.kind IN ('template','profile')").bind(id).bind(&user.user.id).fetch_optional(&state.pool).await.map_err(db)?.ok_or(StatusCode::NOT_FOUND)?;
    Ok((
        workspace::etag(row.get("revision")),
        Json(view(&row, true)?),
    ))
}
pub async fn update(
    DualUser(user): DualUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(input): Json<Update>,
) -> Result<(HeaderMap, Json<Value>), StatusCode> {
    let expected = workspace::precondition(&headers)?.ok_or(StatusCode::BAD_REQUEST)?;
    workspace::limit(&state, &user.user.id, "library-write", 60).await?;
    let mut tx = state.pool.begin().await.map_err(db)?;
    owner_lock(&mut tx, &user.user.id).await?;
    let row = sqlx::query("SELECT kind,slug,revision FROM cloud_library_documents WHERE id=$1 AND owner_user_id=$2 AND kind IN ('template','profile') FOR UPDATE").bind(id).bind(&user.user.id).fetch_optional(&mut *tx).await.map_err(db)?.ok_or(StatusCode::NOT_FOUND)?;
    let kind: String = row.get("kind");
    let revision: i64 = row.get("revision");
    if revision != expected {
        return Err(StatusCode::CONFLICT);
    }
    if !portable::document(&kind, &input.document)
        || input.document["id"] != row.get::<String, _>("slug")
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    if revision >= MAX_VERSIONS {
        return Err(StatusCode::INSUFFICIENT_STORAGE);
    }
    quota(&mut tx, &user.user.id, &input.document).await?;
    let revision = revision + 1;
    insert_version(&mut tx, id, revision, &kind, &input.document).await?;
    sqlx::query("UPDATE cloud_library_documents SET revision=$2 WHERE id=$1")
        .bind(id)
        .bind(revision)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    audit(&mut tx, &user.user.id, id, "library.updated").await?;
    tx.commit().await.map_err(db)?;
    Ok((
        workspace::etag(revision),
        Json(json!({"id":id,"revision":revision,"status":"draft"})),
    ))
}
pub async fn delete(
    DualUser(user): DualUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<StatusCode, StatusCode> {
    let expected = workspace::precondition(&headers)?.ok_or(StatusCode::BAD_REQUEST)?;
    workspace::limit(&state, &user.user.id, "library-write", 60).await?;
    let mut tx = state.pool.begin().await.map_err(db)?;
    owner_lock(&mut tx, &user.user.id).await?;
    let revision: i64 = sqlx::query_scalar(
        "SELECT revision FROM cloud_library_documents WHERE id=$1 AND owner_user_id=$2 AND kind IN ('template','profile') FOR UPDATE",
    )
    .bind(id)
    .bind(&user.user.id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(db)?
    .ok_or(StatusCode::NOT_FOUND)?;
    if revision != expected {
        return Err(StatusCode::CONFLICT);
    }
    sqlx::query("DELETE FROM cloud_library_documents WHERE id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    audit(&mut tx, &user.user.id, id, "library.deleted").await?;
    tx.commit().await.map_err(db)?;
    Ok(StatusCode::NO_CONTENT)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VersionPage {
    after: Option<i64>,
}
pub async fn versions(
    DualUser(user): DualUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(page): Query<VersionPage>,
) -> Result<Json<Value>, StatusCode> {
    if page.after.is_some_and(|n| n < 0) {
        return Err(StatusCode::BAD_REQUEST);
    }
    workspace::limit(&state, &user.user.id, "read", 300).await?;
    let owned: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM cloud_library_documents WHERE id=$1 AND owner_user_id=$2 AND kind IN ('template','profile'))",
    )
    .bind(id)
    .bind(&user.user.id)
    .fetch_one(&state.pool)
    .await
    .map_err(db)?;
    if !owned {
        return Err(StatusCode::NOT_FOUND);
    }
    let rows = sqlx::query("SELECT d.id,d.kind,d.slug,v.revision,v.title,v.sha256,v.status,v.document,d.fork_of_document,d.fork_of_revision FROM cloud_library_documents d JOIN cloud_library_versions v ON v.document_id=d.id WHERE d.id=$1 AND d.owner_user_id=$2 AND v.revision>$3 ORDER BY v.revision LIMIT 51").bind(id).bind(&user.user.id).bind(page.after.unwrap_or(0)).fetch_all(&state.pool).await.map_err(db)?;
    let more = rows.len() > 50;
    let items: Vec<_> = rows
        .iter()
        .take(50)
        .map(|r| view(r, false))
        .collect::<Result<_, _>>()?;
    let next = if more {
        items.last().map(|v| v["revision"].clone())
    } else {
        None
    };
    Ok(Json(json!({"items":items,"nextCursor":next})))
}
pub async fn version(
    DualUser(user): DualUser,
    State(state): State<AppState>,
    Path((id, revision)): Path<(Uuid, i64)>,
) -> Result<(HeaderMap, Json<Value>), StatusCode> {
    workspace::limit(&state, &user.user.id, "read", 300).await?;
    let row = sqlx::query("SELECT d.id,d.kind,d.slug,v.revision,v.title,v.sha256,v.status,v.document,d.fork_of_document,d.fork_of_revision FROM cloud_library_documents d JOIN cloud_library_versions v ON v.document_id=d.id WHERE d.id=$1 AND d.owner_user_id=$2 AND d.kind IN ('template','profile') AND v.revision=$3").bind(id).bind(&user.user.id).bind(revision).fetch_optional(&state.pool).await.map_err(db)?.ok_or(StatusCode::NOT_FOUND)?;
    Ok((workspace::etag(revision), Json(view(&row, true)?)))
}
pub async fn submit(
    DualUser(user): DualUser,
    State(state): State<AppState>,
    Path((id, revision)): Path<(Uuid, i64)>,
) -> Result<StatusCode, StatusCode> {
    workspace::limit(&state, &user.user.id, "library-write", 60).await?;
    let mut tx = state.pool.begin().await.map_err(db)?;
    let row = sqlx::query("SELECT d.kind,v.status,v.sha256,v.document FROM cloud_library_documents d JOIN cloud_library_versions v ON v.document_id=d.id WHERE d.id=$1 AND d.owner_user_id=$2 AND d.kind IN ('template','profile') AND v.revision=$3 FOR UPDATE OF d,v").bind(id).bind(&user.user.id).bind(revision).fetch_optional(&mut *tx).await.map_err(db)?.ok_or(StatusCode::NOT_FOUND)?;
    check_identity(&row)?;
    match row.get::<String, _>("status").as_str() {
        "pending" => return Ok(StatusCode::NO_CONTENT),
        "draft" => (),
        _ => return Err(StatusCode::CONFLICT),
    }
    sqlx::query(
        "UPDATE cloud_library_versions SET status='pending' WHERE document_id=$1 AND revision=$2",
    )
    .bind(id)
    .bind(revision)
    .execute(&mut *tx)
    .await
    .map_err(db)?;
    audit(&mut tx, &user.user.id, id, "library.submitted").await?;
    tx.commit().await.map_err(db)?;
    Ok(StatusCode::NO_CONTENT)
}
pub async fn catalog(
    State(state): State<AppState>,
    Query(page): Query<Page>,
) -> Result<Json<Value>, StatusCode> {
    check_page(&page)?;
    let rows = sqlx::query("SELECT d.id,d.kind,d.slug,v.revision,v.title,v.sha256,v.status,v.document FROM cloud_library_documents d JOIN cloud_library_versions v ON v.document_id=d.id AND v.revision=d.published_revision WHERE v.status='published' AND ($1::uuid IS NULL OR d.id>$1) AND ($2::text IS NULL OR d.kind=$2) AND ($3::text IS NULL OR strpos(lower(v.title),lower($3))>0) ORDER BY d.id LIMIT 51").bind(page.after).bind(page.kind).bind(page.q).fetch_all(&state.pool).await.map_err(db)?;
    Ok(Json(listing(rows)?))
}
pub async fn published(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<(HeaderMap, Json<Value>), StatusCode> {
    let row = sqlx::query("SELECT d.id,d.kind,d.slug,v.revision,v.title,v.sha256,v.status,v.document FROM cloud_library_documents d JOIN cloud_library_versions v ON v.document_id=d.id AND v.revision=d.published_revision WHERE d.id=$1 AND v.status='published'").bind(id).fetch_optional(&state.pool).await.map_err(db)?.ok_or(StatusCode::NOT_FOUND)?;
    Ok((
        workspace::etag(row.get("revision")),
        Json(view(&row, true)?),
    ))
}
pub async fn published_version(
    State(state): State<AppState>,
    Path((id, revision)): Path<(Uuid, i64)>,
) -> Result<(HeaderMap, Json<Value>), StatusCode> {
    let row = sqlx::query("SELECT d.id,d.kind,d.slug,v.revision,v.title,v.sha256,v.status,v.document FROM cloud_library_documents d JOIN cloud_library_versions v ON v.document_id=d.id WHERE d.id=$1 AND v.revision=$2 AND v.status='published'").bind(id).bind(revision).fetch_optional(&state.pool).await.map_err(db)?.ok_or(StatusCode::NOT_FOUND)?;
    Ok((workspace::etag(revision), Json(view(&row, true)?)))
}
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct ForkRequest {
    /// A specific publicly visible revision; defaults to the current published one.
    revision: Option<i64>,
}
/// Copies a template or profile revision that is visible in the public catalog
/// into the caller's own library as a new private draft (revision 1).
///
/// Provenance (`forkOf`) lives on the library document row, never inside the
/// portable value, so the value validators and older clients are unaffected.
/// The copy keeps the source's portable `id` (its slug in the caller's
/// library); a caller who already has that kind and id gets 409. The write
/// limit (60/min) and the 100-document / 8 MiB library quotas apply.
pub async fn fork(
    DualUser(user): DualUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    body: Bytes,
) -> Result<(StatusCode, HeaderMap, Json<Value>), StatusCode> {
    let input: ForkRequest = if body.is_empty() {
        ForkRequest::default()
    } else {
        // `null` and `{}` mean the same as no body.
        serde_json::from_slice::<Option<ForkRequest>>(&body)
            .map_err(|_| StatusCode::BAD_REQUEST)?
            .unwrap_or_default()
    };
    if input.revision.is_some_and(|n| n < 1) {
        return Err(StatusCode::BAD_REQUEST);
    }
    workspace::limit(&state, &user.user.id, "library-write", 60).await?;
    let mut tx = state.pool.begin().await.map_err(db)?;
    owner_lock(&mut tx, &user.user.id).await?;
    // Only a published revision is visible in the public catalog (drafts,
    // pending, rejected and withdrawn revisions never are).
    let source = sqlx::query("SELECT d.kind,d.slug,v.revision,v.sha256,v.document FROM cloud_library_documents d JOIN cloud_library_versions v ON v.document_id=d.id AND v.revision=COALESCE($2::bigint,d.published_revision) WHERE d.id=$1 AND d.kind IN ('template','profile') AND v.status='published'")
        .bind(id)
        .bind(input.revision)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let document = check_identity(&source)?;
    let kind: String = source.get("kind");
    let slug: String = source.get("slug");
    let source_revision: i64 = source.get("revision");
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM cloud_library_documents WHERE owner_user_id=$1 AND kind IN ('template','profile')")
        .bind(&user.user.id)
        .fetch_one(&mut *tx)
        .await
        .map_err(db)?;
    if count >= MAX_DOCUMENTS {
        return Err(StatusCode::INSUFFICIENT_STORAGE);
    }
    quota(&mut tx, &user.user.id, &document).await?;
    let fork = Uuid::new_v4();
    let created = sqlx::query("INSERT INTO cloud_library_documents(id,owner_user_id,kind,slug,fork_of_document,fork_of_revision) VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT(owner_user_id,kind,slug) DO NOTHING")
        .bind(fork)
        .bind(&user.user.id)
        .bind(&kind)
        .bind(&slug)
        .bind(id)
        .bind(source_revision)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    if created.rows_affected() == 0 {
        return Err(StatusCode::CONFLICT);
    }
    insert_version(&mut tx, fork, 1, &kind, &document).await?;
    audit(&mut tx, &user.user.id, fork, "library.forked").await?;
    tx.commit().await.map_err(db)?;
    Ok((
        StatusCode::CREATED,
        workspace::etag(1),
        Json(json!({
            "id":fork,"revision":1,"status":"draft",
            "forkOf":{"documentId":id,"revision":source_revision}
        })),
    ))
}
pub async fn queue(
    AuthUser(user): AuthUser,
    State(state): State<AppState>,
    Query(page): Query<Page>,
) -> Result<Json<Value>, StatusCode> {
    check_page(&page)?;
    workspace::limit(&state, &user.user.id, "read", 300).await?;
    let mut tx = state.pool.begin().await.map_err(db)?;
    crate::moderation::reviewer(&mut tx, &user.user.id).await?;
    // At most one pending version per document is displayed per request. The
    // next version becomes visible after that decision, avoiding cursor skips.
    let rows = sqlx::query("SELECT DISTINCT ON (d.id) d.id,d.kind,d.slug,v.revision,v.title,v.sha256,v.status,v.document FROM cloud_library_documents d JOIN cloud_library_versions v ON v.document_id=d.id WHERE v.status='pending' AND d.owner_user_id<>$1 AND ($2::uuid IS NULL OR d.id>$2) AND ($3::text IS NULL OR d.kind=$3) AND ($4::text IS NULL OR strpos(lower(v.title),lower($4))>0) ORDER BY d.id,v.revision LIMIT 51").bind(&user.user.id).bind(page.after).bind(page.kind).bind(page.q).fetch_all(&mut *tx).await.map_err(db)?;
    let more = rows.len() > 50;
    let items: Vec<_> = rows
        .iter()
        .take(50)
        .map(|r| view(r, true))
        .collect::<Result<_, _>>()?;
    let next = if more {
        items.last().map(|v| v["id"].clone())
    } else {
        None
    };
    tx.commit().await.map_err(db)?;
    Ok(Json(json!({"items":items,"nextCursor":next})))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Decision {
    decision: String,
    reason: String,
}
pub async fn decide(
    AuthUser(user): AuthUser,
    State(state): State<AppState>,
    Path((id, revision)): Path<(Uuid, i64)>,
    Json(input): Json<Decision>,
) -> Result<StatusCode, StatusCode> {
    if !matches!(input.decision.as_str(), "approve" | "reject")
        || input.reason.trim().is_empty()
        || input.reason.len() > 500
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    workspace::limit(&state, &user.user.id, "library-review", 60).await?;
    let mut tx = state.pool.begin().await.map_err(db)?;
    crate::moderation::reviewer(&mut tx, &user.user.id).await?;
    let row=sqlx::query("SELECT d.owner_user_id,v.status,v.document,v.sha256,d.kind,d.published_revision FROM cloud_library_documents d JOIN cloud_library_versions v ON v.document_id=d.id WHERE d.id=$1 AND v.revision=$2 FOR UPDATE OF d,v").bind(id).bind(revision).fetch_optional(&mut *tx).await.map_err(db)?.ok_or(StatusCode::NOT_FOUND)?;
    if row.get::<String, _>("owner_user_id") == user.user.id {
        return Err(StatusCode::FORBIDDEN);
    }
    if row.get::<String, _>("status") != "pending" {
        return Err(StatusCode::CONFLICT);
    }
    check_identity(&row)?;
    let status = if input.decision == "approve" {
        "published"
    } else {
        "rejected"
    };
    sqlx::query("UPDATE cloud_library_versions SET status=$3,reviewed_by=$4,reason=$5 WHERE document_id=$1 AND revision=$2").bind(id).bind(revision).bind(status).bind(&user.user.id).bind(input.reason).execute(&mut *tx).await.map_err(db)?;
    // Reviewing an older version never silently rolls the public pointer back.
    if status == "published"
        && row
            .get::<Option<i64>, _>("published_revision")
            .is_none_or(|n| revision > n)
    {
        sqlx::query("UPDATE cloud_library_documents SET published_revision=$2 WHERE id=$1")
            .bind(id)
            .bind(revision)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
    }
    audit(
        &mut tx,
        &user.user.id,
        id,
        if status == "published" {
            "library.approved"
        } else {
            "library.rejected"
        },
    )
    .await?;
    tx.commit().await.map_err(db)?;
    Ok(StatusCode::NO_CONTENT)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Withdrawal {
    reason: String,
}
pub async fn withdraw(
    DualUser(user): DualUser,
    State(state): State<AppState>,
    Path((id, revision)): Path<(Uuid, i64)>,
    Json(input): Json<Withdrawal>,
) -> Result<StatusCode, StatusCode> {
    if input.reason.trim().is_empty() || input.reason.len() > 500 {
        return Err(StatusCode::BAD_REQUEST);
    }
    workspace::limit(&state, &user.user.id, "library-write", 60).await?;
    let mut tx = state.pool.begin().await.map_err(db)?;
    // Only the author may use this dual-auth route. Moderator withdrawal is
    // separately exposed through a cookie-only route below.
    withdraw_inner(&mut tx, &user.user.id, id, revision, input.reason, false).await?;
    tx.commit().await.map_err(db)?;
    Ok(StatusCode::NO_CONTENT)
}
pub async fn moderate_withdraw(
    AuthUser(user): AuthUser,
    State(state): State<AppState>,
    Path((id, revision)): Path<(Uuid, i64)>,
    Json(input): Json<Withdrawal>,
) -> Result<StatusCode, StatusCode> {
    if input.reason.trim().is_empty() || input.reason.len() > 500 {
        return Err(StatusCode::BAD_REQUEST);
    }
    workspace::limit(&state, &user.user.id, "library-review", 60).await?;
    let mut tx = state.pool.begin().await.map_err(db)?;
    crate::moderation::reviewer(&mut tx, &user.user.id).await?;
    withdraw_inner(&mut tx, &user.user.id, id, revision, input.reason, true).await?;
    tx.commit().await.map_err(db)?;
    Ok(StatusCode::NO_CONTENT)
}
async fn withdraw_inner(
    tx: &mut Transaction<'_, Postgres>,
    user: &str,
    id: Uuid,
    revision: i64,
    reason: String,
    moderator: bool,
) -> Result<(), StatusCode> {
    let row=sqlx::query("SELECT d.owner_user_id,v.status FROM cloud_library_documents d JOIN cloud_library_versions v ON v.document_id=d.id WHERE d.id=$1 AND d.kind IN ('template','profile') AND v.revision=$2 FOR UPDATE OF d,v").bind(id).bind(revision).fetch_optional(&mut **tx).await.map_err(db)?.ok_or(StatusCode::NOT_FOUND)?;
    if !moderator && row.get::<String, _>("owner_user_id") != user {
        return Err(StatusCode::NOT_FOUND);
    }
    if row.get::<String, _>("status") == "withdrawn" {
        return Ok(());
    }
    sqlx::query("UPDATE cloud_library_versions SET status='withdrawn',reason=$3 WHERE document_id=$1 AND revision=$2").bind(id).bind(revision).bind(reason).execute(&mut **tx).await.map_err(db)?;
    sqlx::query("UPDATE cloud_library_documents SET published_revision=NULL WHERE id=$1 AND published_revision=$2").bind(id).bind(revision).execute(&mut **tx).await.map_err(db)?;
    audit(tx, user, id, "library.withdrawn").await
}
pub(crate) async fn export(pool: &sqlx::PgPool, user: &str) -> Result<Value, StatusCode> {
    let mut tx = pool.begin().await.map_err(db)?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    let rows=sqlx::query("SELECT d.id,d.kind,d.slug,v.revision,v.title,v.sha256,v.status,v.document,d.fork_of_document,d.fork_of_revision FROM cloud_library_documents d JOIN cloud_library_versions v ON v.document_id=d.id WHERE d.owner_user_id=$1 AND d.kind IN ('template','profile') ORDER BY d.id,v.revision LIMIT 10001").bind(user).fetch_all(&mut *tx).await.map_err(db)?;
    if rows.len() > 10000 {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }
    let items: Vec<_> = rows
        .iter()
        .map(|r| view(r, true))
        .collect::<Result<_, _>>()?;
    tx.commit().await.map_err(db)?;
    Ok(json!({"items":items}))
}
