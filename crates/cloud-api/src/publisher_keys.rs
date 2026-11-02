//! Publisher keys are scoped to one approved publisher and require independent approval.
use crate::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use better_auth::axum_integration::AuthUser;
use serde::{Deserialize, Serialize};
use sqlx::{Postgres, Row, Transaction};
use tiktools_plugin_package::{KeyId, PublicKey};
use uuid::Uuid;

type Tx<'a> = Transaction<'a, Postgres>;
fn db(_: sqlx::Error) -> StatusCode {
    tracing::error!("publisher key operation failed");
    StatusCode::INTERNAL_SERVER_ERROR
}

async fn publisher(
    tx: &mut Tx<'_>,
    id: Uuid,
    actor: &str,
    manage: bool,
    require_approved: bool,
) -> Result<(), StatusCode> {
    let status: Option<String> =
        sqlx::query_scalar("SELECT status FROM cloud_publishers WHERE id=$1 FOR UPDATE")
            .bind(id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(db)?;
    let role: Option<String> = sqlx::query_scalar(
        "SELECT role FROM cloud_publisher_memberships WHERE publisher_id=$1 AND user_id=$2",
    )
    .bind(id)
    .bind(actor)
    .fetch_optional(&mut **tx)
    .await
    .map_err(db)?;
    if status.is_none() || role.is_none() {
        return Err(StatusCode::NOT_FOUND);
    }
    if manage
        && (!matches!(role.as_deref(), Some("owner" | "admin"))
            || require_approved && status.as_deref() != Some("approved"))
    {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(())
}
async fn admin(tx: &mut Tx<'_>, id: Uuid, actor: &str) -> Result<(), StatusCode> {
    let role: Option<String> = sqlx::query_scalar(
        "SELECT role FROM cloud_account_roles WHERE user_id=$1 AND role='admin' FOR SHARE",
    )
    .bind(actor)
    .fetch_optional(&mut **tx)
    .await
    .map_err(db)?;
    if role.is_none() {
        return Err(StatusCode::FORBIDDEN);
    }
    let status: Option<String> =
        sqlx::query_scalar("SELECT status FROM cloud_publishers WHERE id=$1 FOR UPDATE")
            .bind(id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(db)?;
    if status.is_none() {
        return Err(StatusCode::NOT_FOUND);
    }
    let member: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM cloud_publisher_memberships WHERE publisher_id=$1 AND user_id=$2)").bind(id).bind(actor).fetch_one(&mut **tx).await.map_err(db)?;
    if member {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(())
}
async fn audit(
    tx: &mut Tx<'_>,
    actor: &str,
    publisher: Uuid,
    key: Uuid,
    action: &str,
) -> Result<(), StatusCode> {
    sqlx::query("INSERT INTO cloud_audit_events (id,actor_user_id,publisher_id,subject_id,action) VALUES ($1,$2,$3,$4,$5)")
        .bind(Uuid::new_v4()).bind(actor).bind(publisher).bind(key).bind(action).execute(&mut **tx).await.map_err(db)?;
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Challenge {
    challenge_id: Uuid,
    nonce: String,
    expires_in: u32,
}
pub async fn challenge(
    AuthUser(user): AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Challenge>, StatusCode> {
    if !state.key_enrollment_enabled {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    let mut tx = state.pool.begin().await.map_err(db)?;
    publisher(&mut tx, id, &user.user.id, true, true).await?;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM cloud_publisher_key_challenges WHERE publisher_id=$1 AND actor_user_id=$2 AND consumed_at IS NULL AND expires_at>now()")
        .bind(id).bind(&user.user.id).fetch_one(&mut *tx).await.map_err(db)?;
    if count >= 10 {
        return Err(StatusCode::TOO_MANY_REQUESTS);
    }
    let challenge_id = Uuid::new_v4();
    let nonce = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    sqlx::query("INSERT INTO cloud_publisher_key_challenges (id,publisher_id,actor_user_id,nonce,expires_at) VALUES ($1,$2,$3,$4,now()+interval '5 minutes')")
        .bind(challenge_id).bind(id).bind(&user.user.id).bind(&nonce).execute(&mut *tx).await.map_err(db)?;
    tx.commit().await.map_err(db)?;
    Ok(Json(Challenge {
        challenge_id,
        nonce,
        expires_in: 300,
    }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RequestKey {
    challenge_id: Uuid,
    key_id: String,
    public_key: String,
    signature: String,
}
// Domain separation plus an unpredictable, single-use, publisher- and actor-bound challenge.
pub fn proof_message(
    publisher: Uuid,
    challenge: Uuid,
    nonce: &str,
    key_id: &str,
    public_key: &str,
) -> String {
    format!("tiktools-cloud-publisher-key-v1\n{publisher}\n{challenge}\n{nonce}\n{key_id}\n{public_key}")
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyRecord {
    id: Uuid,
    key_id: String,
    public_key: String,
    fingerprint: String,
    status: String,
}
fn record(row: sqlx::postgres::PgRow) -> KeyRecord {
    KeyRecord {
        id: row.get("id"),
        key_id: row.get("key_id"),
        public_key: row.get("public_key"),
        fingerprint: row.get("fingerprint"),
        status: row.get("status"),
    }
}

pub async fn request_key(
    AuthUser(user): AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(input): Json<RequestKey>,
) -> Result<(StatusCode, Json<KeyRecord>), StatusCode> {
    let key_id = KeyId::parse(&input.key_id).map_err(|_| StatusCode::BAD_REQUEST)?;
    let raw = STANDARD
        .decode(&input.public_key)
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    if STANDARD.encode(&raw) != input.public_key {
        return Err(StatusCode::BAD_REQUEST);
    }
    let key = PublicKey::new(key_id, &raw).map_err(|_| StatusCode::BAD_REQUEST)?;
    let signature = STANDARD
        .decode(&input.signature)
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    if signature.len() != 64 || STANDARD.encode(&signature) != input.signature {
        return Err(StatusCode::BAD_REQUEST);
    }
    let fingerprint = key
        .fingerprint()
        .map_err(|_| StatusCode::BAD_REQUEST)?
        .to_string();
    if !state.key_enrollment_enabled {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    let mut tx = state.pool.begin().await.map_err(db)?;
    publisher(&mut tx, id, &user.user.id, true, true).await?;
    let row=sqlx::query("SELECT nonce FROM cloud_publisher_key_challenges WHERE id=$1 AND publisher_id=$2 AND actor_user_id=$3 AND consumed_at IS NULL AND expires_at>now() FOR UPDATE")
        .bind(input.challenge_id).bind(id).bind(&user.user.id).fetch_optional(&mut *tx).await.map_err(db)?.ok_or(StatusCode::CONFLICT)?;
    let nonce: String = row.get("nonce");
    let message = proof_message(
        id,
        input.challenge_id,
        &nonce,
        &input.key_id,
        &input.public_key,
    );
    key.verify(message.as_bytes(), &signature)
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM cloud_publisher_keys WHERE publisher_id=$1 AND status <> 'revoked'",
    )
    .bind(id)
    .fetch_one(&mut *tx)
    .await
    .map_err(db)?;
    if count >= 10 {
        return Err(StatusCode::TOO_MANY_REQUESTS);
    }
    let key_row = Uuid::new_v4();
    let result=sqlx::query("INSERT INTO cloud_publisher_keys (id,publisher_id,key_id,public_key,fingerprint,status,requested_by) VALUES ($1,$2,$3,$4,$5,'pending',$6)")
        .bind(key_row).bind(id).bind(&input.key_id).bind(&input.public_key).bind(&fingerprint).bind(&user.user.id).execute(&mut *tx).await;
    if let Err(e) = result {
        if e.as_database_error()
            .is_some_and(|e| e.is_unique_violation())
        {
            return Err(StatusCode::CONFLICT);
        }
        return Err(db(e));
    }
    sqlx::query("UPDATE cloud_publisher_key_challenges SET consumed_at=now() WHERE id=$1")
        .bind(input.challenge_id)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    audit(
        &mut tx,
        &user.user.id,
        id,
        key_row,
        "publisher_key.requested",
    )
    .await?;
    tx.commit().await.map_err(db)?;
    Ok((
        StatusCode::CREATED,
        Json(KeyRecord {
            id: key_row,
            key_id: input.key_id,
            public_key: input.public_key,
            fingerprint,
            status: "pending".into(),
        }),
    ))
}
async fn key_list(tx: &mut Tx<'_>, id: Uuid) -> Result<Json<Vec<KeyRecord>>, StatusCode> {
    let rows=sqlx::query("SELECT id,key_id,public_key,fingerprint,status FROM cloud_publisher_keys WHERE publisher_id=$1 ORDER BY created_at,id").bind(id).fetch_all(&mut **tx).await.map_err(db)?;
    Ok(Json(rows.into_iter().map(record).collect()))
}
pub async fn list(
    AuthUser(user): AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<KeyRecord>>, StatusCode> {
    let mut tx = state.pool.begin().await.map_err(db)?;
    publisher(&mut tx, id, &user.user.id, false, false).await?;
    let result = key_list(&mut tx, id).await?;
    tx.commit().await.map_err(db)?;
    Ok(result)
}
pub async fn admin_list(
    AuthUser(user): AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<KeyRecord>>, StatusCode> {
    let mut tx = state.pool.begin().await.map_err(db)?;
    admin(&mut tx, id, &user.user.id).await?;
    let result = key_list(&mut tx, id).await?;
    tx.commit().await.map_err(db)?;
    Ok(result)
}
async fn change(
    state: AppState,
    actor: &str,
    publisher_id: Uuid,
    key: Uuid,
    as_admin: bool,
    approve: bool,
) -> Result<StatusCode, StatusCode> {
    if approve && !state.key_enrollment_enabled {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    let mut tx = state.pool.begin().await.map_err(db)?;
    if as_admin {
        admin(&mut tx, publisher_id, actor).await?;
    } else {
        publisher(&mut tx, publisher_id, actor, true, false).await?;
    }
    let row=sqlx::query("SELECT status, requested_by FROM cloud_publisher_keys WHERE id=$1 AND publisher_id=$2 FOR UPDATE").bind(key).bind(publisher_id).fetch_optional(&mut *tx).await.map_err(db)?.ok_or(StatusCode::NOT_FOUND)?;
    let status: String = row.get("status");
    if approve {
        let publisher_status: String =
            sqlx::query_scalar("SELECT status FROM cloud_publishers WHERE id=$1")
                .bind(publisher_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(db)?;
        if publisher_status != "approved"
            || row.get::<String, _>("requested_by") == actor
            || status != "pending"
        {
            return Err(StatusCode::CONFLICT);
        }
    } else if status == "revoked" {
        tx.commit().await.map_err(db)?;
        return Ok(StatusCode::NO_CONTENT);
    }
    let new_status = if approve { "approved" } else { "revoked" };
    sqlx::query("UPDATE cloud_publisher_keys SET status=$2 WHERE id=$1")
        .bind(key)
        .bind(new_status)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    audit(
        &mut tx,
        actor,
        publisher_id,
        key,
        if approve {
            "publisher_key.approved"
        } else {
            "publisher_key.revoked"
        },
    )
    .await?;
    tx.commit().await.map_err(db)?;
    Ok(StatusCode::NO_CONTENT)
}
pub async fn approve(
    AuthUser(user): AuthUser,
    State(state): State<AppState>,
    Path((id, key)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, StatusCode> {
    change(state, &user.user.id, id, key, true, true).await
}
pub async fn revoke(
    AuthUser(user): AuthUser,
    State(state): State<AppState>,
    Path((id, key)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, StatusCode> {
    change(state, &user.user.id, id, key, false, false).await
}
pub async fn admin_revoke(
    AuthUser(user): AuthUser,
    State(state): State<AppState>,
    Path((id, key)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, StatusCode> {
    change(state, &user.user.id, id, key, true, false).await
}
