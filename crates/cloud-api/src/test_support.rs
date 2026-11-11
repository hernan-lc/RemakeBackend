//! Shared helpers for the database tests.
//!
//! `migrated_url()` is the connection URL of this test's private database, cloned
//! from a template that already holds every migration the fixtures apply.
//! Fixtures still run their own migrations afterwards; on a migrated clone those
//! are no-ops. `empty_url()` is for tests that must migrate a blank database
//! themselves.
use crate::{
    readiness,
    test_db::{self, Prepare, Template},
};
use better_auth::{
    context::AuthContext,
    database::{SqlxDbAdapter, SqlxSecondaryStorage},
};
use better_auth_core::{
    migration::SqlDialect,
    options::{AuthOptions, BaseUrl},
};
use sha2::{Digest, Sha256};
use sqlx::postgres::PgPoolOptions;
use std::{future::Future, pin::Pin, sync::Arc};

pub fn migrated_url() -> String {
    test_db::url(Some(template()))
}

pub fn empty_url() -> String {
    test_db::url(None)
}

fn template() -> Template {
    let mut hash = Sha256::new();
    for migration in readiness::MIGRATOR.iter() {
        hash.update(migration.version.to_be_bytes());
        hash.update(&*migration.checksum);
    }
    // The auth library's schema comes from the pinned dependency, and what the
    // template contains comes from this file: a change to either rebuilds it.
    hash.update(include_str!("../../../Cargo.lock"));
    hash.update(include_str!("test_support.rs"));
    let key: String = hash
        .finalize()
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Template {
        key,
        prepare: prepare as Prepare,
    }
}

fn prepare(url: String) -> Pin<Box<dyn Future<Output = ()> + Send>> {
    Box::pin(async move {
        let adapter = Arc::new(SqlxDbAdapter::postgres(&url).await.unwrap());
        let secondary = Arc::new(SqlxSecondaryStorage::postgres(&url).await.unwrap());
        let context = AuthContext::new(
            AuthOptions {
                secret: "test-only-secret-32-bytes-long!!".into(),
                base_url: Some(BaseUrl::Static("https://cloud.example.test".into())),
                trusted_origins: vec!["https://cloud.example.test".into()],
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
            .max_connections(1)
            .connect(&url)
            .await
            .unwrap();
        readiness::MIGRATOR.run(&pool).await.unwrap();
        pool.close().await;
    })
}

static INTEGRATION_ENV: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// `INTEGRATION_SECRET_KEY` is read from the process environment, which every
/// test thread shares. A test that sets it, or that asserts it is absent, holds
/// this guard for its whole body. The variable is cleared on entry, so a test
/// that panicked while holding it cannot leak it into the next one.
pub async fn integration_env() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = INTEGRATION_ENV.lock().await;
    std::env::remove_var("INTEGRATION_SECRET_KEY");
    guard
}
