//! Scope-limited native credentials for the Cloud sync API.
//!
//! The ordinary native Bearer token is a full Better Auth session: it proves
//! *who* the user is but carries no scope and cannot be handed to a plugin or
//! an out-of-process sync provider without also handing over every first-party
//! service. This module mints a short-lived Ed25519 JWT whose audience is the
//! sync API and whose scopes are restricted to `sync.read`/`sync.write`.
//!
//! Design constraints (deliberately kept independent of the versioning/Lore
//! work on `feat/versioning-lore-embedding`):
//! - no new runtime/database engine; only an in-process key pair and a
//!   revocation cutoff map;
//! - ordinary session tokens keep working unchanged (backward compatible);
//! - audience + issuer are always verified, so a token minted for this origin
//!   is not accepted by another instance or by an unrelated API;
//! - revocation is checked on every request via a per-subject cutoff.
//!
//! The signing key is generated in memory on startup unless
//! `REMAKE_SYNC_SIGNING_KEY_FILE` points to a PKCS#8 DER file (mode 0600).

use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    extract::{FromRef, FromRequestParts, State},
    http::{header, request::Parts, StatusCode},
    Json,
};
use base64::Engine;
use better_auth::{
    axum_integration::NativeAuthUser,
    email_password::{Session, User},
    session::AuthPrincipal,
};
use ed25519_dalek::{
    pkcs8::{DecodePrivateKey, EncodePrivateKey},
    SigningKey,
};
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use rand_core::OsRng;
use serde::{Deserialize, Serialize};

use crate::{dual_auth::DualUser, AppState};

/// Issuer and audience are embedded in every token and verified on use.
pub const ISSUER: &str = "remake-backend";
pub const AUDIENCE: &str = "remake-sync-v1";
pub const LIFETIME_SECONDS: u64 = 300;
pub const SCOPE_READ: &str = "sync.read";
pub const SCOPE_WRITE: &str = "sync.write";
/// Authorizes the v3 remote-control routes (device/ticket/grant/session
/// management and the relay socket) without granting the full session.
pub const SCOPE_REMOTE: &str = "remote.control";
const KEY_ID: &str = "sync-v1";
const ALLOWED_SCOPES: [&str; 3] = [SCOPE_READ, SCOPE_WRITE, SCOPE_REMOTE];

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

