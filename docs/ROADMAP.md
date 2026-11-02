# TikTools Cloud — Roadmap

A new independent backend consuming `better-auth-rs` as a library. Statuses are **planned**, not implemented.

## M0 — Repository and dependency audit (P0)
- [ ] Create `nglmercer/TikTools-cloud`, Rust Axum workspace, MIT/Apache license decision, CI, Docker Compose PostgreSQL, `.env.example`.
- [ ] Pin `nglmercer/better-auth-rs` to a reviewed Git SHA and lockfile; document feature flags and MSRV.
- [ ] Build a small Better Auth integration spike: migrations, HTTP auth routes, `SessionService` extractor, Google fixture callback and logout.
- [ ] Verify existing JWKS policy, native callback restrictions, session cookies/revocation, and migration compatibility against the pinned Better Auth revision; report only proven gaps upstream.
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
- [ ] Generate and serve registry v1 static snapshots using existing TikTools registry CLI/crates.
- [ ] Publish official plugins through CI and validate with TikTools Desktop.
**Exit:** anonymous install/update and offline signature verification work with CDN only.

## M3 — Publisher API and moderation (P1)
- [ ] Publisher namespace ownership/memberships, key enrollment, upload URLs, signed package checks.
- [ ] Immutable releases, staged moderation, publish/yank/advisory/revocation, audit history.
- [ ] Registry snapshot promotion and rollback; rate limits, quotas, malicious package/URL protections.
**Exit:** an authorized third-party publisher publishes a valid signed plugin without server-side access to root keys.

## M4 — Optional sync (P1)
- [ ] Schema-versioned allowlisted records, incremental cursor feed, revisions, tombstones.
- [ ] Client opt-in sync and multi-device conflicts with offline operation retained.
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
- [ ] Cloud integration verified against a locked upstream SHA.
- [ ] Live PostgreSQL, desktop callback, staging proxy and independent security testing pass.

## Principles
Prefer modular monolith and separate repositories; keep generic auth code in Better Auth. Preserve PR #43's static protocol, signed packaging and third-party registry support. No TikTools-specific changes in the auth library unless generally reusable.
