# Scoped native sync credentials

**Status:** implemented on `master`; unit-tested. This is the Lore-free
alternative to the scoped credentials on `feat/versioning-lore-embedding`.

## Problem

The native Bearer token issued by the Desktop handoff is a full Better Auth
session. It answers *who* the user is, but it has no scopes and no audience, so
it cannot be given to an out-of-process sync provider or a plugin without also
giving that component every first-party service of the account.

This document defines a second, narrower credential for the Cloud sync API:
a short-lived Ed25519 JWT with an explicit audience and a small scope set.

## Endpoints

| Route | Auth | Purpose |
|---|---|---|
| `POST /api/v1/native/sync-credentials` | native session (Bearer) or browser session | Mint a scoped credential for the current user |
| `POST /api/v1/native/sync-credentials/revoke` | native session or browser session | Revoke all credentials issued before now |
| `GET /api/v1/native/sync-credentials/jwks.json` | public | JWKS for verifying the token signature |

Request body for issuance (optional; defaults to both scopes):

```json
{ "scopes": ["sync.read", "sync.write"] }
```

Response:

```json
{ "token": "<JWT>", "expiresInSeconds": 300, "scopes": ["sync.read", "sync.write"] }
```

## Token contract

- `alg`: `EdDSA` (Ed25519), `kid`: `sync-v1`
- `origin`: the configured `APP_ORIGIN` of the issuing instance
- `iss`: `remake-backend`
- `aud`: `remake-sync-v1`
- `sub`: Better Auth user id
- `iat` / `exp`: 300 second lifetime
- `iat_ms`: millisecond issue time used for precise revocation
- `jti`: random 128-bit identifier
- `scopes`: subset of `sync.read`, `sync.write`, `remote.control`

`origin`, `iss` and `aud` are all verified on every request. A credential
minted by one RemakeBackend instance is rejected by another instance **even if
the two share a signing key**, because the `origin` claim is compared with the
verifier's configured `APP_ORIGIN`. The ordinary opaque session token remains
bound to an instance by its `token_hash` in that instance's database; the
explicit origin claim exists for the credential path used by plugins and
out-of-process providers.

## v3 remote-control scope

The 27 v3 remote-control routes (`hosts`, `devices`, `challenges`,
`invitations`, `grants`, `sessions`, `tickets`, `socket`, `peers`, `rtc-config`,
`people`) use the `RemoteUser` extractor instead of `Authenticated`. It accepts:

- the ordinary browser cookie / native Bearer session (unchanged behaviour);
- a scoped credential carrying `remote.control`.

The token is verified for signature, `iss`, `aud`, expiry and revocation, then
the `sub` user is loaded from the `user` table. Every handler keeps its existing
database ownership and grant checks; the credential only replaces the
full-session bearer at the authentication boundary. A credential for another
scope is rejected with `403`; an invalid or revoked credential with `401`.

Scopes are disjoint: a `remote.control` credential cannot read/write sync data,
and a `sync.read`/`sync.write` credential cannot reach the remote routes.

## Enforcement

The native sync handlers use `SyncReadUser` / `SyncWriteUser` instead of
`NativeAuthUser`:

- an ordinary opaque session token keeps working with full access
  (backward compatibility with existing Desktop clients);
- a JWT credential is verified against the in-process key pair, checked for
  audience/issuer/expiry/revocation, and required to carry the scope for the
  operation (`sync.read` for list/head/changes/export, `sync.write` for
  put/delete);
- the user is loaded from the `user` table by `sub`, so ownership checks in
  `workspace.rs` continue to use the same user id as before.

Missing scope → `403`; invalid or revoked credential → `401`.

## Key management

`REMAKE_SYNC_SIGNING_KEY_FILE` may point to a PKCS#8 DER private key. The file
must be owner-only (`0600` on Unix); the backend refuses to start otherwise.
When the variable is unset, an in-memory key is generated: credentials issued
by that process are invalid after a restart, which is acceptable for local
development and tests.

## Revocation

`revoke_before(subject, now)` records a per-subject cutoff in an in-memory
`RevocationList`. Every token carries `iat_ms`; a token issued at or before the
cutoff is rejected even if `exp` is still in the future. Revocation is currently
process-local: a multi-replica deployment needs a shared cutoff store (the
`versioning_token_cutoffs` table on the feature branch is a model for this).

## Relationship to remote control

The first-party remote-control plugin in TikTools declares the `remote.control`
capability and owns the gateway lifecycle. It authenticates to these v3 routes
with a `remote.control` credential and still goes through the ordinary
`cloud_remote_grants` ownership checks. See the migration note in the Desktop
repository.
