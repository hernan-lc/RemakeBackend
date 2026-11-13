use super::*;

pub(super) enum Delivery {
    Local(mpsc::OwnedPermit<Outbound>, String),
    Queued,
}
impl Delivery {
    pub(super) fn send(self) {
        if let Self::Local(permit, text) = self {
            permit.send(Outbound::Text(text));
        }
    }
}

pub(super) async fn prepare(
    hub: &Hub,
    tx: &mut Transaction<'_, Postgres>,
    kind: cluster::Kind,
    target: Uuid,
    node: &str,
    generation: i64,
    mut frame: Value,
) -> Result<Delivery, &'static str> {
    if node == hub.node() {
        let sender = {
            let state = hub.lock();
            match kind {
                cluster::Kind::Host => state
                    .hosts
                    .get(&target)
                    .filter(|l| l.fencing == generation)
                    .map(|l| l.tx.clone()),
                cluster::Kind::Session => state
                    .sessions
                    .get(&target)
                    .filter(|l| l.fencing == generation)
                    .map(|l| l.tx.clone()),
            }
        }
        .ok_or("peerOffline")?;
        let permit = sender.try_reserve_owned().map_err(|_| "peerBusy")?;
        return Ok(Delivery::Local(permit, frame.to_string()));
    }
    // Serialize bounded enqueueing per destination, across sessions and nodes.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 724196))")
        .bind(format!("{}:{target}", kind.as_str()))
        .execute(&mut **tx)
        .await
        .map_err(|_| "temporaryFailure")?;
    let pending: i64 = sqlx::query_scalar("SELECT count(*) FROM cloud_relay_frames WHERE target_node=$1 AND resource_kind=$2 AND resource_id=$3")
        .bind(node).bind(kind.as_str()).bind(target).fetch_one(&mut **tx).await.map_err(|_| "temporaryFailure")?;
    if pending >= OUTBOX_CAPACITY as i64 {
        return Err("peerBusy");
    }
    frame["__targetFencing"] = json!(generation);
    sqlx::query("INSERT INTO cloud_relay_frames(target_node,resource_kind,resource_id,payload) VALUES($1,$2,$3,$4)")
        .bind(node).bind(kind.as_str()).bind(target).bind(frame.to_string()).execute(&mut **tx).await.map_err(|_| "temporaryFailure")?;
    Ok(Delivery::Queued)
}

pub(in super::super) fn deliver_local(hub: &Hub, kind: cluster::Kind, id: Uuid, frame: &Value) {
    let Some(generation) = frame["__targetFencing"].as_i64() else {
        return;
    };
    let mut frame = frame.clone();
    if let Some(obj) = frame.as_object_mut() {
        obj.remove("__targetFencing");
    }
    let mut state = hub.lock();
    match kind {
        cluster::Kind::Host => {
            if let Some(link) = state.hosts.get(&id).filter(|l| l.fencing == generation) {
                if link.tx.try_send(Outbound::Text(frame.to_string())).is_err() {
                    // Dropping the last sender closes the slow socket after its
                    // bounded queue drains. Never skip a TLS record and carry on.
                    state.hosts.remove(&id);
                }
            }
        }
        cluster::Kind::Session => {
            if let Some(link) = state.sessions.get(&id).filter(|l| l.fencing == generation) {
                if link.tx.try_send(Outbound::Text(frame.to_string())).is_err() {
                    state.sessions.remove(&id);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn queued_records_cannot_enter_a_replacement_host_socket() {
        let hub = Hub::new("routing-test");
        let host = Uuid::new_v4();
        let (tx, mut rx) = mpsc::channel(OUTBOX_CAPACITY);
        hub.register_host(host, Uuid::new_v4(), 1, 9, tx);
        deliver_local(
            &hub,
            cluster::Kind::Host,
            host,
            &json!({"type":"relay.data","__targetFencing":8,"sequence":0}),
        );
        assert!(rx.try_recv().is_err());
        assert!(hub.lock().hosts.contains_key(&host));
        deliver_local(
            &hub,
            cluster::Kind::Host,
            host,
            &json!({"type":"relay.data","__targetFencing":9,"sequence":0}),
        );
        let Outbound::Text(text) = rx.recv().await.unwrap() else {
            panic!("expected record");
        };
        let value: Value = serde_json::from_str(&text).unwrap();
        assert!(value.get("__targetFencing").is_none());
        assert_eq!(value["sequence"], 0);
    }

    #[tokio::test]
    async fn overflowing_a_remote_delivery_closes_the_slow_stream_instead_of_skipping_records() {
        let hub = Hub::new("routing-test");
        let session = Uuid::new_v4();
        let (tx, mut rx) = mpsc::channel(OUTBOX_CAPACITY);
        hub.register_session(session, Uuid::new_v4(), 1, 3, tx);
        for sequence in 0..OUTBOX_CAPACITY {
            deliver_local(
                &hub,
                cluster::Kind::Session,
                session,
                &json!({"type":"relay.data","__targetFencing":3,"sequence":sequence}),
            );
        }
        assert!(hub.lock().sessions.contains_key(&session));
        deliver_local(
            &hub,
            cluster::Kind::Session,
            session,
            &json!({"type":"relay.data","__targetFencing":3,"sequence":OUTBOX_CAPACITY}),
        );
        assert!(!hub.lock().sessions.contains_key(&session));
        for sequence in 0..OUTBOX_CAPACITY {
            let Outbound::Text(text) = rx.recv().await.unwrap() else {
                panic!("expected record");
            };
            let value: Value = serde_json::from_str(&text).unwrap();
            assert_eq!(value["sequence"], sequence);
        }
        assert!(rx.recv().await.is_none());
    }
}
