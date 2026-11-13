use super::*;

struct Tunnel {
    host: String,
    host_device: String,
    session: String,
    id: uuid::Uuid,
}
impl Tunnel {
    fn open(&self, device: &str, peer: &SigningKey) -> Value {
        let context = json!({"hostId":self.host,"sessionId":self.session,
            "hostDeviceId":self.host_device,"fromDeviceId":device,
            "negotiationId":self.id,"grantRevision":1});
        let certificate =
            URL_SAFE_NO_PAD.encode(b"opaque certificate; native clients validate its DER");
        let mut signed = context.clone();
        signed["certificate"] = json!(certificate);
        let mut message = b"tiktools.relay.tls.v1\0".to_vec();
        message.extend_from_slice(crate::remote::canonical_json(&signed).as_bytes());
        json!({"type":"relay.open","announcement":{"context":context,
            "certificate":certificate,"signature":URL_SAFE_NO_PAD.encode(peer.sign(&message).to_bytes())}})
    }
    fn data(&self, sequence: i64, bytes: &[u8]) -> Value {
        json!({"type":"relay.data","tunnelId":self.id,"sessionId":self.session,
            "sequence":sequence,"data":URL_SAFE_NO_PAD.encode(bytes)})
    }
    fn close(&self, sequence: i64) -> Value {
        json!({"type":"relay.close","tunnelId":self.id,"sessionId":self.session,"sequence":sequence})
    }
}

#[tokio::test]
#[ignore = "requires a dedicated PostgreSQL database in CLOUD_TEST_DATABASE_URL"]
async fn opaque_relay_pins_device_session_revision_order_and_live_socket_generations() {
    run(false).await;
}
#[tokio::test]
#[ignore = "requires a dedicated PostgreSQL database in CLOUD_TEST_DATABASE_URL"]
async fn opaque_relay_crosses_backend_nodes_with_bounded_fenced_queues() {
    run(true).await;
}

