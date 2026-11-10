//! WSS relay between authenticated hosts and controllers.
//!
//! The relay is single-process by design: connection routing lives in
//! [`Hub`], so every replica would need its own ownership and fencing before
//! running more than one instance. A command is never queued for an offline
//! host; a command that was in flight when its host disconnected becomes
//! `unknown` and is never resent automatically.

use super::{
    canonical_json, db, is_command_id, now_ms, operation_class, role_admits, sha256_hex,
    MAX_COMMAND_ARGS_BYTES, MAX_COMMAND_LIFETIME_MS, MAX_RESULT_BYTES,
};
use crate::{
    api_error::{Authenticated, Kind, RequestId, V3Error},
    AppState,
};
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, State,
    },
    http::{header, HeaderMap},
    response::{IntoResponse, Response},
    Extension, Json,
};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use sqlx::{PgPool, Row};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
    time::Duration,
};
use tokio::sync::mpsc;
use uuid::Uuid;

pub const FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(10);
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(45);
pub const MAX_FRAME_BYTES: usize = 128 * 1024;
pub const OUTBOX_CAPACITY: usize = 64;
pub const MAX_CONTROLLERS_PER_HOST: usize = 32;
pub const MAX_INFLIGHT_PER_SESSION: usize = 8;

#[derive(Clone)]
enum Outbound {
    Text(String),
    Close(&'static str),
}

struct HostLink {
    connection: u64,
    device: Uuid,
    tx: mpsc::Sender<Outbound>,
}

struct SessionLink {
    host: Uuid,
    connection: u64,
    tx: mpsc::Sender<Outbound>,
}

#[derive(Default)]
struct HubState {
    hosts: HashMap<Uuid, HostLink>,
    sessions: HashMap<Uuid, SessionLink>,
}

/// In-memory routing table for live connections.
#[derive(Default)]
pub struct Hub {
    state: Mutex<HubState>,
    next_connection: AtomicU64,
}

impl Hub {
    fn connection_id(&self) -> u64 {
        self.next_connection.fetch_add(1, Ordering::Relaxed) + 1
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HubState> {
        // A poisoned lock only means a panic elsewhere; the routing data stays valid.
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn host_online(&self, host: Uuid) -> bool {
        self.lock().hosts.contains_key(&host)
    }

    fn register_host(&self, host: Uuid, device: Uuid, connection: u64, tx: mpsc::Sender<Outbound>) {
        let previous = self.lock().hosts.insert(
            host,
            HostLink {
                connection,
                device,
                tx,
            },
        );
        if let Some(previous) = previous {
            let _ = previous.tx.try_send(Outbound::Close("replaced"));
        }
    }

    fn unregister_host(&self, host: Uuid, connection: u64) -> bool {
        let mut state = self.lock();
        if state
            .hosts
            .get(&host)
            .is_some_and(|link| link.connection == connection)
        {
            state.hosts.remove(&host);
            true
        } else {
            false
        }
    }

    fn register_session(
        &self,
        session: Uuid,
        host: Uuid,
        connection: u64,
        tx: mpsc::Sender<Outbound>,
    ) {
        self.lock().sessions.insert(
            session,
            SessionLink {
                host,
                connection,
                tx,
            },
        );
    }

    fn unregister_session(&self, session: Uuid, connection: u64) {
        let mut state = self.lock();
        if state
            .sessions
            .get(&session)
            .is_some_and(|link| link.connection == connection)
        {
            state.sessions.remove(&session);
        }
    }

    fn controllers_for(&self, host: Uuid) -> usize {
        self.lock()
            .sessions
            .values()
            .filter(|link| link.host == host)
            .count()
    }

    /// Sends a host-directed frame without blocking. A full or closed outbox fails
    /// instead of queueing, so a slow host cannot accumulate commands.
    pub fn send_host(&self, host: Uuid, frame: &Value) -> bool {
        let state = self.lock();
        match state.hosts.get(&host) {
            Some(link) => link.tx.try_send(Outbound::Text(frame.to_string())).is_ok(),
            None => false,
        }
    }

    pub fn route_to_session(&self, session: Uuid, frame: &Value) -> bool {
        let state = self.lock();
        match state.sessions.get(&session) {
            Some(link) => link.tx.try_send(Outbound::Text(frame.to_string())).is_ok(),
            None => false,
        }
    }

    pub fn notify_grants_changed(&self, host: Uuid) {
        self.send_host(host, &json!({"type": "grants.changed"}));
    }

    pub fn close_session(&self, session: Uuid, reason: &'static str) {
        if let Some(link) = self.lock().sessions.remove(&session) {
            let _ = link.tx.try_send(Outbound::Close(reason));
        }
    }

    pub fn close_device(&self, device: Uuid, reason: &'static str) {
        let mut state = self.lock();
        let hosts: Vec<Uuid> = state
            .hosts
            .iter()
            .filter(|(_, link)| link.device == device)
            .map(|(host, _)| *host)
            .collect();
        for host in hosts {
            if let Some(link) = state.hosts.remove(&host) {
                let _ = link.tx.try_send(Outbound::Close(reason));
            }
        }
    }

    pub fn close_host(&self, host: Uuid, reason: &'static str) {
        let mut state = self.lock();
        if let Some(link) = state.hosts.remove(&host) {
            let _ = link.tx.try_send(Outbound::Close(reason));
        }
        let sessions: Vec<Uuid> = state
            .sessions
            .iter()
            .filter(|(_, link)| link.host == host)
            .map(|(session, _)| *session)
            .collect();
        for session in sessions {
            if let Some(link) = state.sessions.remove(&session) {
                let _ = link.tx.try_send(Outbound::Close(reason));
            }
        }
    }
}

pub async fn upgrade(
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    State(state): State<AppState>,
    Extension(rid): Extension<RequestId>,
) -> Result<Response, V3Error> {
    // Browsers always send Origin; native clients may omit it. A present Origin must be trusted.
    if let Some(origin) = headers.get(header::ORIGIN) {
        let trusted = origin.to_str().ok().is_some_and(|value| {
            state
                .auth
                .options
                .trusted_origins
                .iter()
                .any(|t| t == value)
        });
        if !trusted {
            return Err(V3Error::new(&rid, Kind::Forbidden));
        }
    }
    Ok(ws
        .max_message_size(MAX_FRAME_BYTES)
        .max_frame_size(MAX_FRAME_BYTES)
        .on_upgrade(move |socket| run(state, socket))
        .into_response())
}

#[derive(Clone, Copy)]
enum Role {
    Host,
    Controller,
}

struct Authorized {
    role: Role,
    host: Uuid,
    device: Uuid,
    session: Option<Uuid>,
    grant_revision: i64,
}

/// Consumes the ticket on first use, then re-checks everything it authorizes.
/// A ticket is spent even when validation fails.
async fn authenticate(pool: &PgPool, ticket: &str) -> Result<Authorized, &'static str> {
    let row = sqlx::query("UPDATE cloud_remote_tickets SET consumed_at=now() WHERE token_sha256=$1 AND consumed_at IS NULL AND expires_at>now() RETURNING role,owner_user_id,host_id,device_id,grant_id,grant_revision")
        .bind(super::secret_hash(ticket))
        .fetch_optional(pool)
        .await
        .map_err(|_| "temporaryFailure")?
        .ok_or("ticketInvalid")?;
    let role: String = row.get("role");
    let owner: String = row.get("owner_user_id");
    let host: Uuid = row.get("host_id");
    let device: Uuid = row.get("device_id");
    let host_row = sqlx::query("SELECT status,owner_user_id FROM cloud_remote_hosts WHERE id=$1")
        .bind(host)
        .fetch_optional(pool)
        .await
        .map_err(|_| "temporaryFailure")?
        .ok_or("hostUnavailable")?;
    if host_row.get::<String, _>("status") != "active" {
        return Err("hostUnavailable");
    }
    if role == "host" && host_row.get::<String, _>("owner_user_id") != owner {
        return Err("ticketInvalid");
    }
    let device_row = sqlx::query(
        "SELECT kind,host_id,status FROM cloud_remote_devices WHERE id=$1 AND owner_user_id=$2",
    )
    .bind(device)
    .bind(&owner)
    .fetch_optional(pool)
    .await
    .map_err(|_| "temporaryFailure")?
    .ok_or("deviceRevoked")?;
    if device_row.get::<String, _>("status") != "active" {
        return Err("deviceRevoked");
    }
    match role.as_str() {
        "host" => {
            if device_row.get::<String, _>("kind") != "host"
                || device_row.get::<Option<Uuid>, _>("host_id") != Some(host)
            {
                return Err("ticketInvalid");
            }
            Ok(Authorized {
                role: Role::Host,
                host,
                device,
                session: None,
                grant_revision: 0,
            })
        }
        _ => {
            let grant: Uuid = row.get("grant_id");
            let revision: i64 = row.get("grant_revision");
            if device_row.get::<String, _>("kind") != "controller" {
                return Err("ticketInvalid");
            }
            let current = sqlx::query("SELECT status,revision FROM cloud_remote_grants WHERE id=$1 AND host_id=$2 AND device_id=$3 AND requester_user_id=$4")
                .bind(grant)
                .bind(host)
                .bind(device)
                .bind(&owner)
                .fetch_optional(pool)
                .await
                .map_err(|_| "temporaryFailure")?
                .ok_or("grantNotCurrent")?;
            if current.get::<String, _>("status") != "active"
                || current.get::<i64, _>("revision") != revision
            {
                return Err("grantNotCurrent");
            }
            let session = Uuid::new_v4();
            sqlx::query("INSERT INTO cloud_remote_sessions(id,host_id,grant_id,device_id,grant_revision) VALUES($1,$2,$3,$4,$5)")
                .bind(session)
                .bind(host)
                .bind(grant)
                .bind(device)
                .bind(revision)
                .execute(pool)
                .await
                .map_err(|_| "temporaryFailure")?;
            Ok(Authorized {
                role: Role::Controller,
                host,
                device,
                session: Some(session),
                grant_revision: revision,
            })
        }
    }
}

fn frame(kind: &str, fields: Value) -> String {
    let mut object = match fields {
        Value::Object(map) => map,
        _ => serde_json::Map::new(),
    };
    object.insert("type".into(), Value::String(kind.into()));
    Value::Object(object).to_string()
}

async fn run(state: AppState, socket: WebSocket) {
    let (mut sink, mut stream) = socket.split();
    let first = tokio::time::timeout(FIRST_FRAME_TIMEOUT, stream.next()).await;
    let ticket = match first {
        Ok(Some(Ok(Message::Text(text)))) => serde_json::from_str::<Value>(text.as_str())
            .ok()
            .filter(|v| v["type"] == "auth")
            .and_then(|v| v["ticket"].as_str().map(str::to_owned)),
        _ => None,
    };
    let Some(ticket) = ticket else {
        let _ = sink
            .send(Message::Text(
                frame("error", json!({"code": "authRequired"})).into(),
            ))
            .await;
        let _ = sink.send(Message::Close(None)).await;
        return;
    };
    let authorized = match authenticate(&state.pool, &ticket).await {
        Ok(authorized) => authorized,
        Err(code) => {
            let _ = sink
                .send(Message::Text(frame("error", json!({"code": code})).into()))
                .await;
            let _ = sink.send(Message::Close(None)).await;
            return;
        }
    };
    let hub = &state.remote;
    let connection = hub.connection_id();
    let (tx, mut rx) = mpsc::channel::<Outbound>(OUTBOX_CAPACITY);
    match authorized.role {
        Role::Host => hub.register_host(authorized.host, authorized.device, connection, tx),
        Role::Controller => {
            let session = authorized.session.unwrap_or_else(Uuid::new_v4);
            if hub.controllers_for(authorized.host) >= MAX_CONTROLLERS_PER_HOST {
                let _ = sink
                    .send(Message::Text(
                        frame("error", json!({"code": "capacity"})).into(),
                    ))
                    .await;
                let _ = sink.send(Message::Close(None)).await;
                close_session_row(&state.pool, session).await;
                return;
            }
            hub.register_session(session, authorized.host, connection, tx);
        }
    }
    let ready = match authorized.role {
        Role::Host => frame("ready", json!({"role": "host", "hostId": authorized.host})),
        Role::Controller => frame(
            "ready",
            json!({
                "role": "controller",
                "hostId": authorized.host,
                "sessionId": authorized.session,
                "grantRevision": authorized.grant_revision,
            }),
        ),
    };
    if sink.send(Message::Text(ready.into())).await.is_err() {
        cleanup(&state, &authorized, connection).await;
        return;
    }
    if matches!(authorized.role, Role::Host) {
        // The host resynchronizes persisted grants through the owner API after every connect.
        hub.notify_grants_changed(authorized.host);
    }
    loop {
        tokio::select! {
            biased;
            outbound = rx.recv() => match outbound {
                Some(Outbound::Text(text)) => {
                    if sink.send(Message::Text(text.into())).await.is_err() {
                        break;
                    }
                }
                Some(Outbound::Close(reason)) => {
                    let _ = sink.send(Message::Text(frame("closed", json!({"reason": reason})).into())).await;
                    let _ = sink.send(Message::Close(None)).await;
                    break;
                }
                None => break,
            },
            incoming = tokio::time::timeout(IDLE_TIMEOUT, stream.next()) => match incoming {
                Ok(Some(Ok(Message::Text(text)))) => {
                    let reply = handle_frame(&state, &authorized, text.as_str()).await;
                    if let Some(reply) = reply {
                        if sink.send(Message::Text(reply.into())).await.is_err() {
                            break;
                        }
                    }
                }
                Ok(Some(Ok(Message::Ping(payload)))) => {
                    if sink.send(Message::Pong(payload)).await.is_err() {
                        break;
                    }
                }
                Ok(Some(Ok(Message::Pong(_)))) => {}
                Ok(Some(Ok(Message::Binary(_)))) => {
                    let _ = sink.send(Message::Text(frame("error", json!({"code": "binaryUnsupported"})).into())).await;
                }
                _ => break,
            },
        }
    }
    cleanup(&state, &authorized, connection).await;
}

async fn close_session_row(pool: &PgPool, session: Uuid) {
    let _ = sqlx::query("UPDATE cloud_remote_sessions SET status='closed', closed_at=now() WHERE id=$1 AND status='active'")
        .bind(session)
        .execute(pool)
        .await;
}

/// Releases routing state. Commands in flight on a lost host become `unknown`.
async fn cleanup(state: &AppState, authorized: &Authorized, connection: u64) {
    match authorized.role {
        Role::Host => {
            if state.remote.unregister_host(authorized.host, connection) {
                let lost = sqlx::query("UPDATE cloud_remote_commands SET status='unknown', code='hostDisconnected', updated_at=now() WHERE host_id=$1 AND status IN ('accepted','running') RETURNING command_id,session_id")
                    .bind(authorized.host)
                    .fetch_all(&state.pool)
                    .await
                    .unwrap_or_default();
                for row in lost {
                    let session: Uuid = row.get("session_id");
                    let command: String = row.get("command_id");
                    state.remote.route_to_session(session, &json!({"type": "result", "commandId": command, "status": "unknown", "code": "hostDisconnected"}));
                }
            }
        }
        Role::Controller => {
            if let Some(session) = authorized.session {
                state.remote.unregister_session(session, connection);
                close_session_row(&state.pool, session).await;
            }
        }
    }
}

async fn handle_frame(state: &AppState, authorized: &Authorized, text: &str) -> Option<String> {
    if text.len() > MAX_FRAME_BYTES {
        return Some(frame("error", json!({"code": "frameTooLarge"})));
    }
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return Some(frame("error", json!({"code": "invalidFrame"})));
    };
    let kind = value["type"].as_str().unwrap_or_default();
    if kind == "ping" {
        return Some(frame("pong", json!({})));
    }
    match (authorized.role, kind) {
        (Role::Host, "grant.confirmed") => {
            Some(confirm_grant(state, authorized.host, &value).await)
        }
        (Role::Host, "result") => Some(host_result(state, authorized.host, &value).await),
        (Role::Controller, "command") => Some(controller_command(state, authorized, &value).await),
        _ => Some(frame("error", json!({"code": "unsupportedFrame"}))),
    }
}

/// The host reports the revision it persisted. Only that exact revision becomes
/// operational; a newer edit leaves the grant pending.
async fn confirm_grant(state: &AppState, host: Uuid, value: &Value) -> String {
    let grant = value["grantId"]
        .as_str()
        .and_then(|raw| Uuid::parse_str(raw).ok());
    let revision = value["revision"].as_i64();
    let (Some(grant), Some(revision)) = (grant, revision) else {
        return frame("error", json!({"code": "invalidFrame"}));
    };
    let updated = sqlx::query("UPDATE cloud_remote_grants SET status='active', confirmed_revision=$3, updated_at=now() WHERE id=$1 AND host_id=$2 AND status='pendingHost' AND revision=$3")
        .bind(grant)
        .bind(host)
        .bind(revision)
        .execute(&state.pool)
        .await;
    match updated {
        Ok(result) if result.rows_affected() == 1 => frame(
            "ack",
            json!({"grantId": grant, "revision": revision, "status": "active"}),
        ),
        Ok(_) => frame("error", json!({"code": "conflict"})),
        Err(_) => frame("error", json!({"code": "temporaryFailure"})),
    }
}

/// Allowed transitions for a command receipt. Terminal states never change.
fn transition_allowed(from: &str, to: &str) -> bool {
    match from {
        "accepted" => matches!(
            to,
            "running" | "completed" | "failed" | "cancelled" | "expired" | "unknown"
        ),
        "running" => matches!(to, "completed" | "failed" | "cancelled" | "unknown"),
        "unknown" => matches!(to, "completed" | "failed" | "cancelled"),
        _ => false,
    }
}

async fn host_result(state: &AppState, host: Uuid, value: &Value) -> String {
    let Some(command) = value["commandId"].as_str().filter(|c| is_command_id(c)) else {
        return frame("error", json!({"code": "invalidFrame"}));
    };
    let to = value["status"].as_str().unwrap_or_default();
    if !matches!(
        to,
        "running" | "completed" | "failed" | "cancelled" | "expired" | "unknown"
    ) {
        return frame("error", json!({"code": "invalidFrame"}));
    }
    let code = value["code"]
        .as_str()
        .filter(|c| c.len() <= 64)
        .map(str::to_owned);
    let result = match value.get("result") {
        Some(result) if !result.is_null() => {
            if canonical_json(result).len() > MAX_RESULT_BYTES {
                return frame("error", json!({"code": "resultTooLarge"}));
            }
            Some(result.clone())
        }
        _ => None,
    };
    let Ok(Some(row)) = sqlx::query(
        "SELECT status,session_id FROM cloud_remote_commands WHERE host_id=$1 AND command_id=$2",
    )
    .bind(host)
    .bind(command)
    .fetch_optional(&state.pool)
    .await
    else {
        return frame("error", json!({"code": "unknownCommand"}));
    };
    let from: String = row.get("status");
    let session: Uuid = row.get("session_id");
    if from == to {
        return frame("ack", json!({"commandId": command, "status": to}));
    }
    if !transition_allowed(&from, to) {
        return frame("error", json!({"code": "invalidTransition"}));
    }
    let updated = sqlx::query("UPDATE cloud_remote_commands SET status=$3, code=$4, result=$5, updated_at=now() WHERE host_id=$1 AND command_id=$2 AND status=$6")
        .bind(host)
        .bind(command)
        .bind(to)
        .bind(&code)
        .bind(&result)
        .bind(&from)
        .execute(&state.pool)
        .await;
    if !matches!(updated, Ok(ref r) if r.rows_affected() == 1) {
        return frame("error", json!({"code": "conflict"}));
    }
    state.remote.route_to_session(
        session,
        &json!({
            "type": "result",
            "commandId": command,
            "status": to,
            "code": code,
            "result": result,
        }),
    );
    frame("ack", json!({"commandId": command, "status": to}))
}

struct Receipt {
    status: String,
    session: Uuid,
    digest: String,
}

async fn load_receipt(pool: &PgPool, host: Uuid, command: &str) -> Option<Receipt> {
    sqlx::query("SELECT status,session_id,args_sha256 FROM cloud_remote_commands WHERE host_id=$1 AND command_id=$2")
        .bind(host)
        .bind(command)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .map(|row| Receipt {
            status: row.get("status"),
            session: row.get("session_id"),
            digest: row.get("args_sha256"),
        })
}

/// Replays a stored receipt for the same command id, or refuses a different body.
fn replay(command: &str, receipt: &Receipt, session: Uuid, digest: &str) -> String {
    if receipt.session == session && receipt.digest == digest {
        frame(
            "command.status",
            json!({"commandId": command, "status": receipt.status}),
        )
    } else {
        frame(
            "command.rejected",
            json!({"commandId": command, "code": "conflict"}),
        )
    }
}

fn reject(command: &str, code: &str) -> String {
    frame(
        "command.rejected",
        json!({"commandId": command, "code": code}),
    )
}

async fn controller_command(state: &AppState, authorized: &Authorized, value: &Value) -> String {
    // Expire stale receipts first so a late command never races an old lifetime.
    expire_stale(&state.pool, &state.remote).await;
    let Some(session) = authorized.session else {
        return frame("error", json!({"code": "unsupportedFrame"}));
    };
    let command = value["commandId"].as_str().unwrap_or_default().to_owned();
    if !is_command_id(&command) {
        return frame("error", json!({"code": "invalidFrame"}));
    }
    let operation = value["operation"].as_str().unwrap_or_default().to_owned();
    let args = value.get("args").cloned().unwrap_or_else(|| json!({}));
    let expires_at = value["expiresAt"].as_i64();
    let policy_revision = value["policyRevision"].as_i64();
    if !args.is_object() || canonical_json(&args).len() > MAX_COMMAND_ARGS_BYTES {
        return reject(&command, "invalidArguments");
    }
    let digest =
        sha256_hex(canonical_json(&json!({"operation": operation, "args": args})).as_bytes());
    if let Some(receipt) = load_receipt(&state.pool, authorized.host, &command).await {
        return replay(&command, &receipt, session, &digest);
    }
    let now = now_ms();
    let admission = admit(
        state,
        authorized,
        &operation,
        expires_at,
        policy_revision,
        now,
    )
    .await;
    if let Err(code) = admission {
        insert_receipt(
            &state.pool,
            authorized.host,
            &command,
            session,
            &operation,
            &digest,
            "rejected",
            Some(code),
            expires_at.unwrap_or(now),
        )
        .await;
        return reject(&command, code);
    }
    if !state.remote.host_online(authorized.host) {
        insert_receipt(
            &state.pool,
            authorized.host,
            &command,
            session,
            &operation,
            &digest,
            "rejected",
            Some("offline"),
            expires_at.unwrap_or(now),
        )
        .await;
        return reject(&command, "offline");
    }
    let inserted = sqlx::query("INSERT INTO cloud_remote_commands(host_id,command_id,session_id,operation,args_sha256,status,expires_at) VALUES($1,$2,$3,$4,$5,'accepted',to_timestamp($6::double precision / 1000)) ON CONFLICT (host_id,command_id) DO NOTHING")
        .bind(authorized.host)
        .bind(&command)
        .bind(session)
        .bind(&operation)
        .bind(&digest)
        .bind(expires_at.unwrap_or(now) as f64)
        .execute(&state.pool)
        .await;
    match inserted {
        Ok(result) if result.rows_affected() == 1 => {}
        Ok(_) => {
            return match load_receipt(&state.pool, authorized.host, &command).await {
                Some(receipt) => replay(&command, &receipt, session, &digest),
                None => reject(&command, "temporaryFailure"),
            };
        }
        Err(_) => return reject(&command, "temporaryFailure"),
    }
    // Envelope fields from the v3 contract: protocol version, issue time, expiry and
    // the correlation id that the host echoes in its results.
    let envelope = json!({
        "type": "command",
        "protocolVersion": super::PROTOCOL_VERSION,
        "commandId": command,
        "sessionId": session,
        "policyRevision": authorized.grant_revision,
        "operation": operation,
        "args": args,
        "issuedAt": now,
        "expiresAt": expires_at,
        "requestId": Uuid::new_v4().to_string(),
    });
    if !state.remote.send_host(authorized.host, &envelope) {
        // Nothing reached the host, so the receipt can close as rejected without ambiguity.
        let _ = sqlx::query("UPDATE cloud_remote_commands SET status='rejected', code='offline', updated_at=now() WHERE host_id=$1 AND command_id=$2 AND status='accepted'")
            .bind(authorized.host)
            .bind(&command)
            .execute(&state.pool)
            .await;
        return reject(&command, "offline");
    }
    frame("command.accepted", json!({"commandId": command}))
}

/// Checks the bounded admission rules the relay owns: lifetime, revision, grant
/// state, operation class, and the in-flight limit. The host repeats the
/// permission checks when it executes.
async fn admit(
    state: &AppState,
    authorized: &Authorized,
    operation: &str,
    expires_at: Option<i64>,
    policy_revision: Option<i64>,
    now: i64,
) -> Result<(), &'static str> {
    let Some(expires_at) = expires_at else {
        return Err("expired");
    };
    if expires_at <= now || expires_at > now + MAX_COMMAND_LIFETIME_MS {
        return Err("expired");
    }
    if policy_revision != Some(authorized.grant_revision) {
        return Err("stale");
    }
    if operation_class(operation).is_none() {
        return Err("unknownOperation");
    }
    let session = authorized.session.ok_or("forbidden")?;
    let inflight: i64 = sqlx::query_scalar("SELECT count(*) FROM cloud_remote_commands WHERE session_id=$1 AND status IN ('accepted','running')")
        .bind(session)
        .fetch_one(&state.pool)
        .await
        .map_err(|_| "temporaryFailure")?;
    if inflight as usize >= MAX_INFLIGHT_PER_SESSION {
        return Err("busy");
    }
    let grant = sqlx::query("SELECT g.role,g.policy,g.status,g.revision FROM cloud_remote_grants g JOIN cloud_remote_sessions s ON s.grant_id=g.id WHERE s.id=$1")
        .bind(session)
        .fetch_optional(&state.pool)
        .await
        .map_err(|_| "temporaryFailure")?
        .ok_or("revoked")?;
    if grant.get::<String, _>("status") != "active"
        || grant.get::<i64, _>("revision") != authorized.grant_revision
    {
        return Err("revoked");
    }
    let role: String = grant.get("role");
    let policy: Value = grant.get("policy");
    if !role_admits(&role, &policy, operation) {
        return Err("forbidden");
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn insert_receipt(
    pool: &PgPool,
    host: Uuid,
    command: &str,
    session: Uuid,
    operation: &str,
    digest: &str,
    status: &str,
    code: Option<&str>,
    expires_at_ms: i64,
) {
    let _ = sqlx::query("INSERT INTO cloud_remote_commands(host_id,command_id,session_id,operation,args_sha256,status,code,expires_at) VALUES($1,$2,$3,$4,$5,$6,$7,to_timestamp($8::double precision / 1000)) ON CONFLICT (host_id,command_id) DO NOTHING")
        .bind(host)
        .bind(command)
        .bind(session)
        .bind(operation)
        .bind(digest)
        .bind(status)
        .bind(code)
        .bind(expires_at_ms as f64)
        .execute(pool)
        .await;
}

/// Owners of a team, and the controller's own account, may read a receipt.
pub async fn command_status(
    Authenticated(user): Authenticated,
    Extension(rid): Extension<RequestId>,
    State(state): State<AppState>,
    Path((host_id, command_id)): Path<(String, String)>,
) -> Result<Json<Value>, V3Error> {
    let host = super::parse_uuid(&rid, &host_id)?;
    if !is_command_id(&command_id) {
        return Err(V3Error::new(&rid, Kind::InvalidRequest));
    }
    let row = sqlx::query("SELECT c.status,c.code,c.result,c.operation,c.expires_at,h.owner_user_id AS host_owner,d.owner_user_id AS controller_owner FROM cloud_remote_commands c JOIN cloud_remote_hosts h ON h.id=c.host_id JOIN cloud_remote_sessions s ON s.id=c.session_id JOIN cloud_remote_devices d ON d.id=s.device_id WHERE c.host_id=$1 AND c.command_id=$2")
        .bind(host)
        .bind(&command_id)
        .fetch_optional(&state.pool)
        .await
        .map_err(db(&rid))?
        .ok_or(V3Error::new(&rid, Kind::NotFound))?;
    let owner: String = user.user.id.clone();
    if row.get::<String, _>("host_owner") != owner
        && row.get::<String, _>("controller_owner") != owner
    {
        return Err(V3Error::new(&rid, Kind::NotFound));
    }
    Ok(Json(json!({
        "commandId": command_id,
        "operation": row.get::<String, _>("operation"),
        "status": row.get::<String, _>("status"),
        "code": row.get::<Option<String>, _>("code"),
        "result": row.get::<Option<Value>, _>("result"),
    })))
}

/// Marks accepted commands whose lifetime ended as `expired`. Running commands are
/// left alone: the host may still be executing them, so they stay in flight.
pub async fn expire_stale(pool: &PgPool, hub: &Hub) -> u64 {
    let rows = sqlx::query("UPDATE cloud_remote_commands SET status='expired', updated_at=now() WHERE status='accepted' AND expires_at < now() RETURNING command_id,session_id")
        .fetch_all(pool)
        .await
        .unwrap_or_default();
    let count = rows.len() as u64;
    for row in rows {
        let session: Uuid = row.get("session_id");
        let command: String = row.get("command_id");
        hub.route_to_session(
            session,
            &json!({"type": "result", "commandId": command, "status": "expired"}),
        );
    }
    count
}
