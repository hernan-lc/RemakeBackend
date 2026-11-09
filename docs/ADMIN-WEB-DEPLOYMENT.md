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

### Real-browser cookie and Origin regression

The `admin-web` CI job installs dependencies, runs the admin unit tests, builds
the production Vue application, builds the ordinary API, explicitly migrates
a dedicated disposable database and runs Chromium against the built UI served
by that API. It does not use the backend-free demo or a Vite proxy.

The authenticated test creates a disposable account through Better Auth and
signs in through the actual Vue email/password form. Before clicking Sign in,
it registers a bounded 30-second wait for the actual POST
`/api/auth/sign-in/email` response and requires HTTP 200. It then keeps the
five-second navigation assertion and verifies the authenticated identity.
Only this test has a 60-second total budget for real signup/password hashing
and sign-in; it has no retries. Playwright's API request
context shares that browser's real cookie jar and sends a namespace-request
POST with an explicitly rejected Origin. It requires exactly 403 and verifies the
namespace list is unchanged and the session remains valid. The identical
trusted-Origin write through the Vue form must then return 201 and appear in the UI and API. A
missing session, GET-only 405 or authorization failure cannot satisfy that
positive control. No admin role or authentication bypass is seeded.

To run locally, prepare a dedicated loopback PostgreSQL database named
`cloud_admin_e2e` or `cloud_admin_e2e_<suffix>`. Never use an application
database. Build the API and admin UI, then explicitly apply migrations to
that disposable target:

```sh
cargo build --locked -p cloud-api
cd web/admin
npm ci --ignore-scripts
npm run build
npx playwright install chromium
cd ../..
export CLOUD_ADMIN_E2E_DATABASE_URL=postgres://cloud:test-only@127.0.0.1:5432/cloud_admin_e2e
DATABASE_URL="$CLOUD_ADMIN_E2E_DATABASE_URL" \
  AUTH_SECRET=admin-e2e-only-disposable-auth-secret-2026 \
  APP_ORIGIN=http://127.0.0.1:5183 target/debug/cloud-api migrate
cd web/admin
npm run e2e
```

`scripts/admin-e2e-server.mjs` starts the prebuilt API with an isolated
environment, the production `dist` directory and a test-only auth secret.
It refuses a missing, non-loopback or differently named database and never
reads `.env`, migrates, grants roles or contacts Google/object storage.
`ADMIN_URL` may select another explicit HTTP 127.0.0.1 port;
`CLOUD_ADMIN_E2E_BINARY` may select a prebuilt API. The test refuses to reuse
an existing server. Failure traces/screenshots are retained under
`web/admin/test-results` (or `CLOUD_ADMIN_E2E_OUTPUT_DIR`) and contain only
disposable fixture identities/sessions; do not run the harness with real users.

This proves the tested browser-cookie and backend Origin boundary. It does not
prove real Google OAuth, production reverse-proxy behavior, a deployed GitHub
Actions run, or an independent security review.

Local verification on 2026-11-08 used Node 26.13.0, explicitly migrated
disposable databases and the ordinary API serving the production UI build.
Admin typecheck and production build passed, 56 unit tests passed and eight
fixture target-refusal tests passed. Both Chromium tests passed against native
PostgreSQL 18.6 (2/2, 10.6 seconds) and the actual
`malisper/pgrust:v0.4-beta` container (2/2, 11.4 seconds). An earlier pgrust
trace showed signup taking 8.78 seconds and sign-in still pending when the
old five-second navigation deadline expired; the response wait above now
requires real login success before that navigation deadline begins. The
failed trace was preserved and the final run used a separate artifact path.
Workflow YAML and whitespace checks passed. GitHub Actions is configured,
including its pgrust service, but no hosted workflow run was executed here;
the local pgrust browser result is separate evidence.

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
