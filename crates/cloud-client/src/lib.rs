//! Explicitly opt-in sync client. No network is used until a session is opened.
//! Persist `SyncState` in application storage; keep session tokens in an OS keychain.
#![forbid(unsafe_code)]
#[cfg(all(
    feature = "os-keychain",
    not(any(target_os = "linux", target_os = "macos", target_os = "windows"))
))]
compile_error!("os-keychain is supported only on Linux, macOS, and Windows");
#[cfg(feature = "os-keychain")]
mod keychain;
#[cfg(feature = "os-keychain")]
pub use keychain::{Keychain, KeychainError};
mod login;
mod store;
pub use login::{LoginAttempt, LoginResult};
use reqwest::{
    header::{HeaderValue, AUTHORIZATION, IF_MATCH, IF_NONE_MATCH},
    Client, Response, StatusCode,
};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, time::Duration};
pub use store::{StateSnapshot, StateStore, StoreError};
use thiserror::Error;
use url::Url;

const COLLECTION: &str = "plugin-selections";
const MAX_RESPONSE: usize = 1024 * 1024;
const MAX_PAGES: usize = 200;

#[derive(Debug, Error)]
pub enum SyncError {
    #[error("invalid cloud origin or token")]
    Configuration,
    #[error("network request failed")]
    Network,
    #[error("cloud session is not authorized")]
    Unauthorized,
    #[error("sync state belongs to a different account")]
    WrongAccount,
    #[error("remote change conflicts with a local edit")]
    Conflict,
    #[error("sync cursor expired; rebuild is required")]
    CursorExpired,
    #[error("cloud rejected the request ({0})")]
    Http(u16),
    #[error("invalid cloud response")]
    Protocol,
    #[error("too many pages; retry the operation")]
    TooManyPages,
    #[error("no local edit exists")]
    NoPendingEdit,
    #[error("desktop login was cancelled or expired")]
    LoginCancelled,
    #[error("unable to open the system browser")]
    Browser,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginSelection {
    pub schema_version: u8,
    pub plugin_id: String,
    pub version: String,
    pub enabled: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Record {
    pub collection: String,
    pub key: String,
    pub revision: i64,
    pub deleted: bool,
    pub value: Option<PluginSelection>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "operation",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Edit {
    Put(PluginSelection),
    Delete,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Pending {
    pub base_revision: Option<i64>,
    pub edit: Edit,
    pub conflict: bool,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub remote: Option<Record>,
    pub pending: Option<Pending>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SyncState {
    pub origin: String,
    pub account_id: String,
    pub cursor: i64,
    pub entries: BTreeMap<String, Entry>,
}
impl SyncState {
    pub fn new(origin: String, account_id: String) -> Self {
        Self {
            origin,
            account_id,
            cursor: 0,
            entries: BTreeMap::new(),
        }
    }
    /// Stage an edit locally. Repeated local changes retain the original base revision.
    pub fn stage(&mut self, key: String, edit: Edit) -> Result<(), SyncError> {
        if !valid_key(&key)
            || matches!(&edit, Edit::Put(value) if value.schema_version != 1 || value.plugin_id != key || value.version.len() > 64 || semver::Version::parse(&value.version).is_err())
        {
            return Err(SyncError::Configuration);
        }
        let entry = self.entries.entry(key).or_default();
        if let Some(pending) = &mut entry.pending {
            pending.edit = edit;
        } else {
            entry.pending = Some(Pending {
                base_revision: entry.remote.as_ref().map(|r| r.revision),
                edit,
                conflict: false,
            });
        }
        Ok(())
    }
    /// Explicit user choice to keep local data after reviewing a conflict.
    pub fn rebase(&mut self, key: &str) -> Result<(), SyncError> {
        let entry = self.entries.get_mut(key).ok_or(SyncError::NoPendingEdit)?;
        let pending = entry.pending.as_mut().ok_or(SyncError::NoPendingEdit)?;
        pending.base_revision = entry.remote.as_ref().map(|r| r.revision);
        pending.conflict = false;
        Ok(())
    }
    /// Explicit user choice to discard an unsent local edit.
    pub fn discard(&mut self, key: &str) -> Result<(), SyncError> {
        let entry = self.entries.get_mut(key).ok_or(SyncError::NoPendingEdit)?;
        if entry.pending.take().is_none() {
            return Err(SyncError::NoPendingEdit);
        }
        Ok(())
    }
    fn apply(&mut self, record: Record) -> Result<(), SyncError> {
        validate_record(&record)?;
        let entry = self.entries.entry(record.key.clone()).or_default();
        if entry
            .remote
            .as_ref()
            .is_some_and(|old| old.revision >= record.revision)
        {
            return Ok(());
        }
        if let Some(pending) = &mut entry.pending {
            if pending.base_revision != Some(record.revision) {
                pending.conflict = true;
            }
        }
        entry.remote = Some(record);
        Ok(())
    }
}
fn valid_key(key: &str) -> bool {
    key.len() <= 128
        && key.split_once('.').is_some()
        && key.split('.').all(|p| {
            !p.is_empty()
                && p.bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-' || c == b'_')
        })
}
fn validate_record(record: &Record) -> Result<(), SyncError> {
    if record.collection != COLLECTION
        || !valid_key(&record.key)
        || record.revision <= 0
        || record.deleted != record.value.is_none()
        || record.value.as_ref().is_some_and(|v| {
            v.schema_version != 1
                || v.plugin_id != record.key
                || v.version.len() > 64
                || semver::Version::parse(&v.version).is_err()
        })
    {
        return Err(SyncError::Protocol);
    }
    Ok(())
}

#[derive(Deserialize)]
struct Profile {
    id: String,
}
#[derive(Deserialize)]
struct Page {
    changes: Vec<Record>,
    cursor: i64,
    has_more: bool,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Listing {
    records: Vec<Record>,
    next_after: Option<String>,
}
#[derive(Deserialize)]
struct Head {
    cursor: i64,
}

/// A verified account and in-memory token. It is not serializable or printable.
pub struct Session {
    origin: Url,
    client: Client,
    token: SecretString,
    account_id: String,
}
impl Session {
    /// Explicitly connects and verifies the token. `origin` must be a trusted HTTPS origin;
    /// plain HTTP is accepted only for loopback IP literals for local development.
    pub async fn connect(origin: &str, token: String) -> Result<Self, SyncError> {
        Self::connect_secret(origin, SecretString::from(token)).await
    }
    pub async fn connect_secret(origin: &str, token: SecretString) -> Result<Self, SyncError> {
        let url = parse_origin(origin)?;
        if token.expose_secret().is_empty() || token.expose_secret().len() > 4096 {
            return Err(SyncError::Configuration);
        }
        let mut value = HeaderValue::from_str(&format!("Bearer {}", token.expose_secret()))
            .map_err(|_| SyncError::Configuration)?;
        value.set_sensitive(true);
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|_| SyncError::Configuration)?;
        let mut session = Self {
            origin: url,
            client,
            token,
            account_id: String::new(),
        };
        let profile: Profile = session.get("api/v1/native/me", &[]).await?;
        if profile.id.is_empty() || profile.id.len() > 256 {
            return Err(SyncError::Protocol);
        }
        session.account_id = profile.id;
        Ok(session)
    }
    /// Expose only to an OS credential store; never log or serialize the token.
    pub fn token_for_keychain(&self) -> &SecretString {
        &self.token
    }
    pub fn account_id(&self) -> &str {
        &self.account_id
    }
    /// Revoke the current bearer session on the server. Remove any stored keychain
    /// entry separately after success; an offline error means revocation is unknown.
    pub async fn logout(&self) -> Result<(), SyncError> {
        let response = self
            .request(reqwest::Method::POST, "api/v1/native/logout")?
            .send()
            .await
            .map_err(|_| SyncError::Network)?;
        match response.status() {
            StatusCode::NO_CONTENT => Ok(()),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err(SyncError::Unauthorized),
            status => Err(SyncError::Http(status.as_u16())),
        }
    }
    pub fn new_state(&self) -> SyncState {
        SyncState::new(
            self.origin.origin().ascii_serialization(),
            self.account_id.clone(),
        )
    }
    fn verify_account(&self, state: &SyncState) -> Result<(), SyncError> {
        if state.account_id != self.account_id
            || state.origin != self.origin.origin().ascii_serialization()
        {
            return Err(SyncError::WrongAccount);
        }
        Ok(())
    }
    fn request(
        &self,
        method: reqwest::Method,
        path: &str,
    ) -> Result<reqwest::RequestBuilder, SyncError> {
        let url = self
            .origin
            .join(path)
            .map_err(|_| SyncError::Configuration)?;
        let mut auth = HeaderValue::from_str(&format!("Bearer {}", self.token.expose_secret()))
            .map_err(|_| SyncError::Configuration)?;
        auth.set_sensitive(true);
        Ok(self.client.request(method, url).header(AUTHORIZATION, auth))
    }
    async fn get<T: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
        params: &[(&str, String)],
    ) -> Result<T, SyncError> {
        let response = self
            .request(reqwest::Method::GET, path)?
            .query(params)
            .send()
            .await
            .map_err(|_| SyncError::Network)?;
        decode(response).await
    }
    /// Apply one change page atomically in memory; persist it only after this returns.
    pub async fn pull_page(&self, state: &mut SyncState) -> Result<bool, SyncError> {
        self.verify_account(state)?;
        let page: Page = self
            .get(
                "api/v1/native/sync/changes",
                &[("cursor", state.cursor.to_string())],
            )
            .await?;
        if page.cursor < state.cursor || (page.has_more && page.cursor == state.cursor) {
            return Err(SyncError::Protocol);
        }
        let mut candidate = state.clone();
        let mut last = state.cursor;
        for record in page.changes {
            if record.revision <= last || record.revision > page.cursor {
                return Err(SyncError::Protocol);
            }
            last = record.revision;
            candidate.apply(record)?;
        }
        if last != page.cursor {
            return Err(SyncError::Protocol);
        }
        candidate.cursor = page.cursor;
        *state = candidate;
        Ok(page.has_more)
    }
    /// Send one pending edit. A 409 leaves the edit intact for explicit resolution.
    pub async fn push(&self, state: &mut SyncState, key: &str) -> Result<(), SyncError> {
        self.verify_account(state)?;
        if !valid_key(key) {
            return Err(SyncError::Configuration);
        }
        let entry = state.entries.get(key).ok_or(SyncError::NoPendingEdit)?;
        let pending = entry.pending.as_ref().ok_or(SyncError::NoPendingEdit)?;
        if pending.conflict {
            return Err(SyncError::Conflict);
        }
        if pending.base_revision.is_none() && matches!(pending.edit, Edit::Delete) {
            // A previous write could have succeeded despite a lost response, or
            // another device could have created this record while we were offline.
            state
                .entries
                .get_mut(key)
                .unwrap()
                .pending
                .as_mut()
                .unwrap()
                .conflict = true;
            return Err(SyncError::Conflict);
        }
        let path = format!("api/v1/native/sync/collections/{COLLECTION}/{key}");
        let mut request = match &pending.edit {
            Edit::Put(value) => self.request(reqwest::Method::PUT, &path)?.json(value),
            Edit::Delete => self.request(reqwest::Method::DELETE, &path)?,
        };
        request = if let Some(revision) = pending.base_revision {
            request.header(IF_MATCH, format!("\"{revision}\""))
        } else {
            request.header(IF_NONE_MATCH, "*")
        };
        let response = request.send().await.map_err(|_| SyncError::Network)?;
        if response.status() == StatusCode::CONFLICT {
            state
                .entries
                .get_mut(key)
                .unwrap()
                .pending
                .as_mut()
                .unwrap()
                .conflict = true;
            return Err(SyncError::Conflict);
        }
        let record: Record = decode(response).await?;
        validate_record(&record)?;
        if (match &pending.edit {
            Edit::Put(value) => record.value.as_ref() != Some(value) || record.deleted,
            Edit::Delete => !record.deleted || record.value.is_some(),
        }) || record.key != key
            || pending
                .base_revision
                .is_some_and(|revision| record.revision <= revision)
        {
            return Err(SyncError::Protocol);
        }
        let entry = state.entries.get_mut(key).unwrap();
        entry.remote = Some(record);
        entry.pending = None;
        Ok(())
    }
    /// Rebuild after retention expiry without dropping pending edits. Snapshot and
    /// follow-up changes are committed to state only after every page succeeds.
    pub async fn rebuild(&self, state: &mut SyncState) -> Result<(), SyncError> {
        self.verify_account(state)?;
        let head: Head = self.get("api/v1/native/sync/head", &[]).await?;
        if head.cursor < 0 {
            return Err(SyncError::Protocol);
        }
        let mut candidate = SyncState::new(state.origin.clone(), state.account_id.clone());
        candidate.cursor = head.cursor;
        let mut after: Option<String> = None;
        let mut complete = false;
        for _ in 0..MAX_PAGES {
            let params = after
                .as_ref()
                .map(|s| vec![("after", s.clone())])
                .unwrap_or_default();
            let listing: Listing = self
                .get("api/v1/native/sync/collections/plugin-selections", &params)
                .await?;
            let previous = after.clone();
            for record in listing.records {
                if after.as_ref().is_some_and(|a| record.key <= *a) {
                    return Err(SyncError::Protocol);
                }
                after = Some(record.key.clone());
                candidate.apply(record)?;
            }
            if let Some(next) = listing.next_after {
                if Some(&next) != after.as_ref() || previous.as_ref().is_some_and(|p| next <= *p) {
                    return Err(SyncError::Protocol);
                }
            } else {
                complete = true;
                break;
            }
        }
        if !complete {
            return Err(SyncError::TooManyPages);
        }
        for (key, old) in &state.entries {
            if let Some(pending) = &old.pending {
                let entry = candidate.entries.entry(key.clone()).or_default();
                let mut pending = pending.clone();
                if pending.base_revision != entry.remote.as_ref().map(|r| r.revision) {
                    pending.conflict = true;
                }
                entry.pending = Some(pending);
            }
        }
        for _ in 0..MAX_PAGES {
            if !self.pull_page(&mut candidate).await? {
                *state = candidate;
                return Ok(());
            }
        }
        Err(SyncError::TooManyPages)
    }
}
fn parse_origin(origin: &str) -> Result<Url, SyncError> {
    let url = Url::parse(origin).map_err(|_| SyncError::Configuration)?;
    let loopback = matches!(url.host_str(), Some("127.0.0.1" | "[::1]"));
    if !(url.scheme() == "https" || url.scheme() == "http" && loopback)
        || url.host().is_none()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(SyncError::Configuration);
    }
    Ok(url)
}

async fn decode<T: for<'de> Deserialize<'de>>(mut response: Response) -> Result<T, SyncError> {
    match response.status() {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => return Err(SyncError::Unauthorized),
        StatusCode::CONFLICT => return Err(SyncError::Conflict),
        StatusCode::GONE => return Err(SyncError::CursorExpired),
        s if !s.is_success() => return Err(SyncError::Http(s.as_u16())),
        _ => (),
    }
    if response
        .content_length()
        .is_some_and(|n| n > MAX_RESPONSE as u64)
    {
        return Err(SyncError::Protocol);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| SyncError::Network)? {
        if body.len().saturating_add(chunk.len()) > MAX_RESPONSE {
            return Err(SyncError::Protocol);
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body).map_err(|_| SyncError::Protocol)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn refuses_redirects_before_forwarding_bearer() {
        let destination = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target = destination.local_addr().unwrap();
        let source = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", source.local_addr().unwrap());
        let handler = tokio::spawn(async move {
            let (mut socket, _) = source.accept().await.unwrap();
            let mut request = [0; 4096];
            let _ = socket.read(&mut request).await.unwrap();
            socket.write_all(format!("HTTP/1.1 302 Found\r\nLocation: http://{target}/stolen\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
        });
        assert!(matches!(
            Session::connect(&origin, "test-token".into()).await,
            Err(SyncError::Http(302))
        ));
        handler.await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(100), destination.accept())
                .await
                .is_err()
        );
    }

    #[test]
    fn offline_edits_require_explicit_resolution() {
        let mut state = SyncState::new("https://cloud.example".into(), "user".into());
        state.stage("acme.plugin".into(), Edit::Delete).unwrap();
        assert!(state.entries["acme.plugin"].pending.is_some());
        assert!(state
            .stage(
                "acme.plugin".into(),
                Edit::Put(PluginSelection {
                    schema_version: 1,
                    plugin_id: "acme.plugin".into(),
                    version: "not-a-version".into(),
                    enabled: true
                })
            )
            .is_err());
        assert!(state.entries["acme.plugin"].pending.is_some());
        state.discard("acme.plugin").unwrap();
        assert!(state.entries["acme.plugin"].pending.is_none());
    }
}
