# TikTools Cloud — Implementation Plan for Coding Agents

The current single-instance product direction and additive v2 implementation
are defined in [CLOUD-REDESIGN.md](CLOUD-REDESIGN.md) and
[CLOUD-API-V2.md](CLOUD-API-V2.md). They supersede earlier multiple-registry
product assumptions and older missing-feature lists below. Historical rollout
steps remain here; deployment evidence is in IMPLEMENTATION-STATUS.md.

Start from a **new repository**, not a PR targeting `better-auth-rs` or `TikTools-app`. `better-auth-rs` remains a Git-pinned library. Work in focused branches, open reviewable draft PRs, and do not claim tests passed without executing them.

## Progress

The PRs below remain the work breakdown; their status as implemented in this
repository is tracked here, with evidence in [IMPLEMENTATION-STATUS.md](IMPLEMENTATION-STATUS.md)
and exit criteria in [ROADMAP.md](ROADMAP.md). All work is unreleased: publisher
signing-key approval, registry publication, complete sync, desktop integration,
and release gates remain outstanding.

- PR 1 (scaffold and auth spike): implemented — Axum API, layered config,
  tracing, health checks, SQLx/PostgreSQL, explicit Better Auth migrations,
  session-protected `/api/v1/me`, mock OAuth fixture, Compose setup, and CI.
- PR 2 (Google OAuth and native handshake): partially implemented — JWKS refresh
  policy, loopback PKCE handoff with single-use redemption, replay/nonce/audience
  refusal tests, CSRF rejection, trusted-proxy header boundaries, per-client
  redemption rate limits, and an opt-in live Google JWKS smoke test. Remaining:
  real Google end-to-end, process restart and native Desktop client integration.
- PR 3 (official registry): partially implemented — pinned registry protocol and
  CLI fixture with CI checks, private initial-candidate builder with yank and
  advisory support, candidate export and point-in-time verification, and an
  audited, listable promotion-readiness ledger. Remaining: official domain and root keys,
  KMS/HSM signing, snapshot promotion and deployment, CDN and rollback, and
  Desktop validation.
- PR 4 (publishers and RBAC): largely implemented — namespaces, memberships with
  audited role changes, recipient-accepted ownership transfer, member departure,
  Cloud roles with admin approval flow, and key proof-of-possession with
  independent approval implemented but closed pending session-bound step-up.
  Remaining: step-up-gated key enrollment and production hardening.
- PR 5 (upload, review, publication): partially implemented — constrained upload
  reservations, integrity and antivirus verification with a simulated and a
  real-daemon ClamAV path, independent moderation with inspection URLs,
  moderator-recorded yanks and severity-graded advisories enforced across
  publication requests, preparation, approval, candidacy, verification, and
  inspection and surfaced in read APIs, plus audited publication requests with a fenced
  preparation worker. Remaining: public publication, serving yanks and
  advisories to clients, and atomic snapshot promotion and deployment.
- PR 6 (profile sync): partially implemented — revisioned plugin-selection
  records with tombstones and a changes feed, write throttling and read/export
  limits, an opt-in client library with conflict handling and offline
  persistence, account-scoped export, and deletion preparation. Done since:
  `DELETE /api/v1/me` with blocker checks, password re-check, sync removal, and
  audit retention (verified on pgrust). Remaining: full
  schema, Desktop integration with the multi-device offline workflow, and
  complete export/deletion.
- PR 7 (production hardening): partially implemented — readiness gates with
  migration checksums, rate limits, abuse controls, proxy boundaries,
  migration upgrade and `pg_dump`/`pg_restore` drills, audited operator
  commands, and a scoped per-surface feature matrix. Remaining: load and
  security testing, staged deployment and recovery rehearsals, SLOs and
  incident documentation, and independent review.

## PR 1 — Service scaffold and auth integration spike
Branch: `feat/cloud-foundation`
- Create Axum API, layered config validation, tracing/redacted logs, health checks, SQLx/PostgreSQL, migrations and local compose setup.
- Add pinned Better Auth dependency with minimum required features. Reuse `AuthContext`, Postgres adapter, `auth_routes`, `AuthUser`/`NativeAuthUser` and `SessionService`; **do not** duplicate session tables, OAuth validators or handlers.
- Integrate mock OAuth provider and a session-protected `/api/v1/me` route.
- E2E test sign-in/session/sign-out, cached-cookie revocation and restart; document pinned SHA, native-loopback settings and feature/version constraints.
- Ship without production secrets or auto-migrations on server startup.

