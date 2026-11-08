//! V2 adapters. No application files, plugin execution or Desktop controls are changed.
use crate::{
    decode_with_limit, parse_origin, Session, SyncError, WorkspaceChanges, WorkspaceRecord,
    WorkspaceSnapshot,
};
use cloud_contracts::portable;
use reqwest::{
    header::{IF_MATCH, IF_NONE_MATCH},
    Method, StatusCode,
};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use uuid::Uuid;

const MAX_V2_RESPONSE: usize = 32 * 1024 * 1024;
/// Anonymous discovery bound to the operator-provisioned instance origin.
/// This is metadata transport; signed plugin verification still belongs to
/// the pinned registry protocol and must not be replaced by catalog digests.
pub struct CloudCatalog {
    origin: url::Url,
    client: reqwest::Client,
}
impl CloudCatalog {
    pub fn new(instance_origin: &str) -> Result<Self, SyncError> {
        Ok(Self {
            origin: parse_origin(instance_origin)?,
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .map_err(|_| SyncError::Configuration)?,
        })
    }
    async fn get(&self, path: &str, params: &[(&str, String)]) -> Result<Value, SyncError> {
        let url = self
            .origin
            .join(path)
            .map_err(|_| SyncError::Configuration)?;
        let response = self
            .client
            .get(url)
            .query(params)
            .send()
            .await
            .map_err(|_| SyncError::Network)?;
        decode_with_limit(response, MAX_V2_RESPONSE).await
    }
    pub async fn capabilities(&self) -> Result<Value, SyncError> {
        self.get("api/v2/capabilities", &[]).await
    }
    pub async fn plugins(
        &self,
        query: &str,
        after: Option<&str>,
        generation: Option<i64>,
    ) -> Result<Value, SyncError> {
        if query.len() > 100
            || generation.is_some_and(|n| n < 1)
            || after.is_some_and(|id| !portable::valid_key("plugin-selections", id))
        {
            return Err(SyncError::Configuration);
        }
        let mut params = vec![("q", query.into())];
        if let Some(after) = after {
            params.push(("after", after.into()));
        }
        if let Some(generation) = generation {
            params.push(("generation", generation.to_string()));
        }
        self.get("api/v2/catalog/plugins", &params).await
    }
    pub async fn plugin(&self, id: &str, generation: Option<i64>) -> Result<Value, SyncError> {
        if !portable::valid_key("plugin-selections", id) || generation.is_some_and(|n| n < 1) {
            return Err(SyncError::Configuration);
        }
        let params = generation
            .map(|n| vec![("generation", n.to_string())])
            .unwrap_or_default();
        self.get(&format!("api/v2/catalog/plugins/{id}"), &params)
            .await
    }
    pub async fn documents(
        &self,
        kind: Option<&str>,
        after: Option<Uuid>,
    ) -> Result<Value, SyncError> {
        if kind.is_some_and(|k| !matches!(k, "template" | "profile")) {
            return Err(SyncError::Configuration);
        }
        let mut params = Vec::new();
        if let Some(kind) = kind {
            params.push(("kind", kind.into()));
        }
        if let Some(after) = after {
            params.push(("after", after.to_string()));
        }
        self.get("api/v2/catalog/documents", &params).await
    }
    pub async fn document(&self, id: Uuid, revision: Option<i64>) -> Result<Value, SyncError> {
        if revision.is_some_and(|n| n < 1) {
            return Err(SyncError::Configuration);
        }
        let path = revision
            .map(|n| format!("api/v2/catalog/documents/{id}/versions/{n}"))
            .unwrap_or_else(|| format!("api/v2/catalog/documents/{id}"));
        self.get(&path, &[]).await
    }
}
impl Session {
    async fn v2_get<T: DeserializeOwned>(
        &self,
        path: &str,
        params: &[(&str, String)],
    ) -> Result<T, SyncError> {
        let response = self
            .request(Method::GET, path)?
            .query(params)
            .send()
            .await
            .map_err(|_| SyncError::Network)?;
        decode_with_limit(response, MAX_V2_RESPONSE).await
    }
    pub async fn workspace_snapshot(&self) -> Result<WorkspaceSnapshot, SyncError> {
        let snapshot: WorkspaceSnapshot = self.v2_get("api/v2/sync/export", &[]).await?;
        if !snapshot.valid() {
            return Err(SyncError::Protocol);
        }
        Ok(snapshot)
    }
    pub async fn workspace_changes(&self, cursor: i64) -> Result<WorkspaceChanges, SyncError> {
        if cursor < 0 {
            return Err(SyncError::Configuration);
        }
        let page: WorkspaceChanges = self
            .v2_get("api/v2/sync/changes", &[("cursor", cursor.to_string())])
            .await?;
        if !page.valid_after(cursor) {
            return Err(SyncError::Protocol);
        }
        Ok(page)
    }
    pub async fn workspace_record(
        &self,
        collection: &str,
        key: &str,
    ) -> Result<WorkspaceRecord, SyncError> {
        if !portable::valid_key(collection, key) {
            return Err(SyncError::Configuration);
        }
        let record: WorkspaceRecord = self
            .v2_get(&format!("api/v2/sync/collections/{collection}/{key}"), &[])
            .await?;
        if record.collection != collection || record.key != key || !record.valid(record.revision) {
            return Err(SyncError::Protocol);
        }
        Ok(record)
    }
    pub async fn workspace_put(
        &self,
        collection: &str,
        key: &str,
        value: Value,
        base_revision: Option<i64>,
    ) -> Result<WorkspaceRecord, SyncError> {
        if !portable::validate(collection, key, &value) || base_revision.is_some_and(|n| n < 1) {
            return Err(SyncError::Configuration);
        }
        let mut request = self
            .request(
                Method::PUT,
                &format!("api/v2/sync/collections/{collection}/{key}"),
            )?
            .json(&value);
        request = if let Some(n) = base_revision {
            request.header(IF_MATCH, format!("\"{n}\""))
        } else {
            request.header(IF_NONE_MATCH, "*")
        };
        let record: WorkspaceRecord = decode_with_limit(
            request.send().await.map_err(|_| SyncError::Network)?,
            MAX_V2_RESPONSE,
        )
        .await?;
        if record.collection != collection
            || record.key != key
            || record.deleted
            || record.value.as_ref() != Some(&value)
            || !record.valid(record.revision)
        {
            return Err(SyncError::Protocol);
        }
        Ok(record)
    }
    pub async fn workspace_delete(
        &self,
        collection: &str,
        key: &str,
        base_revision: i64,
    ) -> Result<WorkspaceRecord, SyncError> {
        if !portable::valid_key(collection, key) || base_revision < 1 {
            return Err(SyncError::Configuration);
        }
        let response = self
            .request(
                Method::DELETE,
                &format!("api/v2/sync/collections/{collection}/{key}"),
            )?
            .header(IF_MATCH, format!("\"{base_revision}\""))
            .send()
            .await
            .map_err(|_| SyncError::Network)?;
        let record: WorkspaceRecord = decode_with_limit(response, MAX_V2_RESPONSE).await?;
        if record.collection != collection
            || record.key != key
            || !record.deleted
            || !record.valid(record.revision)
        {
            return Err(SyncError::Protocol);
        }
        Ok(record)
    }
    pub async fn library_create(&self, kind: &str, document: Value) -> Result<Value, SyncError> {
        if !portable::document(kind, &document) {
            return Err(SyncError::Configuration);
        }
        let response = self
            .request(Method::POST, "api/v2/library/documents")?
            .json(&json!({"kind":kind,"document":document}))
            .send()
            .await
            .map_err(|_| SyncError::Network)?;
        decode_with_limit(response, MAX_V2_RESPONSE).await
    }
    pub async fn library_documents(&self, after: Option<Uuid>) -> Result<Value, SyncError> {
        let params = after
            .map(|id| vec![("after", id.to_string())])
            .unwrap_or_default();
        self.v2_get("api/v2/library/documents", &params).await
    }
    pub async fn library_document(&self, id: Uuid) -> Result<Value, SyncError> {
        self.v2_get(&format!("api/v2/library/documents/{id}"), &[])
            .await
    }
    pub async fn library_update(
        &self,
        id: Uuid,
        kind: &str,
        document: Value,
        base_revision: i64,
    ) -> Result<Value, SyncError> {
        if base_revision < 1 || !portable::document(kind, &document) {
            return Err(SyncError::Configuration);
        }
        let response = self
            .request(Method::PUT, &format!("api/v2/library/documents/{id}"))?
            .header(IF_MATCH, format!("\"{base_revision}\""))
            .json(&json!({"document":document}))
            .send()
            .await
            .map_err(|_| SyncError::Network)?;
        decode_with_limit(response, MAX_V2_RESPONSE).await
    }
    pub async fn library_submit(&self, id: Uuid, revision: i64) -> Result<(), SyncError> {
        if revision < 1 {
            return Err(SyncError::Configuration);
        }
        let response = self
            .request(
                Method::POST,
                &format!("api/v2/library/documents/{id}/versions/{revision}/submit"),
            )?
            .send()
            .await
            .map_err(|_| SyncError::Network)?;
        if response.status() == StatusCode::NO_CONTENT {
            return Ok(());
        }
        decode_with_limit::<Value>(response, MAX_V2_RESPONSE)
            .await
            .map(|_| ())
    }
    pub async fn library_withdraw(
        &self,
        id: Uuid,
        revision: i64,
        reason: &str,
    ) -> Result<(), SyncError> {
        if revision < 1 || reason.trim().is_empty() || reason.len() > 500 {
            return Err(SyncError::Configuration);
        }
        let response = self
            .request(
                Method::POST,
                &format!("api/v2/library/documents/{id}/versions/{revision}/withdraw"),
            )?
            .json(&json!({"reason":reason}))
            .send()
            .await
            .map_err(|_| SyncError::Network)?;
        if response.status() == StatusCode::NO_CONTENT {
            return Ok(());
        }
        decode_with_limit::<Value>(response, MAX_V2_RESPONSE)
            .await
            .map(|_| ())
    }
    pub async fn export_account_v2(&self) -> Result<Value, SyncError> {
        let value: Value = self.v2_get("api/v2/me/export", &[]).await?;
        if value["schemaVersion"] != 2 || value["account"]["profile"]["id"] != self.account_id {
            return Err(SyncError::Protocol);
        }
        let snapshot: WorkspaceSnapshot = serde_json::from_value(value["workspaceSync"].clone())
            .map_err(|_| SyncError::Protocol)?;
        if !snapshot.valid() {
            return Err(SyncError::Protocol);
        }
        Ok(value)
    }
}
