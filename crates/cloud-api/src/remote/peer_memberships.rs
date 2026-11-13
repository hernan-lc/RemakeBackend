//! Native controllers discover only hosts approved for their own enrolled key.
use super::{db, throttle};
use crate::{
    api_error::{Authenticated, Kind, RequestId, V3Error},
    AppState,
};
use axum::{
    extract::{Path, State},
    Extension, Json,
};
use serde_json::{json, Value};
use sqlx::Row;
use uuid::Uuid;

pub async fn list(
    Authenticated(user): Authenticated,
    Extension(rid): Extension<RequestId>,
    State(state): State<AppState>,
    Path(device): Path<Uuid>,
) -> Result<Json<Value>, V3Error> {
    throttle(&state, &rid, &user.user.id, "peer-memberships", 60).await?;
    let mut tx = state.pool.begin().await.map_err(db(&rid))?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await
        .map_err(db(&rid))?;
    let owned:Option<Uuid>=sqlx::query_scalar("SELECT id FROM cloud_remote_devices WHERE id=$1 AND owner_user_id=$2 AND kind='controller' AND status='active'")
        .bind(device).bind(&user.user.id).fetch_optional(&mut *tx).await.map_err(db(&rid))?;
    if owned.is_none() {
        return Err(V3Error::new(&rid, Kind::NotFound));
    }
    let rows=sqlx::query("SELECT h.id,h.name,g.role,g.policy FROM cloud_remote_grants g JOIN cloud_remote_hosts h ON h.id=g.host_id WHERE g.device_id=$1 AND g.status='active' AND g.confirmed_revision=g.revision AND h.status='active' ORDER BY h.id LIMIT 65")
        .bind(device).fetch_all(&mut *tx).await.map_err(db(&rid))?;
    if rows.len() > 64 {
        return Err(V3Error::new(&rid, Kind::InvalidRequest));
    }
    let items:Vec<Value>=rows.into_iter().map(|row| {
        let policy:Value=row.get("policy");
        json!({"id":row.get::<Uuid,_>("id"),"name":row.get::<String,_>("name"),"role":row.get::<String,_>("role"),"clientKind":policy.get("nativeClientKind").and_then(Value::as_str).unwrap_or("desktop")})
    }).collect();
    tx.commit().await.map_err(db(&rid))?;
    Ok(Json(json!({"items":items})))
}
