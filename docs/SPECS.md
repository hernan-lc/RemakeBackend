# TikTools Cloud — Specifications

The current single-instance product direction and additive v2 implementation
are defined in [CLOUD-REDESIGN.md](CLOUD-REDESIGN.md) and
[CLOUD-API-V2.md](CLOUD-API-V2.md). They supersede earlier multiple-registry
product assumptions and older missing-feature lists below. Historical rollout
steps remain here; deployment evidence is in IMPLEMENTATION-STATUS.md.

Status: architecture proposal. New independent Rust backend; do not move authentication code from `nglmercer/better-auth-rs` into this repository.

## Purpose
Build an optional cloud service for `nglmercer/TikTools-app` that provides accounts, Google login, verified plugin publishers, official plugin registry publication, and opt-in cross-device sync. The TikTools desktop application must retain local operation without cloud connectivity. End users consume one instance; self-hosting is configured by the operator at deployment/build time.

## Architecture
- **Repository:** `nglmercer/TikTools-cloud` (proposed).
- **Framework:** Rust, Axum, Tokio, SQLx/PostgreSQL.
- **Auth:** `better-auth-rs` as a Git revision-pinned Rust dependency (later crates.io version). Do not copy source or fork solely for integration.
- **Storage:** PostgreSQL for application data; S3-compatible object storage for `.plugin` archives; static signed registry files on CDN. Redis is optional, only if scale/coordination tests require it.
- **Modules:** `auth_adapter`, `accounts`, `publishers`, `registry`, `releases`, `sync`, `admin`, `jobs`, `observability`.
- **Boundaries:** Better Auth owns authentication identities, OAuth/session security, credentials, and authentication migrations; Cloud owns publisher ownership, signed-artifact ingestion, moderation, sync records, app authorization and audit events.
- **APIs:** `/api/v1/*` for cloud operations; mount Better Auth's framework-compatible routes under `/api/auth/*`. Version cloud APIs separately from the static registry protocol.

## Verified Better Auth library surface (reviewed 2026-11-02)

These capabilities are present in the upstream repository; **their existence does not establish production readiness in the Cloud deployment**. Pin and test a particular commit before integration.

| Need | Existing API/component | Cloud action |
|---|---|---|
| Google OAuth/OIDC | `HttpOAuthProvider::google_with_jwks`, `JwksRefreshPolicy`, `OAuthService`, PKCE and single-use state | Configure and test with local provider fixtures |
| JWKS lifecycle | `jwks.rs` refreshes at an interval and on unknown `kid`; single-flight, bounded staleness and fail-closed expiration | Reuse; no separate scheduler or validator |
| Native login | `OAuthService::with_loopback_redirects` (restricted loopback origins) | Integrate browser launch, callback listener, and OS token storage in desktop; verify end-to-end |
| Axum routing / identity | `better_auth::axum_integration`, `auth_routes`, `AuthUser` and `NativeAuthUser` | Mount `/api/auth/*` and use native bearer only in explicitly chosen routes |
| Session lifecycle | `SessionService` cookie/bearer modes, expiration and database-backed revocation | Test logout/revocation across multiple Cloud instances |
| Account lifecycle | Email/password, Google/GitHub accounts, account linking policy, account deletion | Configure only required flows; avoid keeping provider tokens unless necessary |
| Database | PostgreSQL adapter, secondary storage, migration plan | Run explicit migrations; integration tests with real PostgreSQL |
| Provider token retention | Opt-in `OAuthService::with_provider_token_retention()`; access token encrypted with versioned secrets, no refresh token retained by default | Leave disabled unless Cloud requires downstream Google API access |
| Security policy | CSRF/origin controls, trusted-proxy settings, rate limits, 2FA | Configure and prove correct behind production proxy |
| Production operations | `docs/operations/PRODUCTION.md` | Apply security settings and perform deployment-specific audit |

Remaining risks: Better Auth's parity matrix is not a certification; full independent security review, deployment hardening, live database evidence, migration upgrades and native desktop integration must still be validated. Do **not** recreate a JWKS manager, OAuth exchange or session subsystem in Cloud.

