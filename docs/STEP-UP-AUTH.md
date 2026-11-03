# Session-bound step-up authentication

Publisher signing-key enrollment and approval require proof that the caller
**recently reauthenticated**, bound to the exact session, action, and target
resource. Step-up verification mints a short-lived, single-use, server-side
grant; the protected endpoints spend it atomically with the mutation. There is
no privileged-session flag and no environment bypass.

## Methods

### Password (`POST /api/v1/step-up/password`)

Body: `{ action, publisherId, keyId?, password }`. The password is re-verified
through Better Auth's `EmailPasswordService::verify_credentials`, which
compares in constant time, runs a dummy verification for unknown accounts,
and returns a generic error either way. The password value is never logged
and no password-derived material is stored outside Better Auth's own tables.

- Wrong password: `401 { error: "invalid_credentials" }`.
- Google-only accounts (no password credential): `403 { error:
  "password_unavailable" }`. These callers must use the OAuth path; they are
  never asked to create a password.
- Attempts are rate-limited per user (10/minute); the password is length
  bounded (1–1024) before any hashing work.

### Google OAuth (`POST /api/v1/step-up/oauth/start`, `GET /api/v1/step-up/oauth/callback`)

Google-only accounts reauthenticate through a **fresh** Authorization Code +
PKCE flow that requests `max_age=300`:

1. `start` stores a one-time state/nonce/PKCE record bound to the caller and
   session (10-minute lifetime) and returns `{ authorizationUrl,
   expiresIn }`. The URL carries `response_type=code`, `scope="openid
   email"`, `state`, `nonce`, `code_challenge`/`S256`, and `max_age=300`.
   `returnTo` must be an in-application path (`/app…`, no scheme, host,
   or fragment) or the request fails with `400`.
2. The browser navigates to the provider and back to
   `/api/v1/step-up/oauth/callback?code=…&state=…`. The pending record is
   **consumed before** the code exchange, so a captured callback URL cannot
   be replayed. The code is exchanged for the exact stored redirect URI; any
   mismatch fails the exchange.
3. The ID token is validated through the library's `JwksIdTokenVerifier`
   (signature, issuer, audience, expiry, nonce) against Google's JWKS, which
   is loaded at startup so a broken key source fails closed before serving.
4. `auth_time` is then required to be present and within the last 300 seconds
   (60 seconds of future skew allowed for clock drift). A missing or stale
   `auth_time` fails closed: an existing login session, a verified email, or
   a refreshed token is **never** treated as recent authentication.
5. The token subject must equal the caller's own linked Google account
   (`account.provider_id='google'`). A token for any other Google account
   fails with `account_mismatch`.
6. Success redirects (`303`) to the stored `returnTo` with a one-time
   `?stepUpGrant=<id>`; every failure redirects with an opaque
   `?stepUpError=<code>` (`provider_denied`, `invalid_callback`,
   `token_exchange_failed`, `invalid_token`, `stale_authentication`,
   `account_mismatch`). The callback requires the same session cookie that
   started the flow; without it the request is `401`.

OAuth step-up is unavailable (`503 { error: "oauth_unavailable" }`) when the
deployment has no Google client configured; password step-up still works.

## Grants

`POST /api/v1/step-up/password` and the OAuth callback return `{ grantId,
expiresIn: 300 }`. Grants live in `cloud_step_up_grants` (migration
`20261102000021_step_up.sql`) and bind user ID, session ID, action
(`publisher.key.enroll` or `publisher.key.approve`), and target resource
(publisher ID, plus the key record ID for approvals). Properties:

- Maximum lifetime: 5 minutes. Expired grants answer `403 {
  error: "step_up_expired" }`; unknown or mismatched grants answer `403 {
  error: "step_up_required" }` without revealing which check failed.
- One-time consumption per sensitive mutation, spent with `UPDATE ...
  WHERE consumed_at IS NULL ... RETURNING` in the **same transaction** as
  the protected mutation. A concurrent second submission finds the row
  consumed and fails.
