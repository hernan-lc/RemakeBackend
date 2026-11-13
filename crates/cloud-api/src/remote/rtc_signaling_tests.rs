use super::*;

struct Negotiation {
    host: String,
    host_device: String,
    session: String,
    id: uuid::Uuid,
}
impl Negotiation {
    fn signal(&self, device: &str, peer: &SigningKey, sequence: u32, signal: Value) -> Value {
        let signed = json!({"hostId":self.host,"sessionId":self.session,
            "hostDeviceId":self.host_device,"fromDeviceId":device,
            "negotiationId":self.id,"grantRevision":1,"sequence":sequence,"signal":signal});
        let mut message = b"tiktools.webrtc.signal.v1\0".to_vec();
        message.extend_from_slice(crate::remote::canonical_json(&signed).as_bytes());
        json!({"type":"rtc.signal","negotiationId":self.id,"sessionId":self.session,
            "hostDeviceId":self.host_device,"sequence":sequence,"signal":signal,
            "signature":URL_SAFE_NO_PAD.encode(peer.sign(&message).to_bytes())})
    }
}
fn description(kind: &str) -> Value {
    json!({"type":kind,"sdp":"v=0\r\na=fingerprint:sha-256 00:11\r\n"})
}

#[tokio::test]
#[ignore = "requires a dedicated PostgreSQL database in CLOUD_TEST_DATABASE_URL"]
async fn signed_webrtc_signaling_routes_only_current_scoped_and_ordered_negotiations() {
    run(false).await;
}

#[tokio::test]
#[ignore = "requires a dedicated PostgreSQL database in CLOUD_TEST_DATABASE_URL"]
async fn signed_webrtc_negotiations_cross_backend_nodes_with_the_same_permission_checks() {
    run(true).await;
}

