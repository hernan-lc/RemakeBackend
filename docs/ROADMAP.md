# TikTools Cloud — Roadmap

A new independent backend consuming `better-auth-rs` as a library. Checked items describe work completed and locally verified in this repository. Unchecked items include unfinished integration, production validation, and milestone exit criteria. See [IMPLEMENTATION-STATUS.md](IMPLEMENTATION-STATUS.md) for evidence and limitations.

## M0 — Repository and dependency audit (P0)
- [x] Create the Rust Axum workspace, choose MIT licensing, and add CI configuration, PostgreSQL Compose configuration, and `.env.example`.
- [ ] Create and configure the `nglmercer/TikTools-cloud` remote repository and validate CI and Compose in their target environments.
- [x] Pin `nglmercer/better-auth-rs` to a reviewed Git SHA and lockfile; record feature flags and declared MSRV.
- [x] Check the full workspace with Rust 1.95 and the all-features client with Rust 1.89; CI now repeats both checks.
- [x] Build a small Better Auth integration spike: migrations, HTTP auth routes, `SessionService` extractor, Google fixture callback and logout.
- [x] Exercise mock OAuth, session cookies/revocation, native callback restrictions, and fresh migrations against the pinned Better Auth revision.
- [x] Verify local PostgreSQL migration upgrades and restore a disposable database from `pg_dump`.
- [x] Fetch and parse Google's live JWKS through the pinned Better Auth provider using an opt-in smoke test.
- [ ] Verify this deployment's scheduled/unknown-key JWKS refresh and outage/stale-key behavior; report proven gaps upstream.
**Exit:** local auth integration compiles and passes mock OAuth + session tests.

## M1 — Identity and security (P0)
- [ ] Integrate existing Better Auth Google login end-to-end; connect desktop system-browser/loopback PKCE flow without reproducing OAuth security logic; add GitHub only on demand.
- [ ] Session resolution, CSRF/CORS, trusted proxies, rate limits, audit trail, key rotation and revocation E2E.
- [ ] Use existing self-refreshing JWKS and loopback OAuth APIs; verify invalid/expired tokens, callback failure modes, proxy boundaries and desktop handoff end-to-end.
- [ ] `/me`, account linking constraints, account deletion and secure local token storage guidance.
**Exit:** repeated negative/replay tests and live callback integration green.

## M2 — Official signed registry (P0)
- [ ] Set official domain/base URL and pin root public keys in TikTools Desktop.
- [ ] Create offline root keys, delegated index/publisher keys and safe key rotation/revocation procedure.
- [x] Generate and validate private initial registry v1 candidates with the pinned TikTools writer and client; stage versioned files and export them for inspection.
- [ ] Generate updates to an existing registry and serve promoted snapshots.
- [ ] Publish official plugins through CI and validate with TikTools Desktop.
**Exit:** anonymous install/update and offline signature verification work with CDN only.

## M3 — Publisher API and moderation (P1)
- [x] Implement publisher namespace requests, memberships, version-bound upload reservations, and signed package checks with local integration coverage.
- [x] Implement recipient-accepted, expiring publisher ownership transfer with audit events.
- [ ] Enable key enrollment and approval after session-bound step-up is available and verified.
- [x] Implement release submission, independent moderation, audited publication requests, and private artifact preparation.
- [ ] Implement public publication, yanks, advisories, and registry revocation propagation.
- [ ] Registry snapshot promotion and rollback; rate limits, quotas, malicious package/URL protections.
**Exit:** an authorized third-party publisher publishes a valid signed plugin without server-side access to root keys.

## M4 — Optional sync (P1)
- [x] Implement experimental schema-versioned plugin-selection records, incremental cursor feed, revisions, and tombstones.
- [ ] Complete the full sync schema and production abuse controls.
- [x] Implement and locally test an opt-in sync client library with conflict handling and offline persistence.
- [ ] Integrate sync into TikTools Desktop and verify the full multi-device offline workflow.
- [ ] Account export/deletion and sensitive data exclusion.
**Exit:** deterministic two-device E2E for edits, conflicts, offline resumption and deletion.

## M5 — Production readiness (P0 before public launch)
- [ ] Migration/restore drills, staged deployment and reproducible CI, observability and incident response.
- [ ] Abuse/load tests for OAuth/public catalogue/upload/sync, S3/CDN and queue failure modes.
- [ ] Independent review of auth, signing pipeline, publisher RBAC and sync isolation.
**Exit:** documented accepted risks, recovery proof, no unmitigated high/critical issues.

## Later (P2)
- [ ] Publisher web dashboard; analytics, ratings, reviews and notifications if justified.
- [ ] Additional identity providers, custom domains and premium features only after actual demand.

## Auth dependency checkpoint (reviewed 2026-11-02)
- [x] Upstream code exposes automatic bounded JWKS refresh for Google.
- [x] Upstream code documents native loopback redirect support and Axum `AuthUser` / `NativeAuthUser`.
- [x] Upstream code documents database-validated revocation, optional token retention and OAuth state replay protections.
- [x] Cloud integration tested against a locked upstream SHA using local OAuth and PostgreSQL fixtures.
- [ ] Live PostgreSQL, desktop callback, staging proxy and independent security testing pass.

## Principles
Prefer modular monolith and separate repositories; keep generic auth code in Better Auth. Preserve PR #43's static protocol, signed packaging and third-party registry support. No TikTools-specific changes in the auth library unless generally reusable.
