//! Supervised queue processing. Signing/promotion remains an explicit operator action.
use crate::{publication_worker, scanner::ClamAv, worker};
use cloud_storage::QuarantineStore;
use sqlx::PgPool;

pub async fn run(
    pool: &PgPool,
    store: &QuarantineStore,
    scanner: &ClamAv,
) -> Result<(), Box<dyn std::error::Error>> {
    let shutdown = crate::shutdown_signal();
    tokio::pin!(shutdown);
    loop {
        // Existing workers use fenced, expiring leases. If interrupted, a
        // subsequent worker safely reclaims the job after the lease expires.
        let cycle = async {
            let verified = worker::once(pool, store, scanner).await?;
            let prepared = publication_worker::once(pool, store, scanner).await?;
            Ok::<bool, Box<dyn std::error::Error>>(verified || prepared)
        };
        let worked = tokio::select! {
            _=&mut shutdown=>{tracing::info!("queue worker shutdown requested");return Ok(());},
            result=cycle=>result?,
        };
        let delay = if worked {
            std::time::Duration::from_millis(50)
        } else {
            std::time::Duration::from_secs(2)
        };
        tokio::select! {
            _=&mut shutdown=>return Ok(()),
            _=tokio::time::sleep(delay)=>{},
        }
    }
}
