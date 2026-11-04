//! Trust-environment gate for snapshot signing material.
//!
//! Staging is the default: disposable fixture keys (`fixture-*` ids) and a
//! local seed directory are accepted for development and rehearsal runs.
//!
//! Production (`SNAPSHOT_TRUST_ENV=production`) fails closed:
//! - key material whose root or index id starts with `fixture-` is refused;
//! - `SNAPSHOT_ROOT_FINGERPRINT` must be provisioned through a channel
//!   independent of the pin file and must equal the hex SHA-256 fingerprint
//!   of the loaded root public key.
//!
//! Until official trust material is provisioned, production mode always
//! fails: staging keys are never production anchors.
use tiktools_plugin_package::TrustedKey;

/// Which trust rules apply to snapshot signing material.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustEnv {
    Staging,
    Production,
}

/// Read the trust environment. Unset or blank means staging.
pub fn trust_env() -> Result<TrustEnv, String> {
    match std::env::var("SNAPSHOT_TRUST_ENV")
        .ok()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .as_deref()
    {
        None | Some("staging") => Ok(TrustEnv::Staging),
        Some("production") => Ok(TrustEnv::Production),
        Some(other) => Err(format!(
            "SNAPSHOT_TRUST_ENV must be 'staging' or 'production', got '{other}'"
        )),
    }
}

/// Enforce the trust rules for loaded snapshot signing material.
/// Returns the active environment on success.
pub fn require_trust(root: &TrustedKey, index_key_id: &str) -> Result<TrustEnv, String> {
    let env = trust_env()?;
    if env == TrustEnv::Staging {
        return Ok(env);
    }
    let root_id = root.key.key_id.as_str();
    if root_id.starts_with("fixture-") || index_key_id.starts_with("fixture-") {
        return Err(
            "production trust refuses fixture signing keys; provision official trust material"
                .to_owned(),
        );
    }
    let expected = std::env::var("SNAPSHOT_ROOT_FINGERPRINT")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            "SNAPSHOT_ROOT_FINGERPRINT must be provisioned for production trust".to_owned()
        })?;
    let actual = root
        .key
        .fingerprint()
        .map_err(|_| "unable to fingerprint the loaded root pin".to_owned())?;
    let matches = tiktools_plugin_package::Sha256Digest::parse(expected.trim())
        .is_some_and(|expected| expected == actual);
    if !matches {
        return Err("loaded root pin does not match SNAPSHOT_ROOT_FINGERPRINT".to_owned());
    }
    Ok(env)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, MutexGuard};
    use tiktools_plugin_package::{KeyId, PublicKey};

    /// Process env is process-global: serialize every test that touches it.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct EnvGuard {
        _lock: MutexGuard<'static, ()>,
        trust: Option<String>,
        fingerprint: Option<String>,
    }

    impl EnvGuard {
        fn hold() -> Self {
            let guard = Self {
                _lock: ENV_LOCK.lock().unwrap_or_else(|poison| poison.into_inner()),
                trust: std::env::var("SNAPSHOT_TRUST_ENV").ok(),
                fingerprint: std::env::var("SNAPSHOT_ROOT_FINGERPRINT").ok(),
            };
            unsafe {
                std::env::remove_var("SNAPSHOT_TRUST_ENV");
                std::env::remove_var("SNAPSHOT_ROOT_FINGERPRINT");
            }
            guard
        }

        fn set_production(&self, fingerprint: &str) {
            unsafe {
                std::env::set_var("SNAPSHOT_TRUST_ENV", "production");
                std::env::set_var("SNAPSHOT_ROOT_FINGERPRINT", fingerprint);
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            unsafe {
                std::env::remove_var("SNAPSHOT_TRUST_ENV");
                std::env::remove_var("SNAPSHOT_ROOT_FINGERPRINT");
                if let Some(value) = self.trust.take() {
                    std::env::set_var("SNAPSHOT_TRUST_ENV", value);
                }
                if let Some(value) = self.fingerprint.take() {
                    std::env::set_var("SNAPSHOT_ROOT_FINGERPRINT", value);
                }
            }
        }
    }

    fn root_pin(key_id: &str) -> TrustedKey {
        let key = PublicKey {
            key_id: KeyId::parse(key_id).expect("test key id"),
            algorithm: "ed25519".to_owned(),
            public_key: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_owned(),
        };
        serde_json::from_value(serde_json::json!({
            "keyId": key.key_id.as_str(),
            "algorithm": key.algorithm,
            "publicKey": key.public_key,
            "role": "root",
        }))
        .expect("test root pin")
    }

    #[test]
    fn staging_is_default_and_accepts_fixture_keys() {
        let _guard = EnvGuard::hold();
        assert_eq!(trust_env().expect("default env"), TrustEnv::Staging);
        let root = root_pin("fixture-root");
        assert_eq!(
            require_trust(&root, "fixture-index").expect("staging trust"),
            TrustEnv::Staging
        );
    }

    #[test]
    fn invalid_trust_env_is_rejected() {
        let _guard = EnvGuard::hold();
        unsafe {
            std::env::set_var("SNAPSHOT_TRUST_ENV", "prod");
        }
        assert!(trust_env().is_err());
    }

    #[test]
    fn production_refuses_fixture_keys() {
        let _guard = EnvGuard::hold();
        let root = root_pin("fixture-root");
        let fingerprint = root.key.fingerprint().expect("fingerprint").to_string();
        _guard.set_production(&fingerprint);
        assert!(require_trust(&root, "index-1").is_err());
        let root = root_pin("official-root");
        assert!(require_trust(&root, "fixture-index").is_err());
    }

    #[test]
    fn production_requires_matching_fingerprint() {
        let _guard = EnvGuard::hold();
        let root = root_pin("official-root");
        let fingerprint = root.key.fingerprint().expect("fingerprint").to_string();
        _guard.set_production(&fingerprint);
        assert_eq!(
            require_trust(&root, "official-index").expect("production trust"),
            TrustEnv::Production
        );
        _guard.set_production(
            "sha256:0000000000000000000000000000000000000000000000000000000000000000",
        );
        assert!(require_trust(&root, "official-index").is_err());
    }

    #[test]
    fn production_requires_a_provisioned_fingerprint() {
        let _guard = EnvGuard::hold();
        unsafe {
            std::env::set_var("SNAPSHOT_TRUST_ENV", "production");
        }
        let root = root_pin("official-root");
        assert!(require_trust(&root, "official-index").is_err());
    }
}
