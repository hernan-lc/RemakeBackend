//! Per-test PostgreSQL databases, cloned from a migrated template.
//!
//! `CLOUD_TEST_DATABASE_URL` names an administrative database. Each test thread
//! asks for its own database here and gets a private clone, so tests never share
//! rows, rate-limit counters or fixed identifiers and can run in parallel.
//!
//! * `url(Some(template))` clones a template that is built once and reused by
//!   every later run until its key changes (the key covers the schema).
//! * `url(None)` creates an empty database, for tests that migrate it themselves.
//!
//! A test thread always receives the same database for every call, so a test
//! may ask twice and reconnect to its own data. The clone is dropped when the
//! thread ends; clones left by a killed run are swept on the next run.
//!
//! `CLOUD_TEST_ISOLATE=0` returns the administrative URL unchanged (the old,
//! shared and serial behavior), for servers where `CREATE DATABASE` is refused.
//!
//! This file is also compiled into `tests/startup.rs`, so it must not use any
//! other module of the crate.

use sqlx::{Connection, PgConnection};
use std::{
    cell::RefCell,
    future::Future,
    pin::Pin,
    sync::Once,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const ADMIN_ENV: &str = "CLOUD_TEST_DATABASE_URL";
const ISOLATE_ENV: &str = "CLOUD_TEST_ISOLATE";
// Every database this module creates starts with `cloud_test`, which is also
// what the recovery drill requires of a test database name.
const TEMPLATE_PREFIX: &str = "cloud_test_tpl_";
const CLONE_PREFIX: &str = "cloud_test_t_";
const LOCK_KEY: i64 = 0x636c_6f75_6474_6573;
const STALE_AFTER: Duration = Duration::from_secs(30 * 60);

/// Brings an empty database to the shape every migrated clone starts from.
pub type Prepare = fn(String) -> Pin<Box<dyn Future<Output = ()> + Send>>;

/// A template: a content key plus the function that builds it.
pub struct Template {
    pub key: String,
    pub prepare: Prepare,
}

struct Lease {
    admin: String,
    name: String,
    url: String,
}

impl Drop for Lease {
    fn drop(&mut self) {
        let (admin, name) = (self.admin.clone(), self.name.clone());
        // A failed drop is harmless: the next run sweeps stale clones.
        let _ = run(async move {
            let mut conn = PgConnection::connect(&admin).await?;
            sqlx::query(&format!("DROP DATABASE IF EXISTS {} WITH (FORCE)", quote(&name)))
                .execute(&mut conn)
                .await?;
            Ok::<_, sqlx::Error>(())
        });
    }
}

thread_local! {
    static CURRENT: RefCell<Option<Lease>> = const { RefCell::new(None) };
}

/// Returns the connection URL of this test's private database.
pub fn url(template: Option<Template>) -> String {
    let admin = std::env::var(ADMIN_ENV).unwrap_or_else(|_| {
        panic!("set {ADMIN_ENV} to a disposable PostgreSQL database (or run `cargo remake test`)")
    });
    if std::env::var(ISOLATE_ENV).is_ok_and(|value| value == "0") {
        return admin;
    }
    CURRENT.with(|current| {
        let mut current = current.borrow_mut();
        if let Some(lease) = current.as_ref() {
            return lease.url.clone();
        }
        let lease = run(create(admin.clone(), template));
        let url = lease.url.clone();
        *current = Some(lease);
        url
    })
}

/// Runs a future to completion on its own thread and runtime, so the helper
/// works from synchronous tests and from inside a running `#[tokio::test]`.
fn run<F>(future: F) -> F::Output
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test database runtime")
            .block_on(future)
    })
    .join()
    .expect("test database helper thread panicked")
}

/// `LIKE` pattern for a literal prefix: `_` is a wildcard in `LIKE`, so escape it.
fn like_prefix(prefix: &str) -> String {
    format!("{}%", prefix.replace('\\', "\\\\").replace('_', "\\_"))
}

fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn with_database(admin: &str, database: &str) -> String {
    let mut url = url::Url::parse(admin).expect("CLOUD_TEST_DATABASE_URL must be a URL");
    url.set_path(&format!("/{database}"));
    url.to_string()
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

async fn create(admin: String, template: Option<Template>) -> Lease {
    let mut conn = PgConnection::connect(&admin)
        .await
        .unwrap_or_else(|error| panic!("cannot reach {ADMIN_ENV}: {error}"));
    static SWEEP: Once = Once::new();
    let first = !SWEEP.is_completed();
    SWEEP.call_once(|| {});
    if first {
        sweep_stale_clones(&mut conn).await;
    }
    let source = match template {
        Some(template) => Some(ensure_template(&mut conn, &admin, template).await),
        None => None,
    };
    let name = format!(
        "{CLONE_PREFIX}{}_{}",
        now_secs(),
        &uuid::Uuid::new_v4().simple().to_string()[..12]
    );
    let statement = match &source {
        Some(source) => format!("CREATE DATABASE {} TEMPLATE {}", quote(&name), quote(source)),
        None => format!("CREATE DATABASE {}", quote(&name)),
    };
    sqlx::query(&statement).execute(&mut conn).await.unwrap_or_else(|error| {
        panic!(
            "cannot create a test database ({error}); grant CREATEDB or set {ISOLATE_ENV}=0 \
             to share {ADMIN_ENV}"
        )
    });
    let url = with_database(&admin, &name);
    Lease { admin, name, url }
}

async fn sweep_stale_clones(conn: &mut PgConnection) {
    let names: Vec<String> = sqlx::query_scalar("SELECT datname FROM pg_database WHERE datname LIKE $1")
        .bind(like_prefix(CLONE_PREFIX))
        .fetch_all(&mut *conn)
        .await
        .unwrap_or_default();
    for name in names {
        let created = name
            .strip_prefix(CLONE_PREFIX)
            .and_then(|rest| rest.split('_').next())
            .and_then(|secs| secs.parse::<u64>().ok());
        if created.is_some_and(|created| now_secs().saturating_sub(created) > STALE_AFTER.as_secs()) {
            let _ = sqlx::query(&format!("DROP DATABASE IF EXISTS {} WITH (FORCE)", quote(&name)))
                .execute(&mut *conn)
                .await;
        }
    }
}

/// Returns the name of the template for `template.key`, building it first if it
/// does not exist yet. Processes serialize on an advisory lock.
async fn ensure_template(conn: &mut PgConnection, admin: &str, template: Template) -> String {
    let name = format!("{TEMPLATE_PREFIX}{}", template.key);
    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(LOCK_KEY)
        .execute(&mut *conn)
        .await
        .expect("template lock");
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_database WHERE datname = $1)")
        .bind(&name)
        .fetch_one(&mut *conn)
        .await
        .expect("template lookup");
    if !exists {
        // Holding the lock, every other template (stale keys, builds left by a
        // killed process) is unreachable and safe to remove.
        let others: Vec<String> =
            sqlx::query_scalar("SELECT datname FROM pg_database WHERE datname LIKE $1 AND datname <> $2")
                .bind(like_prefix(TEMPLATE_PREFIX))
                .bind(&name)
                .fetch_all(&mut *conn)
                .await
                .unwrap_or_default();
        for other in others {
            let _ = sqlx::query(&format!("DROP DATABASE IF EXISTS {} WITH (FORCE)", quote(&other)))
                .execute(&mut *conn)
                .await;
        }
        let building = format!("{TEMPLATE_PREFIX}build_{}", uuid::Uuid::new_v4().simple());
        sqlx::query(&format!("CREATE DATABASE {}", quote(&building)))
            .execute(&mut *conn)
            .await
            .expect("create template build database");
        (template.prepare)(with_database(admin, &building)).await;
        // Pools the builder dropped may still be closing; nothing may stay
        // connected to a database that is renamed and then cloned.
        sqlx::query("SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = $1 AND pid <> pg_backend_pid()")
            .bind(&building)
            .execute(&mut *conn)
            .await
            .expect("disconnect template builder");
        sqlx::query(&format!("ALTER DATABASE {} RENAME TO {}", quote(&building), quote(&name)))
            .execute(&mut *conn)
            .await
            .expect("publish template");
    }
    sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(LOCK_KEY)
        .execute(&mut *conn)
        .await
        .expect("template unlock");
    name
}
