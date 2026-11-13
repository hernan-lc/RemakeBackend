use better_auth::JwksRefreshPolicy;
use ipnet::IpNet;
use std::{env, net::SocketAddr, time::Duration};
use url::Url;

// Railway sets these in every build and deployment. Any one of them marks a
// managed Railway deployment, where stricter bind and origin checks apply.
const RAILWAY_MARKERS: [&str; 3] = [
    "RAILWAY_ENVIRONMENT_NAME",
    "RAILWAY_PROJECT_ID",
    "RAILWAY_SERVICE_ID",
];

// Deliberately no Debug: this type contains credentials.
pub struct Config {
    pub bind: SocketAddr,
    pub peer: crate::remote::peers::PeerConfig,
    /// True when running on Railway (see `RAILWAY_MARKERS`).
    pub railway: bool,
    pub trusted_proxy_cidrs: Vec<IpNet>,
    pub database_url: String,
    pub auth_secret: String,
    pub origin: String,
    pub google: Option<(String, String)>,
    pub jwks_policy: JwksRefreshPolicy,
    pub object_storage: Option<StorageConfig>,
    pub clamav_addr: Option<SocketAddr>,
    pub admin_dist_dir: Option<std::path::PathBuf>,
    /// Temporary operator override: emits short-lived confirmation codes to logs.
    pub temporary_log_approvals: bool,
}

