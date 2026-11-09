//! Real PostgreSQL and HTTP/SDK coverage for the single-instance product API.
use super::*;
use axum::{
    body::Body,
    http::{header, Request},
    response::Response,
};
use better_auth::email_password::{EmailPasswordService, SignUpInput};
use serde_json::{json, Value};
use tower::ServiceExt;
const ORIGIN: &str = "https://cloud.example.test";
const PASSWORD: &str = "test password with enough characters";

struct User {
    id: String,
    cookie: String,
    token: String,
}
struct Fixture {
    router: Router,
    pool: PgPool,
    users: Vec<User>,
    origin: String,
    server: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}
async fn fixture() -> Fixture {
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
    for _ in 0..3 {
        let user = EmailPasswordService::new(context.clone())
            .unwrap()
            .sign_up(
                SignUpInput {
                    email: format!("{}@example.test", uuid::Uuid::new_v4()),
                    name: "Cloud V2 Test".into(),
                    password: PASSWORD.into(),
                },
                true,
            )
            .await
            .unwrap();
        users.push(User {
            id: user.user.id,
            cookie: format!("{}={}", user.cookie.name, user.cookie.value),
            token: user.session_token,
        });
    }
    let router = app(
        context.clone(),
        pool.clone(),
        AuthRouter::new(context).unwrap(),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
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
        origin,
        server,
    }
}
fn template() -> Value {
    json!({"templateVersion":1,"id":"gift-rule","title":"Gift rule","actions":[{"name":"Notify","typeId":"core.notify","enabled":true,"config":{"message":"Thanks {{ event.user.nickname }}"}}],"event":{"name":"Gift","trigger":"tiktok.gift"}})
}
async fn request(
    f: &Fixture,
    user: Option<usize>,
    method: &str,
    path: &str,
    value: Value,
    condition: Option<(&str, &str)>,
) -> Response {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(user) = user {
        request = request
            .header(header::COOKIE, &f.users[user].cookie)
            .header(header::ORIGIN, ORIGIN);
    }
    if let Some((key, value)) = condition {
        request = request.header(key, value);
    }
    f.router
        .clone()
        .oneshot(request.body(Body::from(value.to_string())).unwrap())
        .await
        .unwrap()
}
async fn json_body(response: Response) -> Value {
    let bytes = axum::body::to_bytes(response.into_body(), 32 * 1024 * 1024)
        .await
        .unwrap();
    if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    }
}