fn random_id() -> String {
    use rand_core::RngCore;
    let mut bytes = [0u8; 16];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Claims {
    pub sub: String,
    /// The instance origin that minted the credential. Verified on use so a
    /// token from another RemakeBackend instance is never accepted here.
    pub origin: String,
    pub iss: String,
    pub aud: String,
    pub iat: u64,
    pub exp: u64,
    /// Millisecond issue time, used for precise revocation cutoffs.
    pub iat_ms: u64,
    pub jti: String,
    pub scopes: Vec<String>,
}

/// Ed25519 JWT signer/verifier for sync credentials.
#[derive(Clone)]
pub struct CredentialBridge {
    origin: String,
    issuer: String,
    audience: String,
    encoding_key: EncodingKey,
    decoding_key: DecodingKey,
    public_bytes: Vec<u8>,
}

impl CredentialBridge {
    pub fn generate(origin: &str) -> Result<Self, String> {
        let signing = SigningKey::generate(&mut OsRng);
        let der = signing
            .to_pkcs8_der()
            .map_err(|e| format!("serialize sync signing key: {e}"))?;
        Self::from_pkcs8_der(der.as_bytes(), origin, ISSUER, AUDIENCE)
    }

    pub fn from_pkcs8_der(
        der: &[u8],
        origin: &str,
        issuer: &str,
        audience: &str,
    ) -> Result<Self, String> {
        let signing = SigningKey::from_pkcs8_der(der)
            .map_err(|e| format!("invalid sync signing key: {e}"))?;
        // ring (used by jsonwebtoken for EdDSA) expects the raw 32-byte public
        // key for verification, not the SPKI/DER encoding.
        let public_bytes = signing.verifying_key().to_bytes().to_vec();
        Ok(Self {
            origin: origin.to_owned(),
            issuer: issuer.to_owned(),
            audience: audience.to_owned(),
            encoding_key: EncodingKey::from_ed_der(der),
            decoding_key: DecodingKey::from_ed_der(&public_bytes),
            public_bytes,
        })
    }

    pub fn mint(
        &self,
        subject: &str,
        scopes: &[String],
        lifetime_seconds: u64,
    ) -> Result<(String, u64), String> {
        if subject.is_empty() || lifetime_seconds == 0 {
            return Err("invalid credential subject or lifetime".into());
        }
        let mut granted = scopes.to_vec();
        granted.sort();
        granted.dedup();
        if granted.is_empty() || granted.len() > ALLOWED_SCOPES.len() {
            return Err("invalid sync scopes".into());
        }
        if granted
            .iter()
            .any(|scope| !ALLOWED_SCOPES.contains(&scope.as_str()))
        {
            return Err("unsupported sync scope".into());
        }
        let iat = now_seconds();
        let claims = Claims {
            sub: subject.to_owned(),
            origin: self.origin.clone(),
            iss: self.issuer.clone(),
            aud: self.audience.clone(),
            iat,
            exp: iat.saturating_add(lifetime_seconds),
            iat_ms: now_ms(),
            jti: random_id(),
            scopes: granted,
        };
        let mut header = Header::new(Algorithm::EdDSA);
        header.kid = Some(KEY_ID.to_owned());
        let token = encode(&header, &claims, &self.encoding_key)
            .map_err(|e| format!("sign sync credential: {e}"))?;
        Ok((token, lifetime_seconds))
    }

    /// Verify signature, origin, issuer, audience, expiry and shape. Scope and
    /// revocation are enforced by the extractor, which knows the operation.
    pub fn verify(&self, token: &str) -> Result<Claims, String> {
        let mut validation = Validation::new(Algorithm::EdDSA);
        validation.set_issuer(&[&self.issuer]);
        validation.set_audience(&[&self.audience]);
        validation.validate_exp = true;
        let data = decode::<Claims>(token, &self.decoding_key, &validation)
            .map_err(|e| format!("invalid sync credential: {e}"))?;
        if data.claims.jti.is_empty() || data.claims.sub.is_empty() {
            return Err("malformed sync credential".into());
        }
        if data.claims.origin != self.origin {
            return Err("credential was minted for a different Cloud origin".into());
        }
        Ok(data.claims)
    }

    pub fn jwks_json(&self) -> String {
        let x = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&self.public_bytes);
        format!(
            r#"{{"keys":[{{"kty":"OKP","crv":"Ed25519","alg":"EdDSA","use":"sig","kid":"{KEY_ID}","x":"{x}"}}]}}"#
        )
    }
}

/// Per-subject revocation cutoffs. A token issued before the cutoff is
/// rejected even if its `exp` is still in the future.
#[derive(Clone, Default)]
pub struct RevocationList {
    cutoffs: Arc<RwLock<HashMap<String, u64>>>,
}

impl RevocationList {
    pub fn revoke_before(&self, subject: &str, at_ms: u64) {
        let mut map = self.cutoffs.write().expect("revocation lock poisoned");
        let entry = map.entry(subject.to_owned()).or_insert(0);
        *entry = (*entry).max(at_ms);
    }

    pub fn is_revoked(&self, subject: &str, issued_ms: Option<u64>) -> bool {
        let map = self.cutoffs.read().expect("revocation lock poisoned");
        match (map.get(subject), issued_ms) {
            (Some(&cutoff), Some(issued)) => issued <= cutoff,
            (Some(_), None) => true,
            (None, _) => false,
        }
    }
}

/// Shared state installed into [`AppState`].
#[derive(Clone)]
pub struct SyncCredentials {
    pub bridge: CredentialBridge,
    pub revocations: RevocationList,
}

impl SyncCredentials {
    pub fn generate(origin: &str) -> Result<Self, String> {
        Ok(Self {
            bridge: CredentialBridge::generate(origin)?,
            revocations: RevocationList::default(),
        })
    }

