//! Real PostgreSQL and WebSocket coverage for the v3 product surface: catalog
//! projection, device proof, invitations, grants and the relay lifecycle.
use super::*;
use crate::remote::proof_message;
use axum::{
    body::Body,
    http::{header, HeaderMap, Request},
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use better_auth::email_password::{EmailPasswordService, SignUpInput};
use ed25519_dalek::{Signer, SigningKey};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::time::Duration;
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};
use tower::ServiceExt;

const ORIGIN: &str = "https://cloud.example.test";
const PASSWORD: &str = "test password with enough characters";

struct User {
    id: String,
    cookie: String,
}

struct Fixture {
    router: Router,
    pool: PgPool,
    users: Vec<User>,
    ws_url: String,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

async fn fixture(users_wanted: usize) -> Fixture {
    let database =
        std::env::var("CLOUD_TEST_DATABASE_URL").expect("use a disposable PostgreSQL database");
    let adapter = Arc::new(SqlxDbAdapter::postgres(&database).await.unwrap());
    let secondary = Arc::new(SqlxSecondaryStorage::postgres(&database).await.unwrap());
    let context = AuthContext::new(
        AuthOptions {
            secret: "test-only-secret-32-bytes-long!!".into(),
            base_url: Some(BaseUrl::Static(ORIGIN.into())),
            trusted_origins: vec![ORIGIN.into()],
            ..Default::default()
        },
        adapter.clone(),
        Some(secondary.clone()),
        vec![Arc::new(better_auth::UsernamePlugin)],
    )
    .unwrap();
    adapter
        .apply_migrations(&context.migration_plan(), SqlDialect::Postgres)
        .await
        .unwrap();
    secondary.migrate().await.unwrap();
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&database)
        .await
        .unwrap();
    readiness::MIGRATOR.run(&pool).await.unwrap();
    let mut users = Vec::new();
    for _ in 0..users_wanted {
        let user = EmailPasswordService::new(context.clone())
            .unwrap()
            .sign_up(
                SignUpInput {
                    email: format!("{}@example.test", uuid::Uuid::new_v4()),
                    name: "V3 Test".into(),
                    password: PASSWORD.into(),
                },
                true,
            )
            .await
            .unwrap();
        users.push(User {
            id: user.user.id,
            cookie: format!("{}={}", user.cookie.name, user.cookie.value),
        });
    }
    let router = app(
        context.clone(),
        pool.clone(),
        AuthRouter::new(context).unwrap(),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ws_url = format!(
        "ws://{}/api/v3/remote/socket",
        listener.local_addr().unwrap()
    );
    let served = router.clone();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            served.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap();
    });
    Fixture {
        router,
        pool,
        users,
        ws_url,
        server,
    }
}

