# Sync API (experimental)

Sync is not connected to TikTools Desktop and is not production-ready. Browser endpoints use session cookies; explicitly bearer-only native equivalents are under `/api/v1/native/sync/`, and `/api/v1/native/me` resolves a bearer session. Browser endpoints do not accept bearer tokens, and native endpoints do not accept cookies. A backend native login handoff and optional keychain adapter are available, but they are not integrated into Desktop; see DESKTOP-AUTH.md. The only accepted collection is `plugin-selections`; all other collections and unknown fields are rejected. Version 1 values have this exact shape:

```json
{"schemaVersion":1,"pluginId":"acme.plugin","version":"1.2.3","enabled":true}
```

The path key must equal `pluginId`; IDs use lowercase ASCII namespace segments and versions must be valid semver. No arbitrary configuration, tokens, credentials or binary data fields are accepted. This schema restriction does not prove that a client has not put sensitive content into an allowed string. Clients must filter their data, and broader schemas require separate review before enabling them.

Authenticated clients use `PUT /api/v1/sync/collections/plugin-selections/{pluginId}`. Create with `If-None-Match: *`; updates require the previous `If-Match: "<revision>"`. A missing precondition returns 428; a stale precondition returns 409. Successful writes return a record and an `ETag`. `DELETE` requires the current `If-Match` and creates a tombstone with a new revision. Recreating a tombstoned record requires its latest revision, rather than a create precondition.

`GET /api/v1/sync/collections/plugin-selections` returns up to 100 current records, including tombstones. Follow `nextAfter` using `?after=<key>` to page. For incremental updates, `GET /api/v1/sync/changes?cursor=0` returns ordered changes, a cursor and `has_more`; repeat with the returned cursor until caught up. The cursor is per user. Writes for each user are serialized with cursor allocation, preventing a later committed change from overtaking an earlier cursor. Persist the cursor only after applying the whole response. Collection pages are not a point-in-time snapshot: use the changes feed to reconcile concurrent writes.

Each account is limited to 1,000 distinct records, including tombstones; a new record beyond that limit returns 507. On each write, only the latest 10,000 changes are retained. A cursor older than that window returns 410. To rebuild, first read `GET /api/v1/sync/head`, then enumerate all collection pages, and finally consume changes starting at that saved head. Apply a record only if its revision is newer than the local revision. If the cursor expires during rebuilding, restart the process. Old history from an upgraded deployment is pruned on its next write. Tombstones count toward the quota and no compaction protocol is implemented.

When Better Auth deletes its user row, foreign keys remove that user's sync cursor, records and retained history in the same database transaction. The migration refuses existing orphan rows for operator investigation. Deletion is blocked while the user holds a Cloud role or publisher membership; those must be removed explicitly first. This does not implement the full Cloud account-deletion policy: audit retention remains unresolved, and the upstream self-deletion route requires a password, which may not exist for social-only accounts. Backups retain data according to the separately required backup policy.

Complete account export, production abuse controls, secret-leakage review, Desktop opt-in, offline conflict UI and other record schemas are not implemented. Do not use this for sensitive or production data yet.

## Experimental client library

`cloud-client` is an opt-in Rust client for the native API. Creating local `SyncState` and staging changes require no network; `Session::connect` explicitly contacts a trusted Cloud origin using a bearer token supplied by the caller. The session keeps the token in memory and disables HTTP redirects; the optional keychain adapter stores tokens separately from sync state. Serialized state contains the origin, account ID, remote revisions and pending edits, but no session token. State is bound to both origin and account so it cannot accidentally be sent under another Cloud identity. Plain HTTP is permitted only for loopback IP literals for local tests.

`StateStore` can persist this state in an application-owned private directory. `load` returns a generation and `save` requires the last loaded generation (or `None` for a new file); a stale writer receives `ConcurrentChange` and must reload and reconcile instead of overwriting another process. It writes a temporary file, syncs it and atomically replaces the state file; on Unix it also syncs the directory and rejects a final directory with group/other mode permissions or a symlink. Locks are advisory: every process must use the same store and must not delete its lock file. The caller must supply a trusted private path, protect ancestor directories and ACLs, use separate directories for different accounts/origins, and run blocking filesystem operations off the UI thread. State is not encrypted. A reported filesystem error after replacement can still mean the write committed, so reload before retrying. Windows ACLs and crash durability have not been verified. Persist after staging and after each successful page/write. A network error retains pending edits. A 409 marks the edit conflicted; pull current changes and present the local and remote values before calling `rebase` to keep the edit or `discard` to discard it. If a write succeeded but its response was lost, retrying may report a conflict: do not silently discard it. A delete with no known remote revision is also treated as a conflict until the caller checks remote state. `rebuild` handles a 410 by reading the head, enumerating current records, reconciling subsequent changes and retaining pending edits; it replaces state only after the entire rebuild succeeds.