async fn run(cross_node: bool) {
    let f = fixture(2).await;
    let (url, other_server) = if cross_node {
        let (url, server) = second_node(&f).await;
        (url, Some(server))
    } else {
        (f.ws_url.clone(), None)
    };
    let (host, host_device, host_signing) = host(&f).await;
    let host_peer = key();
    assert_eq!(
        publish(&f, 0, &host_device, &host_signing, &host_peer, 0, false)
            .await
            .0,
        StatusCode::OK
    );
    let controller_signing = key();
    let controller_peer = key();
    let device = enroll(
        &f,
        1,
        "Native relay controller",
        "controller",
        None,
        &controller_signing,
    )
    .await;
    let device = device["id"].as_str().unwrap();
    assert_eq!(
        publish(
            &f,
            1,
            device,
            &controller_signing,
            &controller_peer,
            0,
            false
        )
        .await
        .0,
        StatusCode::OK
    );
    let grant = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO cloud_remote_grants(id,host_id,device_id,requester_user_id,role,status,revision,confirmed_revision) VALUES($1,$2,$3,$4,'observer','active',1,1)")
        .bind(grant).bind(host.parse::<uuid::Uuid>().unwrap()).bind(device.parse::<uuid::Uuid>().unwrap())
        .bind(&f.users[1].id).execute(&f.pool).await.unwrap();
    let (status, issued) = ticket(&f, 0, &host, &host_device, "host", &host_signing).await;
    assert_eq!(status, StatusCode::CREATED);
    let (mut host_ws, _) = connect(&f, issued["ticket"].as_str().unwrap()).await;
    let (status, issued) = ticket(&f, 1, &host, device, "controller", &controller_signing).await;
    assert_eq!(status, StatusCode::CREATED);
    let (mut controller_ws, ready) = connect_at(&url, issued["ticket"].as_str().unwrap()).await;
    let mut tunnel = Tunnel {
        host,
        host_device,
        session: ready["sessionId"].as_str().unwrap().into(),
        id: uuid::Uuid::new_v4(),
    };

    let (_, issued) = ticket(
        &f,
        1,
        &tunnel.host,
        device,
        "controller",
        &controller_signing,
    )
    .await;
    let (mut second, _) = connect_at(&url, issued["ticket"].as_str().unwrap()).await;
    send(&mut second, tunnel.open(device, &controller_peer)).await;
    assert_eq!(next_frame(&mut second, "error").await["code"], "forbidden");
    second.close(None).await.unwrap();
    send(&mut controller_ws, tunnel.open(device, &controller_signing)).await;
    assert_eq!(
        next_frame(&mut controller_ws, "error").await["code"],
        "forbidden"
    );
    let mut altered = tunnel.open(device, &controller_peer);
    altered["announcement"]["certificate"] = json!(URL_SAFE_NO_PAD.encode(b"substituted"));
    send(&mut controller_ws, altered).await;
    assert_eq!(
        next_frame(&mut controller_ws, "error").await["code"],
        "forbidden"
    );
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM cloud_remote_relay_tunnels WHERE session_id=$1")
            .bind(tunnel.session.parse::<uuid::Uuid>().unwrap())
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(count, 0, "invalid proofs cannot reserve tunnel state");
    let offer = tunnel.open(device, &controller_peer);
    send(&mut controller_ws, offer.clone()).await;
    next_frame(&mut controller_ws, "relay.accepted").await;
    let delivered = next_frame(&mut host_ws, "relay.open").await;
    assert_eq!(delivered["announcement"], offer["announcement"]);
    assert_eq!(delivered["fromEndpointId"], endpoint_id(&controller_peer));
    assert!(delivered.get("__targetFencing").is_none());
    send(&mut controller_ws, offer).await;
    assert_eq!(
        next_frame(&mut controller_ws, "error").await["code"],
        "stale"
    );
    send(
        &mut controller_ws,
        tunnel.data(0, b"TLS records before host answer"),
    )
    .await;
    assert_eq!(
        next_frame(&mut controller_ws, "error").await["code"],
        "stale"
    );

    let answer = tunnel.open(&tunnel.host_device, &host_peer);
    send(&mut host_ws, answer.clone()).await;
    next_frame(&mut host_ws, "relay.accepted").await;
    assert_eq!(
        next_frame(&mut controller_ws, "relay.open").await["announcement"],
        answer["announcement"]
    );
    send(&mut host_ws, answer).await;
    assert_eq!(next_frame(&mut host_ws, "error").await["code"], "stale");
    for sequence in 0..4 {
        let payload = tunnel.data(sequence, &vec![sequence as u8; 16 * 1024]);
        send(&mut controller_ws, payload.clone()).await;
        let ack = next_frame(&mut controller_ws, "relay.accepted").await;
        assert_eq!(ack["sequence"], sequence);
        let received = next_frame(&mut host_ws, "relay.data").await;
        assert_eq!(received["data"], payload["data"]);
        assert_eq!(received["sequence"], sequence);
        assert_eq!(received["fromDeviceId"], device);
        assert!(received.get("__targetFencing").is_none());
    }
    send(&mut controller_ws, tunnel.data(3, b"replayed")).await;
    assert_eq!(
        next_frame(&mut controller_ws, "error").await["code"],
        "stale"
    );
    send(&mut controller_ws, tunnel.data(5, b"gap")).await;
    assert_eq!(
        next_frame(&mut controller_ws, "error").await["code"],
        "stale"
    );
    send(&mut controller_ws, tunnel.data(4, &vec![0; 16 * 1024 + 1])).await;
    assert_eq!(
        next_frame(&mut controller_ws, "error").await["code"],
        "invalidFrame"
    );
    let mut malformed = tunnel.data(4, b"valid");
    malformed["data"] = json!("not!base64");
    send(&mut controller_ws, malformed).await;
    assert_eq!(
        next_frame(&mut controller_ws, "error").await["code"],
        "invalidFrame"
    );
    send(&mut host_ws, tunnel.data(0, b"opaque encrypted response")).await;
    next_frame(&mut host_ws, "relay.accepted").await;
    assert_eq!(
        next_frame(&mut controller_ws, "relay.data").await["sequence"],
        0
    );
    send(&mut controller_ws, tunnel.close(4)).await;
    next_frame(&mut controller_ws, "relay.accepted").await;
    next_frame(&mut host_ws, "relay.close").await;
    send(&mut host_ws, tunnel.data(1, b"closed stream")).await;
    assert_eq!(next_frame(&mut host_ws, "error").await["code"], "stale");

    // New tunnels are bounded independently of WebRTC negotiations.
    for _ in 0..2 {
        tunnel.id = uuid::Uuid::new_v4();
        send(&mut controller_ws, tunnel.open(device, &controller_peer)).await;
        next_frame(&mut controller_ws, "relay.accepted").await;
        next_frame(&mut host_ws, "relay.open").await;
    }
    let last = tunnel.id;
    tunnel.id = uuid::Uuid::new_v4();
    send(&mut controller_ws, tunnel.open(device, &controller_peer)).await;
    assert_eq!(
        next_frame(&mut controller_ws, "error").await["code"],
        "busy"
    );
    tunnel.id = last;
    sqlx::query(
        "UPDATE cloud_remote_relay_tunnels SET expires_at=now()-interval '1 second' WHERE id=$1",
    )
    .bind(tunnel.id)
    .execute(&f.pool)
    .await
    .unwrap();
    send(&mut host_ws, tunnel.open(&tunnel.host_device, &host_peer)).await;
    assert_eq!(next_frame(&mut host_ws, "error").await["code"], "stale");
    tunnel.id = uuid::Uuid::new_v4();
    send(&mut controller_ws, tunnel.open(device, &controller_peer)).await;
    next_frame(&mut controller_ws, "relay.accepted").await;
    next_frame(&mut host_ws, "relay.open").await;
    // A current controller cannot use a tunnel negotiated with an older host
    // socket generation, even while its own original socket remains active.
    sqlx::query("UPDATE cloud_relay_leases SET fencing_token=fencing_token+1 WHERE resource_kind='host' AND resource_id=$1")
        .bind(tunnel.host.parse::<uuid::Uuid>().unwrap()).execute(&f.pool).await.unwrap();
    send(&mut controller_ws, tunnel.data(0, b"wrong host generation")).await;
    assert_eq!(
        next_frame(&mut controller_ws, "error").await["code"],
        "stale"
    );
    send(&mut host_ws, tunnel.open(&tunnel.host_device, &host_peer)).await;
    assert_eq!(next_frame(&mut host_ws, "error").await["code"], "revoked");
    sqlx::query("UPDATE cloud_remote_grants SET status='revoked',revision=2 WHERE id=$1")
        .bind(grant)
        .execute(&f.pool)
        .await
        .unwrap();
    send(&mut controller_ws, tunnel.data(0, b"revoked grant")).await;
    assert_eq!(
        next_frame(&mut controller_ws, "error").await["code"],
        "revoked"
    );
    let commands: i64 =
        sqlx::query_scalar("SELECT count(*) FROM cloud_remote_commands WHERE session_id=$1")
            .bind(tunnel.session.parse::<uuid::Uuid>().unwrap())
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(
        commands, 0,
        "opaque records must not become backend commands"
    );

    // Deleting a lease must not reset its fencing token for the next socket.
    let node = format!("fencing-test-{}", uuid::Uuid::new_v4());
    let id = uuid::Uuid::new_v4();
    let device = device.parse().unwrap();
    crate::remote::cluster::heartbeat(&f.pool, &node)
        .await
        .unwrap();
    let first = crate::remote::cluster::acquire(
        &f.pool,
        &node,
        crate::remote::cluster::Kind::Host,
        id,
        device,
    )
    .await
    .unwrap()
    .unwrap();
    crate::remote::cluster::release(
        &f.pool,
        &node,
        crate::remote::cluster::Kind::Host,
        id,
        first,
    )
    .await
    .unwrap();
    let second = crate::remote::cluster::acquire(
        &f.pool,
        &node,
        crate::remote::cluster::Kind::Host,
        id,
        device,
    )
    .await
    .unwrap()
    .unwrap();
    assert!(second > first);
    if cross_node {
        crate::remote::relay::assert_native_relay_queue_bound(&f.pool).await;
    }
    if let Some(server) = other_server {
        server.abort();
    }
}
