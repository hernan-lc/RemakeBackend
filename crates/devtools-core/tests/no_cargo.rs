//! `db up` must never run cargo. A `cargo` shim that records every call and fails is put first on
//! PATH, then `up` is driven through every path: not installed, installed but the server never
//! answers (start, time out, clean up). The shim must never have been called.
//!
//! This file holds a single test because it changes PATH for the whole process.

mod support;

use devtools_core::database::DatabaseManager;
use std::fs;

#[tokio::test]
async fn db_up_never_invokes_cargo() {
    let dir = tempfile::tempdir().unwrap();
    let shim_dir = dir.path().join("shim");
    fs::create_dir_all(&shim_dir).unwrap();
    let calls = dir.path().join("cargo-calls.log");
    support::write_script(
        &shim_dir.join("cargo"),
        &format!(
            "#!/bin/sh\necho \"cargo $@\" >> '{}'\nexit 1\n",
            calls.display()
        ),
    );
    let path = format!("{}:{}", shim_dir.display(), std::env::var("PATH").unwrap());
    std::env::set_var("PATH", path);
    std::env::set_var("REMAKE_DEV_START_TIMEOUT_SECS", "2");

    // 1. Not installed: an instruction to install, and no build.
    let env = support::env_in(dir.path(), support::free_port());
    let err = DatabaseManager::new(env)
        .up()
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("db install"), "{err}");

    // 2. Installed, but the server never answers SQL: it is started, times out and is stopped.
    let env = support::installed_env(dir.path(), support::free_port());
    let db = DatabaseManager::new(env.clone());
    let err = db.up().await.unwrap_err();
    assert!(format!("{err:#}").contains("timed out"), "{err:#}");
    assert!(
        db.managed_pid().is_none(),
        "a failed start must not leave a managed server behind"
    );
    assert!(!env.pid_file().exists());
    assert!(
        env.data_dir().join("PG_VERSION").is_file(),
        "initdb of the install ran"
    );

    assert!(
        !calls.exists(),
        "cargo was invoked: {}",
        fs::read_to_string(&calls).unwrap_or_default()
    );
}
