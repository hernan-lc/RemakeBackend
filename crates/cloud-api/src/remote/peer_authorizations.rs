//! Complete, owner-only snapshot for the native host's local permission bridge.
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

pub async fn snapshot(
    Authenticated(user): Authenticated,
    Extension(rid): Extension<RequestId>,
    State(state): State<AppState>,
    Path(host): Path<Uuid>,
) -> Result<Json<Value>, V3Error> {
    throttle(&state, &rid, &user.user.id, "peer-authorizations", 60).await?;
    let mut tx = state.pool.begin().await.map_err(db(&rid))?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await
        .map_err(db(&rid))?;
    // Check ownership in the same snapshot as the grant/device records.
    let host_status: Option<String> = sqlx::query_scalar(
        "SELECT status FROM cloud_remote_hosts WHERE id=$1 AND owner_user_id=$2",
    )
    .bind(host)
    .bind(&user.user.id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(db(&rid))?;
    let host_status = host_status.ok_or(V3Error::new(&rid, Kind::NotFound))?;
    let rows = sqlx::query("SELECT g.id,g.device_id,g.role,g.policy,g.status,g.revision,g.confirmed_revision,d.label,d.status AS device_status,p.endpoint_id FROM cloud_remote_grants g JOIN cloud_remote_devices d ON d.id=g.device_id LEFT JOIN cloud_remote_peer_endpoints p ON p.device_id=d.id WHERE g.host_id=$1 ORDER BY g.id LIMIT 257")
        .bind(host).fetch_all(&mut *tx).await.map_err(db(&rid))?;
    // A truncated snapshot must never be mistaken for a revocation list.
    if rows.len() > 256 {
        return Err(V3Error::new(&rid, Kind::InvalidRequest));
    }
    let items: Vec<Value> = rows.into_iter().map(|row| json!({
        "id":row.get::<Uuid,_>("id"),"deviceId":row.get::<Uuid,_>("device_id"),
        "deviceLabel":row.get::<String,_>("label"),"deviceStatus":row.get::<String,_>("device_status"),
        "role":row.get::<String,_>("role"),"policy":row.get::<Value,_>("policy"),
        "status":row.get::<String,_>("status"),"revision":row.get::<i64,_>("revision"),
        "confirmedRevision":row.get::<Option<i64>,_>("confirmed_revision"),
        "endpointId":row.get::<Option<String>,_>("endpoint_id")
    })).collect();
    tx.commit().await.map_err(db(&rid))?;
    Ok(Json(
        json!({"hostId":host,"hostStatus":host_status,"items":items}),
    ))
}
