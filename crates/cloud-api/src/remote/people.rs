//! Bounded nickname discovery for targeted invitations. Emails are never returned.
use super::{db, throttle};
use crate::{
    api_error::{Authenticated, Kind, RequestId, V3Error},
    AppState,
};
use axum::{
    extract::{Query, State},
    Extension, Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Search {
    q: String,
}

pub async fn search(
    Authenticated(user): Authenticated,
    Extension(rid): Extension<RequestId>,
    State(state): State<AppState>,
    Query(query): Query<Search>,
) -> Result<Json<Value>, V3Error> {
    throttle(&state, &rid, &user.user.id, "remote-people", 30).await?;
    let q = query.q.trim().to_lowercase();
    if !(3..=64).contains(&q.chars().count()) || q.contains('@') || q.chars().any(char::is_control)
    {
        return Err(V3Error::new(&rid, Kind::InvalidRequest));
    }
    // Escape LIKE metacharacters so a wildcard cannot enumerate all accounts.
    let pattern = format!(
        "{}%",
        q.replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    );
    let rows = sqlx::query("SELECT * FROM (SELECT id,left(split_part(email,'@',1),128) AS nickname, count(*) OVER (PARTITION BY lower(split_part(email,'@',1))) > 1 AS ambiguous FROM \"user\" WHERE lower(split_part(email,'@',1)) LIKE $1) candidates WHERE id<>$2 ORDER BY lower(nickname),id LIMIT 10")
        .bind(pattern).bind(&user.user.id).fetch_all(&state.pool).await.map_err(db(&rid))?;
    let items: Vec<Value> = rows.into_iter().map(|row| json!({"id":row.get::<String,_>("id"),"nickname":row.get::<String,_>("nickname"),"ambiguous":row.get::<bool,_>("ambiguous")})).collect();
    Ok(Json(json!({"items":items})))
}