async fn run(cross_node: bool) {
    let f = fixture(2).await;
    let (controller_url, other_server) = if cross_node {
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
    let controller = enroll(&f, 1, "Controller", "controller", None, &controller_signing).await;
    let device = controller["id"].as_str().unwrap();
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
        .bind(grant).bind(host.parse::<uuid::Uuid>().unwrap()).bind(device.parse::<uuid::Uuid>().unwrap()).bind(&f.users[1].id).execute(&f.pool).await.unwrap();
    let (status, issued) = ticket(&f, 0, &host, &host_device, "host", &host_signing).await;
    assert_eq!(status, StatusCode::CREATED);
    let (mut host_ws, _) = connect(&f, issued["ticket"].as_str().unwrap()).await;
    // Model device rotation after the old device is revoked in storage but
    // before its socket lease has drained on another node. A newly enrolled
    // device must not be advertised as the owner of that old live socket.
    sqlx::query("UPDATE cloud_remote_devices SET status='revoked' WHERE id=$1")
        .bind(host_device.parse::<uuid::Uuid>().unwrap())
        .execute(&f.pool)
        .await
        .unwrap();
    let replacement_signing = key();
    let replacement_peer = key();
    let replacement = enroll(
        &f,
        0,
        "Replacement host",
        "host",
        Some(&host),
        &replacement_signing,
    )
    .await;
    let replacement_id = replacement["id"].as_str().unwrap();
    assert_eq!(
        publish(
            &f,
            0,
            replacement_id,
            &replacement_signing,
            &replacement_peer,
            0,
            false
        )
        .await
        .0,
        StatusCode::OK
    );
    assert!(!peers(&f, 1, &host).await["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["kind"] == "host"));
    // Restore the original fixture so the signaling/fencing checks below run
    // against its still-current socket. Only one host identity stays active.
    sqlx::query("UPDATE cloud_remote_devices SET status='revoked' WHERE id=$1")
        .bind(replacement_id.parse::<uuid::Uuid>().unwrap())
        .execute(&f.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE cloud_remote_devices SET status='active' WHERE id=$1")
        .bind(host_device.parse::<uuid::Uuid>().unwrap())
        .execute(&f.pool)
        .await
        .unwrap();
    let directory = peers(&f, 1, &host).await;
    assert_eq!(
        directory["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["kind"] == "host")
            .unwrap()["deviceId"],
        host_device
    );
    let (status, issued) = ticket(&f, 1, &host, device, "controller", &controller_signing).await;
    assert_eq!(status, StatusCode::CREATED);
    let (mut controller_ws, ready) =
        connect_at(&controller_url, issued["ticket"].as_str().unwrap()).await;
    let mut negotiation = Negotiation {
        host,
        host_device,
        session: ready["sessionId"].as_str().unwrap().into(),
        id: uuid::Uuid::new_v4(),
    };

    // A second authenticated socket, even for the same account/device/key,
    // cannot inject a signal into the first socket's session.
    let (_, issued) = ticket(
        &f,
        1,
        &negotiation.host,
        device,
        "controller",
        &controller_signing,
    )
    .await;
    let (mut second, _) = connect_at(&controller_url, issued["ticket"].as_str().unwrap()).await;
    let offer = negotiation.signal(device, &controller_peer, 0, description("offer"));
    send(&mut second, offer.clone()).await;
    assert_eq!(next_frame(&mut second, "error").await["code"], "forbidden");
    second.close(None).await.unwrap();
    send(
        &mut controller_ws,
        negotiation.signal(device, &controller_signing, 0, description("offer")),
    )
    .await;
    assert_eq!(
        next_frame(&mut controller_ws, "error").await["code"],
        "forbidden"
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM cloud_remote_rtc_negotiations")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(
        count, 0,
        "invalid proofs must not reserve negotiation state"
    );
    send(&mut controller_ws, offer.clone()).await;
    next_frame(&mut controller_ws, "rtc.accepted").await;
    let forwarded = next_frame(&mut host_ws, "rtc.signal").await;
    assert_eq!(forwarded["fromDeviceId"], device);
    assert_eq!(forwarded["fromEndpointId"], endpoint_id(&controller_peer));
    assert_eq!(forwarded["signal"], offer["signal"]);
    send(&mut controller_ws, offer).await;
    assert_eq!(
        next_frame(&mut controller_ws, "error").await["code"],
        "stale"
    );

    let answer = negotiation.signal(
        &negotiation.host_device,
        &host_peer,
        0,
        description("answer"),
    );
    send(&mut host_ws, answer.clone()).await;
    next_frame(&mut host_ws, "rtc.accepted").await;
    assert_eq!(
        next_frame(&mut controller_ws, "rtc.signal").await["signal"],
        answer["signal"]
    );
    let candidate = json!({"type":"candidate","candidate":"candidate:1 1 UDP 1 192.0.2.1 9999 typ host","sdpMid":"data","sdpMLineIndex":0,"usernameFragment":null});
    send(
        &mut controller_ws,
        negotiation.signal(device, &controller_peer, 2, candidate.clone()),
    )
    .await;
    assert_eq!(
        next_frame(&mut controller_ws, "error").await["code"],
        "stale"
    );
    send(
        &mut controller_ws,
        negotiation.signal(device, &controller_peer, 1, candidate.clone()),
    )
    .await;
    next_frame(&mut controller_ws, "rtc.accepted").await;
    assert_eq!(
        next_frame(&mut host_ws, "rtc.signal").await["signal"],
        candidate
    );
    send(
        &mut controller_ws,
        negotiation.signal(device, &controller_peer, 2, description("answer")),
    )
    .await;
    assert_eq!(
        next_frame(&mut controller_ws, "error").await["code"],
        "invalidFrame"
    );

    sqlx::query(
        "UPDATE cloud_remote_rtc_negotiations SET expires_at=now()-interval '1 second' WHERE id=$1",
    )
    .bind(negotiation.id)
    .execute(&f.pool)
    .await
    .unwrap();
    send(
        &mut host_ws,
        negotiation.signal(
            &negotiation.host_device,
            &host_peer,
            1,
            json!({"type":"endOfCandidates"}),
        ),
    )
    .await;
    assert_eq!(next_frame(&mut host_ws, "error").await["code"], "stale");
    for _ in 0..4 {
        negotiation.id = uuid::Uuid::new_v4();
        send(
            &mut controller_ws,
            negotiation.signal(device, &controller_peer, 0, description("offer")),
        )
        .await;
        next_frame(&mut controller_ws, "rtc.accepted").await;
        next_frame(&mut host_ws, "rtc.signal").await;
    }
    let last = negotiation.id;
    negotiation.id = uuid::Uuid::new_v4();
    send(
        &mut controller_ws,
        negotiation.signal(device, &controller_peer, 0, description("offer")),
    )
    .await;
    assert_eq!(
        next_frame(&mut controller_ws, "error").await["code"],
        "busy"
    );
    negotiation.id = last;
    // An old host socket cannot answer after another listener incarnation
    // has taken over its lease, even while its authenticated socket is alive.
    sqlx::query("UPDATE cloud_relay_leases SET fencing_token=fencing_token+1 WHERE resource_kind='host' AND resource_id=$1")
        .bind(negotiation.host.parse::<uuid::Uuid>().unwrap()).execute(&f.pool).await.unwrap();
    send(
        &mut host_ws,
        negotiation.signal(
            &negotiation.host_device,
            &host_peer,
            0,
            description("answer"),
        ),
    )
    .await;
    assert_eq!(next_frame(&mut host_ws, "error").await["code"], "revoked");
    // Leave the socket alive deliberately: every signal rechecks DB authority.
    sqlx::query("UPDATE cloud_remote_grants SET status='revoked',revision=2 WHERE id=$1")
        .bind(grant)
        .execute(&f.pool)
        .await
        .unwrap();
    send(
        &mut controller_ws,
        negotiation.signal(
            device,
            &controller_peer,
            1,
            json!({"type":"endOfCandidates"}),
        ),
    )
    .await;
    assert_eq!(
        next_frame(&mut controller_ws, "error").await["code"],
        "revoked"
    );
    let commands: i64 = sqlx::query_scalar("SELECT count(*) FROM cloud_remote_commands")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(commands, 0, "signaling must not create or execute commands");
    if let Some(server) = other_server {
        server.abort();
    }
}