- Server-side only. The client echoes the opaque ID back in the request body
  (`{ stepUpGrant }`); nothing is kept in localStorage/sessionStorage.
- Session-bound: consumption rechecks the Better Auth session row, so logout,
  session expiry, or account deletion revokes outstanding grants even if the
  grant row itself is still fresh.
- Cross-user, cross-session, cross-publisher, and cross-action use is
  rejected. The enrollment grant spans proof-of-possession challenge issuance
  (validated, not spent) and key registration (spent); the approval grant is
  spent by the approval itself.
- Current RBAC is rechecked inside the consuming transaction (owner/admin
  membership, independent-admin rule, approved publisher status). A role
  revoked between verification and the mutation fails the operation.
- Verification and grant-use attempts are rate-limited per user; callbacks
  additionally per session. All step-up outcomes are audited as
  `step_up.granted` / `step_up.failed` alongside the existing key lifecycle
  audit events.

## Protected endpoints

- `POST /api/v1/publishers/{id}/keys/challenges` — body `{ stepUpGrant }`;
  validates an enrollment grant without spending it.
- `POST /api/v1/publishers/{id}/keys` — enrollment proof plus `{
  stepUpGrant }`; spends the grant with the key insert.
- `POST /api/v1/admin/publishers/{id}/keys/{key_id}/approve` — body `{
  stepUpGrant }`; spends the approval grant with the status update.

All other authorization, publisher-status, key-validation, independent
approval, and audit behavior is unchanged. Revocation stays grant-free.

## Better Auth reuse audit (Task 2.1)

Pinned revision `c694051` (`better-auth-rs`) provides, and this integration
reuses: `EmailPasswordService::verify_credentials` (password re-check with
dummy-verify timing protection), `PkcePair` (S256 code verifiers),
`OAuthProvider::exchange_code` (token redemption), `JwksIdTokenVerifier` /
`OidcIdTokenValidator` (signature, issuer, audience, expiry, nonce), session
resolution with revocation/expiry (`SessionService` via the `AuthUser`
extractor), one-time OAuth state consumption semantics, CSRF enforcement for
unsafe methods, and the per-key rate limiter with secondary storage.

The pinned library does **not** provide: `max_age` authorization parameters,
`auth_time` claim validation (`OidcClaims` carries no `auth_time`), or a
session-bound grant primitive. Those live in `crates/cloud-api/src/step_up.rs`
with the same guarantees, rather than in a forked authentication system or a
second independent login flow. No upstream change was published: the
repository cannot publish a new `better-auth-rs` revision, so the grant
tables, `auth_time` extraction, and the Google step-up flow are implemented
against stable public library APIs. A future upstream step-up API could
replace `cloud_step_up_grants` and the `auth_time` extraction; the endpoint
contract would not need to change.

## Dashboard integration

`web/admin` reuses the existing dashboard. `StepUpDialog.vue` offers password
verification and Google reauthentication for a pending action, keeps
credentials ephemeral (cleared on success, cancel, and close), and returns
the grant to the caller. Google round-trips resume the pending enrollment
(Portal) or approval (Publishers) action from the callback query, consume the
grant into memory, and strip it from the URL. Cancellation, expiration, 401,
403, and 409 outcomes surface distinct messages; 401 returns to `/login`.

## Live-provider behavior

The Google flow depends on Google honoring `max_age` by returning a fresh
`auth_time`. If Google ever omits or backdates `auth_time`, verification
fails closed (`stale_authentication`) and Google-only users cannot complete
step-up until the provider establishes recent authentication; this is
intentional. No test in this repository performs live Google verification:
`crates/cloud-api/src/step_up_tests.rs` uses a localhost OIDC fixture with
`provider_id "fixture"`, fixture JWKS, and throwaway RSA keys, and asserts
real validation (signature, issuer, audience, expiry, nonce, `auth_time`,
account binding) through the production code path.