pub enum StorageConfig {
    S3(cloud_storage::Config),
    Local {
        root: std::path::PathBuf,
        origin: String,
        secret: String,
    },
}
impl StorageConfig {
    pub fn build(self) -> Result<cloud_storage::QuarantineStore, cloud_storage::StorageError> {
        match self {
            Self::S3(config) => cloud_storage::QuarantineStore::new(config),
            Self::Local {
                root,
                origin,
                secret,
            } => cloud_storage::QuarantineStore::local(&root, &origin, secret.as_bytes()),
        }
    }
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        Self::from_lookup(|key| env::var(key).ok())
    }

    fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Self, String> {
        let required = |key: &str| {
            get(key)
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| format!("{key} is required"))
        };
        let railway = RAILWAY_MARKERS
            .iter()
            .any(|key| get(key).is_some_and(|value| !value.trim().is_empty()));
        // PORT is validated only when it selects the listener or Railway requires it,
        // so an explicit BIND_ADDR outside Railway keeps its previous behavior.
        let port = if railway || get("BIND_ADDR").is_none() {
            parse_port(get("PORT"))?
        } else {
            None
        };
        if railway && port.is_none() {
            return Err(
                "PORT is required on Railway: the service must listen on 0.0.0.0:$PORT".into(),
            );
        }
        // An explicit bind address wins for local development. Railway and
        // similar platforms supply PORT and require a listener on all interfaces.
        // On Railway a loopback listener is unreachable from the edge (502), so
        // BIND_ADDR must be a non-loopback address on the PORT that Railway routes to.
        let bind: SocketAddr = match get("BIND_ADDR") {
            Some(value) => {
                let bind: SocketAddr = value.parse().map_err(|_| "invalid BIND_ADDR")?;
                if railway {
                    if bind.ip().is_loopback() {
                        return Err(
                            "BIND_ADDR must not be loopback on Railway: unset BIND_ADDR so the service listens on 0.0.0.0:$PORT"
                                .into(),
                        );
                    }
                    if port != Some(bind.port()) {
                        return Err("BIND_ADDR port must equal PORT on Railway".into());
                    }
                }
                bind
            }
            None => match port {
                Some(port) => SocketAddr::from(([0, 0, 0, 0], port)),
                None => SocketAddr::from(([127, 0, 0, 1], 3000)),
            },
        };
        let trusted_proxy_cidrs = get("TRUSTED_PROXY_CIDRS")
            .map(|value| {
                value
                    .split(',')
                    .map(str::trim)
                    .map(|item| {
                        if item.is_empty() {
                            return Err("TRUSTED_PROXY_CIDRS contains an empty entry".to_string());
                        }
                        let network = item.parse::<IpNet>().map_err(|_| {
                            "TRUSTED_PROXY_CIDRS must be comma-separated IP CIDRs".to_string()
                        })?;
                        if network.prefix_len() == 0 {
                            return Err(
                                "TRUSTED_PROXY_CIDRS must not contain default routes".to_string()
                            );
                        }
                        Ok(network)
                    })
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?
            .unwrap_or_default();
        let database_url = required("DATABASE_URL")?;
        let db = Url::parse(&database_url).map_err(|_| "invalid DATABASE_URL")?;
        if !matches!(db.scheme(), "postgres" | "postgresql") || db.host().is_none() {
            return Err("DATABASE_URL must be a PostgreSQL URL".into());
        }
        let auth_secret = required("AUTH_SECRET")?;
        if auth_secret.len() < 32 {
            return Err("AUTH_SECRET must be at least 32 bytes".into());
        }
        let origin = required("APP_ORIGIN")?;
        let url = Url::parse(&origin).map_err(|_| "invalid APP_ORIGIN")?;
        let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "::1"));
        if !(url.scheme() == "https" || url.scheme() == "http" && local)
            || url.host().is_none()
            || url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(
                "APP_ORIGIN must be an HTTPS origin (HTTP is allowed only on loopback)".into(),
            );
        }
        if railway {
            if url.scheme() != "https" {
                return Err("APP_ORIGIN must use https on Railway".into());
            }
            if is_loopback_host(&url) {
                return Err(
                    "APP_ORIGIN must be the public https URL of this service on Railway, not localhost"
                        .into(),
                );
            }
        }
        let google = match (get("GOOGLE_CLIENT_ID"), get("GOOGLE_CLIENT_SECRET")) {
            (None, None) => None,
            (Some(id), Some(secret)) if !id.trim().is_empty() && !secret.trim().is_empty() => {
                Some((id, secret))
            }
            _ => return Err("GOOGLE_CLIENT_ID and GOOGLE_CLIENT_SECRET must both be set".into()),
        };
        let storage_names = [
            "STORAGE_ENDPOINT",
            "STORAGE_REGION",
            "STORAGE_BUCKET",
            "STORAGE_ACCESS_KEY",
            "STORAGE_SECRET_KEY",
        ];
        let storage_values: Vec<_> = storage_names.iter().map(|name| get(name)).collect();
        let local_root = get("STORAGE_LOCAL_DIR");
        let object_storage = if let Some(path) = local_root {
            if storage_values.iter().any(Option::is_some)
                || path.trim().is_empty()
                || !std::path::Path::new(&path).is_absolute()
            {
                return Err("STORAGE_LOCAL_DIR must be absolute and cannot be combined with STORAGE_* S3 settings".into());
            }
            Some(StorageConfig::Local {
                root: path.into(),
                origin: url.origin().ascii_serialization(),
                secret: auth_secret.clone(),
            })
        } else if storage_values.iter().all(Option::is_none) {
            None
        } else if storage_values
            .iter()
            .all(|v| v.as_ref().is_some_and(|s| !s.trim().is_empty()))
        {
            let mut values = storage_values.into_iter().map(Option::unwrap);
            Some(StorageConfig::S3(cloud_storage::Config {
                endpoint: values.next().unwrap(),
                region: values.next().unwrap(),
                bucket: values.next().unwrap(),
                access_key: values.next().unwrap(),
                secret_key: values.next().unwrap(),
            }))
        } else {
            return Err("all STORAGE_* settings must be supplied together".into());
        };
        let clamav_addr = get("CLAMAV_ADDR")
            .map(|value| {
                let addr: SocketAddr = value
                    .parse()
                    .map_err(|_| "CLAMAV_ADDR must be a loopback IP and port")?;
                if !addr.ip().is_loopback() || addr.port() == 0 {
                    return Err("CLAMAV_ADDR must be a loopback IP and port");
                }
                Ok(addr)
            })
            .transpose()?;
        let seconds = |name: &str, fallback: u64| -> Result<u64, String> {
            match get(name) {
                Some(value) => value
                    .parse()
                    .map_err(|_| format!("{name} must be an integer number of seconds")),
                None => Ok(fallback),
            }
        };
        let refresh = seconds("GOOGLE_JWKS_REFRESH_SECONDS", 3600)?;
        let stale = seconds("GOOGLE_JWKS_MAX_STALE_SECONDS", 86400)?;
        let retry = seconds("GOOGLE_JWKS_MIN_RETRY_SECONDS", 60)?;
        if !(60..=86400).contains(&refresh)
            || !(refresh..=86400).contains(&stale)
            || !(1..=3600).contains(&retry)
        {
            return Err("invalid Google JWKS policy: refresh must be 60..86400 seconds, max stale must be between refresh and 86400, and retry must be 1..3600".into());
        }
        let jwks_policy = JwksRefreshPolicy {
            refresh_after: Duration::from_secs(refresh),
            max_stale: Duration::from_secs(stale),
            min_retry_interval: Duration::from_secs(retry),
        };
        Ok(Self {
            peer: crate::remote::peers::PeerConfig::parse(get("PEER_RELAY_URLS"))?
                .with_stun_urls(get("RTC_STUN_URLS"))?
                .with_relay_only(get("RTC_RELAY_ONLY"))?,
            bind,
            railway,
            trusted_proxy_cidrs,
            database_url,
            auth_secret,
            origin: url.origin().ascii_serialization(),
            google,
            jwks_policy,
            object_storage,
            clamav_addr,
            admin_dist_dir: get("ADMIN_DIST_DIR")
                .filter(|v| !v.trim().is_empty())
                .map(Into::into),
            temporary_log_approvals: match get("TEMPORARY_LOG_APPROVALS").as_deref() {
                None | Some("false") => false,
                Some("true") => true,
                Some(_) => return Err("TEMPORARY_LOG_APPROVALS must be true or false".into()),
            },
        })
    }
}