This library is not yet integrated into TikTools Desktop. The application must explicitly obtain and store tokens in the OS keychain, select a trusted Cloud origin and private state directory, preserve local-only mode, present conflicts, and handle logout and account switching. No automatic plugin installation or update occurs. The two-session local integration test exercises API behavior, not a complete Desktop end-to-end flow.

## Exporting Cloud sync data

An authenticated user can request `GET /api/v1/sync/export` with a browser session,
or `GET /api/v1/native/sync/export` with a native bearer session. Both responses use
`Cache-Control: no-store` and contain `schemaVersion`, a point-in-time cursor, and
all current plugin-selection records including tombstones and revisions. The cursor
and records come from one repeatable-read database snapshot. Exports are limited to
1,000 records and fail closed if stored records do not match the allowlisted schema;
unknown fields such as credentials are never returned by this endpoint.

This is an export of the current Cloud sync dataset, not a complete account export:
it excludes identity, publisher, moderation, audit, historical changes and local
offline edits. Complete account deletion and audit retention remain separate unresolved work. Protect downloaded exports as user data.

Migration `20261102000015_account_deletion_guards.sql` adds restrictive foreign keys
for publisher memberships and Cloud roles. Existing orphan memberships make migration
fail and must be investigated first. Leave non-owner memberships using `DELETE /api/v1/publishers/{id}/membership`
and revoke Cloud roles through the audited operator command before deletion.
A publisher owner must first transfer ownership to an existing member and may then
leave the publisher. The guard prevents a deletion from bypassing the operator command’s
last-admin check; direct database role changes still require operator controls.
The upstream deletion endpoint may report a generic error when blocked. Audit records,
backups, pending uploads and other historical identifiers are not erased by this guard.

To identify Cloud deletion blockers, an authenticated browser can call
`GET /api/v1/me/deletion-blockers`; a native bearer client can call
`GET /api/v1/native/me/deletion-blockers`. The response lists the caller's Cloud
roles and up to 100 publisher memberships with IDs, namespaces and roles. When
`nextAfter` is non-null, pass it as the `after` query parameter for the next page.
Each page is a separate point-in-time snapshot, so refresh the list after making
changes. Responses are not cached.

An empty list does not guarantee account deletion will succeed: Better Auth
reauthentication, concurrent changes, historical data and retention are outside this
endpoint. The database constraints remain authoritative.

The pinned Better Auth password deletion path performs its database deletes in a
single transaction. A local PostgreSQL integration test exercises deletion blocked
by both a publisher membership and a Cloud role, and confirms the user session,
credential account and Cloud sync state remain after the failure. Cloud does not
currently register user-deletion hooks; any future hook that causes external side
effects before that transaction needs its own failure and retry design. Social-only
deletion remains unsupported by the upstream password-only path.

The native client exposes `Session::export_sync()` for this endpoint. It validates
the export schema, record identities, revisions, duplicate keys and the 1,000-record
limit before returning it. It does not mutate local sync state, merge pending edits,
or write a file; the application must provide an explicit export action and protect
any file it creates. The exported cursor describes the server snapshot only.

Validly shaped sync writes are limited to 120 attempts per account per minute,
shared across browser and native sessions. Conflicting attempts count toward the
limit; a rejected write returns HTTP 429 without advancing the cursor. The native
client reports `SyncError::RateLimited` with the optional retry delay and retains
the pending edit for retry. The
limiter uses Better Auth secondary storage and fails closed if that storage is
unavailable. This per-account limit does not replace deployment-wide or edge abuse
controls.

Collection listing, changes and head reads share a separate 300-attempts-per-minute
account budget across browser and native sessions. Sync export has its own
30-per-minute budget because it can return the entire dataset. Both return HTTP 429 with `Retry-After: 60`
when exhausted and fail closed if secondary storage is unavailable. These limits do
not restrict other accounts and do not replace edge-level protections; clients should
back off before retrying rather than treating a 429 as a conflict.

All throttled sync responses include `Retry-After: 60`, a conservative delay for
the current one-minute limiter window. The client accepts a single numeric delay
of 1–86,400 seconds and reports `None` for missing, malformed, ambiguous, or
out-of-range values. It does not retry automatically.

Native applications can call `Session::deletion_blockers(after)` to retrieve and
validate one page of Cloud roles and publisher memberships. An empty result does
not guarantee deletion; pages are independently read and the UI must refresh them
after changes. This method neither leaves a publisher nor deletes the account.

The pinned Desktop version does not define a core theme or locale setting, and
plugin settings may contain arbitrary data. Those values are not accepted by sync;
new collections require an explicit credential-free schema and a client-compatible
change-feed strategy.
