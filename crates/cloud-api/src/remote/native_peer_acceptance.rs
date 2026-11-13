//! Launch the real native SDK against this disposable PostgreSQL-backed server.
//! Enabled only for the explicit cross-repository acceptance runner.
use super::*;
use tokio::io::AsyncBufReadExt;

#[tokio::test]
#[ignore = "requires scripts/collaboration-cloud-lab.py from the native app checkout"]
async fn native_app_cloud_onboarding_reaches_real_host() {
    run(false).await;
}

#[tokio::test]
#[ignore = "requires scripts/collaboration-cloud-lab.py from the native app checkout"]
async fn native_app_cloud_falls_back_to_native_relay() {
    run(true).await;
}

async fn run(relay_only: bool) {
    let executable = std::env::var_os("TIKTOOLS_PEER_ACCEPTANCE_TEST_BINARY")
        .expect("native test executable must be supplied by the isolated runner");
    assert!(std::path::Path::new(&executable).is_absolute());
    let f = fixture_with_peer(
        2,
        crate::remote::peers::PeerConfig {
            relay_only,
            ..Default::default()
        },
    )
    .await;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("native-fixture.json");
    let control = directory.path().join("cloud-control");
    std::fs::write(&control, "online").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&control, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    let upstream = f
        .ws_url
        .strip_suffix("/api/v3/remote/socket")
        .unwrap()
        .replacen("ws://", "http://", 1);
    let required =
        |name| std::env::var_os(name).expect("runner must provide isolated TLS material");
    let mut proxy = tokio::process::Command::new("python3")
        .arg(required("TIKTOOLS_PEER_ACCEPTANCE_TLS_PROXY"))
        .arg("--upstream")
        .arg(upstream)
        .arg("--cert")
        .arg(required("TIKTOOLS_PEER_ACCEPTANCE_TLS_CERT"))
        .arg("--key")
        .arg(required("TIKTOOLS_PEER_ACCEPTANCE_TLS_KEY"))
        .arg("--control")
        .arg(&control)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut reader = tokio::io::BufReader::new(proxy.stdout.take().unwrap());
    let mut line = String::new();
    tokio::time::timeout(Duration::from_secs(5), reader.read_line(&mut line))
        .await
        .unwrap()
        .unwrap();
    let port: u16 = line
        .trim()
        .parse()
        .expect("TLS fixture must publish its loopback port");
    let origin = format!("https://127.0.0.1:{port}");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(&path).unwrap();
    serde_json::to_writer(
        file,
        &json!({
            "origin":origin,"browserOrigin":ORIGIN,"cloudControlPath":control,
            "ownerCookie":f.users[0].cookie,"moderatorCookie":f.users[1].cookie
        }),
    )
    .unwrap();
    let child = tokio::process::Command::new(executable)
        .args([
            "--ignored",
            "--exact",
            if relay_only {
                "cloud_onboarding_uses_relay_when_webrtc_unavailable"
            } else {
                "cloud_onboarding_reaches_the_real_native_host"
            },
            "--nocapture",
        ])
        .env("TIKTOOLS_PEER_ACCEPTANCE_CONFIG", &path)
        .kill_on_drop(true)
        .output();
    let output = tokio::time::timeout(Duration::from_secs(120), child)
        .await
        .expect("native fixture must finish before its deadline")
        .unwrap();
    assert!(
        output.status.success(),
        "native SDK acceptance failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if relay_only {
        let tunnels: i64 =
            sqlx::query_scalar("SELECT count(*) FROM cloud_remote_relay_tunnels WHERE answered")
                .fetch_one(&f.pool)
                .await
                .unwrap();
        assert!(
            tunnels > 0,
            "the normal native SDK must establish an authenticated relay tunnel"
        );
        let commands: i64 = sqlx::query_scalar("SELECT count(*) FROM cloud_remote_commands")
            .fetch_one(&f.pool)
            .await
            .unwrap();
        assert_eq!(
            commands, 0,
            "native relay RPC must never execute as a backend command"
        );
    }
    proxy.kill().await.unwrap();
    proxy.wait().await.unwrap();
}