    /// Load the signing key from a 0600 PKCS#8 DER file, or generate an
    /// in-memory key when the variable is unset. The configured `APP_ORIGIN`
    /// becomes the credential's origin claim; a missing value falls back to
    /// the issuer constant so tokens are still bound to an origin string.
    pub fn from_environment(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, String> {
        let origin = lookup("APP_ORIGIN")
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| ISSUER.to_owned());
        let Some(path) = lookup("REMAKE_SYNC_SIGNING_KEY_FILE") else {
            return Self::generate(&origin);
        };
        if path.is_empty() {
            return Self::generate(&origin);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let meta = std::fs::metadata(&path)
                .map_err(|e| format!("read sync signing key metadata: {e}"))?;
            if meta.permissions().mode() & 0o077 != 0 {
                return Err("sync signing key file must be owner-only (0600)".into());
            }
        }
        let der = std::fs::read(&path).map_err(|e| format!("read sync signing key: {e}"))?;
        Ok(Self {
            bridge: CredentialBridge::from_pkcs8_der(&der, &origin, ISSUER, AUDIENCE)?,
            revocations: RevocationList::default(),
        })
    }
}

async fn load_principal(
    pool: &sqlx::PgPool,
    user_id: &str,
    session: Session,
) -> Result<AuthPrincipal, StatusCode> {
    let row: Option<(String, String, String, bool, Option<String>, Option<String>)> =
        sqlx::query_as(
            "SELECT id, email, name, email_verified, image, additional_fields \
             FROM \"user\" WHERE id = $1",
        )
        .bind(user_id)
        .fetch_optional(pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let (id, email, name, email_verified, image, additional) =
        row.ok_or(StatusCode::UNAUTHORIZED)?;
    let additional_fields = additional
        .and_then(|value| {
            serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&value).ok()
        })
        .unwrap_or_default();
    Ok(AuthPrincipal {
        user: User {
            id,
            email,
            name,
            email_verified,
            image,
            additional_fields,
        },
        session,
    })
}

fn bearer(parts: &Parts) -> Option<&str> {
    let value = parts.headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    value.strip_prefix("Bearer ").map(str::trim)
}

fn looks_like_jwt(token: &str) -> bool {
    token.bytes().filter(|b| *b == b'.').count() == 2
}

/// Pure verification of a credential for one operation scope. Kept separate
/// from request extraction so scope enforcement can be tested without a DB.
fn verify_for_scope(
    bridge: &CredentialBridge,
    revocations: &RevocationList,
    token: &str,
    required_scope: &str,
) -> Result<Claims, StatusCode> {
    let claims = bridge.verify(token).map_err(|_| StatusCode::UNAUTHORIZED)?;
    if !claims.scopes.iter().any(|scope| scope == required_scope) {
        return Err(StatusCode::FORBIDDEN);
    }
    if revocations.is_revoked(&claims.sub, Some(claims.iat_ms)) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(claims)
}

/// Authenticate a scoped native credential when the request presents one.
///
/// `Ok(None)` means the request carries no credential JWT and the caller must
/// fall back to its normal session/cookie authentication. This lets v3 routes
/// (notably remote control) opt into credential support with a single extractor
/// change while keeping their existing ownership checks.
pub async fn authenticate_credential<S>(
    parts: &Parts,
    state: &S,
    required_scope: &str,
) -> Result<Option<AuthPrincipal>, StatusCode>
where
    S: Send + Sync,
    AppState: FromRef<S>,
{
    let app = AppState::from_ref(state);
    let Some(token) = bearer(parts).filter(|token| looks_like_jwt(token)) else {
        return Ok(None);
    };
    let claims = verify_for_scope(
        &app.sync_credentials.bridge,
        &app.sync_credentials.revocations,
        token,
        required_scope,
    )?;
    let session = Session {
        id: format!("sync-credential:{}", claims.jti),
        user_id: claims.sub.clone(),
        expires_at: claims.exp,
    };
    load_principal(&app.pool, &claims.sub, session)
        .await
        .map(Some)
}

async fn authenticate<S>(
    parts: &mut Parts,
    state: &S,
    required_scope: &str,
) -> Result<AuthPrincipal, StatusCode>
where
    S: Send + Sync,
    AppState: FromRef<S>,
    better_auth::AuthContext: FromRef<S>,
{
    if let Some(principal) = authenticate_credential(parts, state, required_scope).await? {
        return Ok(principal);
    }
    // Ordinary native session token: unchanged behaviour, full access.
    NativeAuthUser::from_request_parts(parts, state)
        .await
        .map(|native| native.0)
        .map_err(|_| StatusCode::UNAUTHORIZED)
}