## Required Cloud endpoints (proposed, not existing)
| Method | Endpoint | Auth | Description |
|---|---|---|---|
| GET | `/api/v1/me` | Session | Minimal profile and roles |
| GET | `/api/v1/publishers` | Session | Publisher memberships |
| POST | `/api/v1/publishers` | Session | Request publisher namespace |
| POST | `/api/v1/publishers/{id}/keys` | Publisher admin + step-up | Register signing public key; delegated trust changes require separate approval |
| POST | `/api/v1/plugins/uploads` | Publisher | Create short-lived upload session |
| POST | `/api/v1/plugins/releases` | Publisher | Submit signed package metadata for validation/moderation |
| GET | `/api/v1/plugins/releases/{id}` | Public | Publication status and metadata |
| POST | `/api/v1/plugins/releases/{id}/publish` | Moderator/automation | Publish reviewed release atomically |
| POST | `/api/v1/plugins/releases/{id}/yank` | Authorized publisher/admin | Withdraw discoverability, retain audit |
| GET | `/api/v1/sync/collections/{name}` | Session | List records with cursor |
| PUT | `/api/v1/sync/collections/{name}/{key}` | Session | Revision-checked upsert |
| DELETE | `/api/v1/sync/collections/{name}/{key}` | Session | Tombstone with revision check |
| GET | `/api/v1/sync/changes` | Session | Incremental change feed |
| GET | `/health/live`, `/health/ready` | Ops | Liveness/readiness |

**Registry compatibility:** Keep PR #43 registry protocol (`root.json`, signed `keys.json`/`config.json`, sharded `index`, `catalog/page-*`, artifacts). Generate immutable versioned snapshots and atomically promote a complete signed snapshot. Retain the existing `GET {api}/search` schema. The registry must work anonymously over HTTPS and be independently verified by the desktop; user sessions cannot bypass signature verification.

## Security invariants
1. Never allow an authenticated Google account alone to sign as a package publisher. Require verified namespace membership and delegated public keys.
2. Root registry signing key must remain offline. Protect index signing credentials with restricted CI/HSM/KMS and explicit approvals; never expose them in request handlers.
3. Verify package digest, manifest identity, signature, key scope, archive limits, malware/security policy and target compatibility **before** publishing. Reject unsafe paths and SSRF-capable artifact URLs.
4. OAuth Authorization Code + PKCE, strict state/nonce/redirect checks; use system browser for desktop; no client secrets embedded in desktop builds.
5. Cloud authorization is checked per resource: roles, publisher scope, ownership, and tenant boundaries. Privileged mutations produce redacted audit events.
6. Rate-limit login and publishing; enforce body/archive quotas, presigned-URL expiry, idempotency keys and request IDs. Encrypt secrets and rotate credentials.
7. Sync only allowlisted schemas (profiles, plugin IDs/versions/config without credentials, settings, automation templates). Do **not** sync tokens, cookies, private signing keys, TikTok credentials, or native binaries.
8. Support offline edits with cursor-based incremental sync, ETags/revisions, conflicts (409) and tombstones; avoid silent last-write-wins on important configuration.
9. User deletion must address auth account, cloud records and retention/audit obligations; document backup and disaster recovery.

## Data ownership
- Auth-owned tables: users, accounts, sessions, verification, factors (as defined by Better Auth migrations).
- Cloud-owned tables: `publishers`, `publisher_memberships`, `publisher_keys`, `plugin_releases`, `release_artifacts`, `publication_jobs`, `moderation_events`, `sync_records`, `sync_changes`, `audit_events`.
- Foreign keys/reference policy across auth/cloud schemas must be designed around Better Auth user ID format; never assume numeric IDs.

## Non-goals for v1
Marketplace payments, recommendation algorithms, hosting third-party runtimes, full bi-directional secret backup, plugin execution on server, realtime collaboration, or microservices.

## Release gates
- Integration E2E for Google login, restart/revocation, RBAC, upload/verification/publish/revoke, signed registry client verification, sync conflict/recovery and app-offline behavior.
- Live PostgreSQL test database; migration upgrade and rollback rehearsals; object storage/CDN smoke tests.
- Threat model, abuse testing, secure secret management, observability, backups, independent review before public production launch.
