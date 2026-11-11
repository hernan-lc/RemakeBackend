mod support;

use devtools_core::binary::BinaryManager;
use devtools_core::database::DatabaseManager;
use std::fs;
use std::net::TcpListener;

#[tokio::test]
async fn a_port_held_by_another_server_is_neither_reused_nor_touched() {
    let dir = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let env = support::installed_env(dir.path(), port);
    let db = DatabaseManager::new(env.clone());
    let err = db.up().await.unwrap_err().to_string();
    assert!(err.contains("did not start"), "{err}");
    // It did not start anything, did not write a pid file and did not initialize a cluster.
    assert!(!env.pid_file().exists());
    assert!(!env.data_dir().join("PG_VERSION").exists());
    // The foreign listener is still there.
    assert!(std::net::TcpStream::connect(("127.0.0.1", port)).is_ok());
    drop(listener);
}

#[tokio::test]
async fn status_of_an_uninstalled_database_is_a_clean_report_not_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let env = support::env_in(dir.path(), support::free_port());
    let status = DatabaseManager::new(env).status().await.unwrap();
    assert!(!status.installed && !status.running && status.healthy.is_none());
}

#[test]
fn modifying_the_backend_never_changes_the_installed_binary_or_its_hash() {
    let dir = tempfile::tempdir().unwrap();
    let env = support::installed_env(dir.path(), 5999);
    let bin = BinaryManager::new(env.install_dir(), env.root.clone());
    let before = bin.verify(true).unwrap();
    // "Modify the backend": touch source files and a fake target dir next to the install.
    fs::create_dir_all(dir.path().join("crates/cloud-api/src")).unwrap();
    fs::write(
        dir.path().join("crates/cloud-api/src/main.rs"),
        b"fn main() { println!(\"v2\"); }",
    )
    .unwrap();
    fs::create_dir_all(dir.path().join("target/debug")).unwrap();
    fs::write(dir.path().join("target/debug/cloud-api"), b"rebuilt").unwrap();
    let after = bin.verify(true).unwrap();
    assert_eq!(before, after);
    assert_eq!(before.binary_sha256, after.binary_sha256);
}

#[test]
fn installing_twice_needs_force_and_a_failed_upgrade_keeps_the_working_install() {
    let dir = tempfile::tempdir().unwrap();
    let env = support::installed_env(dir.path(), 5999);
    let db = DatabaseManager::new(env.clone());
    let opts = devtools_core::database::InstallOptions {
        source: devtools_core::database::InstallSource::Prebuilt(dir.path().join("release")),
        force: false,
        upgrade: false,
        slim: false,
        features: None,
        accept_data_risk: false,
    };
    assert!(db
        .install(&opts)
        .unwrap_err()
        .to_string()
        .contains("already installed"));
    // An upgrade from a directory that is not pgrust is refused and changes nothing.
    let bad = dir.path().join("not-pgrust");
    fs::create_dir_all(&bad).unwrap();
    let before = db.binaries().verify(true).unwrap();
    let upgrade = devtools_core::database::InstallOptions {
        source: devtools_core::database::InstallSource::Prebuilt(bad),
        force: true,
        upgrade: true,
        ..opts
    };
    assert!(db.install(&upgrade).is_err());
    assert_eq!(db.binaries().verify(true).unwrap(), before);
}

#[tokio::test]
async fn reset_refuses_without_yes_and_never_touches_the_dev_database() {
    let dir = tempfile::tempdir().unwrap();
    let env = support::installed_env(dir.path(), support::free_port());
    let db = DatabaseManager::new(env);
    assert!(db
        .reset_test_database(false)
        .await
        .unwrap_err()
        .to_string()
        .contains("--yes"));
    // With --yes it still refuses when the server is not running (nothing to reset).
    assert!(db
        .reset_test_database(true)
        .await
        .unwrap_err()
        .to_string()
        .contains("not running"));
}
