# TikTools Cloud — Roadmap

The current single-instance product direction and additive v2 implementation
are defined in [CLOUD-REDESIGN.md](CLOUD-REDESIGN.md) and
[CLOUD-API-V2.md](CLOUD-API-V2.md). They supersede earlier multiple-registry
product assumptions and older missing-feature lists below. Historical rollout
steps remain here; deployment evidence is in IMPLEMENTATION-STATUS.md.

A new independent backend consuming `better-auth-rs` as a library. Checked items describe work completed and locally verified in this repository. Unchecked items include unfinished integration, production validation, and milestone exit criteria. See [IMPLEMENTATION-STATUS.md](IMPLEMENTATION-STATUS.md) for evidence and limitations.

## M0 — Repository and dependency audit (P0)
- [x] Create the Rust Axum workspace, choose MIT licensing, and add CI configuration, pgrust (PostgreSQL 18.6-compatible, self-hosted) Compose configuration, and `.env.example`.
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
- [x] Expose `/me`, locally verify verified-email account-linking constraints, and document secure local token storage.
- [ ] Complete account deletion and its social-only reauthentication flow. Done in-repo slice: `DELETE /api/v1/me` with blocker checks, password re-check, sync removal, and audit retention; verified on pgrust. Still outstanding: social-only re-authentication path and any `native` deletion route.
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
- [x] Model moderator-recorded release yanks and security advisories, enforce them across publication, preparation, approval, candidacy, and verification, and surface them in release, review, and queue reads.
- [ ] Implement public publication and serve yanks, advisories, and revocation state to registry clients. Done in-repo slice: `GET /api/v1/plugins/releases/{id}/revocation` serves release-level yank, advisory, and key-revocation state to unauthenticated clients with integration coverage. Still outstanding: public publication flow, registry-document serving, snapshot promotion/rollback, CDN, and Desktop validation.
- [x] Record audited promotion-readiness (`verified`/`blocked`) decisions for private candidates without deploying.
- [ ] Registry snapshot promotion and rollback; rate limits, quotas, malicious package/URL protections.
**Exit:** an authorized third-party publisher publishes a valid signed plugin without server-side access to root keys.

## M4 — Optional sync (P1)
- [x] Implement experimental schema-versioned plugin-selection records, incremental cursor feed, revisions, and tombstones.
- [ ] Complete the full sync schema and production abuse controls.
- [x] Implement and locally test an opt-in sync client library with conflict handling and offline persistence.
- [ ] Integrate sync into TikTools Desktop and verify the full multi-device offline workflow.
- [ ] Account export/deletion and sensitive data exclusion. Done in-repo slice: bounded export plus `DELETE /api/v1/me` as above. Still outstanding: Desktop integration, multi-device offline workflow, and any broader sensitive-data purge beyond sync state.
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
Prefer modular monolith and separate repositories; keep generic auth code in Better Auth. Preserve the static protocol and signed packaging. End users consume one instance; operators configure their own deployment and independent trust pins. No TikTools-specific changes in the auth library unless generally reusable.
