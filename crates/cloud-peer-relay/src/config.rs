use iroh_relay::server::{CertConfig, TlsConfig};
use std::{env, fs, io::BufReader, net::SocketAddr, path::PathBuf, sync::Arc};

pub struct Config {
    pub database_url: String,
    pub http_bind: SocketAddr,
    pub quic_bind: SocketAddr,
    pub metrics_bind: SocketAddr,
    tls_bind: SocketAddr,
    certificate: Option<PathBuf>,
    key: Option<PathBuf>,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        Self::parse(|key| env::var(key).ok())
    }

    fn parse(get: impl Fn(&str) -> Option<String>) -> Result<Self, String> {
        let database_url = get("DATABASE_URL")
            .filter(|value| !value.trim().is_empty())
            .ok_or("DATABASE_URL is required")?;
        let http_bind: SocketAddr = get("PEER_RELAY_HTTP_BIND")
            .unwrap_or_else(|| "127.0.0.1:8091".into())
            .parse()
            .map_err(|_| "invalid PEER_RELAY_HTTP_BIND")?;
        let tls_bind: SocketAddr = get("PEER_RELAY_HTTPS_BIND")
            .unwrap_or_else(|| "0.0.0.0:8443".into())
            .parse()
            .map_err(|_| "invalid PEER_RELAY_HTTPS_BIND")?;
        let quic_bind: SocketAddr = get("PEER_RELAY_QUIC_BIND")
            .unwrap_or_else(|| "0.0.0.0:7842".into())
            .parse()
            .map_err(|_| "invalid PEER_RELAY_QUIC_BIND")?;
        let metrics_bind: SocketAddr = get("PEER_RELAY_METRICS_BIND")
            .unwrap_or_else(|| "127.0.0.1:9091".into())
            .parse()
            .map_err(|_| "invalid PEER_RELAY_METRICS_BIND")?;
        if !metrics_bind.ip().is_loopback() {
            return Err("relay metrics must bind to loopback".into());
        }
        let certificate = get("PEER_RELAY_TLS_CERT").map(PathBuf::from);
        let key = get("PEER_RELAY_TLS_KEY").map(PathBuf::from);
        if certificate.is_some() != key.is_some() {
            return Err("both PEER_RELAY_TLS_CERT and PEER_RELAY_TLS_KEY are required".into());
        }
        let proxy = get("PEER_RELAY_TRUST_TLS_PROXY").as_deref() == Some("true");
        if certificate.is_none() && !http_bind.ip().is_loopback() && !proxy {
            return Err("non-loopback HTTP requires an explicitly trusted TLS proxy".into());
        }
        Ok(Self {
            database_url,
            http_bind,
            quic_bind,
            metrics_bind,
            tls_bind,
            certificate,
            key,
        })
    }

    pub fn tls(&self) -> Result<Option<TlsConfig>, String> {
        let (Some(cert), Some(key)) = (&self.certificate, &self.key) else {
            return Ok(None);
        };
        for path in [cert, key] {
            let metadata = fs::symlink_metadata(path).map_err(|_| "TLS file is unavailable")?;
            if !metadata.is_file() || metadata.len() > 65536 {
                return Err("TLS files must be bounded regular files".into());
            }
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if fs::metadata(key)
                .map_err(|_| "TLS key unavailable")?
                .permissions()
                .mode()
                & 0o077
                != 0
            {
                return Err("TLS private key must have owner-only permissions".into());
            }
        }
        let certificates = rustls_pemfile::certs(&mut BufReader::new(
            fs::File::open(cert).map_err(|_| "TLS certificate unavailable")?,
        ))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "invalid TLS certificate")?;
        let key = rustls_pemfile::private_key(&mut BufReader::new(
            fs::File::open(key).map_err(|_| "TLS key unavailable")?,
        ))
        .map_err(|_| "invalid TLS key")?
        .ok_or("TLS key is missing")?;
        let mut tls = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .map_err(|_| "TLS versions unavailable")?
        .with_no_client_auth()
        .with_single_cert(certificates, key)
        .map_err(|_| "TLS certificate and key do not match")?;
        tls.alpn_protocols = vec![b"http/1.1".to_vec()];
        Ok(Some(TlsConfig::new(
            self.tls_bind,
            CertConfig::Manual { server_config: tls },
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config(values: &[(&str, &str)]) -> Result<Config, String> {
        Config::parse(|key| {
            if key == "DATABASE_URL" {
                Some("postgres://test@localhost/disposable".into())
            } else {
                values
                    .iter()
                    .find(|(name, _)| *name == key)
                    .map(|(_, value)| value.to_string())
            }
        })
    }
    #[test]
    fn http_exposure_requires_proxy_opt_in_and_tls_requires_both_files() {
        assert!(config(&[]).unwrap().http_bind.ip().is_loopback());
        assert!(config(&[("PEER_RELAY_HTTP_BIND", "0.0.0.0:8091")]).is_err());
        assert!(config(&[
            ("PEER_RELAY_HTTP_BIND", "0.0.0.0:8091"),
            ("PEER_RELAY_TRUST_TLS_PROXY", "true")
        ])
        .is_ok());
        assert!(config(&[("PEER_RELAY_TLS_KEY", "/private/key")]).is_err());
        assert!(config(&[("PEER_RELAY_METRICS_BIND", "0.0.0.0:9091")]).is_err());
    }
}
