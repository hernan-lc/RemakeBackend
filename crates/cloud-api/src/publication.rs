//! A durable publication request, not a signed snapshot or a deployed release.
use crate::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use better_auth::axum_integration::AuthUser;
use serde::Serialize;
use sqlx::Row;
use uuid::Uuid;
fn db(_: sqlx::Error) -> StatusCode {
    tracing::error!("publication request failed");
    StatusCode::INTERNAL_SERVER_ERROR
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicationRequest {
    job_id: Uuid,
    status: String,
    processing_available: bool,
}
pub async fn request(
    AuthUser(user): AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<(StatusCode, Json<PublicationRequest>), StatusCode> {
    crate::uploads::limit(&state, &user.user.id, "publication-request").await?;
    let mut tx = state.pool.begin().await.map_err(db)?;
    crate::moderation::reviewer(&mut tx, &user.user.id).await?;
    let row = sqlx::query(
        "SELECT publisher_id,publisher_key_id,upload_id FROM cloud_plugin_releases WHERE id=$1",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(db)?
    .ok_or(StatusCode::NOT_FOUND)?;
    let publisher: Uuid = row.get("publisher_id");
    let key: Uuid = row.get("publisher_key_id");
    let upload: Uuid = row.get("upload_id");
    let publisher_status: String =
        sqlx::query_scalar("SELECT status FROM cloud_publishers WHERE id=$1 FOR UPDATE")
            .bind(publisher)
            .fetch_one(&mut *tx)
            .await
            .map_err(db)?;
    let member:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM cloud_publisher_memberships WHERE publisher_id=$1 AND user_id=$2)").bind(publisher).bind(&user.user.id).fetch_one(&mut *tx).await.map_err(db)?;
    if member {
        return Err(StatusCode::FORBIDDEN);
    }
    let (key_status,key_requester):(String,String)=sqlx::query_as("SELECT status,requested_by FROM cloud_publisher_keys WHERE id=$1 AND publisher_id=$2 FOR UPDATE").bind(key).bind(publisher).fetch_one(&mut *tx).await.map_err(db)?;
    let row=sqlx::query("SELECT r.status,r.requested_by AS release_requester,u.requested_by AS upload_requester,j.status AS verification_status,j.result_class,m.decision FROM cloud_plugin_releases r JOIN cloud_upload_sessions u ON u.id=r.upload_id AND u.publisher_id=r.publisher_id JOIN cloud_verification_jobs j ON j.upload_id=u.id AND j.release_id=r.id LEFT JOIN cloud_moderation_events m ON m.release_id=r.id WHERE r.id=$1 AND u.id=$2 FOR UPDATE OF r,j")
        .bind(id).bind(upload).fetch_one(&mut *tx).await.map_err(db)?;
    if key_requester == user.user.id
        || row.get::<String, _>("release_requester") == user.user.id
        || row.get::<String, _>("upload_requester") == user.user.id
    {
        return Err(StatusCode::FORBIDDEN);
    }
    if publisher_status != "approved"
        || key_status != "approved"
        || row.get::<String, _>("status") != "review_approved"
        || row.get::<String, _>("verification_status") != "antivirus_passed"
        || row.get::<Option<String>, _>("result_class").as_deref() != Some("antivirus_passed")
        || row.get::<Option<String>, _>("decision").as_deref() != Some("approve")
    {
        return Err(StatusCode::CONFLICT);
    }
    if let Some((job_id, status)) = sqlx::query_as::<_, (Uuid, String)>(
        "SELECT id,status FROM cloud_publication_jobs WHERE release_id=$1",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(db)?
    {
        tx.commit().await.map_err(db)?;
        return Ok((
            StatusCode::ACCEPTED,
            Json(PublicationRequest {
                job_id,
                status,
                processing_available: false,
            }),
        ));
    }
    let job_id = Uuid::new_v4();
    sqlx::query("INSERT INTO cloud_publication_jobs (id,release_id,requested_by,status) VALUES ($1,$2,$3,'queued')").bind(job_id).bind(id).bind(&user.user.id).execute(&mut *tx).await.map_err(db)?;
    sqlx::query("INSERT INTO cloud_audit_events (id,actor_user_id,publisher_id,subject_id,action) VALUES ($1,$2,$3,$4,'release.publication_requested')").bind(Uuid::new_v4()).bind(&user.user.id).bind(publisher).bind(id).execute(&mut *tx).await.map_err(db)?;
    tx.commit().await.map_err(db)?;
    Ok((
        StatusCode::ACCEPTED,
        Json(PublicationRequest {
            job_id,
            status: "queued".into(),
            processing_available: false,
        }),
    ))
}
