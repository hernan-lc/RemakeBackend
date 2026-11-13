//! Database-backed relay membership. The relay never sees RPC plaintext.
use iroh_relay::server::{Access, AccessControl, ClientRequest, ConnectionId};
use sqlx::{PgPool, Row};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

#[derive(Debug)]
pub struct Admission {
    pool: PgPool,
    slots: Arc<Semaphore>,
    connections: Mutex<HashMap<ConnectionId, (String, OwnedSemaphorePermit)>>,
}

impl Admission {
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            slots: Arc::new(Semaphore::new(256)),
            connections: Mutex::new(HashMap::new()),
        }
    }

    pub fn endpoints(&self) -> Vec<String> {
        self.connections
            .lock()
            .map(|connections| {
                connections
                    .values()
                    .map(|(key, _)| key.clone())
                    .collect::<std::collections::BTreeSet<_>>()
                    .into_iter()
                    .collect()
            })
            .unwrap_or_default()
    }

    pub async fn allowed(&self, key: &str) -> bool {
        tokio::time::timeout(Duration::from_secs(2), self.lookup(key))
            .await
            .unwrap_or(false)
    }

    async fn lookup(&self, key: &str) -> bool {
        let row = match sqlx::query("SELECT d.id,d.kind,d.host_id FROM cloud_remote_peer_endpoints p JOIN cloud_remote_devices d ON d.id=p.device_id WHERE p.endpoint_id=$1 AND p.relay_authorized_until>now() AND d.status='active'")
            .bind(key).fetch_optional(&self.pool).await {
                Ok(Some(row)) => row, _ => return false,
            };
        let kind: String = row.get("kind");
        if kind == "host" {
            let host: Option<uuid::Uuid> = row.get("host_id");
            return matches!(
                sqlx::query_scalar::<_, uuid::Uuid>(
                    "SELECT id FROM cloud_remote_hosts WHERE id=$1 AND status='active'"
                )
                .bind(host)
                .fetch_optional(&self.pool)
                .await,
                Ok(Some(_))
            );
        }
        let device: uuid::Uuid = row.get("id");
        matches!(sqlx::query_scalar::<_,uuid::Uuid>("SELECT g.id FROM cloud_remote_grants g JOIN cloud_remote_hosts h ON h.id=g.host_id WHERE g.device_id=$1 AND g.status='active' AND g.confirmed_revision=g.revision AND h.status='active' LIMIT 1")
            .bind(device).fetch_optional(&self.pool).await, Ok(Some(_)))
    }
}

impl AccessControl for Admission {
    async fn on_connect(&self, request: &ClientRequest) -> Access {
        let Ok(permit) = self.slots.clone().try_acquire_owned() else {
            return Access::Deny { reason: None };
        };
        let key = request.endpoint_id().to_string();
        if !self.allowed(&key).await {
            return Access::Deny { reason: None };
        }
        let Ok(mut connections) = self.connections.lock() else {
            return Access::Deny { reason: None };
        };
        if connections
            .values()
            .filter(|(endpoint, _)| endpoint == &key)
            .count()
            >= 2
        {
            return Access::Deny { reason: None };
        }
        connections.insert(request.connection_id(), (key, permit));
        Access::Allow
    }

    fn on_disconnect(&self, _: iroh_base::EndpointId, connection: ConnectionId) {
        if let Ok(mut connections) = self.connections.lock() {
            connections.remove(&connection);
        }
    }
}

/// Recheck established relay links as well as new connections. The interval is
/// infrastructure policy; it never extends the authorization lease in storage.
pub async fn enforce_revocations(
    admission: Arc<Admission>,
    service: iroh_relay::server::http_server::RelayService,
    interval: Duration,
) {
    use futures_util::{stream, StreamExt};
    let mut clock = tokio::time::interval(interval);
    loop {
        clock.tick().await;
        let checks = stream::iter(admission.endpoints())
            .map(|endpoint| {
                let admission = admission.clone();
                async move {
                    let allowed = admission.allowed(&endpoint).await;
                    (endpoint, allowed)
                }
            })
            .buffer_unordered(16);
        tokio::pin!(checks);
        while let Some((endpoint, allowed)) = checks.next().await {
            if !allowed {
                if let Ok(endpoint) = endpoint.parse() {
                    service.clients().disconnect(endpoint, None);
                }
            }
        }
    }
}
