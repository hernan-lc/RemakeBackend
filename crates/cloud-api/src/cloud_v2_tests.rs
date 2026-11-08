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
