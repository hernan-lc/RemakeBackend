# RemakeBackend — Web Login and Admin Dashboard Plan

Status: **implemented** (PRs A–D in one change; split per-branch on review
request). Admin read APIs live in `crates/cloud-api/src/admin.rs`; the web
app lives in `web/admin/`; deployment hardening in
`docs/ADMIN-WEB-DEPLOYMENT.md` and `deploy/admin-reverse-proxy.example.conf`.
Keep these changes in a separate PR from the optional local
pgrust runner. The backend remains an independent Rust/Axum service; the
TikTools desktop application stays usable offline.

## Stack and boundaries

- **Frontend:** Vite + Vue 3 + TypeScript, Vue Router, Tailwind CSS,
  shadcn-vue components (Radix Vue-based primitives where appropriate),
  TanStack Query for API/cache, and Pinia only for client-only UI state.
  Vue is preferable to React here because `nglmercer/TikTools-app` already
  uses Vue/Vite. Keep `web/admin/` independent from its desktop WebView.
- **Backend:** reuse existing Axum routes and the pinned `better-auth-rs`
  library; do not reimplement OAuth, account storage, sessions, CSRF, JWKS,
  password handling, or token rotation in JavaScript.
- **Database:** existing SQLx and Better Auth migrations. pgrust or official
  PostgreSQL remains behind the same PostgreSQL wire protocol; the web app
  must never connect directly to PostgreSQL.
- **Browser security:** use cookie-based Better Auth sessions and same-origin
  `/api/` URLs. Vite dev proxy `/api` to the backend; set `APP_ORIGIN` to
  the public browser origin, and verify OAuth redirect and cookie behavior
  through that proxy. Production should put UI and API on one HTTPS origin
  via reverse proxy. Never put a client secret or DB URL in Vite env vars.
  Native desktop bearer tokens must not be stored in the browser.

## Existing functionality (verified against RemakeBackend)

- `/api/auth/*` is backed by the pinned Better Auth/Axum integration.
- `GET /api/v1/me` returns `id`, `email`, `name`, and Cloud `roles`.
- `POST /api/v1/admin/publishers/{id}/approve` checks Cloud admin role and
  blocks approving a publisher of which the admin is a member.
- Moderation endpoints already exist at `/api/v1/moderation/releases*`.
- Cloud roles are separate from publisher membership and can currently be
  bootstrapped via `cloud-api grant-role USER_ID admin OPERATOR`.
- Key enrollment/approval requires validated session-bound step-up grants
  (see STEP-UP-AUTH.md); **the UI must not bypass or toggle this gate** — it
  reauthenticates through the step-up dialog and spends the granted flow.

## Proposed small PRs

### PR A — Vite/Vue admin app and session login

Branch `feat/admin-web-foundation`

1. Scaffold `web/admin/` with typed API client, Vue Router, query client,
   accessible responsive shell and Spanish/English translations.
2. Provide `/login` (Google login if configured, otherwise available Better
   Auth login methods), `/auth/callback`, `/app`, `/403` and `/logout`.
   Use upstream auth entry points; do not hand-craft OAuth token exchange.
3. Load `/api/v1/me` on page refresh; display signed-in identity and roles.
   Route guards are for UX only: the API must independently authorize each call.
4. Sign out through the Better Auth endpoint with CSRF handling; invalidate
   cached user data and redirect. Never persist access tokens in localStorage.
5. Unit tests for anonymous/expired session, 401, 403, session refresh,
   callbacks, and redirects; Playwright cookie + CSRF integration tests.

Exit: auth works after reload and revocation without duplicated auth logic.

### PR B — Minimal read-only admin APIs and dashboard

Branch `feat/admin-read-models`

1. Add `GET /api/v1/admin/overview`: counts derived from existing tables,
   constrained and cached where justified; only accessible to Cloud admins.
2. Add `GET /api/v1/admin/publishers?status=pending&cursor=...`: filtered,
   paginated projection for review (no user secrets).
3. Add `GET /api/v1/admin/audit?cursor=...`: redacted, paginated audit events,
   with server-side role checks and bounded response sizes.
4. Show pending publishers, review status, audit summaries and errors in the
   web dashboard. Reuse existing `POST .../approve` operation.
5. Validate pagination, malicious filters, cross-role 403, and audit redaction.

Exit: a real Cloud admin can review and approve publishers; a normal user
cannot enumerate or mutate administrative data.

### PR C — Moderation workflows and publisher portal

Branch `feat/admin-moderation-ui`

1. Reuse moderation queue, inspection and decision endpoints, with confirmation
   and error handling. Gate key actions behind step-up reauthentication,
   never as authorization to work around the grant requirement.
2. Provide a separate publisher-scoped view for the existing namespace,
   membership and release APIs. Do not equate publisher owner/admin roles with
   Cloud admin.
3. Add accessible loading/empty/error states, pagination and audit links.
   Keep destructive decisions behind explicit confirmation.

Exit: complete negative-path and role-isolation E2E with the actual Rust API.

### PR D — Deployment/security readiness

Branch `chore/admin-web-hardening`

1. Version API DTOs/OpenAPI or equivalent contract tests to detect drift.
2. Test CSRF, cookies, origin/proxy headers, rate limits, logout revocation,
   OAuth callback allowlists and no-store policies against the real server.
3. Secure HTTPS reverse proxy, CSP, secrets policy, structured audit and
   documented deploy/rollback. No publication/key authorization expansion.
4. Verify Linux/macOS local database bootstrap and native pgrust migrations
   with smoke tests; use official PostgreSQL as production fallback until
   pgrust has sufficient reliability evidence.

## Suggested UI navigation

- Dashboard: health, publishers pending, review workload
- Publishers: requests, namespaces, memberships
- Moderation: queued releases, inspections, decisions
- Audit: time-ordered redacted events
- Account: current session and logout

**Explicitly defer:** payments, public marketplace, arbitrary SQL consoles,
browser bearer tokens, and direct user/role management until a separately
reviewed security model exists. Use existing CLI role bootstrap in the interim.

## Integration policy with TikTools-app

The desktop continues local-first with SQLite and independent plugin registry
verification. Use its current Cloud handoff and opt-in sync client integration
plan; do not copy the dashboard into the desktop or require Cloud login for
offline behavior. Native OAuth handoff and browser login are separate clients
of the same auth backend.