fn parse_port(value: Option<String>) -> Result<Option<u16>, String> {
    value
        .map(|value| match value.parse::<u16>() {
            Ok(port) if port != 0 => Ok(port),
            _ => Err("invalid PORT".to_string()),
        })
        .transpose()
}

fn is_loopback_host(url: &Url) -> bool {
    match url.host() {
        Some(url::Host::Domain(domain)) => domain == "localhost" || domain.ends_with(".localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn temporary_log_approval_is_explicit_and_disabled_by_default() {
        assert!(!config(&[]).unwrap().temporary_log_approvals);
        assert!(
            config(&[("TEMPORARY_LOG_APPROVALS", "true")])
                .unwrap()
                .temporary_log_approvals
        );
        assert!(
            !config(&[("TEMPORARY_LOG_APPROVALS", "false")])
                .unwrap()
                .temporary_log_approvals
        );
        assert!(config(&[("TEMPORARY_LOG_APPROVALS", "1")]).is_err());
    }
    fn config(overrides: &[(&str, &str)]) -> Result<Config, String> {
        Config::from_lookup(|key| {
            overrides
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.to_string())
                .or_else(|| match key {
                    "DATABASE_URL" => Some("postgres://localhost/cloud".into()),
                    "AUTH_SECRET" => Some("s".repeat(32)),
                    "APP_ORIGIN" => Some("http://localhost:3000".into()),
                    _ => None,
                })
        })
    }
    #[test]
    fn validates_origin_and_credentials() {
        assert!(config(&[]).is_ok());
        assert!(config(&[("CLAMAV_ADDR", "127.0.0.1:3310")]).is_ok());
        assert!(config(&[("CLAMAV_ADDR", "1.2.3.4:3310")]).is_err());
        assert!(config(&[("CLAMAV_ADDR", "localhost:3310")]).is_err());
        assert!(config(&[("STORAGE_ENDPOINT", "https://example.com")]).is_err());
        assert!(config(&[("APP_ORIGIN", "http://example.com")]).is_err());
        assert!(config(&[("APP_ORIGIN", "https://example.com/path")]).is_err());
        assert!(config(&[("AUTH_SECRET", "short")]).is_err());
        assert!(config(&[("GOOGLE_CLIENT_ID", "id")]).is_err());
        assert!(config(&[("GOOGLE_JWKS_REFRESH_SECONDS", "0")]).is_err());
        assert!(config(&[
            ("GOOGLE_JWKS_MAX_STALE_SECONDS", "3601"),
            ("GOOGLE_JWKS_REFRESH_SECONDS", "3602")
        ])
        .is_err());
        assert!(config(&[("GOOGLE_JWKS_MAX_STALE_SECONDS", "86401")]).is_err());
        assert!(config(&[("GOOGLE_JWKS_MIN_RETRY_SECONDS", "0")]).is_err());
        assert!(config(&[("GOOGLE_JWKS_REFRESH_SECONDS", "1800")]).is_ok());
        assert!(config(&[("TRUSTED_PROXY_CIDRS", "127.0.0.1/32,10.0.0.0/8")]).is_ok());
        assert!(config(&[("TRUSTED_PROXY_CIDRS", "not-a-cidr")]).is_err());
        assert!(config(&[("TRUSTED_PROXY_CIDRS", "0.0.0.0/0")]).is_err());
    }

    #[test]
    fn platform_port_and_explicit_bind() {
        assert_eq!(config(&[]).unwrap().bind.to_string(), "127.0.0.1:3000");
        assert_eq!(
            config(&[("PORT", "8080")]).unwrap().bind.to_string(),
            "0.0.0.0:8080"
        );
        assert_eq!(
            config(&[("PORT", "8080"), ("BIND_ADDR", "127.0.0.1:4000")])
                .unwrap()
                .bind
                .to_string(),
            "127.0.0.1:4000"
        );
        assert!(config(&[("PORT", "0")]).is_err());
        assert!(config(&[("PORT", "abc")]).is_err());
    }

    // Environment for a managed Railway deployment. `set` overrides `base`;
    // `unset` removes a variable entirely.
    fn railway_env(set: &[(&str, &str)], unset: &[&str]) -> Result<Config, String> {
        let base = [
            ("DATABASE_URL", "postgres://db.example.com/cloud"),
            ("AUTH_SECRET", "0123456789abcdef0123456789abcdef"),
            ("APP_ORIGIN", "https://api.example.com"),
            ("RAILWAY_ENVIRONMENT_NAME", "production"),
            ("PORT", "8080"),
        ];
        Config::from_lookup(|key| {
            if unset.contains(&key) {
                return None;
            }
            set.iter()
                .chain(base.iter())
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.to_string())
        })
    }

    #[test]
    fn railway_rules_bind_port_and_origin() {
        // Railway listens on all interfaces at PORT, and any one marker enables the checks.
        let config = railway_env(&[], &[]).unwrap();
        assert!(config.railway);
        assert_eq!(config.bind.to_string(), "0.0.0.0:8080");
        assert!(railway_env(&[("BIND_ADDR", "0.0.0.0:8080")], &[]).is_ok());
        assert!(railway_env(&[("BIND_ADDR", "[::]:8080")], &[]).is_ok());
        let by_project = railway_env(
            &[("RAILWAY_PROJECT_ID", "p"), ("BIND_ADDR", "127.0.0.1:8080")],
            &["RAILWAY_ENVIRONMENT_NAME"],
        );
        assert!(
            by_project.is_err(),
            "project id alone must enable the checks"
        );
        assert!(
            railway_env(
                &[("RAILWAY_SERVICE_ID", "s")],
                &["RAILWAY_ENVIRONMENT_NAME"]
            )
            .unwrap()
            .railway
        );

        // Loopback binds are unreachable from the edge.
        for bind in ["127.0.0.1:8080", "[::1]:8080"] {
            let error = match railway_env(&[("BIND_ADDR", bind)], &[]) {
                Ok(_) => panic!("loopback bind {bind} must be rejected on Railway"),
                Err(error) => error,
            };
            assert!(
                error.contains("loopback") && error.contains("PORT"),
                "{error}"
            );
            assert!(!error.contains("postgres://") && !error.contains("0123456789abcdef"));
        }

        // PORT is required, and an explicit BIND_ADDR must use it.
        let missing = match railway_env(&[], &["PORT"]) {
            Ok(_) => panic!("missing PORT must be rejected on Railway"),
            Err(error) => error,
        };
        assert!(missing.contains("PORT is required"), "{missing}");
        let mismatch = match railway_env(&[("BIND_ADDR", "0.0.0.0:9000")], &[]) {
            Ok(_) => panic!("BIND_ADDR port differing from PORT must be rejected"),
            Err(error) => error,
        };
        assert!(mismatch.contains("must equal PORT"), "{mismatch}");

        // APP_ORIGIN must be the public https origin.
        assert!(railway_env(&[("APP_ORIGIN", "https://api.example.com")], &[]).is_ok());
        assert!(railway_env(&[("APP_ORIGIN", "http://api.example.com")], &[]).is_err());
        for origin in [
            "https://localhost",
            "https://api.localhost",
            "https://127.0.0.1",
            "https://[::1]",
        ] {
            assert!(
                railway_env(&[("APP_ORIGIN", origin)], &[]).is_err(),
                "{origin} must be rejected on Railway"
            );
        }
    }

    #[test]
    fn local_development_keeps_loopback_defaults() {
        let local = config(&[]).unwrap();
        assert!(!local.railway);
        assert_eq!(local.bind.to_string(), "127.0.0.1:3000");
        assert!(config(&[("BIND_ADDR", "127.0.0.1:4000")]).is_ok());
        // Outside Railway an explicit BIND_ADDR ignores PORT, as before.
        assert!(config(&[("BIND_ADDR", "127.0.0.1:4000"), ("PORT", "abc")]).is_ok());
        assert!(config(&[("APP_ORIGIN", "http://localhost:3000")]).is_ok());
    }

    #[test]
    fn local_storage_is_explicit_and_exclusive_with_s3() {
        assert!(matches!(
            config(&[("STORAGE_LOCAL_DIR", "/tmp/cloud-objects")])
                .unwrap()
                .object_storage,
            Some(StorageConfig::Local { .. })
        ));
        assert!(config(&[("STORAGE_LOCAL_DIR", "relative")]).is_err());
        assert!(config(&[("STORAGE_LOCAL_DIR", "")]).is_err());
        assert!(config(&[
            ("STORAGE_LOCAL_DIR", "/tmp/cloud-objects"),
            ("STORAGE_BUCKET", "bucket")
        ])
        .is_err());
    }

    #[test]
    fn jwks_policy_maps_env_values() {
        let policy = config(&[
            ("GOOGLE_JWKS_REFRESH_SECONDS", "1800"),
            ("GOOGLE_JWKS_MAX_STALE_SECONDS", "7200"),
            ("GOOGLE_JWKS_MIN_RETRY_SECONDS", "30"),
        ])
        .unwrap()
        .jwks_policy;
        assert_eq!(policy.refresh_after, Duration::from_secs(1800));
        assert_eq!(policy.max_stale, Duration::from_secs(7200));
        assert_eq!(policy.min_retry_interval, Duration::from_secs(30));
        assert_ne!(policy, JwksRefreshPolicy::default());
    }

    #[test]
    fn jwks_policy_default_skips_startup_reload() {
        // main.rs only reloads Google's JWKS at startup when the configured
        // policy differs from the library default, so a stock configuration
        // must round-trip exactly and never trigger that network call.
        assert_eq!(
            config(&[]).unwrap().jwks_policy,
            JwksRefreshPolicy::default()
        );
    }
}
