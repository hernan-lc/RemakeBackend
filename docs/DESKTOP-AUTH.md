# Desktop handoff (backend foundation)

The backend provides a browser-to-native handoff using the pinned Better Auth `DesktopHandoffService`. The Cloud consent page and reusable native client support the handoff, but they are not integrated into TikTools Desktop. This flow is experimental and has not been reviewed for production.

After authenticating in the system browser, an intentional same-origin POST to `/api/v1/desktop/handoff` requires a cookie session and passes Better Auth's CSRF check. The JSON body has `redirectUri`, `codeChallenge` (PKCE S256), and `state`. The response contains `callbackUrl`. Only HTTP loopback IP literals with an explicit port and non-root path are accepted; the browser must navigate to that URL. The native app must create the loopback listener before starting, generate unpredictable state and a PKCE verifier with a cryptographically secure random generator, verify the returned state, and never put the verifier in the browser URL.

The app sends `code`, the exact `redirectUri`, and `codeVerifier` to `POST /api/v1/native/handoff/redeem`. The code is valid for at most 60 seconds, bound to the fixed `tiktools-desktop` client ID, callback URI and PKCE challenge, and consumed before verification; failed redemption requires starting again. A successful response includes a bearer `sessionToken` and `expiresAt`. Store the token only in the OS keychain and send it as `Authorization: Bearer …` to the explicitly native routes. Do not log tokens, callback URLs or authorization codes. All handoff responses are marked `Cache-Control: no-store`.

## Login methods

Every login surface (admin UI, desktop consent page) accepts a username or an
email address in one identifier field: input containing `@` signs in through
`/api/auth/sign-in/email`, anything else through `/api/auth/sign-in/username`
(before first use, set a username on the signed-in account with
`POST /api/auth/username/set`; usernames are unique). Google sign-in is offered
only when the server is configured with Google credentials, advertised by
`GET /api/v1/auth/providers` (`{"google": true|false}`). With no Google
configuration the button is hidden rather than left to fail.

A Google sign-in from the same verified email address lands on the existing
account automatically (upstream Better Auth verified-email linking); an
unverified local email keeps its account separate and the Google login creates
its own. `GET /api/v1/me/providers` lists the caller's linked providers
(password credential plus social logins, never tokens) and
`POST /api/v1/me/providers/unlink` removes one, refusing the last remaining
method so the account stays reachable. Email delivery (verification,
password reset) is not configured: treat a local email as unverified, so
Google linking requires the Google side to carry the verified address, and
document SMTP provisioning before relying on email verification anywhere.

Issuance is limited per authenticated user; redemption is limited per client IP using Better Auth secondary storage. When the TCP peer belongs to `TRUSTED_PROXY_CIDRS`, a single valid `X-Forwarded-For` IP identifies the client. Missing, malformed, duplicate, or multi-hop values fall back to the socket peer. For other peers Cloud discards supplied forwarding headers and inserts the socket IP. Configure the proxy to overwrite `X-Forwarded-For` with one client IP, limit network access so clients cannot impersonate an allowlisted peer, and independently test edge rate limiting and proxy behavior before deployment. Handoff codes already issued by Better Auth are not currently tied to the lifetime of the issuing browser session: signing out does not explicitly invalidate an outstanding code, which can remain redeemable until consumed or expired. The bearer-only native logout endpoint revokes the current session. There is no refresh flow.

The `cloud-client` `LoginAttempt` generates random state and a PKCE verifier, binds Better Auth's loopback receiver, and returns the consent-page URL. With the optional `system-browser` feature the caller can explicitly open it in the system browser. The page stores only the handoff parameters in tab-scoped session storage, resumes after the Better Auth Google callback returns to `/`, displays the signed-in account and asks for explicit consent. It uses a restrictive CSP, no-referrer and no-store headers. The verifier never enters the browser URL. `complete` waits for the callback, redeems the code, and checks the returned bearer session.

With the optional `os-keychain` feature, `Keychain` uses Linux Secret Service, macOS Keychain, or Windows Credential Manager. It keys entries by a hash of the Cloud origin and account ID and refuses to store a session for another identity. It does not fall back to plaintext files; calls are blocking and should run off the UI thread. The application chooses when to store or delete credentials. On logout, revoke the session first and delete the local keychain entry; if offline, server revocation cannot be confirmed. The OS credential-store backend is selected directly, not through a global mock fallback. Real keychain behavior, unlocked-service availability, Windows and macOS builds, and user-facing recovery have not been tested here.

The local PostgreSQL tests exercise the native client and Better Auth loopback receiver, wrong state, blocked redirect, CSRF rejection, redemption, replay, wrong verifier, expiration, rate limiting and logout. Browser-page tests use a JavaScript DOM mock. They do not exercise an actual browser, a real Google login, an OS keychain or TikTools Desktop.