#[tokio::test]
#[ignore = "requires a dedicated PostgreSQL database in CLOUD_TEST_DATABASE_URL"]
async fn v1_snapshot_pagination_uses_byte_order_for_punctuation_and_rebuild() {
    let f = fixture().await;
    let namespace = format!("ordering{}", uuid::Uuid::new_v4().simple());
    // The punctuation group straddles the first 100-record page, so this
    // checks both ordering and the exclusive continuation comparison.
    let mut ids: Vec<String> = (0..98)
        .map(|index| format!("{namespace}.a{index:03}"))
        .collect();
    ids.extend(
        ["m-0", "m.0", "m0", "m_0", "z0", "z1", "z2", "z3"]
            .into_iter()
            .map(|suffix| format!("{namespace}.{suffix}")),
    );
    ids.sort();
    assert_eq!(ids.len(), 106);
    assert_eq!(ids[99], format!("{namespace}.m.0"));
    assert_eq!(ids[100], format!("{namespace}.m0"));

    sqlx::query("INSERT INTO cloud_sync_cursors(user_id,cursor) VALUES($1,$2)")
        .bind(&f.users[0].id)
        .bind(ids.len() as i64)
        .execute(&f.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO cloud_sync_records(user_id,collection,key,revision,value,deleted) SELECT $1,'plugin-selections',key,revision,jsonb_build_object('schemaVersion',1,'pluginId',key,'version','1.0.0','enabled',true),false FROM unnest($2::text[]) WITH ORDINALITY AS fixture(key,revision)")
        .bind(&f.users[0].id).bind(&ids).execute(&f.pool).await.unwrap();

    let path = "/api/v1/sync/collections/plugin-selections";
    let first = request(&f, Some(0), "GET", path, Value::Null, None).await;
    assert_eq!(first.status(), StatusCode::OK);
    let first = json_body(first).await;
    let first_records = first["records"].as_array().unwrap();
    assert_eq!(first_records.len(), 100);
    let after = first["nextAfter"].as_str().unwrap();
    assert_eq!(after, ids[99]);
    let second_path = format!("{path}?after={after}");
    let second = request(&f, Some(0), "GET", &second_path, Value::Null, None).await;
    assert_eq!(second.status(), StatusCode::OK);
    let second = json_body(second).await;
    assert!(second["nextAfter"].is_null());
    let mut actual: Vec<String> = first_records
        .iter()
        .map(|row| row["key"].as_str().unwrap().to_owned())
        .collect();
    actual.extend(
        second["records"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["key"].as_str().unwrap().to_owned()),
    );
    assert_eq!(actual, ids);
    let isolated = request(&f, Some(1), "GET", path, Value::Null, None).await;
    assert!(json_body(isolated).await["records"]
        .as_array()
        .unwrap()
        .is_empty());

    // A real native SDK rebuild uses the native route and requires strictly
    // byte-ordered pages. Existing pending edits must survive the snapshot.
    let session = cloud_client::Session::connect(&f.origin, f.users[0].token.clone())
        .await
        .unwrap();
    let mut state = session.new_state();
    state
        .stage("local.pending".into(), cloud_client::Edit::Delete)
        .unwrap();
    assert_eq!(session.rebuild_with_count(&mut state).await.unwrap(), 106);
    assert!(state.entries["local.pending"].pending.is_some());
    assert!(ids.iter().all(|key| state.entries[key].remote.is_some()));
}

#[tokio::test]
#[ignore = "requires a dedicated PostgreSQL database in CLOUD_TEST_DATABASE_URL"]
async fn workspace_two_sessions_conflicts_pagination_and_v1_isolation() {
    let f = fixture().await;
    let a = cloud_client::Session::connect(&f.origin, f.users[0].token.clone())
        .await
        .unwrap();
    let b = cloud_client::Session::connect(&f.origin, f.users[0].token.clone())
        .await
        .unwrap();
    let other = cloud_client::Session::connect(&f.origin, f.users[1].token.clone())
        .await
        .unwrap();
    let catalog = cloud_client::CloudCatalog::new(&f.origin).unwrap();
    let capabilities = catalog.capabilities().await.unwrap();
    assert_eq!(capabilities["instanceMode"], "single");
    assert_eq!(capabilities["userConfigurableRegistries"], false);
    let first = a
        .workspace_put(
            "preferences",
            "desktop",
            json!({"schemaVersion":1,"theme":"dark"}),
            None,
        )
        .await
        .unwrap();
    assert_eq!(first.revision, 1);
    assert_eq!(b.workspace_snapshot().await.unwrap().records.len(), 1);
    let updated = b
        .workspace_put(
            "preferences",
            "desktop",
            json!({"schemaVersion":1,"theme":"light"}),
            Some(1),
        )
        .await
        .unwrap();
    assert_eq!(updated.revision, 2);
    assert!(matches!(
        a.workspace_put(
            "preferences",
            "desktop",
            json!({"schemaVersion":1,"theme":"dark"}),
            Some(1)
        )
        .await,
        Err(cloud_client::SyncError::Conflict)
    ));
    assert_eq!(
        b.workspace_put(
            "preferences",
            "desktop",
            updated.value.clone().unwrap(),
            Some(2)
        )
        .await
        .unwrap()
        .revision,
        2
    );
    assert!(other.workspace_snapshot().await.unwrap().records.is_empty());
    assert!(matches!(
        other.workspace_record("preferences", "desktop").await,
        Err(cloud_client::SyncError::Http(404))
    ));
    a.workspace_put("rule-templates", "gift-rule", template(), None)
        .await
        .unwrap();
    a.workspace_put("profiles","gift-pack",json!({"profileVersion":1,"id":"gift-pack","name":"Gift pack","templates":[{"template":template()}]}),None).await.unwrap();
    a.workspace_put(
        "plugin-selections",
        "acme.plugin",
        json!({"schemaVersion":1,"pluginId":"acme.plugin","version":"1.2.3","enabled":true}),
        None,
    )
    .await
    .unwrap();
    for i in 0..51 {
        let id = format!("acme.plugin{i}");
        a.workspace_put(
            "plugin-settings",
            &id,
            json!({"schemaVersion":1,"pluginId":id,"settings":{"volume":50}}),
            None,
        )
        .await
        .unwrap();
    }
    let page = a.workspace_changes(0).await.unwrap();
    assert_eq!(page.changes.len(), 50);
    assert!(page.has_more);
    let page2 = b.workspace_changes(page.cursor).await.unwrap();
    assert!(!page2.has_more);
    assert_eq!(page2.changes.len(), 6);
    let snapshot = a.workspace_snapshot().await.unwrap();
    assert_eq!(snapshot.records.len(), 55);
    // Legacy callers receive exactly their old plugin-only dataset and DTO.
    assert!(a.export_sync().await.unwrap().records.is_empty());
    assert!(a.export_account().await.unwrap().sync.records.is_empty());
    let export = a.export_account_v2().await.unwrap();
    assert_eq!(
        export["workspaceSync"]["records"].as_array().unwrap().len(),
        55
    );
    let tombstone = a
        .workspace_delete("preferences", "desktop", 2)
        .await
        .unwrap();
    assert!(tombstone.deleted);
    assert!(
        b.workspace_record("preferences", "desktop")
            .await
            .unwrap()
            .deleted
    );
    let path = "/api/v2/sync/collections/preferences/desktop";
    assert_eq!(
        request(&f, Some(0), "PUT", path, json!({"schemaVersion":1}), None)
            .await
            .status(),
        StatusCode::PRECONDITION_REQUIRED
    );
    assert_eq!(
        request(
            &f,
            Some(0),
            "PUT",
            path,
            json!({"schemaVersion":1,"registryUrl":"https://other.test"}),
            Some(("if-match", "\"2\""))
        )
        .await
        .status(),
        StatusCode::BAD_REQUEST
    );
    let request = Request::builder()
        .method("PUT")
        .uri(path)
        .header(header::COOKIE, &f.users[0].cookie)
        .header(header::ORIGIN, "https://evil.test")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::IF_MATCH, "\"2\"")
        .body(Body::from("{\"schemaVersion\":1}"))
        .unwrap();
    assert!(f
        .router
        .clone()
        .oneshot(request)
        .await
        .unwrap()
        .status()
        .is_client_error());
    // Force a retained-feed gap while preserving a valid snapshot.
    sqlx::query("DELETE FROM cloud_workspace_changes WHERE user_id=$1 AND cursor<=50")
        .bind(&f.users[0].id)
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(matches!(
        b.workspace_changes(0).await,
        Err(cloud_client::SyncError::CursorExpired)
    ));
    assert!(b.workspace_snapshot().await.unwrap().valid());
    assert!(b
        .workspace_changes(tombstone.revision)
        .await
        .unwrap()
        .changes
        .is_empty());
    // Two devices racing from the same base can never both win.
    let (left, right) = tokio::join!(
        a.workspace_put(
            "preferences",
            "desktop",
            json!({"schemaVersion":1,"theme":"dark"}),
            Some(tombstone.revision)
        ),
        b.workspace_put(
            "preferences",
            "desktop",
            json!({"schemaVersion":1,"theme":"light"}),
            Some(tombstone.revision)
        )
    );
    assert_ne!(left.is_ok(), right.is_ok());
    assert!(
        matches!(left, Err(cloud_client::SyncError::Conflict))
            || matches!(right, Err(cloud_client::SyncError::Conflict))
    );
    assert_eq!(
        a.workspace_record("preferences", "desktop")
            .await
            .unwrap()
            .revision,
        tombstone.revision + 1
    );
}

