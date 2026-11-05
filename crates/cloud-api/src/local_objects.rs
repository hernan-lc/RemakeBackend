//! Short-lived, signed local-volume capabilities. URLs are bearer secrets.
use crate::AppState;
use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use base64::{engine::general_purpose::STANDARD, Engine};
use futures_util::{StreamExt, TryStreamExt};
use serde::Deserialize;
use std::{io, sync::OnceLock, time::Duration};
use tokio::{
    io::{AsyncSeekExt, SeekFrom},
    sync::Semaphore,
};
use tokio_util::io::{ReaderStream, StreamReader};
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UploadGrant {
    expires: u64,
    size: i64,
    sha256: String,
    signature: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InspectionGrant {
    expires: u64,
    version: String,
    signature: String,
}

fn key(publisher: Uuid, upload: Uuid) -> String {
    format!("quarantine/{publisher}/{upload}")
}
fn semaphore() -> &'static Semaphore {
    static LIMIT: OnceLock<Semaphore> = OnceLock::new();
    LIMIT.get_or_init(|| Semaphore::new(2))
}

pub async fn upload(
    State(state): State<AppState>,
    Path((publisher, upload)): Path<(Uuid, Uuid)>,
    Query(grant): Query<UploadGrant>,
    headers: HeaderMap,
    body: Body,
) -> Result<StatusCode, StatusCode> {
    let key = key(publisher, upload);
    let store = state.object_storage.as_ref().ok_or(StatusCode::NOT_FOUND)?;
    let local = store
        .local_upload(
            &key,
            grant.size,
            &grant.sha256,
            grant.expires,
            &grant.signature,
        )
        .map_err(|_| StatusCode::FORBIDDEN)?;
    let checksum = hex::decode(&grant.sha256).map_err(|_| StatusCode::BAD_REQUEST)?;
    if headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        != Some(grant.size.to_string().as_str())
        || headers
            .get(header::IF_NONE_MATCH)
            .and_then(|v| v.to_str().ok())
            != Some("*")
        || headers
            .get("x-amz-checksum-sha256")
            .and_then(|v| v.to_str().ok())
            != Some(STANDARD.encode(checksum).as_str())
        || headers.contains_key(header::CONTENT_ENCODING)
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    let _permit = semaphore()
        .try_acquire()
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let stream = body
        .into_data_stream()
        .map_err(|_| io::Error::other("upload stream failed"));
    let reader = StreamReader::new(stream);
    tokio::time::timeout(
        Duration::from_secs(120),
        local.put(&key, grant.size, &grant.sha256, reader),
    )
    .await
    .map_err(|_| StatusCode::REQUEST_TIMEOUT)?
    .map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;
    Ok(StatusCode::OK)
}

pub async fn inspection(
    State(state): State<AppState>,
    Path((publisher, upload)): Path<(Uuid, Uuid)>,
    Query(grant): Query<InspectionGrant>,
) -> Result<Response, StatusCode> {
    let key = key(publisher, upload);
    let store = state.object_storage.as_ref().ok_or(StatusCode::NOT_FOUND)?;
    let (local, version) = store
        .local_inspection(&key, &grant.version, grant.expires, &grant.signature)
        .await
        .map_err(|_| StatusCode::FORBIDDEN)?;
    let permit = semaphore()
        .try_acquire()
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    // Complete verification before sending any bytes to the reviewer.
    let file = tempfile::tempfile().map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let mut file = tokio::fs::File::from_std(file);
    tokio::time::timeout(
        Duration::from_secs(120),
        local.download(&key, &version, &mut file),
    )
    .await
    .map_err(|_| StatusCode::REQUEST_TIMEOUT)?
    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    file.seek(SeekFrom::Start(0))
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    // Retain the permit until the client finishes or abandons the response,
    // bounding the number of simultaneously open verified temporary files.
    let stream = ReaderStream::new(file).map(move |chunk| {
        let _ = &permit;
        chunk
    });
    let mut response = Body::from_stream(stream).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        "application/octet-stream".parse().unwrap(),
    );
    response
        .headers_mut()
        .insert(header::CONTENT_DISPOSITION, "attachment".parse().unwrap());
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    response.headers_mut().insert(
        header::CONTENT_LENGTH,
        version
            .size
            .to_string()
            .parse()
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
    );
    Ok(response)
}
