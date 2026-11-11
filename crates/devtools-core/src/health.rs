//! HealthChecker: a real SQL round trip, plus an identity check, instead of "the port answers".

use crate::config::Connection;
use crate::process::port_in_use;
use anyhow::{bail, Context, Result};
use serde::Serialize;
use sqlx::postgres::{PgConnectOptions, PgSslMode};
use sqlx::{Connection as _, PgConnection};
use std::time::Duration;

pub fn connect_options(conn: &Connection, database: &str) -> PgConnectOptions {
    PgConnectOptions::new()
        .host(&conn.host)
        .port(conn.port)
        .username(&conn.user)
        .password(&conn.password)
        .database(database)
        .ssl_mode(PgSslMode::Disable) // loopback only
}

pub async fn connect(conn: &Connection, database: &str) -> Result<PgConnection> {
    let opts = connect_options(conn, database);
    tokio::time::timeout(Duration::from_secs(5), PgConnection::connect_with(&opts))
        .await
        .context("timed out connecting to the database")?
        .with_context(|| {
            format!(
                "cannot connect to {}:{} as {}",
                conn.host, conn.port, conn.user
            )
        })
}

/// `SELECT version()` on the maintenance database.
pub async fn server_version(conn: &Connection) -> Result<String> {
    let mut c = connect(conn, "postgres").await?;
    let version: String = sqlx::query_scalar("SELECT version()")
        .fetch_one(&mut c)
        .await
        .context("SELECT version() failed")?;
    let _ = c.close().await;
    Ok(version)
}

/// A server that answers on the port is not necessarily pgrust: ask it who it is.
pub fn require_pgrust_identity(version: &str) -> Result<()> {
    if !version.to_lowercase().contains("pgrust") {
        bail!("the server on this port does not identify as pgrust (version() = `{version}`)");
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct HealthReport {
    pub port_open: bool,
    pub sql_ok: bool,
    pub identity_ok: bool,
    pub server_version: Option<String>,
    pub error: Option<String>,
}

pub async fn check(conn: &Connection) -> HealthReport {
    let port_open = port_in_use(&conn.host, conn.port);
    if !port_open {
        return HealthReport {
            port_open,
            sql_ok: false,
            identity_ok: false,
            server_version: None,
            error: Some(format!(
                "nothing is listening on {}:{}",
                conn.host, conn.port
            )),
        };
    }
    match server_version(conn).await {
        Ok(version) => {
            let identity = require_pgrust_identity(&version);
            HealthReport {
                port_open,
                sql_ok: true,
                identity_ok: identity.is_ok(),
                server_version: Some(version),
                error: identity.err().map(|e| e.to_string()),
            }
        }
        Err(e) => HealthReport {
            port_open,
            sql_ok: false,
            identity_ok: false,
            server_version: None,
            error: Some(format!("{e:#}")),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_requires_pgrust_in_version() {
        assert!(
            require_pgrust_identity("PostgreSQL 18.6 (pgrust 0.4-beta, build abc) on x86_64")
                .is_ok()
        );
        assert!(require_pgrust_identity("PostgreSQL 16.2 on x86_64-pc-linux-gnu").is_err());
    }

    #[tokio::test]
    async fn a_closed_port_is_unhealthy_without_a_sql_attempt() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let conn = Connection {
            host: "127.0.0.1".into(),
            port,
            user: "u".into(),
            password: "p".into(),
            database: "d".into(),
        };
        let report = check(&conn).await;
        assert!(!report.port_open && !report.sql_ok && !report.identity_ok);
        assert!(report.error.unwrap().contains("nothing is listening"));
    }
}