## PR 2 — Google OAuth and native-client handshake
Branch: `feat/google-oauth`
- Use `HttpOAuthProvider::google_with_jwks`, `JwksRefreshPolicy`, `OAuthStateManager`, PKCE and server-side callback wiring. Do not implement a parallel JWKS refresh scheduler.
- Reuse `OAuthService::with_loopback_redirects` where suitable for native loopback callbacks; handle system-browser launch, local listener and OS keychain in the desktop integration. If backend-mediated exchange is chosen, make it single-use and short-lived.
- Configure the existing JWKS refresh policy and document fail-closed behavior; file upstream issues only for evidenced missing generic APIs.
- E2E: state replay, missing/forged nonce, unverified email, expired session, blocked redirect, account collision.

## PR 3 — Static official registry bootstrap
Branch: `feat/official-registry`
- Reuse TikTools PR #43 registry protocol and CLI for signed static documents.
- Document root/index/publisher key lifecycle, prepare CI artifact generation, CDN deployment and rollback.
- Validate pinned roots, signed index/download in a real TikTools client integration fixture.

## PR 4 — Publishers and RBAC
Branch: `feat/publishers`
- Cloud-owned publisher tables, memberships, key approval and account roles.
- Check permission on every read/mutation and enforce key-to-namespace mapping.
- E2E test cross-publisher access refusal, role changes, key removal, security audit trail.

## PR 5 — Upload, review and publication
Branch: `feat/plugin-publication`
- Signed upload URL, upload completion, antivirus/validation extension, immutable digest and artifact checks.
- Queue verification, moderation, signed snapshot generation and atomic deployment, yanking/revocation/advisories.
- Tests: forged signature, duplicate version, invalid namespace/target, zip-bomb/path traversal, race/idempotency, interrupted publish.

## PR 6 — Optional profile sync
Branch: `feat/profile-sync`
- Revisioned records, namespace ownership, schema allowlists, ETag conflicts, incremental cursors and tombstones.
- Integrate TikTools Desktop via opt-in client, preserving local-only mode.
- E2E: two devices, offline edits, conflict, reconnect, deletion, no secret leakage.

## PR 7 — Production hardening
Branch: `chore/cloud-production-readiness`
- Load/security tests, audit logs, limits, database/object-store restoration, staging migration, SLOs and incident documentation.
- Independent review and upstream Better Auth fixes for security blockers.
- Publish a scoped feature matrix with all limitations; no blanket production-ready statement without evidence.

## Proposed repository layout
```text
TikTools-cloud/
  Cargo.toml
  crates/cloud-api/src/{main.rs,config.rs,routes/,middleware/}
  crates/cloud-domain/src/{accounts/,publishers/,registry/,sync/}
  crates/cloud-infra/src/{database/,storage/,registry_publish/,jobs/}
  migrations/
  tests/{auth/,registry/,sync/}
  deploy/
  docs/
  SPECS.md
  ROADMAP.md
  PLAN.md
```

## Verified upstream availability (2026-11-02)
- Google JWKS single-flight periodic/on-unknown-kid refresh is implemented; use it.
- Restrictive native loopback OAuth redirects are supported; integrate rather than recreate.
- Axum `auth_routes`, `AuthUser` and `NativeAuthUser` are documented; do not write new auth extractors without evidence.
- Provider token persistence is optional; default to not retaining Google access tokens.
- These are code/documentation findings, not evidence of Cloud integration test success.

## Definition of done per PR
- Focused diff, migration notes and rollback procedure.
- Unit + integration + negative security tests, with executed command results.
- No authentication logic reimplemented from Better Auth.
- No static registry protocol break or signed-package verification bypass.
- CI and security scanners pass; skipped tests explicitly disclosed.
- Changes to Better Auth and TikTools Desktop submitted separately when necessary.
