mod config;
use cloud_peer_relay::admission::Admission;
use iroh_relay::server::{ClientRateLimit, QuicConfig, RelayConfig, Server, ServerConfig};
use std::{num::NonZeroU32, sync::Arc, time::Duration};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    if let Err(message) = run().await {
        eprintln!("cloud-peer-relay: {message}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let config = config::Config::from_env()?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(2))
        .connect(&config.database_url)
        .await
        .map_err(|_| "relay database is unavailable")?;
    // Never silently run an open relay when migrations or credentials are wrong.
    sqlx::query("SELECT relay_authorized_until FROM cloud_remote_peer_endpoints LIMIT 0")
        .execute(&pool)
        .await
        .map_err(|_| "peer directory migration is required")?;
    let admission = Arc::new(Admission::new(pool));
    let mut relay = RelayConfig::new(config.http_bind);
    relay.tls = config.tls()?;
    relay.access = admission.clone();
    relay.key_cache_capacity = Some(4096);
    relay.limits.client_rx = Some(ClientRateLimit::new(
        NonZeroU32::new(65536).expect("nonzero constant"),
    ));
    let mut settings = ServerConfig::default();
    settings.relay = Some(relay);
    settings.quic = Some(QuicConfig::new(config.quic_bind));
    settings.metrics_addr = Some(config.metrics_bind);
    let mut server = Server::spawn(settings)
        .await
        .map_err(|_| "relay listener could not start")?;
    tracing::info!(http=?server.http_addr(),https=?server.https_addr(),quic=?server.quic_addr(),"private-admission peer relay started");
    let service = server
        .relay_service()
        .cloned()
        .ok_or("relay service is missing")?;
    let monitoring = cloud_peer_relay::admission::enforce_revocations(
        admission,
        service,
        Duration::from_secs(20),
    );
    tokio::pin!(monitoring);
    tokio::select! {
        _=shutdown_signal()=>{},
        result=server.join()=>{ result.map_err(|_|"relay task failed")?.map_err(|_|"relay service failed")?; },
        _=&mut monitoring=>{},
    }
    server
        .shutdown()
        .await
        .map_err(|_| "relay shutdown failed")?;
    Ok(())
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        match signal(SignalKind::terminate()) {
            Ok(mut terminate) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {},
                    _ = terminate.recv() => {},
                }
            }
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
