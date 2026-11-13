use super::*;
#[path = "native_relay_tests.rs"]
mod native_relay_tests;
#[path = "rtc_signaling_tests.rs"]
mod rtc_signaling_tests;

async fn host(f: &Fixture) -> (String, String, SigningKey) {
    let (status, _, host) = call(
        f,
        Some(0),
        "POST",
        "/api/v3/remote/hosts",
        Some(json!({"name":"Peer host","protocolVersion":1})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let id = host["id"].as_str().unwrap().to_owned();
    let signing = key();
    let device = enroll(f, 0, "Host", "host", Some(&id), &signing).await;
    (id, device["id"].as_str().unwrap().to_owned(), signing)
}

fn endpoint_id(key: &SigningKey) -> String {
    key.verifying_key()
        .to_bytes()
        .iter()
        .map(|v| format!("{v:02x}"))
        .collect()
}

async fn publish(
    f: &Fixture,
    user: usize,
    device: &str,
    signing: &SigningKey,
    peer: &SigningKey,
    revision: i64,
    bad_proof: bool,
) -> (StatusCode, Value, Value) {
    let binding = json!({"deviceId":device,"endpointId":endpoint_id(peer),"addresses":["127.0.0.1:23456"],"expectedRevision":revision});
    let (status, _, challenge) = call(
        f,
        Some(user),
        "POST",
        "/api/v3/remote/challenges",
        Some(json!({"purpose":"peer","binding":binding})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{challenge}");
    let nonce = URL_SAFE_NO_PAD
        .decode(challenge["nonce"].as_str().unwrap())
        .unwrap();
    let message = proof_message("peer", &nonce, challenge["bindingSha256"].as_str().unwrap());
    let body = json!({"challengeId":challenge["id"],"binding":binding,"signature":URL_SAFE_NO_PAD.encode(signing.sign(&message).to_bytes()),"endpointSignature":URL_SAFE_NO_PAD.encode(if bad_proof { signing.sign(b"wrong context").to_bytes() } else { peer.sign(&message).to_bytes() })});
    let (status, _, result) = call(
        f,
        Some(user),
        "POST",
        "/api/v3/remote/peers",
        Some(body.clone()),
    )
    .await;
    (status, result, body)
}

async fn peers(f: &Fixture, user: usize, host: &str) -> Value {
    let (status, _, value) = call(
        f,
        Some(user),
        "GET",
        &format!("/api/v3/remote/hosts/{host}/peers"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    value
}

#[tokio::test]
#[ignore = "requires a dedicated PostgreSQL database in CLOUD_TEST_DATABASE_URL"]
async fn peer_directory_requires_both_keys_cas_and_a_fresh_single_use_challenge() {
    let f = fixture(2).await;
    let (host, device, signing) = host(&f).await;
    let peer = key();
    assert_eq!(
        publish(&f, 0, &device, &signing, &peer, 0, true).await.0,
        StatusCode::UNAUTHORIZED
    );
    let (status, first, replay) = publish(&f, 0, &device, &signing, &peer, 0, false).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["revision"], 1);
    let own_path = format!("/api/v3/remote/peers/{device}");
    let (status, _, own) = call(&f, Some(0), "GET", &own_path, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(own["revision"], 1);
    assert_eq!(
        call(&f, Some(1), "GET", &own_path, None).await.0,
        StatusCode::NOT_FOUND
    );
    let (status,_,_) = call(&f,Some(0),"POST","/api/v3/remote/challenges",Some(json!({"purpose":"peer","binding":{"deviceId":device,"endpointId":endpoint_id(&peer),"addresses":[],"relayUrls":["https://unapproved.example"],"expectedRevision":1}}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        call(&f, Some(0), "POST", "/api/v3/remote/peers", Some(replay))
            .await
            .0,
        StatusCode::GONE
    );
    let (status, next, _) = publish(&f, 0, &device, &signing, &peer, 1, false).await;
    assert_eq!(status, StatusCode::OK, "{next}");
    assert_eq!(next["revision"], 2);
    assert_eq!(
        publish(&f, 0, &device, &signing, &peer, 1, false).await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        publish(&f, 0, &device, &signing, &key(), 2, false).await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        peers(&f, 0, &host).await["items"].as_array().unwrap().len(),
        1
    );
    assert!(peers(&f, 1, &host).await["items"]
        .as_array()
        .unwrap()
        .is_empty());
    let (status, _, _) = call(&f, Some(1), "POST", "/api/v3/remote/challenges", Some(json!({"purpose":"peer","binding":{"deviceId":device,"endpointId":endpoint_id(&peer),"addresses":[],"expectedRevision":2}}))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
#[ignore = "requires a dedicated PostgreSQL database in CLOUD_TEST_DATABASE_URL"]
async fn peer_discovery_tracks_confirmed_grants_revocation_and_expiry() {
    let f = fixture(3).await;
    let (host, device, signing) = host(&f).await;
    let admission = cloud_peer_relay::admission::Admission::new(f.pool.clone());
    let host_peer = key();
    assert!(!admission.allowed(&endpoint_id(&host_peer)).await);
    assert_eq!(
        publish(&f, 0, &device, &signing, &host_peer, 0, false)
            .await
            .0,
        StatusCode::OK
    );
    assert!(admission.allowed(&endpoint_id(&host_peer)).await);
    let controller_key = key();
    let controller_peer = key();
    let controller = enroll(&f, 1, "Controller", "controller", None, &controller_key).await;
    let controller_id = controller["id"].as_str().unwrap();
    let member_path = format!("/api/v3/remote/devices/{controller_id}/peer-hosts");
    assert_eq!(
        call(&f, Some(2), "GET", &member_path, None).await.0,
        StatusCode::NOT_FOUND
    );
    let (_, _, members) = call(&f, Some(1), "GET", &member_path, None).await;
    assert!(members["items"].as_array().unwrap().is_empty());
    let authority_path = format!("/api/v3/remote/hosts/{host}/peer-authorizations");
    assert_eq!(
        call(&f, Some(1), "GET", &authority_path, None).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        publish(
            &f,
            1,
            controller_id,
            &controller_key,
            &controller_peer,
            0,
            false
        )
        .await
        .0,
        StatusCode::OK
    );
    assert!(!admission.allowed(&endpoint_id(&controller_peer)).await);
    let grant = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO cloud_remote_grants(id,host_id,device_id,requester_user_id,role,status,revision) VALUES($1,$2,$3,$4,'observer','pendingHost',1)")
        .bind(grant).bind(host.parse::<uuid::Uuid>().unwrap()).bind(controller_id.parse::<uuid::Uuid>().unwrap()).bind(&f.users[1].id).execute(&f.pool).await.unwrap();
    assert!(peers(&f, 1, &host).await["items"]
        .as_array()
        .unwrap()
        .is_empty());
    sqlx::query("UPDATE cloud_remote_grants SET status='active',confirmed_revision=1 WHERE id=$1")
        .bind(grant)
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(admission.allowed(&endpoint_id(&controller_peer)).await);
    let (status, _, members) = call(&f, Some(1), "GET", &member_path, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(members["items"][0]["id"], host);
    assert_eq!(members["items"][0]["clientKind"], "desktop");

    let (status, _, snapshot) = call(&f, Some(0), "GET", &authority_path, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(snapshot["hostStatus"], "active");
    assert_eq!(
        snapshot["items"][0]["endpointId"],
        endpoint_id(&controller_peer)
    );
    assert_eq!(snapshot["items"][0]["confirmedRevision"], 1);

    assert_eq!(
        peers(&f, 1, &host).await["items"].as_array().unwrap().len(),
        2
    );
    assert!(peers(&f, 2, &host).await["items"]
        .as_array()
        .unwrap()
        .is_empty());
    sqlx::query("UPDATE cloud_remote_devices SET status='revoked',revoked_at=now() WHERE id=$1")
        .bind(controller_id.parse::<uuid::Uuid>().unwrap())
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(peers(&f, 1, &host).await["items"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(!admission.allowed(&endpoint_id(&controller_peer)).await);
    assert_eq!(
        call(&f, Some(1), "GET", &member_path, None).await.0,
        StatusCode::NOT_FOUND
    );
    let (_, _, snapshot) = call(&f, Some(0), "GET", &authority_path, None).await;
    assert_eq!(snapshot["items"][0]["deviceStatus"], "revoked");
    assert_eq!(
        snapshot["items"][0]["endpointId"],
        endpoint_id(&controller_peer)
    );
    assert_eq!(
        peers(&f, 0, &host).await["items"].as_array().unwrap().len(),
        1
    );
    sqlx::query("UPDATE cloud_remote_peer_endpoints SET expires_at=now()-interval '1 second'")
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(peers(&f, 0, &host).await["items"]
        .as_array()
        .unwrap()
        .is_empty());
    // Discovery expires quickly; a previously authorized relay session has its own lease.
    assert!(admission.allowed(&endpoint_id(&host_peer)).await);
    sqlx::query(
        "UPDATE cloud_remote_peer_endpoints SET relay_authorized_until=now()-interval '1 second'",
    )
    .execute(&f.pool)
    .await
    .unwrap();
    assert!(!admission.allowed(&endpoint_id(&host_peer)).await);
}

#[test]
fn relay_configuration_only_accepts_bounded_operator_https_origins() {
    use crate::remote::peers::PeerConfig;
    assert!(PeerConfig::parse(None).unwrap().relay_urls.is_empty());
    assert_eq!(
        PeerConfig::parse(Some("https://relay.example, https://relay.example/".into()))
            .unwrap()
            .relay_urls
            .len(),
        1
    );
    for raw in [
        "http://relay.example",
        "https://user:password@relay.example",
        "https://relay.example/path",
        "https://relay.example?secret=x",
    ] {
        assert!(PeerConfig::parse(Some(raw.into())).is_err());
    }
}

#[tokio::test]
#[ignore = "requires a dedicated PostgreSQL database in CLOUD_TEST_DATABASE_URL"]
async fn controller_can_renew_an_expired_relay_lease_but_not_restore_a_revoked_grant() {
    let f = fixture(2).await;
    let (host, _, _) = host(&f).await;
    let signing = key();
    let peer = key();
    let controller = enroll(&f, 1, "Moderator", "controller", None, &signing).await;
    let device = controller["id"].as_str().unwrap();
    let grant = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO cloud_remote_grants(id,host_id,device_id,requester_user_id,role,status,revision,confirmed_revision) VALUES($1,$2,$3,$4,'observer','active',1,1)")
        .bind(grant).bind(host.parse::<uuid::Uuid>().unwrap()).bind(device.parse::<uuid::Uuid>().unwrap()).bind(&f.users[1].id).execute(&f.pool).await.unwrap();
    let admission = cloud_peer_relay::admission::Admission::new(f.pool.clone());
    assert_eq!(
        publish(&f, 1, device, &signing, &peer, 0, false).await.0,
        StatusCode::OK
    );
    assert!(admission.allowed(&endpoint_id(&peer)).await);
    sqlx::query("UPDATE cloud_remote_peer_endpoints SET expires_at=now()-interval '1 second',relay_authorized_until=now()-interval '1 second' WHERE device_id=$1")
        .bind(device.parse::<uuid::Uuid>().unwrap()).execute(&f.pool).await.unwrap();
    assert!(!admission.allowed(&endpoint_id(&peer)).await);
    // Own metadata remains available after expiry, so a restarted client can
    // recover the revision and prove possession of the same two private keys.
    let (status, _, own) = call(
        &f,
        Some(1),
        "GET",
        &format!("/api/v3/remote/peers/{device}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let revision = own["revision"].as_i64().unwrap();
    assert_eq!(
        publish(&f, 1, device, &signing, &peer, revision, false)
            .await
            .0,
        StatusCode::OK
    );
    assert!(admission.allowed(&endpoint_id(&peer)).await);
    sqlx::query("UPDATE cloud_remote_grants SET status='revoked',revision=revision+1 WHERE id=$1")
        .bind(grant)
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(!admission.allowed(&endpoint_id(&peer)).await);
    // Publication refreshes reachability, never host approval or permissions.
    assert_eq!(
        publish(&f, 1, device, &signing, &peer, revision + 1, false)
            .await
            .0,
        StatusCode::OK
    );
    assert!(!admission.allowed(&endpoint_id(&peer)).await);
}

#[tokio::test]
#[ignore = "requires a dedicated PostgreSQL database in CLOUD_TEST_DATABASE_URL"]
async fn native_host_confirms_the_exact_owner_approved_grant_without_a_relay_socket() {
    let f = fixture(2).await;
    let (host, device, signing) = host(&f).await;
    let controller_key = key();
    let controller = enroll(&f, 1, "Controller", "controller", None, &controller_key).await;
    let grant = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO cloud_remote_grants(id,host_id,device_id,requester_user_id,role,status,revision) VALUES($1,$2,$3,$4,'moderator','pendingHost',1)")
        .bind(grant).bind(host.parse::<uuid::Uuid>().unwrap())
        .bind(controller["id"].as_str().unwrap().parse::<uuid::Uuid>().unwrap())
        .bind(&f.users[1].id).execute(&f.pool).await.unwrap();
    let binding = json!({"deviceId":device,"hostId":host,"grantId":grant,"revision":1});
    let (denied, _, _) = call(
        &f,
        Some(1),
        "POST",
        "/api/v3/remote/challenges",
        Some(json!({"purpose":"peer-grant","binding":binding})),
    )
    .await;
    assert_eq!(denied, StatusCode::NOT_FOUND);
    let (status, _, challenge) = call(
        &f,
        Some(0),
        "POST",
        "/api/v3/remote/challenges",
        Some(json!({"purpose":"peer-grant","binding":binding})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{challenge}");
    let nonce = URL_SAFE_NO_PAD
        .decode(challenge["nonce"].as_str().unwrap())
        .unwrap();
    let message = proof_message(
        "peer-grant",
        &nonce,
        challenge["bindingSha256"].as_str().unwrap(),
    );
    let mut body = json!({"challengeId":challenge["id"],"binding":binding,"signature":URL_SAFE_NO_PAD.encode(controller_key.sign(&message).to_bytes())});
    assert_eq!(
        call(
            &f,
            Some(0),
            "POST",
            "/api/v3/remote/peer-grants/confirm",
            Some(body.clone())
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    body["signature"] = json!(URL_SAFE_NO_PAD.encode(signing.sign(&message).to_bytes()));
    let (status, _, result) = call(
        &f,
        Some(0),
        "POST",
        "/api/v3/remote/peer-grants/confirm",
        Some(body.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["status"], "active");
    assert_eq!(
        call(
            &f,
            Some(0),
            "POST",
            "/api/v3/remote/peer-grants/confirm",
            Some(body)
        )
        .await
        .0,
        StatusCode::GONE
    );
    sqlx::query("UPDATE cloud_remote_grants SET revision=2,status='pendingHost' WHERE id=$1")
        .bind(grant)
        .execute(&f.pool)
        .await
        .unwrap();
    let (status, _, _) = call(
        &f,
        Some(0),
        "POST",
        "/api/v3/remote/challenges",
        Some(json!({"purpose":"peer-grant","binding":binding})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
#[ignore = "requires a dedicated PostgreSQL database in CLOUD_TEST_DATABASE_URL"]
async fn private_relay_moves_encrypted_peer_frames_and_disconnects_revoked_devices() {
    use cloud_peer_relay::admission::{enforce_revocations, Admission};
    use iroh::{endpoint::presets, tls::CaTlsConfig, Endpoint, RelayMode, SecretKey};
    use std::{sync::Arc, time::Duration};
    tokio::time::timeout(Duration::from_secs(30), async {
        let f = fixture(2).await;
        let (host, device, signing) = host(&f).await;
        let host_key = key();
        assert_eq!(publish(&f,0,&device,&signing,&host_key,0,false).await.0,StatusCode::OK);
        let controller_signing = key();
        let controller_key = key();
        let controller = enroll(&f,1,"Controller","controller",None,&controller_signing).await;
        let controller_id = controller["id"].as_str().unwrap();
        assert_eq!(publish(&f,1,controller_id,&controller_signing,&controller_key,0,false).await.0,StatusCode::OK);
        sqlx::query("INSERT INTO cloud_remote_grants(id,host_id,device_id,requester_user_id,role,status,revision,confirmed_revision) VALUES($1,$2,$3,$4,'observer','active',1,1)")
            .bind(uuid::Uuid::new_v4()).bind(host.parse::<uuid::Uuid>().unwrap())
            .bind(controller_id.parse::<uuid::Uuid>().unwrap()).bind(&f.users[1].id)
            .execute(&f.pool).await.unwrap();
        let admission = Arc::new(Admission::new(f.pool.clone()));
        let (relays,_,server) = iroh::test_utils::run_relay_server_with_access(true,admission.clone()).await.unwrap();
        let monitor = tokio::spawn(enforce_revocations(admission.clone(),server.relay_service().unwrap().clone(),Duration::from_millis(50)));
        let make = |secret: &SigningKey| Endpoint::builder(presets::Minimal)
            .secret_key(SecretKey::from_bytes(&secret.to_bytes()))
            .relay_mode(RelayMode::Custom(relays.clone()))
            // Only this isolated fixture trusts its generated relay certificate.
            .ca_tls_config(CaTlsConfig::insecure_skip_verify())
            .clear_ip_transports()
            .alpns(vec![b"tiktools/collaboration/1".to_vec()]);
        let host = make(&host_key).bind().await.unwrap();
        let controller = make(&controller_key).bind().await.unwrap();
        let unknown = make(&key()).bind().await.unwrap();
        tokio::join!(host.online(),controller.online());
        // A valid QUIC key is not sufficient for private relay membership.
        assert!(tokio::time::timeout(Duration::from_millis(300),unknown.online()).await.is_err());
        assert!(!admission.endpoints().contains(&unknown.id().to_string()));
        let dial = controller.connect(host.addr(),b"tiktools/collaboration/1");
        let accept = async { host.accept().await.unwrap().await.unwrap() };
        let (connected,accepted) = tokio::join!(dial,accept);
        let connected = connected.unwrap();
        assert_eq!(accepted.remote_id(),controller.id());
        let (mut send,_) = connected.open_bi().await.unwrap();
        send.write_all(b"encrypted-through-private-relay").await.unwrap();
        send.finish().unwrap();
        let (_,mut receive) = accepted.accept_bi().await.unwrap();
        assert_eq!(receive.read_to_end(128).await.unwrap(),b"encrypted-through-private-relay");
        sqlx::query("UPDATE cloud_remote_devices SET status='revoked',revoked_at=now() WHERE id=$1")
            .bind(controller_id.parse::<uuid::Uuid>().unwrap()).execute(&f.pool).await.unwrap();
        // Exercise the same monitor used by the service, not just its SQL predicate.
        while admission.endpoints().contains(&controller.id().to_string()) {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert!(!admission.allowed(&controller.id().to_string()).await);
        unknown.close().await;
        controller.close().await;
        host.close().await;
        monitor.abort();
        let _ = monitor.await;
        server.shutdown().await.unwrap();
    }).await.unwrap();
}
