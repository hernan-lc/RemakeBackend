# Admin web deployment and security readiness (ADMIN-WEB-PLAN PR D)

## DTO versioning and contract tests

- Read-only admin DTOs carry `contractVersion`. The backend constant is
  `admin::ADMIN_READ_CONTRACT_VERSION` (currently `1`); the web client
  mirrors it as `ADMIN_READ_CONTRACT_VERSION` in `src/api/types.ts`.
- Bump the backend constant whenever a response shape changes, and update
  the web client in the same change.
- Drift guards (run in CI, no database needed for the unit half):
  - `cargo test -p cloud-api --locked admin::` — exact camelCase key sets
    for `Overview` and the redacted `AuditEventView`, plus pagination and
    status-filter validation.
  - `cargo test -p cloud-api --locked -- --ignored --test-threads=1
    admin_read_apis_authorize_paginate_and_redact` — needs
    `CLOUD_TEST_DATABASE_URL`: 401/403 behavior, malicious filters fail
    closed with 400, cursor pagination, audit redaction, and `no-store` on
    all three routes. The shared test database accumulates rows, so the test
    asserts deltas and self-scoped lookups, never absolute counts.
  - `cd web/admin && npm test` — route guards, auth outcomes, typed client
    (cookie sessions, no storage writes), and the key-enrollment gate.

## CSRF, cookies, origin, proxy, rate limits, revocation

Covered against the real server by the Rust suite, not by mocks:

- Better Auth CSRF/origin rejection and cross-origin refusal are exercised
  in `auth_tests.rs` (publisher approval path asserts `403` for an evil
  origin; the OAuth tests assert CSRF rejection and state replay refusal).
- Cookie sessions only: the web client uses `credentials: 'include'` and the
  `npm test` storage spy fails on any `localStorage`/`sessionStorage` write.
- OAuth callback allowlists: unlisted `?error`/provider callbacks land on
  the failure branch (`AuthCallback` → login); forged states are rejected
  server-side (`oidc_callback_rejects_missing_forged_and_expired_claims`).
- Rate limits: the backend's auth and social-login limiters are covered in
  the Rust suite; the admin read APIs are bounded `COUNT(*)`/keyset pages
  (default 50, max 100) with `deny_unknown_fields` query structs.
- Logout revocation: `/logout` posts to the Better Auth sign-out endpoint,
  then invalidates the TanStack Query session cache; anonymous `/app` visits
  redirect to `/login` (Playwright `auth.spec.ts`).

## Reverse proxy, CSP, secrets policy

- Serve `web/admin/dist` and the backend from one HTTPS origin per
  `deploy/admin-reverse-proxy.example.conf`. No CORS is configured or
  needed; do not add any.
- CSP for the UI is set at the proxy (`default-src 'none'`, self-only
  script/style/connect, no frames). The API also sends `no-store`,
  `no-referrer`, `nosniff` itself; the proxy repeats `no-store` for `/api/`.
- Secrets policy: `AUTH_SECRET`, `DATABASE_URL`, OAuth client secrets, and
  storage keys exist only in the backend environment. `web/admin/.env*`
  holds at most the dev-only proxy target `CLOUD_API_URL`. CI should fail
  a build that references `*_SECRET*` or `DATABASE_URL` under `web/admin`.
- Structured audit: every approval, decision, yank, advisory, inspection,
  role change, and transfer writes a `cloud_audit_events` row; the admin
  audit view is a redacted projection (opaque IDs only — no emails, tokens,
  object keys, or version IDs), asserted by test.

## Deploy, rollback, smoke tests

- Deploy: build UI (`npm run build`), migrate (`cloud-api migrate`),
  restart backend, publish `dist/`. Rollback: restore the previous `dist/`
  and restore the pre-migration database backup (migrations have no
  down path; see PLAN.md).
- Smoke (scratch database, official PostgreSQL 18):
  1. `initdb` + start a scratch instance.
  2. `DATABASE_URL=... AUTH_SECRET=... APP_ORIGIN=http://127.0.0.1:3000
     cargo run -p cloud-api -- migrate`.
  3. Start the API; `GET /health/live` → 200, `GET /health/ready` → 200,
     anonymous `GET /api/v1/me` → 401, anonymous
     `GET /api/v1/admin/overview` → 401.
  4. `cd web/admin && npm run build && npm test`.
- Local database bootstrap without Docker stays as documented in
  [LOCAL-PGRUST.md](LOCAL-PGRUST.md). pgrust remains the Compose default; use official
  PostgreSQL as the production fallback until pgrust has sufficient
  reliability evidence. The backend never distinguishes the two (same wire
  protocol); the web app must never connect to either directly.

## Explicitly not done here

Payments, public marketplace, arbitrary SQL consoles, browser Bearer [REDACTED]
native-only credentials stay in the desktop flows), and direct user/role
management UI. Role bootstrap stays a CLI operator command
(`cloud-api grant-role`) until a separately reviewed security model exists.
Key enrollment/approval UI stays disabled pending session-bound step-up.