async fn call(
    f: &Fixture,
    user: Option<usize>,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> (StatusCode, HeaderMap, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(user) = user {
        request = request
            .header(header::COOKIE, &f.users[user].cookie)
            .header(header::ORIGIN, ORIGIN);
    }
    let body = body.map(|b| b.to_string()).unwrap_or_default();
    let response = f
        .router
        .clone()
        .oneshot(request.body(Body::from(body)).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, headers, value)
}

fn key() -> SigningKey {
    SigningKey::from_bytes(&rand::random::<[u8; 32]>())
}

fn public(key: &SigningKey) -> String {
    URL_SAFE_NO_PAD.encode(key.verifying_key().to_bytes())
}

fn sign(key: &SigningKey, purpose: &str, challenge: &Value) -> String {
    let nonce = URL_SAFE_NO_PAD
        .decode(challenge["nonce"].as_str().unwrap())
        .unwrap();
    let digest = challenge["bindingSha256"].as_str().unwrap();
    let message = proof_message(purpose, &nonce, digest);
    URL_SAFE_NO_PAD.encode(key.sign(&message).to_bytes())
}

/// Enrolls a device after the caller proves possession of its key.
async fn enroll(
    f: &Fixture,
    user: usize,
    label: &str,
    kind: &str,
    host: Option<&str>,
    key: &SigningKey,
) -> Value {
    let mut binding = json!({"label": label, "kind": kind, "publicKey": public(key)});
    if let Some(host) = host {
        binding["hostId"] = json!(host);
    }
    let (status, _, challenge) = call(
        f,
        Some(user),
        "POST",
        "/api/v3/remote/challenges",
        Some(json!({"purpose": "enroll", "binding": binding.clone()})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{challenge}");
    let signature = sign(key, "enroll", &challenge);
    let (status, _, device) = call(
        f,
        Some(user),
        "POST",
        "/api/v3/remote/devices",
        Some(json!({"challengeId": challenge["id"], "binding": binding, "signature": signature})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{device}");
    device
}

/// Obtains a connection ticket for one exact (host, device, role) binding.
async fn ticket(
    f: &Fixture,
    user: usize,
    host: &str,
    device: &str,
    role: &str,
    key: &SigningKey,
) -> (StatusCode, Value) {
    let binding = json!({"hostId": host, "deviceId": device, "role": role});
    let (status, _, challenge) = call(
        f,
        Some(user),
        "POST",
        "/api/v3/remote/challenges",
        Some(json!({"purpose": "ticket", "binding": binding.clone()})),
    )
    .await;
    if status != StatusCode::CREATED {
        return (status, challenge);
    }
    let signature = sign(key, "ticket", &challenge);
    let (status, _, body) = call(
        f,
        Some(user),
        "POST",
        "/api/v3/remote/tickets",
        Some(json!({"challengeId": challenge["id"], "binding": binding, "signature": signature})),
    )
    .await;
    (status, body)
}

async fn seed_published(pool: &PgPool, owner: &str, kind: &str, title: &str) -> uuid::Uuid {
    let id = uuid::Uuid::new_v4();
    let slug = format!("s{}", id.simple());
    sqlx::query(
        "INSERT INTO cloud_library_documents(id,owner_user_id,kind,slug) VALUES($1,$2,$3,$4)",
    )
    .bind(id)
    .bind(owner)
    .bind(kind)
    .bind(&slug)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO cloud_library_versions(document_id,revision,title,document,sha256,status) VALUES($1,1,$2,$3,$4,'published')")
        .bind(id)
        .bind(title)
        .bind(json!({"title": title, "description": format!("About {title}")}))
        .bind("a".repeat(64))
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("UPDATE cloud_library_documents SET published_revision=1 WHERE id=$1")
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
    id
}

async fn next_frame(ws: &mut WsStream, kind: &str) -> Value {
    let mut skipped = Vec::new();
    loop {
        let message = tokio::time::timeout(Duration::from_secs(10), ws.next())
            .await
            .unwrap_or_else(|_| panic!("timed out waiting for {kind}; skipped frames: {skipped:?}"))
            .expect("socket closed")
            .expect("socket error");
        if let Message::Text(text) = message {
            let value: Value = serde_json::from_str(text.as_str()).unwrap();
            if value["type"] == kind {
                return value;
            }
            skipped.push(text.to_string());
        }
    }
}

type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn connect(f: &Fixture, ticket: &str) -> (WsStream, Value) {
    let (mut ws, _) = tokio_tungstenite::connect_async(&f.ws_url).await.unwrap();
    ws.send(Message::Text(
        json!({"type": "auth", "ticket": ticket}).to_string().into(),
    ))
    .await
    .unwrap();
    let ready = next_frame(&mut ws, "ready").await;
    (ws, ready)
}

async fn send(ws: &mut WsStream, value: Value) {
    ws.send(Message::Text(value.to_string().into()))
        .await
        .unwrap();
}

fn millis_from_now(ms: i64) -> i64 {
    chrono::Utc::now().timestamp_millis() + ms
}

#[tokio::test]
#[ignore = "requires a dedicated PostgreSQL database in CLOUD_TEST_DATABASE_URL"]
async fn errors_use_the_envelope_and_capabilities_stay_additive() {
    let f = fixture(1).await;
    let (status, headers, capabilities) = call(&f, None, "GET", "/api/v2/capabilities", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        capabilities["apiVersion"], 2,
        "v2 fields keep their meaning"
    );
    assert_eq!(capabilities["presentationCatalog"]["apiVersion"], 3);
    assert_eq!(capabilities["remoteControl"]["supported"], true);
    assert_eq!(capabilities["integrations"]["providers"], 0);
    assert!(
        headers.get("x-request-id").is_none(),
        "v2 responses are unchanged"
    );

    let (status, headers, body) = call(
        &f,
        None,
        "GET",
        "/api/v3/catalog/items/plugin/absent.plugin",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], "notFound");
    assert_eq!(body["retryable"], false);
    let request_id = headers.get("x-request-id").unwrap().to_str().unwrap();
    assert_eq!(
        body["requestId"], request_id,
        "the header and envelope share one id"
    );

    let (status, _, body) = call(&f, None, "GET", "/api/v3/library/items", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["code"], "unauthorized");
    assert!(body["requestId"].as_str().is_some_and(|id| !id.is_empty()));
}

#[tokio::test]
#[ignore = "requires a dedicated PostgreSQL database in CLOUD_TEST_DATABASE_URL"]
async fn catalog_lists_published_documents_with_cursors_bound_to_their_query() {
    let f = fixture(1).await;
    let owner = f.users[0].id.clone();
    // A per-run token isolates this test from documents other tests leave in the shared database.
    let token = uuid::Uuid::new_v4().simple().to_string();
    let alpha = seed_published(
        &f.pool,
        &owner,
        "template",
        &format!("{token} alpha gift rule"),
    )
    .await;
    let beta = seed_published(
        &f.pool,
        &owner,
        "template",
        &format!("{token} beta gift rule"),
    )
    .await;
    let profile = seed_published(
        &f.pool,
        &owner,
        "profile",
        &format!("{token} gamma profile"),
    )
    .await;
    // Unpublished revisions never appear in the public catalog.
    let draft = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO cloud_library_documents(id,owner_user_id,kind,slug) VALUES($1,$2,'template',$3)")
        .bind(draft)
        .bind(&owner)
        .bind(format!("d{}", draft.simple()))
        .execute(&f.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO cloud_library_versions(document_id,revision,title,document,sha256,status) VALUES($1,1,$2,'{}'::jsonb,$3,'draft')")
        .bind(draft)
        .bind(format!("{token} draft rule"))
        .bind("b".repeat(64))
        .execute(&f.pool)
        .await
        .unwrap();

    let search = format!("/api/v3/catalog/items?kind=template&q={token}&limit=1");
    let (status, _, first) = call(&f, None, "GET", &search, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(first["items"].as_array().unwrap().len(), 1);
    let cursor = first["nextCursor"]
        .as_str()
        .expect("a second page exists")
        .to_owned();
    let (status, _, second) =
        call(&f, None, "GET", &format!("{search}&cursor={cursor}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(second["items"].as_array().unwrap().len(), 1);
    assert!(
        second["nextCursor"].is_null(),
        "exactly two published templates match this token"
    );

    let mut seen: Vec<String> = Vec::new();
    for item in first["items"]
        .as_array()
        .unwrap()
        .iter()
        .chain(second["items"].as_array().unwrap())
    {
        seen.push(item["identity"]["id"].as_str().unwrap().to_owned());
    }
    seen.sort();
    let mut expected = vec![alpha.to_string(), beta.to_string()];
    expected.sort();
    assert_eq!(
        seen, expected,
        "the draft never appears and both published templates do"
    );

    let (status, _, body) = call(
        &f,
        None,
        "GET",
        &format!("/api/v3/catalog/items?kind=template&q=beta&limit=1&cursor={cursor}"),
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "a cursor cannot be reused for another query: {body}"
    );
    assert_eq!(body["code"], "invalidRequest");

    let (_, _, profiles) = call(
        &f,
        None,
        "GET",
        &format!("/api/v3/catalog/items?kind=profile&q={token}"),
        None,
    )
    .await;
    assert_eq!(profiles["items"].as_array().unwrap().len(), 1);
    assert_eq!(profiles["items"][0]["identity"]["id"], profile.to_string());
    assert_eq!(
        profiles["items"][0]["presentation"]["summary"],
        format!("About {token} gamma profile")
    );

    let (status, _, _) = call(&f, None, "GET", "/api/v3/catalog/items?bogus=1", None).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "unknown query keys are refused"
    );

    let (_, _, owned) = call(&f, Some(0), "GET", "/api/v3/library/items?limit=50", None).await;
    assert_eq!(
        owned["items"].as_array().unwrap().len(),
        4,
        "owners see drafts too"
    );
}

#[tokio::test]
#[ignore = "requires a dedicated PostgreSQL database in CLOUD_TEST_DATABASE_URL"]
async fn device_proof_and_invitations_are_single_use_and_bound() {
    let f = fixture(2).await;
    let host_owner = 0;
    let guest = 1;
    let (status, _, host) = call(
        &f,
        Some(host_owner),
        "POST",
        "/api/v3/remote/hosts",
        Some(json!({"name": "Desk PC", "protocolVersion": 1})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{host}");
    let host_id = host["id"].as_str().unwrap().to_owned();
    assert_eq!(
        call(
            &f,
            Some(host_owner),
            "POST",
            "/api/v3/remote/hosts",
            Some(json!({"name": "X", "protocolVersion": 9}))
        )
        .await
        .0,
        StatusCode::NOT_IMPLEMENTED
    );

    // A signature from a different key must not enroll the device.
    let real = key();
    let forged = key();
    let binding = json!({"label": "Controller", "kind": "controller", "publicKey": public(&real)});
    let (_, _, challenge) = call(
        &f,
        Some(guest),
        "POST",
        "/api/v3/remote/challenges",
        Some(json!({"purpose": "enroll", "binding": binding.clone()})),
    )
    .await;
    let bad = sign(&forged, "enroll", &challenge);
    let (status, _, body) = call(
        &f,
        Some(guest),
        "POST",
        "/api/v3/remote/devices",
        Some(json!({"challengeId": challenge["id"], "binding": binding.clone(), "signature": bad})),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    // A failed proof rolls back and leaves the challenge unconsumed, so the right key still enrolls once.
    let good = sign(&real, "enroll", &challenge);
    let (status, _, _) = call(&f, Some(guest), "POST", "/api/v3/remote/devices", Some(json!({"challengeId": challenge["id"], "binding": binding.clone(), "signature": good.clone()}))).await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _, _) = call(
        &f,
        Some(guest),
        "POST",
        "/api/v3/remote/devices",
        Some(json!({"challengeId": challenge["id"], "binding": binding, "signature": good})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::GONE,
        "a spent challenge cannot enroll twice"
    );

    let controller = enroll(&f, guest, "Phone", "controller", None, &key()).await;
    let host_key = key();
    let host_device = enroll(
        &f,
        host_owner,
        "Desk host",
        "host",
        Some(&host_id),
        &host_key,
    )
    .await;
    assert_eq!(host_device["kind"], "host");

    // Invitation: the secret is returned once, and only one redemption can consume it.
    let (status, _, invitation) = call(
        &f,
        Some(host_owner),
        "POST",
        &format!("/api/v3/remote/hosts/{host_id}/invitations"),
        Some(json!({"requestedRole": "moderator", "ttlSeconds": 600})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{invitation}");
    let token = invitation["token"].as_str().unwrap().to_owned();
    let (_, _, listed) = call(
        &f,
        Some(host_owner),
        "GET",
        &format!("/api/v3/remote/hosts/{host_id}/invitations"),
        None,
    )
    .await;
    assert!(
        listed["items"][0].get("token").is_none(),
        "listing never reveals secrets"
    );
    let (status, _, redeemed) = call(
        &f,
        Some(guest),
        "POST",
        "/api/v3/remote/invitations/redeem",
        Some(json!({"token": token, "deviceId": controller["id"]})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{redeemed}");
    assert_eq!(redeemed["status"], "pending", "redeeming never approves");
    let (status, _, _) = call(
        &f,
        Some(guest),
        "POST",
        "/api/v3/remote/invitations/redeem",
        Some(json!({"token": token, "deviceId": controller["id"]})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "single use");

    // Unknown tokens are reported as not found, never as a different state.
    let (status, _, _) = call(
        &f,
        Some(host_owner),
        "POST",
        &format!("/api/v3/remote/hosts/{host_id}/invitations"),
        Some(json!({"requestedRole": "observer"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _, body) = call(
        &f,
        Some(guest),
        "POST",
        "/api/v3/remote/invitations/redeem",
        Some(json!({"token": "not-a-token", "deviceId": controller["id"]})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

    // Another account cannot read this team's requests or grants.
    let (status, _, _) = call(
        &f,
        Some(guest),
        "GET",
        &format!("/api/v3/remote/hosts/{host_id}/grants"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Approving creates a grant that is not operational until the host confirms it.
    let (_, _, requests) = call(
        &f,
        Some(host_owner),
        "GET",
        &format!("/api/v3/remote/hosts/{host_id}/requests"),
        None,
    )
    .await;
    let request_id = requests["items"][0]["id"].as_str().unwrap().to_owned();
    let (status, _, decided) = call(
        &f,
        Some(host_owner),
        "POST",
        &format!("/api/v3/remote/hosts/{host_id}/requests/{request_id}/decision"),
        Some(json!({"decision": "approve"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{decided}");
    assert_eq!(decided["grant"]["status"], "pendingHost");
    let (status, _, body) = call(&f, Some(guest), "POST", "/api/v3/remote/tickets", None).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "tickets need a proof, not an empty body: {body}"
    );
    let (status, body) = ticket(
        &f,
        guest,
        &host_id,
        controller["id"].as_str().unwrap(),
        "controller",
        &key(),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a controller ticket needs an active grant and the matching key: {body}"
    );
}

#[tokio::test]
#[ignore = "requires a dedicated PostgreSQL database in CLOUD_TEST_DATABASE_URL"]
async fn relay_routes_commands_through_confirmed_grants_and_closes_on_revocation() {
    let f = fixture(2).await;
    let (host_owner, guest) = (0, 1);
    let (_, _, host) = call(
        &f,
        Some(host_owner),
        "POST",
        "/api/v3/remote/hosts",
        Some(json!({"name": "Desk PC", "protocolVersion": 1})),
    )
    .await;
    let host_id = host["id"].as_str().unwrap().to_owned();
    let host_key = key();
    let host_device = enroll(
        &f,
        host_owner,
        "Desk host",
        "host",
        Some(&host_id),
        &host_key,
    )
    .await;
    let host_device_id = host_device["id"].as_str().unwrap().to_owned();
    let controller_key = key();
    let controller = enroll(&f, guest, "Phone", "controller", None, &controller_key).await;
    let controller_id = controller["id"].as_str().unwrap().to_owned();

    let (_, _, invitation) = call(
        &f,
        Some(host_owner),
        "POST",
        &format!("/api/v3/remote/hosts/{host_id}/invitations"),
        Some(json!({"requestedRole": "moderator"})),
    )
    .await;
    let token = invitation["token"].as_str().unwrap().to_owned();
    call(
        &f,
        Some(guest),
        "POST",
        "/api/v3/remote/invitations/redeem",
        Some(json!({"token": token, "deviceId": controller_id})),
    )
    .await;
    let (_, _, requests) = call(
        &f,
        Some(host_owner),
        "GET",
        &format!("/api/v3/remote/hosts/{host_id}/requests"),
        None,
    )
    .await;
    let request_id = requests["items"][0]["id"].as_str().unwrap().to_owned();
    let (_, _, decided) = call(
        &f,
        Some(host_owner),
        "POST",
        &format!("/api/v3/remote/hosts/{host_id}/requests/{request_id}/decision"),
        Some(json!({"decision": "approve"})),
    )
    .await;
    let grant_id = decided["grant"]["id"].as_str().unwrap().to_owned();

    // The host connects with its own ticket and confirms the exact revision it persisted.
    let (status, host_ticket) =
        ticket(&f, host_owner, &host_id, &host_device_id, "host", &host_key).await;
    assert_eq!(status, StatusCode::CREATED, "{host_ticket}");
    let (mut host_ws, ready) = connect(&f, host_ticket["ticket"].as_str().unwrap()).await;
    assert_eq!(ready["role"], "host");
    let (mut replay_ws, _) = tokio_tungstenite::connect_async(&f.ws_url).await.unwrap();
    replay_ws
        .send(Message::Text(
            json!({"type": "auth", "ticket": host_ticket["ticket"]})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    let refused = next_frame(&mut replay_ws, "error").await;
    assert_eq!(
        refused["code"], "ticketInvalid",
        "a spent ticket never reaches ready"
    );
    send(
        &mut host_ws,
        json!({"type": "grant.confirmed", "grantId": grant_id, "revision": 1}),
    )
    .await;
    next_frame(&mut host_ws, "ack").await;

    // The approved controller now opens a session and sends an admitted command.
    let (status, control_ticket) = ticket(
        &f,
        guest,
        &host_id,
        &controller_id,
        "controller",
        &controller_key,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{control_ticket}");
    let (mut control_ws, ready) = connect(&f, control_ticket["ticket"].as_str().unwrap()).await;
    assert_eq!(ready["role"], "controller");
    assert_eq!(ready["grantRevision"], 1);
    let command = json!({"type": "command", "commandId": "cmd-00000001", "operation": "points.adjust", "args": {"viewer": "v1", "delta": 5}, "policyRevision": 1, "expiresAt": millis_from_now(30_000)});
    send(&mut control_ws, command.clone()).await;
    assert_eq!(
        next_frame(&mut control_ws, "command.accepted").await["commandId"],
        "cmd-00000001"
    );
    let forwarded = next_frame(&mut host_ws, "command").await;
    assert_eq!(forwarded["operation"], "points.adjust");
    assert_eq!(forwarded["args"]["delta"], 5);
    send(
        &mut host_ws,
        json!({"type": "result", "commandId": "cmd-00000001", "status": "running"}),
    )
    .await;
    assert_eq!(
        next_frame(&mut control_ws, "result").await["status"],
        "running"
    );
    send(&mut host_ws, json!({"type": "result", "commandId": "cmd-00000001", "status": "completed", "result": {"ok": true}})).await;
    let done = next_frame(&mut control_ws, "result").await;
    assert_eq!(done["status"], "completed");
    assert_eq!(done["result"]["ok"], true);

    // The same id replays its receipt; the same id with another body is refused.
    send(&mut control_ws, command.clone()).await;
    assert_eq!(
        next_frame(&mut control_ws, "command.status").await["status"],
        "completed"
    );
    let mut other = command.clone();
    other["args"] = json!({"viewer": "v2", "delta": 5});
    send(&mut control_ws, other).await;
    assert_eq!(
        next_frame(&mut control_ws, "command.rejected").await["code"],
        "conflict"
    );

    // Stale revisions and expired lifetimes are refused before reaching the host.
    send(&mut control_ws, json!({"type": "command", "commandId": "cmd-00000002", "operation": "live.status", "args": {}, "policyRevision": 9, "expiresAt": millis_from_now(30_000)})).await;
    assert_eq!(
        next_frame(&mut control_ws, "command.rejected").await["code"],
        "stale"
    );
    send(&mut control_ws, json!({"type": "command", "commandId": "cmd-00000003", "operation": "live.status", "args": {}, "policyRevision": 1, "expiresAt": millis_from_now(-1_000)})).await;
    assert_eq!(
        next_frame(&mut control_ws, "command.rejected").await["code"],
        "expired"
    );

    // Editing the grant closes the open session and makes the new revision wait for the host.
    let (status, _, edited) = call(
        &f,
        Some(host_owner),
        "PATCH",
        &format!("/api/v3/remote/hosts/{host_id}/grants/{grant_id}"),
        Some(json!({"role": "observer", "expectedRevision": 1})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{edited}");
    assert_eq!(edited["status"], "pendingHost");
    assert_eq!(
        next_frame(&mut control_ws, "closed").await["reason"],
        "grantChanged"
    );
    send(
        &mut host_ws,
        json!({"type": "grant.confirmed", "grantId": grant_id, "revision": 2}),
    )
    .await;
    next_frame(&mut host_ws, "ack").await;
    let (status, observer_ticket) = ticket(
        &f,
        guest,
        &host_id,
        &controller_id,
        "controller",
        &controller_key,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{observer_ticket}");
    let (mut observer_ws, ready) = connect(&f, observer_ticket["ticket"].as_str().unwrap()).await;
    assert_eq!(ready["grantRevision"], 2);
    send(&mut observer_ws, json!({"type": "command", "commandId": "cmd-00000004", "operation": "points.adjust", "args": {"viewer": "v1", "delta": 1}, "policyRevision": 2, "expiresAt": millis_from_now(30_000)})).await;
    assert_eq!(
        next_frame(&mut observer_ws, "command.rejected").await["code"],
        "forbidden",
        "an observer cannot adjust points"
    );
    send(&mut observer_ws, json!({"type": "command", "commandId": "cmd-00000005", "operation": "live.status", "args": {}, "policyRevision": 2, "expiresAt": millis_from_now(30_000)})).await;
    assert_eq!(
        next_frame(&mut observer_ws, "command.accepted").await["commandId"],
        "cmd-00000005"
    );
    next_frame(&mut host_ws, "command").await;

    // Revoking closes every link. The host loses its in-flight command as unknown, never resent.
    let (status, _, revoked) = call(
        &f,
        Some(host_owner),
        "POST",
        &format!("/api/v3/remote/hosts/{host_id}/grants/{grant_id}/revoke"),
        Some(json!({"expectedRevision": 2})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{revoked}");
    assert_eq!(
        next_frame(&mut observer_ws, "closed").await["reason"],
        "grantRevoked"
    );
    let (status, _, _) = call(
        &f,
        Some(host_owner),
        "POST",
        &format!("/api/v3/remote/hosts/{host_id}/grants/{grant_id}/revoke"),
        Some(json!({"expectedRevision": 99})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "revoking an already revoked grant is idempotent"
    );

    let (status, _) = ticket(
        &f,
        guest,
        &host_id,
        &controller_id,
        "controller",
        &controller_key,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a revoked grant issues no new tickets"
    );

    drop(host_ws);
    // The server handles the disconnect asynchronously; poll the receipt until it settles.
    let mut status_body = Value::Null;
    for _ in 0..100 {
        let (_, _, body) = call(
            &f,
            Some(guest),
            "GET",
            &format!("/api/v3/remote/hosts/{host_id}/commands/cmd-00000005"),
            None,
        )
        .await;
        status_body = body;
        if status_body["status"] == "unknown" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(status_body["status"], "unknown", "{status_body}");
    assert_eq!(status_body["code"], "hostDisconnected");
}

#[tokio::test]
#[ignore = "requires a dedicated PostgreSQL database in CLOUD_TEST_DATABASE_URL"]
async fn socket_refuses_untrusted_origins_and_integrations_publish_no_provider() {
    let f = fixture(1).await;
    let addr = f
        .ws_url
        .trim_start_matches("ws://")
        .split('/')
        .next()
        .unwrap()
        .to_owned();
    let mut request = format!("ws://{addr}/api/v3/remote/socket")
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("origin", "https://evil.example".parse().unwrap());
    let error = tokio_tungstenite::connect_async(request)
        .await
        .expect_err("untrusted origin");
    assert!(error.to_string().contains("403"), "{error}");

    let (status, _, providers) =
        call(&f, None, "GET", "/api/v3/integrations/providers", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(providers["providers"].as_array().unwrap().len(), 0);
    let (status, _, body) = call(
        &f,
        Some(0),
        "POST",
        "/api/v3/integrations/obs/connect",
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
    assert_eq!(body["code"], "unsupported");
}

#[tokio::test]
#[ignore = "requires a dedicated PostgreSQL database in CLOUD_TEST_DATABASE_URL"]
async fn expiry_sweep_expires_accepted_receipts_and_keeps_running_ones_in_flight() {
    let f = fixture(1).await;
    let owner = f.users[0].id.clone();
    let host = uuid::Uuid::new_v4();
    let host_device = uuid::Uuid::new_v4();
    let controller = uuid::Uuid::new_v4();
    let grant = uuid::Uuid::new_v4();
    let session = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO cloud_remote_hosts(id,owner_user_id,name,protocol_version) VALUES($1,$2,'Sweep PC',1)")
        .bind(host)
        .bind(&owner)
        .execute(&f.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO cloud_remote_devices(id,owner_user_id,label,kind,host_id,public_key) VALUES($1,$2,'host','host',$3,$4)")
        .bind(host_device)
        .bind(&owner)
        .bind(host)
        .bind(rand::random::<[u8; 32]>().to_vec())
        .execute(&f.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO cloud_remote_devices(id,owner_user_id,label,kind,public_key) VALUES($1,$2,'phone','controller',$3)")
        .bind(controller)
        .bind(&owner)
        .bind(rand::random::<[u8; 32]>().to_vec())
        .execute(&f.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO cloud_remote_grants(id,host_id,device_id,requester_user_id,role,policy,status,revision,confirmed_revision) VALUES($1,$2,$3,$4,'observer','{}','active',1,1)")
        .bind(grant)
        .bind(host)
        .bind(controller)
        .bind(&owner)
        .execute(&f.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO cloud_remote_sessions(id,host_id,grant_id,device_id,grant_revision) VALUES($1,$2,$3,$4,1)")
        .bind(session)
        .bind(host)
        .bind(grant)
        .bind(controller)
        .execute(&f.pool)
        .await
        .unwrap();
    let digest = "c".repeat(64);
    for (command, status) in [("sweep-accepted", "accepted"), ("sweep-running", "running")] {
        sqlx::query("INSERT INTO cloud_remote_commands(host_id,command_id,session_id,operation,args_sha256,status,expires_at) VALUES($1,$2,$3,'live.status',$4,$5,now() - interval '1 second')")
            .bind(host)
            .bind(command)
            .bind(session)
            .bind(&digest)
            .bind(status)
            .execute(&f.pool)
            .await
            .unwrap();
    }
    let hub = crate::remote::relay::Hub::default();
    crate::remote::relay::expire_stale(&f.pool, &hub).await;
    let state_of = |command: &'static str| {
        let pool = f.pool.clone();
        async move {
            sqlx::query_scalar::<_, String>(
                "SELECT status FROM cloud_remote_commands WHERE host_id=$1 AND command_id=$2",
            )
            .bind(host)
            .bind(command)
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };
    assert_eq!(state_of("sweep-accepted").await, "expired");
    assert_eq!(
        state_of("sweep-running").await,
        "running",
        "the host may still be executing it"
    );
}

fn tiny_png(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    bytes.extend_from_slice(&13u32.to_be_bytes());
    bytes.extend_from_slice(b"IHDR");
    bytes.extend_from_slice(&width.to_be_bytes());
    bytes.extend_from_slice(&height.to_be_bytes());
    bytes.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
    bytes
}

async fn call_raw(
    f: &Fixture,
    user: usize,
    method: &str,
    path: &str,
    body: Vec<u8>,
    content_type: &str,
) -> (StatusCode, HeaderMap, Vec<u8>) {
    let request = Request::builder()
        .method(method)
        .uri(path)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::COOKIE, &f.users[user].cookie)
        .header(header::ORIGIN, ORIGIN)
        .body(Body::from(body))
        .unwrap();
    let response = f.router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap()
        .to_vec();
    (status, headers, bytes)
}

#[tokio::test]
#[ignore = "requires a dedicated PostgreSQL database in CLOUD_TEST_DATABASE_URL"]
async fn assets_overlays_and_packages_are_owned_validated_and_preflighted() {
    let f = fixture(2).await;
    let (owner, other) = (0, 1);

    // Assets: only verified rasters are stored, duplicates return the same id, reads stay with the owner.
    let icon = tiny_png(64, 64);
    let (status, _, body) = call_raw(
        &f,
        owner,
        "POST",
        "/api/v3/assets",
        icon.clone(),
        "image/png",
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&body)
    );
    let uploaded: Value = serde_json::from_slice(&body).unwrap();
    let asset = uploaded["id"].as_str().unwrap().to_owned();
    assert_eq!(uploaded["mime"], "image/png");
    let (status, _, again) = call_raw(&f, owner, "POST", "/api/v3/assets", icon, "image/png").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        serde_json::from_slice::<Value>(&again).unwrap()["id"],
        asset.as_str()
    );
    let (status, _, _) = call_raw(
        &f,
        owner,
        "POST",
        "/api/v3/assets",
        b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>".to_vec(),
        "image/svg+xml",
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "SVG is never admitted");
    let (status, headers, bytes) = call_raw(
        &f,
        owner,
        "GET",
        &format!("/api/v3/assets/{asset}"),
        Vec::new(),
        "application/octet-stream",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers.get("content-type").unwrap(), "image/png");
    assert_eq!(headers.get("x-content-type-options").unwrap(), "nosniff");
    assert_eq!(bytes.len(), tiny_png(64, 64).len());
    let (status, _, _) = call_raw(
        &f,
        other,
        "GET",
        &format!("/api/v3/assets/{asset}"),
        Vec::new(),
        "application/octet-stream",
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "another account cannot read the asset"
    );

    // Overlays: a typed layout referencing an owned asset; stale revisions conflict; foreign assets are not found.
    let overlay = json!({
        "overlayVersion": 1,
        "kind": "gift",
        "title": "Gift alert",
        "canvas": {"width": 1920, "height": 1080},
        "elements": [
            {"id": "logo", "type": "art", "x": 0, "y": 0, "width": 64, "height": 64, "assetId": asset},
            {"id": "who", "type": "text", "x": 80, "y": 0, "width": 400, "height": 40, "source": {"kind": "field", "field": "viewer.name"}}
        ],
        "capabilities": ["live.read"]
    });
    let (status, _, created) = call(
        &f,
        Some(owner),
        "POST",
        "/api/v3/overlays",
        Some(json!({"document": overlay.clone()})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let overlay_id = created["id"].as_str().unwrap().to_owned();
    let (status, _, _) = call(
        &f,
        Some(other),
        "POST",
        "/api/v3/overlays",
        Some(json!({"document": overlay.clone()})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "the other account cannot reference this asset"
    );
    let (status, _, stale) = call(
        &f,
        Some(owner),
        "POST",
        &format!("/api/v3/overlays/{overlay_id}/revisions"),
        Some(json!({"document": overlay.clone(), "expectedRevision": 7})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{stale}");
    let mut revised = overlay.clone();
    revised["title"] = json!("Gift alert v2");
    let (status, _, next) = call(
        &f,
        Some(owner),
        "POST",
        &format!("/api/v3/overlays/{overlay_id}/revisions"),
        Some(json!({"document": revised, "expectedRevision": 1})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{next}");
    assert_eq!(next["revision"], 2);
    let (status, _, _) = call(
        &f,
        Some(other),
        "GET",
        &format!("/api/v3/overlays/{overlay_id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Packages pin the current overlay revision; revising the overlay afterwards blocks application in preflight.
    let (status, _, package) = call(
        &f,
        Some(owner),
        "POST",
        "/api/v3/packages",
        Some(json!({
            "title": "Gift pack",
            "items": [{"kind": "overlay", "id": overlay_id, "revision": 2, "required": true}]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{package}");
    let package_id = package["id"].as_str().unwrap().to_owned();
    let (status, _, ready) = call(
        &f,
        Some(owner),
        "GET",
        &format!("/api/v3/packages/{package_id}/preflight"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ready["canApply"], true, "{ready}");
    assert_eq!(ready["applyMode"], "clientJournal");
    let (status, _, _) = call(
        &f,
        Some(owner),
        "POST",
        "/api/v3/packages",
        Some(json!({
            "title": "Stale pin",
            "items": [{"kind": "overlay", "id": overlay_id, "revision": 1, "required": true}]
        })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "pinning an old revision is refused at creation"
    );
    let (status, _, _) = call(
        &f,
        Some(owner),
        "POST",
        "/api/v3/packages",
        Some(json!({
            "title": "Plugin pin",
            "items": [{"kind": "plugin", "id": "acme.absent", "version": "1.0.0", "required": true}]
        })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a plugin absent from the active generation is not pinned"
    );

    let mut second = revised.clone();
    second["title"] = json!("Gift alert v3");
    let (status, _, _) = call(
        &f,
        Some(owner),
        "POST",
        &format!("/api/v3/overlays/{overlay_id}/revisions"),
        Some(json!({"document": second, "expectedRevision": 2})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (_, _, drifted) = call(
        &f,
        Some(owner),
        "GET",
        &format!("/api/v3/packages/{package_id}/preflight"),
        None,
    )
    .await;
    assert_eq!(drifted["canApply"], false, "{drifted}");
    assert_eq!(drifted["blocking"][0]["status"], "revisionChanged");
    assert_eq!(drifted["items"][0]["activation"], "disabledUntilExplicit");
}