/// Sync read operations accept a session token or a `sync.read` credential.
pub struct SyncReadUser(pub AuthPrincipal);

impl<S> FromRequestParts<S> for SyncReadUser
where
    S: Send + Sync,
    AppState: FromRef<S>,
    better_auth::AuthContext: FromRef<S>,
{
    type Rejection = StatusCode;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        authenticate(parts, state, SCOPE_READ).await.map(Self)
    }
}

/// Sync write operations require a session token or a `sync.write` credential.
pub struct SyncWriteUser(pub AuthPrincipal);

impl<S> FromRequestParts<S> for SyncWriteUser
where
    S: Send + Sync,
    AppState: FromRef<S>,
    better_auth::AuthContext: FromRef<S>,
{
    type Rejection = StatusCode;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        authenticate(parts, state, SCOPE_WRITE).await.map(Self)
    }
}

#[derive(Debug, Deserialize)]
pub struct IssueInput {
    #[serde(default)]
    pub scopes: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct IssueOutput {
    pub token: String,
    pub expires_in_seconds: u64,
    pub scopes: Vec<String>,
}

/// Mint a scoped credential from an authenticated first-party session.
pub async fn issue(
    DualUser(user): DualUser,
    State(state): State<AppState>,
    Json(input): Json<IssueInput>,
) -> Result<Json<IssueOutput>, StatusCode> {
    crate::uploads::limit(&state, &user.user.id, "sync-credential-issue").await?;
    let scopes = if input.scopes.is_empty() {
        vec![SCOPE_READ.to_owned(), SCOPE_WRITE.to_owned()]
    } else {
        input.scopes
    };
    let (token, expires_in_seconds) = state
        .sync_credentials
        .bridge
        .mint(&user.user.id, &scopes, LIFETIME_SECONDS)
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(IssueOutput {
        token,
        expires_in_seconds,
        scopes,
    }))
}

/// Revoke all credentials issued for the current user before now.
pub async fn revoke(
    DualUser(user): DualUser,
    State(state): State<AppState>,
) -> Result<StatusCode, StatusCode> {
    crate::uploads::limit(&state, &user.user.id, "sync-credential-revoke").await?;
    state
        .sync_credentials
        .revocations
        .revoke_before(&user.user.id, now_ms());
    Ok(StatusCode::NO_CONTENT)
}

