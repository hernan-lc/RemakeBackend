# Production readiness

Staging is the only supported deployment. Production promotion is gated and
fails closed until official trust material, authorized operators, and reviewed
infrastructure exist. This document records the gate, the staging runbook, the
ceremonies that must still be executed, and what local validation does and
does not prove.

## Trust gate

`SNAPSHOT_TRUST_ENV` defaults to `staging`: disposable `fixture-*` keys and a
local seed directory are accepted for development and rehearsal runs
(`scripts/build-registry-fixture.sh`, `docs/REGISTRY-OPERATIONS.md`).

`SNAPSHOT_TRUST_ENV=production` fails closed on every snapshot command
(`verify-candidate`, `check-promotion-readiness`, `publish-generation`,
`rollback-generation`, `build-initial-candidate`,
`prepare-publication-once`):

- any root or index key id starting with `fixture-` is refused;
- `SNAPSHOT_ROOT_FINGERPRINT` must be provisioned through a channel
  independent of the pin file and must equal the hex SHA-256 fingerprint of
  the loaded root public key (`PublicKey::fingerprint`).

Until the fingerprint below is provisioned, production mode always refuses:
staging keys and local runs are the only supported path, and no staging
artifact may be presented as an official release.

## Root-key ceremony (documented, NOT executed)

No official trust material exists. No ceremony has been performed. When
production is authorized, perform these steps on an offline machine and
record the evidence; do not improvise a shorter version.

1. On an air-gapped machine, generate the root key and two index keys with
   the pinned TikTools CLI (`keygen --id <name> --role root|index`).
2. Export only the public parts. The root private key never leaves the
   offline machine and is never exposed to Cloud services.
3. Compute the root fingerprint with the CLI and distribute it to operators
   out of band (signed email plus a second channel). The fingerprint is
   public; the distribution channels must be independently authenticated.
4. Provision the fingerprint as `SNAPSHOT_ROOT_FINGERPRINT` on the production
   host through the secrets manager, never through the repository or chat.
5. Store the index private key in a KMS/HSM-backed signing service (not yet
   integrated: `SNAPSHOT_INDEX_KEY_FILE` currently takes a file, so index
   signing still requires a restricted host). Prefer a KMS/HSM integration
   before production.
6. Rehearse rotation and revocation on staging first: generate a successor
   index key, delegate, publish a generation, revoke the predecessor, and
   confirm Desktop clients with the old pin reject new metadata while
   accepting a rebuilt corrected generation. Rollback after a compromise is
   a rebuild, never a restore of revoked material
   (`rollback-generation` refuses compromised keys, revoked digests, yanked
   releases, and stale advisories).

## Staging deployment runbook

1. Provision a PostgreSQL-compatible database, either versioned object
   storage or a private single-instance local volume, and a ClamAV daemon.
   pgrust (`docs/LOCAL-PGRUST.md`) is for local development; the scanner
   fails closed on ambiguity and `CLAMAV_ADDR` must be loopback. See
   `docs/RAILWAY-STAGING.md` for the single-instance configuration.
2. Copy `.env.example` to `.env`, set `AUTH_SECRET` (32+ bytes),
   `APP_ORIGIN` (https in production, loopback http only for dev), storage
   settings as a complete set, and `TRUSTED_PROXY_CIDRS` covering the proxy
   egress (otherwise forwarded-IP rate limiting sees only the proxy peer).
3. `cargo run -p cloud-api -- migrate`, then start behind the reverse proxy
   (`deploy/cloud-api-reverse-proxy.example.conf` for API-only,
   `deploy/admin-reverse-proxy.example.conf` for UI+API).
4. Seed staging trust via `SNAPSHOT_*` files, run `build-initial-candidate`,
   `verify-candidate`, `check-promotion-readiness`, `publish-generation`,
   and confirm `/api/v1/registry/current` advances.
5. Rollback drill: `rollback-generation <generation> <operator>` rechecks
   authorization, yanks, advisories, and revocations before reactivating a
   preserved generation; `gc-registry-artifacts` only collects staged
   leftovers and consumed staging originals.

## Operations baseline (already in the service)

- Health: `/health/live` (liveness) and `/health/ready` (applied
  migrations plus required schema/privilege checks, 3s timeout).
- Logs: structured `tracing` output, `RUST_LOG`/`EnvFilter` controlled.
  No tokens, secrets, or key material are logged.
- Resource limits: 2-4 KiB body limits on JSON routes, Better Auth
  rate limiting on social-login GET/callback paths, connection pool capped
  at 5, graceful shutdown on SIGINT and SIGTERM.
- Security headers: the backend sets `X-Content-Type-Options: nosniff`,
  `X-Frame-Options: DENY`, `Referrer-Policy: no-referrer`, and a restrictive
  `Permissions-Policy` on every response, plus HSTS when `APP_ORIGIN` is
  https. The proxy repeats caching headers so a backend regression cannot
  start caching auth, sync, or moderation data.
- Backups: rehearse `pg_dump`/`pg_restore` per `docs/DATABASE-RECOVERY.md`
  before every image upgrade; pgrust is beta and on-disk formats have
  changed between betas. Object-store versions referenced by `prepared`
  releases must be preserved by bucket policy through publication.

## Validation status (this PR)

Validated locally:

- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets
  --locked -- -D warnings`, `cargo test --workspace --locked`.
- New suites: `snapshot_trust` (5 tests: default staging, invalid env,
  fixture refusal, fingerprint match/mismatch, missing fingerprint) and
  `security_headers` (2 tests: baseline headers, HSTS https-only).
- Disposable PostgreSQL 18 and source-built pgrust database-backed suites
  via `CLOUD_TEST_DATABASE_URL`: 64 passed on each, with the live Google and
  separately configured staging checks excluded. Most tests use FakeS3 and
  fake ClamAV; a local-volume publication test exercises filesystem storage.
- An independently pinned staging registry with nine packages was published,
  backed up with 86 local objects, restored into a fresh PostgreSQL database
  and volume, and installed using the TikTools client. A separate empty-object
  backup and restore succeeded on pgrust. Staging signing keys and the scanner
  signature were disposable test material.

Explicitly NOT validated and NOT claimed:

- Real S3-compatible storage, real ClamAV daemon behavior under load, real
  Google OAuth callbacks, real OS credential stores, KMS/HSM index signing
  (not integrated), key rotation/revocation against live clients, offsite
  recovery or point-in-time restore of a production system, concurrent-request and worker-
  failure load behavior, or cache behavior behind a real CDN.
- Mocked S3/OAuth/scanner coverage does not validate production
  infrastructure.

Production enablement requires verified trust material, operational controls,
and an independent security review. Do not proceed to production with
disposable registry trust keys or unverified external infrastructure.