#[tokio::test]
#[ignore = "requires a dedicated PostgreSQL database in CLOUD_TEST_DATABASE_URL"]
async fn library_independent_publication_immutable_versions_withdrawal_and_cleanup() {
    let f = fixture().await;
    let a = cloud_client::Session::connect(&f.origin, f.users[0].token.clone())
        .await
        .unwrap();
    let other = cloud_client::Session::connect(&f.origin, f.users[2].token.clone())
        .await
        .unwrap();
    let public = cloud_client::CloudCatalog::new(&f.origin).unwrap();
    let created = a.library_create("template", template()).await.unwrap();
    let id = uuid::Uuid::parse_str(created["id"].as_str().unwrap()).unwrap();
    assert!(matches!(
        public.document(id, None).await,
        Err(cloud_client::SyncError::Http(404))
    ));
    assert!(matches!(
        other.library_document(id).await,
        Err(cloud_client::SyncError::Http(404))
    ));
    assert!(matches!(
        a.library_create("template", template()).await,
        Err(cloud_client::SyncError::Conflict)
    ));
    a.library_submit(id, 1).await.unwrap();
    for user in [0, 1] {
        sqlx::query("INSERT INTO cloud_account_roles(user_id,role) VALUES($1,'moderator')")
            .bind(&f.users[user].id)
            .execute(&f.pool)
            .await
            .unwrap();
    }
    let decision = format!("/api/v2/moderation/documents/{id}/versions/1/decision");
    let approval = json!({"decision":"approve","reason":"Reviewed portable actions"});
    assert_eq!(
        request(&f, Some(0), "POST", &decision, approval.clone(), None)
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    let queue = json_body(
        request(
            &f,
            Some(1),
            "GET",
            "/api/v2/moderation/documents",
            Value::Null,
            None,
        )
        .await,
    )
    .await;
    assert!(queue["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v["id"] == id.to_string()));
    assert_eq!(
        request(&f, Some(1), "POST", &decision, approval, None)
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        public.document(id, None).await.unwrap()["document"],
        template()
    );
    let mut changed = template();
    changed["title"] = json!("Updated gift rule");
    a.library_update(id, "template", changed.clone(), 1)
        .await
        .unwrap();
    assert_eq!(public.document(id, None).await.unwrap()["revision"], 1);
    assert_eq!(a.library_document(id).await.unwrap()["revision"], 2);
    assert!(matches!(
        a.library_update(id, "template", changed, 1).await,
        Err(cloud_client::SyncError::Conflict)
    ));
    a.library_withdraw(id, 1, "Replace outdated guidance")
        .await
        .unwrap();
    assert!(matches!(
        public.document(id, None).await,
        Err(cloud_client::SyncError::Http(404))
    ));
    assert!(matches!(
        public.document(id, Some(1)).await,
        Err(cloud_client::SyncError::Http(404))
    ));
    let export = a.export_account_v2().await.unwrap();
    assert_eq!(export["library"]["items"].as_array().unwrap().len(), 2);
    // Deleting a library document cascades its revisions even with a public pointer.
    let path = format!("/api/v2/library/documents/{id}");
    assert_eq!(
        request(
            &f,
            Some(2),
            "DELETE",
            &path,
            Value::Null,
            Some(("if-match", "\"2\""))
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(
            &f,
            Some(0),
            "DELETE",
            &path,
            Value::Null,
            Some(("if-match", "\"1\""))
        )
        .await
        .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        request(
            &f,
            Some(0),
            "DELETE",
            &path,
            Value::Null,
            Some(("if-match", "\"2\""))
        )
        .await
        .status(),
        StatusCode::NO_CONTENT
    );
    let remaining: i64 =
        sqlx::query_scalar("SELECT count(*) FROM cloud_library_versions WHERE document_id=$1")
            .bind(id)
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(remaining, 0);
    // The existing account deletion operation removes both new private stores.
    let owner = cloud_client::Session::connect(&f.origin, f.users[2].token.clone())
        .await
        .unwrap();
    owner
        .workspace_put(
            "preferences",
            "desktop",
            json!({"schemaVersion":1,"theme":"dark"}),
            None,
        )
        .await
        .unwrap();
    let own = owner.library_create("template", template()).await.unwrap();
    let deleted = request(
        &f,
        Some(2),
        "DELETE",
        "/api/v1/me",
        json!({"password":PASSWORD}),
        None,
    )
    .await;
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM cloud_library_documents WHERE id=$1")
        .bind(uuid::Uuid::parse_str(own["id"].as_str().unwrap()).unwrap())
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM cloud_workspace_records WHERE user_id=$1")
            .bind(&f.users[2].id)
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
#[ignore = "requires a dedicated PostgreSQL database in CLOUD_TEST_DATABASE_URL"]
async fn workspace_quotas_and_untrusted_inputs_are_enforced_by_the_api() {
    let f = fixture().await;
    let path = "/api/v2/sync/collections/plugin-settings/acme.plugin";
    for settings in [
        json!({"password":"do-not-store"}),
        json!({"headers":{"Authorization":"Bearer secret"}}),
        json!({"registryUrl":"https://other.test"}),
    ] {
        let body = json!({"schemaVersion":1,"pluginId":"acme.plugin","settings":settings});
        assert_eq!(
            request(&f, Some(0), "PUT", path, body, Some(("if-none-match", "*")))
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    sqlx::query("INSERT INTO cloud_workspace_cursors(user_id,cursor) VALUES($1,1)")
        .bind(&f.users[0].id)
        .execute(&f.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO cloud_workspace_records(user_id,collection,key,revision,value,deleted) SELECT $1,'plugin-settings','acme.plugin'||n,1,jsonb_build_object('schemaVersion',1,'pluginId','acme.plugin'||n,'settings','{}'::jsonb),false FROM generate_series(1,1000) n").bind(&f.users[0].id).execute(&f.pool).await.unwrap();
    assert_eq!(
        request(
            &f,
            Some(0),
            "PUT",
            "/api/v2/sync/collections/preferences/desktop",
            json!({"schemaVersion":1}),
            Some(("if-none-match", "*"))
        )
        .await
        .status(),
        StatusCode::INSUFFICIENT_STORAGE
    );
    // Modifications and tombstones remain possible at the record-count limit.
    assert_eq!(
        request(
            &f,
            Some(0),
            "DELETE",
            "/api/v2/sync/collections/plugin-settings/acme.plugin1",
            Value::Null,
            Some(("if-match", "\"1\""))
        )
        .await
        .status(),
        StatusCode::OK
    );
    let response = request(&f, None, "GET", "/api/v2/capabilities", Value::Null, None).await;
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(
        request(
            &f,
            None,
            "GET",
            "/api/v2/catalog/plugins?registry=https://other.test",
            Value::Null,
            None
        )
        .await
        .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(
            &f,
            None,
            "GET",
            "/api/v2/library/documents",
            Value::Null,
            None
        )
        .await
        .status(),
        StatusCode::UNAUTHORIZED
    );
}

fn wire_digest(document: &Value) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(serde_json::to_vec(document).unwrap()))
}

fn numeric_template() -> Value {
    let mut document = template();
    document["actions"][0]["config"]["numbers"] = serde_json::from_str(
        r#"{"large":1e18,"negativeZero":-0.0,"fraction":1e-6,"whole":1.0,"exact":18446744073709551615}"#,
    )
    .unwrap();
    document
}

#[tokio::test]
#[ignore = "requires a dedicated PostgreSQL database in CLOUD_TEST_DATABASE_URL"]
async fn jsonb_numeric_normalization_preserves_library_identity_and_workspace_noops() {
    let f = fixture().await;
    let owner = cloud_client::Session::connect(&f.origin, f.users[0].token.clone())
        .await
        .unwrap();
    let input = numeric_template();
    let created = owner
        .library_create("template", input.clone())
        .await
        .unwrap();
    let id = uuid::Uuid::parse_str(created["id"].as_str().unwrap()).unwrap();
    let stored = owner.library_document(id).await.unwrap();
    assert_ne!(wire_digest(&input), wire_digest(&stored["document"]));
    assert_eq!(stored["sha256"], wire_digest(&stored["document"]));
    assert_eq!(
        stored["document"]["actions"][0]["config"]["numbers"]["large"],
        json!(1_000_000_000_000_000_000_u64)
    );
    assert_eq!(
        stored["document"]["actions"][0]["config"]["numbers"]["exact"],
        json!(u64::MAX)
    );
    let mut edited = input.clone();
    edited["title"] = json!("Numeric update");
    owner
        .library_update(id, "template", edited.clone(), 1)
        .await
        .unwrap();
    let updated = owner.library_document(id).await.unwrap();
    assert_ne!(wire_digest(&edited), wire_digest(&updated["document"]));
    assert_eq!(updated["sha256"], wire_digest(&updated["document"]));
    let immutable_response = request(
        &f,
        Some(0),
        "GET",
        &format!("/api/v2/library/documents/{id}/versions/1"),
        Value::Null,
        None,
    )
    .await;
    assert_eq!(immutable_response.status(), StatusCode::OK);
    let immutable = json_body(immutable_response).await;
    assert_eq!(immutable["sha256"], stored["sha256"]);
    assert_eq!(immutable["document"], stored["document"]);

    let value = json!({"schemaVersion":1,"pluginId":"acme.numeric","settings":input["actions"][0]["config"]["numbers"]});
    let ack = owner
        .workspace_put("plugin-settings", "acme.numeric", value.clone(), None)
        .await
        .unwrap();
    let get = owner
        .workspace_record("plugin-settings", "acme.numeric")
        .await
        .unwrap();
    let feed = owner.workspace_changes(0).await.unwrap();
    let snapshot = owner.workspace_snapshot().await.unwrap();
    assert_eq!(ack, get);
    assert_eq!(feed.changes, vec![ack.clone()]);
    assert_eq!(snapshot.records, vec![ack.clone()]);
    // A repeated original exponent-form request is a true no-op, and its ACK
    // still contains the stored representation used by the unchanged feed.
    let repeat = owner
        .workspace_put("plugin-settings", "acme.numeric", value, Some(ack.revision))
        .await
        .unwrap();
    assert_eq!(repeat, ack);
    assert!(owner
        .workspace_changes(ack.revision)
        .await
        .unwrap()
        .changes
        .is_empty());
    // Equal integer/decimal representations also preserve the original body.
    let mut equivalent = ack.value.clone().unwrap();
    equivalent["settings"]["whole"] = json!(1);
    let repeat = owner
        .workspace_put(
            "plugin-settings",
            "acme.numeric",
            equivalent,
            Some(ack.revision),
        )
        .await
        .unwrap();
    assert_eq!(repeat, ack);
    assert_eq!(
        owner.workspace_snapshot().await.unwrap().cursor,
        ack.revision
    );

    // JSONB can expand exponent notation past the portable document limit.
    // Both APIs revalidate before committing any source, revision or cursor.
    let expanded_settings =
        json!({"numbers":vec![vec![json!(1e18);256];8],"padding":vec!["a".repeat(8000);30]});
    let expanded_workspace =
        json!({"schemaVersion":1,"pluginId":"acme.expanded","settings":expanded_settings});
    assert!(portable::validate(
        "plugin-settings",
        "acme.expanded",
        &expanded_workspace
    ));
    let normalized: Value = sqlx::query_scalar("SELECT $1::jsonb")
        .bind(&expanded_workspace)
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert!(!portable::validate(
        "plugin-settings",
        "acme.expanded",
        &normalized
    ));
    assert_eq!(
        request(
            &f,
            Some(0),
            "PUT",
            "/api/v2/sync/collections/plugin-settings/acme.expanded",
            expanded_workspace,
            Some(("if-none-match", "*"))
        )
        .await
        .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        owner.workspace_snapshot().await.unwrap().cursor,
        ack.revision
    );
    let mut expanded_document = template();
    expanded_document["id"] = json!("expanded-rule");
    expanded_document["actions"][0]["config"] = expanded_settings;
    assert!(portable::document("template", &expanded_document));
    assert_eq!(
        request(
            &f,
            Some(0),
            "POST",
            "/api/v2/library/documents",
            json!({"kind":"template","document":expanded_document}),
            None
        )
        .await
        .status(),
        StatusCode::BAD_REQUEST
    );
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM cloud_library_documents WHERE owner_user_id=$1")
            .bind(&f.users[0].id)
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(count, 1);
    expanded_document["id"] = json!("gift-rule");
    assert_eq!(
        request(
            &f,
            Some(0),
            "PUT",
            &format!("/api/v2/library/documents/{id}"),
            json!({"document":expanded_document}),
            Some(("if-match", "\"2\""))
        )
        .await
        .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(owner.library_document(id).await.unwrap()["revision"], 2);
}

#[tokio::test]
#[ignore = "requires a dedicated PostgreSQL database in CLOUD_TEST_DATABASE_URL"]
async fn legacy_library_digest_mismatch_fails_closed_and_repairs_only_reviewed_metadata() {
    use crate::library_digest_repair::{self as repair, AuditCursor, RepairError};
    let f = fixture().await;
    let owner = cloud_client::Session::connect(&f.origin, f.users[0].token.clone())
        .await
        .unwrap();
    let input = numeric_template();
    let created = owner
        .library_create("template", input.clone())
        .await
        .unwrap();
    let id = uuid::Uuid::parse_str(created["id"].as_str().unwrap()).unwrap();
    owner.library_submit(id, 1).await.unwrap();
    sqlx::query("INSERT INTO cloud_account_roles(user_id,role) VALUES($1,'moderator')")
        .bind(&f.users[1].id)
        .execute(&f.pool)
        .await
        .unwrap();
    let decision = format!("/api/v2/moderation/documents/{id}/versions/1/decision");
    assert_eq!(
        request(
            &f,
            Some(1),
            "POST",
            &decision,
            json!({"decision":"approve","reason":"Reviewed stored numeric content"}),
            None
        )
        .await
        .status(),
        StatusCode::NO_CONTENT
    );
    let current = owner.library_document(id).await.unwrap();
    let old_hash = wire_digest(&input);
    let normalized_hash = current["sha256"].as_str().unwrap();
    assert_ne!(old_hash, normalized_hash);
    // Simulate exactly the pre-fix write, without changing approved content.
    sqlx::query("UPDATE cloud_library_versions SET sha256=$2 WHERE document_id=$1 AND revision=1")
        .bind(id)
        .bind(&old_hash)
        .execute(&f.pool)
        .await
        .unwrap();
    let before: Value = sqlx::query_scalar(
        "SELECT to_jsonb(v) FROM cloud_library_versions v WHERE document_id=$1 AND revision=1",
    )
    .bind(id)
    .fetch_one(&f.pool)
    .await
    .unwrap();
    let document_before: Value =
        sqlx::query_scalar("SELECT to_jsonb(d) FROM cloud_library_documents d WHERE id=$1")
            .bind(id)
            .fetch_one(&f.pool)
            .await
            .unwrap();
    for (user, path) in [
        (Some(0), format!("/api/v2/library/documents/{id}")),
        (
            Some(0),
            format!("/api/v2/library/documents/{id}/versions/1"),
        ),
        (Some(0), format!("/api/v2/library/documents/{id}/versions")),
        (Some(0), "/api/v2/library/documents".into()),
        (Some(0), "/api/v2/me/export".into()),
        (None, format!("/api/v2/catalog/documents/{id}")),
        (None, format!("/api/v2/catalog/documents/{id}/versions/1")),
        (None, "/api/v2/catalog/documents?q=Gift%20rule".into()),
    ] {
        assert_eq!(
            request(&f, user, "GET", &path, Value::Null, None)
                .await
                .status(),
            StatusCode::SERVICE_UNAVAILABLE,
            "{path}"
        );
    }
    let audit = repair::audit(&f.pool, None, 100).await.unwrap();
    // Other tests can leave rows in this dedicated database; scan all pages.
    let mut items = audit.items;
    let mut cursor = audit.next_after;
    while let Some(after) = cursor {
        let page = repair::audit(&f.pool, Some(after), 100).await.unwrap();
        items.extend(page.items);
        cursor = page.next_after;
    }
    let issue = items
        .iter()
        .find(|item| item.document_id == id && item.revision == 1)
        .unwrap();
    assert_eq!(issue.status, "published");
    assert_eq!(issue.stored_sha256, old_hash);
    assert_eq!(issue.normalized_sha256, normalized_hash);
    assert!(issue.portable);
    let still_bad: String = sqlx::query_scalar(
        "SELECT sha256 FROM cloud_library_versions WHERE document_id=$1 AND revision=1",
    )
    .bind(id)
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(still_bad, old_hash);
    assert!(matches!(
        repair::repair(
            &f.pool,
            id,
            1,
            &"f".repeat(64),
            normalized_hash,
            "fixture-operator"
        )
        .await,
        Err(RepairError::Conflict)
    ));
    assert!(matches!(
        repair::repair(
            &f.pool,
            id,
            1,
            &old_hash,
            &"e".repeat(64),
            "fixture-operator"
        )
        .await,
        Err(RepairError::Conflict)
    ));
    let receipt = repair::repair(
        &f.pool,
        id,
        1,
        &old_hash,
        normalized_hash,
        "fixture-operator",
    )
    .await
    .unwrap();
    assert_eq!(receipt.normalized_sha256, normalized_hash);
    let after: Value = sqlx::query_scalar(
        "SELECT to_jsonb(v) FROM cloud_library_versions v WHERE document_id=$1 AND revision=1",
    )
    .bind(id)
    .fetch_one(&f.pool)
    .await
    .unwrap();
    let mut expected = before;
    expected["sha256"] = json!(normalized_hash);
    assert_eq!(after, expected);
    let document_after: Value =
        sqlx::query_scalar("SELECT to_jsonb(d) FROM cloud_library_documents d WHERE id=$1")
            .bind(id)
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(document_after, document_before);
    assert_eq!(
        cloud_client::CloudCatalog::new(&f.origin)
            .unwrap()
            .document(id, None)
            .await
            .unwrap(),
        current
    );
    assert!(matches!(
        repair::repair(
            &f.pool,
            id,
            1,
            &old_hash,
            normalized_hash,
            "fixture-operator"
        )
        .await,
        Err(RepairError::Conflict)
    ));
    let evidence: Value = sqlx::query_scalar("SELECT jsonb_build_object('old',old_sha256,'new',normalized_sha256,'operator',operator) FROM cloud_library_digest_repairs WHERE document_id=$1 AND revision=1").bind(id).fetch_one(&f.pool).await.unwrap();
    assert_eq!(
        evidence,
        json!({"old":old_hash,"new":normalized_hash,"operator":"fixture-operator"})
    );

    // An empty audit page still advances past valid rows to a later mismatch.
    owner
        .library_update(id, "template", input.clone(), 1)
        .await
        .unwrap();
    owner
        .library_update(id, "template", input.clone(), 2)
        .await
        .unwrap();
    sqlx::query("UPDATE cloud_library_versions SET sha256=$2 WHERE document_id=$1 AND revision=3")
        .bind(id)
        .bind(&old_hash)
        .execute(&f.pool)
        .await
        .unwrap();
    let empty = repair::audit(
        &f.pool,
        Some(AuditCursor {
            document_id: id,
            revision: 1,
        }),
        1,
    )
    .await
    .unwrap();
    assert!(empty.items.is_empty());
    let next = empty.next_after.unwrap();
    assert_eq!(next.document_id, id);
    assert_eq!(next.revision, 2);
    let page = repair::audit(&f.pool, Some(next), 1).await.unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].revision, 3);
    assert_eq!(
        request(
            &f,
            Some(0),
            "POST",
            &format!("/api/v2/library/documents/{id}/versions/3/submit"),
            Value::Null,
            None
        )
        .await
        .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    sqlx::query(
        "UPDATE cloud_library_versions SET status='pending' WHERE document_id=$1 AND revision=3",
    )
    .bind(id)
    .execute(&f.pool)
    .await
    .unwrap();
    assert_eq!(
        request(
            &f,
            Some(1),
            "GET",
            "/api/v2/moderation/documents?q=Gift%20rule",
            Value::Null,
            None
        )
        .await
        .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        request(
            &f,
            Some(1),
            "POST",
            &format!("/api/v2/moderation/documents/{id}/versions/3/decision"),
            json!({"decision":"approve","reason":"Must not approve a false identity"}),
            None
        )
        .await
        .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    let status: String = sqlx::query_scalar(
        "SELECT status FROM cloud_library_versions WHERE document_id=$1 AND revision=3",
    )
    .bind(id)
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(status, "pending");
    // Corrective withdrawal remains available even for a broken digest.
    owner
        .library_withdraw(id, 3, "Withdraw invalid identity")
        .await
        .unwrap();
    sqlx::query("UPDATE cloud_library_versions SET document=jsonb_set(document,'{title}','\"\"'::jsonb) WHERE document_id=$1 AND revision=3").bind(id).execute(&f.pool).await.unwrap();
    let invalid: Value = sqlx::query_scalar(
        "SELECT document FROM cloud_library_versions WHERE document_id=$1 AND revision=3",
    )
    .bind(id)
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert!(matches!(
        repair::repair(
            &f.pool,
            id,
            3,
            &old_hash,
            &wire_digest(&invalid),
            "fixture-operator"
        )
        .await,
        Err(RepairError::InvalidPortable)
    ));
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM cloud_library_digest_repairs WHERE document_id=$1",
    )
    .bind(id)
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(count, 1);
    assert_eq!(
        request(
            &f,
            Some(0),
            "DELETE",
            &format!("/api/v2/library/documents/{id}"),
            Value::Null,
            Some(("if-match", "\"3\""))
        )
        .await
        .status(),
        StatusCode::NO_CONTENT
    );
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM cloud_library_digest_repairs WHERE document_id=$1",
    )
    .bind(id)
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
#[ignore = "requires a dedicated PostgreSQL database in CLOUD_TEST_DATABASE_URL"]
async fn ascii_punctuation_key_and_plugin_pages_use_locale_independent_exclusive_cursors() {
    let f = fixture().await;
    let namespace = format!("ordering{}", uuid::Uuid::new_v4().simple());
    let mut ids: Vec<String> = ["a-b", "a.b", "a_b", "aa"]
        .into_iter()
        .map(|suffix| format!("{namespace}.{suffix}"))
        .collect();
    ids.extend((0..52).map(|index| format!("{namespace}.p{index:02}")));
    ids.sort();
    assert_eq!(ids.len(), 56);
    assert!(ids.windows(2).all(|pair| pair[0] < pair[1]));

    // Where libc locales exist, prove these IDs distinguish byte order from
    // locale order. The HTTP assertions below always run; they neither change
    // the database locale nor depend on a non-C locale being installed in CI.
    for locale in ["en_US.utf8", "es_PE.utf8"] {
        let available:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_collation c JOIN pg_namespace n ON n.oid=c.collnamespace WHERE n.nspname='pg_catalog' AND c.collname=$1 AND c.collprovider='c' AND c.collencoding IN (-1,pg_char_to_encoding('UTF8')))").bind(locale).fetch_one(&f.pool).await.unwrap();
        if available {
            let query = format!(
                r#"SELECT id FROM unnest($1::text[]) AS id ORDER BY id COLLATE pg_catalog."{locale}""#
            );
            let localized: Vec<String> = sqlx::query_scalar(&query)
                .bind(&ids)
                .fetch_all(&f.pool)
                .await
                .unwrap();
            assert_ne!(
                localized, ids,
                "punctuation fixture must distinguish installed libc locale {locale}"
            );
            break;
        }
    }

    // Seed only metadata/data for pagination: this test does not claim signed
    // publication, package verification, or plugin execution authorization.
    sqlx::query("INSERT INTO cloud_workspace_cursors(user_id,cursor) VALUES($1,$2)")
        .bind(&f.users[0].id)
        .bind(ids.len() as i64)
        .execute(&f.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO cloud_workspace_records(user_id,collection,key,revision,value,deleted) SELECT $1,'plugin-settings',key,revision,jsonb_build_object('schemaVersion',1,'pluginId',key,'settings','{}'::jsonb),false FROM unnest($2::text[]) WITH ORDINALITY AS fixture(key,revision)")
        .bind(&f.users[0].id).bind(&ids).execute(&f.pool).await.unwrap();
    let owner = cloud_client::Session::connect(&f.origin, f.users[0].token.clone())
        .await
        .unwrap();
    let mut received = Vec::new();
    let mut after: Option<String> = None;
    for page_number in 0..2 {
        let page = owner
            .workspace_collection("plugin-settings", after.as_deref())
            .await
            .unwrap();
        let rows = page["records"].as_array().unwrap();
        assert_eq!(rows.len(), if page_number == 0 { 50 } else { 6 });
        received.extend(
            rows.iter()
                .map(|row| row["key"].as_str().unwrap().to_owned()),
        );
        after = page["nextAfter"].as_str().map(str::to_owned);
        assert_eq!(after.is_some(), page_number == 0);
    }
    assert_eq!(received, ids);
    let checkpoint = &ids[1];
    let remainder = owner
        .workspace_collection("plugin-settings", Some(checkpoint))
        .await
        .unwrap();
    let first = remainder["records"][0]["key"].as_str().unwrap();
    assert_eq!(first, ids[2]);

    let publisher = uuid::Uuid::new_v4();
    let key = uuid::Uuid::new_v4();
    let candidate = uuid::Uuid::new_v4();
    let generation = (uuid::Uuid::new_v4().as_u128() & 0x3fff_ffff_ffff_ffff) as i64 + 1;
    let hash = "a".repeat(64);
    sqlx::query("INSERT INTO cloud_publishers(id,namespace,status) VALUES($1,$2,'approved')")
        .bind(publisher)
        .bind(&namespace)
        .execute(&f.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO cloud_publisher_keys(id,publisher_id,key_id,public_key,fingerprint,status,requested_by) VALUES($1,$2,$3,$4,$5,'approved',$6)")
        .bind(key).bind(publisher).bind(format!("{namespace}-key")).bind("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=").bind(uuid::Uuid::new_v4().simple().to_string().repeat(2)).bind(&f.users[0].id).execute(&f.pool).await.unwrap();
    // Catalog IDs follow manifest grammar; workspace sync deliberately keeps
    // its separate stricter key grammar. These rows are synthetic metadata.
    let single = format!("{namespace}foo");
    let repeated_dots = format!("b..{namespace}");
    let parent_shard = format!("ab..{namespace}");
    let mut catalog_ids = ids.clone();
    catalog_ids.extend([single.clone(), repeated_dots.clone(), parent_shard.clone()]);
    let uploads: Vec<_> = (0..catalog_ids.len())
        .map(|_| uuid::Uuid::new_v4())
        .collect();
    let releases: Vec<_> = (0..catalog_ids.len())
        .map(|_| uuid::Uuid::new_v4())
        .collect();
    sqlx::query("INSERT INTO cloud_upload_sessions(id,publisher_id,requested_by,idempotency_key,expected_size,expected_sha256,object_key,status,expires_at) SELECT id,$2,$3,id::text,100,$4,'ordering/uploads/'||id::text,'pending',now()+interval '1 hour' FROM unnest($1::uuid[]) AS fixture(id)")
        .bind(&uploads).bind(publisher).bind(&f.users[0].id).bind(&hash).execute(&f.pool).await.unwrap();
    sqlx::query("INSERT INTO cloud_plugin_releases(id,publisher_id,upload_id,publisher_key_id,plugin_id,version,target,status,requested_by) SELECT id,$4,upload_id,$5,plugin_id,'1.0.0','any','awaiting_verification',$6 FROM unnest($1::uuid[],$2::uuid[],$3::text[]) AS fixture(id,upload_id,plugin_id)")
        .bind(&releases).bind(&uploads).bind(&catalog_ids).bind(publisher).bind(key).bind(&f.users[0].id).execute(&f.pool).await.unwrap();
    sqlx::query("INSERT INTO cloud_snapshot_candidates(id,status,root_fingerprint,inventory_sha256,release_count,file_count) VALUES($1,'candidate',$2,$2,$3,1)")
        .bind(candidate).bind(&hash).bind(catalog_ids.len() as i32).execute(&f.pool).await.unwrap();
    sqlx::query("INSERT INTO cloud_registry_generations(id,generation,candidate_id,inventory_sha256,manifest_sha256,root_fingerprint,index_key_id,release_count,file_count,status) VALUES($1,$2,$3,$4,$4,$4,'ordering-test-index',$5,1,'superseded')")
        .bind(uuid::Uuid::new_v4()).bind(generation).bind(candidate).bind(&hash).bind(catalog_ids.len() as i32).execute(&f.pool).await.unwrap();
    sqlx::query("INSERT INTO cloud_registry_generation_releases(generation,plugin_id,version,target,release_id,sha256,size,object_key,version_id) SELECT $1,plugin_id,'1.0.0','any',release_id,$4,100,'ordering/releases/'||release_id::text,'v1' FROM unnest($2::text[],$3::uuid[]) AS fixture(plugin_id,release_id)")
        .bind(generation).bind(&catalog_ids).bind(&releases).bind(&hash).execute(&f.pool).await.unwrap();
    let catalog = cloud_client::CloudCatalog::new(&f.origin).unwrap();
    let mut received = Vec::new();
    let mut after: Option<String> = None;
    for page_number in 0..2 {
        let page = catalog
            .plugins_filtered(
                &format!("{namespace}."),
                after.as_deref(),
                Some(generation),
                Some("x86_64-unknown-linux-gnu"),
            )
            .await
            .unwrap();
        assert_eq!(page["generation"], generation);
        let items = page["items"].as_array().unwrap();
        assert_eq!(items.len(), if page_number == 0 { 50 } else { 6 });
        received.extend(
            items
                .iter()
                .map(|item| item["id"].as_str().unwrap().to_owned()),
        );
        after = page["nextCursor"].as_str().map(str::to_owned);
        assert_eq!(after.is_some(), page_number == 0);
    }
    assert_eq!(received, ids);
    let remainder = catalog
        .plugins_filtered(
            &format!("{namespace}."),
            Some(checkpoint),
            Some(generation),
            None,
        )
        .await
        .unwrap();
    assert_eq!(remainder["items"][0]["id"], ids[2]);
    for id in [&single, &repeated_dots, &parent_shard] {
        assert_eq!(
            catalog.plugin(id, Some(generation)).await.unwrap()["plugin"]["id"],
            *id
        );
        assert!(catalog
            .plugins_filtered("", Some(id), Some(generation), None)
            .await
            .is_ok());
        assert!(!crate::sync::valid_plugin_id(id));
    }
    let canonical_path =
        format!("/api/v1/registry/generations/{generation}/files/index/{parent_shard}.json");
    let parent_summary = catalog
        .plugin_summaries(Some(&parent_shard), None, Some(generation), None)
        .await
        .unwrap();
    assert_eq!(parent_summary.items.len(), 1);
    assert_eq!(parent_summary.items[0].id, parent_shard);
    assert_eq!(parent_summary.items[0].metadata_path, canonical_path);
    let parent_versions = catalog
        .plugin_version_page(&parent_shard, None, Some(generation), None)
        .await
        .unwrap();
    assert_eq!(parent_versions.metadata_path, canonical_path);
    assert_eq!(parent_versions.items.len(), 1);
    for id in ["hotkeys", "textintel"] {
        // The synthetic generation does not contain these legacy IDs; 404,
        // rather than 400, proves that the route accepts their grammar.
        let path = format!("/api/v2/catalog/plugins/{id}?generation={generation}");
        assert_eq!(
            request(&f, None, "GET", &path, json!({}), None)
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
    }
    for path in [
        "/api/v2/catalog/plugins/1bad",
        "/api/v2/catalog/plugins?after=1bad",
    ] {
        assert_eq!(
            request(&f, None, "GET", path, json!({}), None)
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    // The additive bounded summary endpoint pins the complete query in its cursor.
    let q = format!("{namespace}.");
    let prefix = format!("/api/v2/catalog/plugin-summaries?generation={generation}&q={q}&target=x86_64-unknown-linux-gnu");
    let first = request(&f, None, "GET", &prefix, json!({}), None).await;
    assert_eq!(first.status(), StatusCode::OK);
    let first = json_body(first).await;
    assert_eq!(first["generation"], generation.to_string());
    assert_eq!(first["items"].as_array().unwrap().len(), 50);
    let cursor = first["nextCursor"].as_str().unwrap();
    let changed = format!("/api/v2/catalog/plugin-summaries?generation={generation}&q=other&target=x86_64-unknown-linux-gnu&cursor={cursor}");
    assert_eq!(
        request(&f, None, "GET", &changed, json!({}), None)
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    let second = request(
        &f,
        None,
        "GET",
        &format!("{prefix}&cursor={cursor}"),
        json!({}),
        None,
    )
    .await;
    assert_eq!(second.status(), StatusCode::OK);
    let second = json_body(second).await;
    assert_eq!(second["items"].as_array().unwrap().len(), 6);
    assert!(second["nextCursor"].is_null());
    let got: Vec<_> = first["items"]
        .as_array()
        .unwrap()
        .iter()
        .chain(second["items"].as_array().unwrap())
        .map(|item| item["id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(got, ids);

    // Add many releases of one ID. A second page must observe current yank
    // state without dropping or repeating immutable generation members.
    let plugin = &ids[0];
    let additional: Vec<String> = (1..=70)
        .map(|i| {
            let prefix = format!("1.0.{i}+");
            format!("{prefix}{}", "a".repeat(128 - prefix.len()))
        })
        .collect();
    let long_target = "x".repeat(128);
    let more_uploads: Vec<_> = additional.iter().map(|_| uuid::Uuid::new_v4()).collect();
    let more_releases: Vec<_> = additional.iter().map(|_| uuid::Uuid::new_v4()).collect();
    sqlx::query("INSERT INTO cloud_upload_sessions(id,publisher_id,requested_by,idempotency_key,expected_size,expected_sha256,object_key,status,expires_at) SELECT id,$2,$3,id::text,100,$4,'ordering/uploads/'||id::text,'pending',now()+interval '1 hour' FROM unnest($1::uuid[]) AS fixture(id)")
        .bind(&more_uploads).bind(publisher).bind(&f.users[0].id).bind(&hash).execute(&f.pool).await.unwrap();
    sqlx::query("INSERT INTO cloud_plugin_releases(id,publisher_id,upload_id,publisher_key_id,plugin_id,version,target,status,requested_by) SELECT id,$4,upload_id,$5,$6,version,$8,'awaiting_verification',$7 FROM unnest($1::uuid[],$2::uuid[],$3::text[]) AS fixture(id,upload_id,version)")
        .bind(&more_releases).bind(&more_uploads).bind(&additional).bind(publisher).bind(key).bind(plugin).bind(&f.users[0].id).bind(&long_target).execute(&f.pool).await.unwrap();
    sqlx::query("INSERT INTO cloud_registry_generation_releases(generation,plugin_id,version,target,release_id,sha256,size,object_key,version_id) SELECT $1,$2,version,$6,release_id,$5,100,'ordering/releases/'||release_id::text,'v1' FROM unnest($3::text[],$4::uuid[]) AS fixture(version,release_id)")
        .bind(generation).bind(plugin).bind(&additional).bind(&more_releases).bind(&hash).bind(&long_target).execute(&f.pool).await.unwrap();
    // Advisory fields satisfy the actual moderation validator, including its
    // 1024-byte message limit; JSON escaping and the multibyte URL make a
    // valid row large enough to exercise the page byte boundary.
    let mut advisory_releases = more_releases.clone();
    advisory_releases.push(releases[0]);
    let advisory_message = "\"".repeat(1024);
    let advisory_url = format!("https://example.test/{}", "𐀀".repeat(2027));
    assert!(
        crate::moderation::valid_advisory("low", &advisory_message, Some(&advisory_url)).is_ok()
    );
    sqlx::query("INSERT INTO cloud_release_advisories(release_id,severity,message,url,created_by) SELECT id,'low',$2,$3,'fixture' FROM unnest($1::uuid[]) AS fixture(id)")
        .bind(&advisory_releases).bind(&advisory_message).bind(&advisory_url).execute(&f.pool).await.unwrap();
    let version_prefix = format!("/api/v2/catalog/plugins/{plugin}/version-pages?generation={generation}&target={long_target}");
    let first_versions = request(&f, None, "GET", &version_prefix, json!({}), None).await;
    assert_eq!(first_versions.status(), StatusCode::OK);
    let first_versions = json_body(first_versions).await;
    let first_len = first_versions["items"].as_array().unwrap().len();
    assert!(
        (1..50).contains(&first_len),
        "the byte limit must end this page before the item limit"
    );
    assert!(serde_json::to_vec(&first_versions).unwrap().len() <= 512 * 1024);
    let cursor = first_versions["nextCursor"].as_str().unwrap();
    let changed = format!("/api/v2/catalog/plugins/{plugin}/version-pages?generation={generation}&target=other&cursor={cursor}");
    assert_eq!(
        request(&f, None, "GET", &changed, json!({}), None)
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    sqlx::query(
        "INSERT INTO cloud_release_yanks(release_id,yanked_by,reason) VALUES($1,'fixture','test')",
    )
    .bind(more_releases[8])
    .execute(&f.pool)
    .await
    .unwrap();
    let second_versions = request(
        &f,
        None,
        "GET",
        &format!("{version_prefix}&cursor={cursor}"),
        json!({}),
        None,
    )
    .await;
    assert_eq!(second_versions.status(), StatusCode::OK);
    let second_versions = json_body(second_versions).await;
    assert_eq!(
        second_versions["items"].as_array().unwrap().len(),
        71 - first_len
    );
    assert!(serde_json::to_vec(&second_versions).unwrap().len() <= 512 * 1024);
    assert!(second_versions["nextCursor"].is_null());
    let rows: Vec<_> = first_versions["items"]
        .as_array()
        .unwrap()
        .iter()
        .chain(second_versions["items"].as_array().unwrap())
        .collect();
    assert_eq!(rows.len(), 71);
    assert!(rows
        .windows(2)
        .all(|v| v[0]["version"].as_str().unwrap() < v[1]["version"].as_str().unwrap()));
    assert_eq!(
        rows.iter()
            .find(|v| v["version"].as_str().unwrap().starts_with("1.0.9+"))
            .unwrap()["yanked"],
        true
    );
}

#[tokio::test]
#[ignore = "requires a dedicated PostgreSQL database in CLOUD_TEST_DATABASE_URL"]
async fn workspace_selection_accepts_legacy_ids_and_release_version_boundaries() {
    let f = fixture().await;
    let version = format!("1.0.0+{}", "a".repeat(122));
    assert_eq!(version.len(), 128);
    let valid = json!({"schemaVersion":1,"pluginId":"hotkeys","version":version,"enabled":false});
    let response = request(
        &f,
        Some(0),
        "PUT",
        "/api/v2/sync/collections/plugin-selections/hotkeys",
        valid.clone(),
        Some(("if-none-match", "*")),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(json_body(response).await["value"], valid);
    let returned = request(
        &f,
        Some(0),
        "GET",
        "/api/v2/sync/collections/plugin-selections/hotkeys",
        json!({}),
        None,
    )
    .await;
    assert_eq!(returned.status(), StatusCode::OK);
    assert_eq!(json_body(returned).await["value"], valid);
    let previous_v2_key =
        json!({"schemaVersion":1,"pluginId":"1.old","version":"1.0.0","enabled":false});
    assert_eq!(
        request(
            &f,
            Some(0),
            "PUT",
            "/api/v2/sync/collections/plugin-selections/1.old",
            previous_v2_key,
            Some(("if-none-match", "*"))
        )
        .await
        .status(),
        StatusCode::CREATED
    );
    let invalid_id = json!({"schemaVersion":1,"pluginId":"1bad","version":"1.0.0","enabled":false});
    assert_eq!(
        request(
            &f,
            Some(0),
            "PUT",
            "/api/v2/sync/collections/plugin-selections/1bad",
            invalid_id,
            Some(("if-none-match", "*"))
        )
        .await
        .status(),
        StatusCode::BAD_REQUEST
    );
    let too_long = format!("1.0.0+{}", "a".repeat(123));
    let invalid_version =
        json!({"schemaVersion":1,"pluginId":"textintel","version":too_long,"enabled":false});
    assert_eq!(
        request(
            &f,
            Some(0),
            "PUT",
            "/api/v2/sync/collections/plugin-selections/textintel",
            invalid_version,
            Some(("if-none-match", "*"))
        )
        .await
        .status(),
        StatusCode::BAD_REQUEST
    );
}