/// Public JWKS so a credential can be verified without the signing key.
pub async fn jwks(State(state): State<AppState>) -> String {
    state.sync_credentials.bridge.jwks_json()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_ORIGIN: &str = "https://cloud.example";

    fn bridge() -> CredentialBridge {
        CredentialBridge::generate(TEST_ORIGIN).expect("generate key")
    }

    #[test]
    fn minted_credentials_are_scope_audience_and_expiry_bound() {
        let bridge = bridge();
        let (token, ttl) = bridge
            .mint("user-1", &[SCOPE_READ.to_owned()], LIFETIME_SECONDS)
            .unwrap();
        assert_eq!(ttl, LIFETIME_SECONDS);
        let claims = bridge.verify(&token).unwrap();
        assert_eq!(claims.sub, "user-1");
        assert_eq!(claims.iss, ISSUER);
        assert_eq!(claims.aud, AUDIENCE);
        assert_eq!(claims.scopes, vec![SCOPE_READ]);
        assert!(claims.exp > claims.iat);
        assert!(!claims.jti.is_empty());
    }

    #[test]
    fn unsupported_and_empty_scopes_are_rejected() {
        let bridge = bridge();
        assert!(bridge.mint("u", &[], 60).is_err());
        assert!(bridge.mint("u", &["catalog.read".to_owned()], 60).is_err());
        assert!(bridge
            .mint("u", &[SCOPE_READ.to_owned(), SCOPE_WRITE.to_owned()], 0)
            .is_err());
    }

    #[test]
    fn wrong_issuer_audience_or_signature_is_rejected() {
        let issuer = bridge();
        let other = CredentialBridge::generate(TEST_ORIGIN).unwrap();
        let (token, _) = issuer
            .mint("u", &[SCOPE_READ.to_owned()], LIFETIME_SECONDS)
            .unwrap();
        assert!(other.verify(&token).is_err());
    }

    #[test]
    fn revoked_subject_rejects_tokens_issued_before_the_cutoff() {
        let bridge = bridge();
        let (token, _) = bridge
            .mint("user-2", &[SCOPE_READ.to_owned()], LIFETIME_SECONDS)
            .unwrap();
        let claims = bridge.verify(&token).unwrap();
        let revocations = RevocationList::default();
        assert!(!revocations.is_revoked("user-2", Some(claims.iat_ms)));
        revocations.revoke_before("user-2", claims.iat_ms + 1);
        assert!(revocations.is_revoked("user-2", Some(claims.iat_ms)));
        assert!(!revocations.is_revoked("user-3", Some(claims.iat_ms)));
    }

    #[test]
    fn remote_control_scope_is_enforced_on_v3_remote_routes() {
        let bridge = bridge();
        let revocations = RevocationList::default();
        let (sync_token, _) = bridge
            .mint("user-remote", &[SCOPE_READ.to_owned()], LIFETIME_SECONDS)
            .unwrap();
        let (remote_token, _) = bridge
            .mint("user-remote", &[SCOPE_REMOTE.to_owned()], LIFETIME_SECONDS)
            .unwrap();

        // A sync-only credential cannot authorize a remote-control route.
        assert_eq!(
            verify_for_scope(&bridge, &revocations, &sync_token, SCOPE_REMOTE).unwrap_err(),
            StatusCode::FORBIDDEN
        );
        // The remote credential is accepted for that scope.
        let claims = verify_for_scope(&bridge, &revocations, &remote_token, SCOPE_REMOTE).unwrap();
        assert_eq!(claims.sub, "user-remote");
        assert!(claims.scopes.iter().any(|s| s == SCOPE_REMOTE));
        // And it cannot be reused as a sync credential.
        assert_eq!(
            verify_for_scope(&bridge, &revocations, &remote_token, SCOPE_READ).unwrap_err(),
            StatusCode::FORBIDDEN
        );
    }

    #[test]
    fn credentials_are_bound_to_the_issuing_origin() {
        let signing = SigningKey::generate(&mut OsRng);
        let der = signing.to_pkcs8_der().unwrap();
        let issuer =
            CredentialBridge::from_pkcs8_der(der.as_bytes(), TEST_ORIGIN, ISSUER, AUDIENCE)
                .unwrap();
        let (token, _) = issuer
            .mint("user-origin", &[SCOPE_REMOTE.to_owned()], LIFETIME_SECONDS)
            .unwrap();
        let claims = issuer.verify(&token).unwrap();
        assert_eq!(claims.origin, TEST_ORIGIN);

        // Same signing material but a different origin must reject the token,
        // even if issuer/audience/expiry all match.
        let other_origin = CredentialBridge::from_pkcs8_der(
            der.as_bytes(),
            "https://other-instance.example",
            ISSUER,
            AUDIENCE,
        )
        .unwrap();
        assert!(other_origin.verify(&token).is_err());
    }

    #[test]
    fn revoked_credential_cannot_authorize_remote_routes() {
        let bridge = bridge();
        let revocations = RevocationList::default();
        let (token, _) = bridge
            .mint(
                "user-remote-2",
                &[SCOPE_REMOTE.to_owned()],
                LIFETIME_SECONDS,
            )
            .unwrap();
        let claims = bridge.verify(&token).unwrap();
        assert!(verify_for_scope(&bridge, &revocations, &token, SCOPE_REMOTE).is_ok());
        revocations.revoke_before("user-remote-2", claims.iat_ms + 1);
        assert_eq!(
            verify_for_scope(&bridge, &revocations, &token, SCOPE_REMOTE).unwrap_err(),
            StatusCode::UNAUTHORIZED
        );
    }

    #[test]
    fn jwks_is_public_metadata_only() {
        let bridge = bridge();
        let jwks = bridge.jwks_json();
        assert!(jwks.contains("\"kty\":\"OKP\""));
        assert!(jwks.contains("\"alg\":\"EdDSA\""));
        assert!(!jwks.contains("\"d\":"));
    }
}
