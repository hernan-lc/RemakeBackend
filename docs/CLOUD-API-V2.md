# Cloud API v2

The product consumes one operator-configured RemakeBackend instance. All paths
below are relative to that origin. Requests cannot add a registry, select an
external source, upload root trust pins or change the instance URL. Operators
configure the origin, storage and signing material when deploying; the future
Desktop build must independently pin that instance and its trust roots.

The implementation plan and repository boundary are in
[CLOUD-REDESIGN.md](CLOUD-REDESIGN.md). This API does not change Desktop, the
local control protocol or LIVE collaboration.

## Authentication and transport

Public catalog and capability routes are anonymous. Library, workspace sync
and v2 account export accept a native Better Auth bearer session, or a browser
session cookie with the existing trusted-origin/CSRF checks. An Authorization
header selects native authentication; an invalid bearer never falls back to
an ambient cookie. Moderation routes remain cookie-only and require a live
`moderator` or `admin` role. Author approval is refused even for an admin.

All v2 responses use `Cache-Control: no-store`. Registry generation files
retain their existing immutable caching rules. The body limit is 256 KiB plus
1 KiB for the request envelope; each portable document itself must fit 256 KiB.
Malformed and unknown fields are rejected. This is additive to v1; none of the
existing cookie/native identity, publisher, upload or registry routes move.

## Instance discovery and plugins

| Method | Path | Result |
| --- | --- | --- |
| GET | `/api/v2/capabilities` | API version, single-instance policy, relative routes, collections and limits |
| GET | `/api/v2/catalog/plugins` | Generation-scoped plugin listing |
| GET | `/api/v2/catalog/plugins/{id}` | Published versions, targets, sizes, digests and signed metadata path |

Plugin queries accept `q` (at most 100 bytes, literal substring of the plugin
ID), `after` (plugin ID), `target`, and `generation` (positive integer).
Listings return at most 50 plugins, `nextCursor` and `generation`. Pass the
returned generation with the next page to keep the inventory stable across
publication. Without an active generation, listing returns an empty catalog
and `generation: null`; an unknown/unpublished requested generation or plugin
returns 404. Staged generations never appear.

Versions sort by semantic version. `latestStableVersion` excludes prereleases,
yanked releases, revoked publisher keys and unapproved publishers. Current
yank, advisory and authorization state is read even when the inventory is
pinned to an older generation. `metadataPath` points to this instance's
existing signed index document; its `.sig` sidecar and package signatures
remain authoritative. Catalog JSON and its digests never replace independent
signature verification or root pins. Discovery searches plugin IDs; rich
display metadata remains in the signed package documents.

## Private template and profile library

| Method | Path | Result |
| --- | --- | --- |
| GET, POST | `/api/v2/library/documents` | List own documents or create a private revision |
| GET, PUT, DELETE | `/api/v2/library/documents/{id}` | Read latest draft, append a revision or delete all revisions |
| GET | `/api/v2/library/documents/{id}/versions` | List own revision metadata |
| GET | `/api/v2/library/documents/{id}/versions/{revision}` | Read a specific own revision |
| POST | `/api/v2/library/documents/{id}/versions/{revision}/submit` | Submit a draft for independent review |
| POST | `/api/v2/library/documents/{id}/versions/{revision}/withdraw` | Author withdrawal with a reason |

Create takes `{"kind":"template","document":{...}}` or kind `profile`.
The original `.tiktemplate.json` / `.tikprofile.json` v1 object is stored in
`document`; the envelope is not itself an interchange document. Duplicate
`(owner, kind, document.id)` creates return 409. Read responses contain `id`,
`kind`, `slug`, `revision`, `title`, `sha256`, `status` and `document`.

PUT takes `{"document":{...}}` and `If-Match: "N"`. The kind and interchange
document ID cannot change. Each edit appends a new immutable document revision
and leaves any public pointer untouched. DELETE also requires the current
revision's strong ETag and removes all versions. Missing preconditions return
428; stale revisions return 409. Cross-account access returns 404.

Lists accept `after` (UUID), `kind` and `q` (literal title substring), returning
50 items and `nextCursor`. Revision lists accept numeric `after`, returning
50 revision metadata entries. Content limits are 100 documents, 100 revisions
per document and 8 MiB of JSONB document text per account, including history.
Quota violations return 507. Private reads share the 300/minute read budget;
library writes allow 60/minute per account. Owners serialize quota checks in
the transaction, so concurrent creates cannot bypass quotas.

## Public documents and moderation

| Method | Path | Result |
| --- | --- | --- |
| GET | `/api/v2/catalog/documents` | Current independently published revisions |
| GET | `/api/v2/catalog/documents/{id}` | Current public revision and interchange object |
| GET | `/api/v2/catalog/documents/{id}/versions/{revision}` | A specific published revision |
| GET | `/api/v2/moderation/documents` | Pending documents from other authors, including content |
| POST | `/api/v2/moderation/documents/{id}/versions/{revision}/decision` | Independent approval or rejection |
| POST | `/api/v2/moderation/documents/{id}/versions/{revision}/withdraw` | Moderator withdrawal |

The state machine is `draft -> pending -> published | rejected`; any existing
revision can be withdrawn. Only drafts can be submitted; repeated submission
of a pending revision is idempotent. A rejected revision must be edited into
a new draft before resubmission. Decisions take
`{"decision":"approve","reason":"Reviewed actions and parameters"}` or
`reject`; withdrawal takes `{"reason":"Outdated instructions"}`. Reasons are
required and limited to 500 bytes. Decisions and withdrawals are audited.

Approval revalidates the stored document. Approving an older revision cannot
roll the public pointer back. Earlier approved revisions remain addressable
until individually withdrawn or the document/account is deleted. Withdrawing
the current revision clears its public pointer and never silently restores
older content. Public readers receive 404 for drafts, pending, rejected or
withdrawn revisions. A downloaded copy cannot be remotely recalled.

The review queue emits the oldest pending revision of each document, with
UUID pagination. Refresh from the first page after decisions to discover the
next pending revision of documents already processed. Its read budget is
300/minute; decisions and moderator withdrawals share 60/minute per reviewer.

## Workspace synchronization

| Method | Path | Result |
| --- | --- | --- |
| GET | `/api/v2/sync/head` | Current account cursor |
| GET | `/api/v2/sync/collections/{collection}` | Key-paginated records, including tombstones |
| GET, PUT, DELETE | `/api/v2/sync/collections/{collection}/{key}` | Read or conditionally mutate one record |
| GET | `/api/v2/sync/changes?cursor=N` | Ordered incremental changes |
| GET | `/api/v2/sync/export` | Consistent bounded snapshot for rebuild/backup |

| Collection | Key | Value |
| --- | --- | --- |
| `plugin-selections` | plugin ID | Existing `{schemaVersion:1,pluginId,version,enabled}` schema |
| `rule-templates` | document ID | Original template v1 object |
| `profiles` | document ID | Original profile v1 object with inline templates |
| `plugin-settings` | plugin ID | `{schemaVersion:1,pluginId,settings:{...}}` |
| `preferences` | `desktop` | `{schemaVersion:1,theme?,locale?,reducedMotion?,updateChannel?}` |

Preferences allow `system/light/dark`, a bounded locale identifier, a boolean
and `stable/beta`, respectively. Unknown preferences and collections fail
closed. Plugin/document IDs are validated by `cloud-contracts`.

Create uses `If-None-Match: *`. Replace/delete use exactly one strong
`If-Match: "N"`. Missing preconditions return 428, malformed/weak/repeated
preconditions return 400 and a stale base returns 409. Delete requires an
existing revision and records a tombstone. A matching write with identical
content preserves its revision and emits no change. Repeating a request whose
first response was lost may conflict: read the current record before rebasing.
Neither the server nor SDK silently chooses a side of a conflict.

Records are `{collection,key,revision,deleted,value}`. Snapshots are
`{schemaVersion:2,cursor,records}`. Change pages are
`{changes,cursor,has_more}` with at most 50 entries. Collection pages are
`{records,nextAfter}` and accept a key-valued `after`. Changes from all v2
collections share a monotonically increasing account cursor. Its allocation,
record mutation and feed append commit atomically under the account cursor
lock. Pagination and snapshot rebuilds never use another account's cursor.

The feed retains a suffix bounded by 10,000 changes and 16 MiB of JSONB text
plus a per-row allowance. A cursor older than that suffix returns 410; rebuild
from `/sync/export` and resume at the snapshot cursor. Negative/future cursors
return 400. The feed expiry check and page read hold a shared cursor lock to
prevent concurrent pruning from skipping changes.

Each account allows 1,000 records (including tombstones) and 8 MiB of current
JSONB value text. Writes share 120/minute; reads share 300/minute; exports
allow 10/minute. A 429 response includes `Retry-After: 60`. A tombstone retains
its identity for conflict safety and counts toward the record quota. This
API does not execute, install, enable or download plugins automatically.

V2 has separate tables and cursors. V1 remains plugin-selection-only, including
its account export DTO. There is no implicit migration or double writing
between versions; the subsequent Desktop adapter must explicitly stage its
existing local selections once when enabling v2, then use one sync engine.

## Portable data and credentials

`cloud-contracts` is the shared backend-owned validator, used by the API and
`cloud-client`. It validates Desktop v1 interchange shapes, action/filter
counts, known fields, maximum depth, node counts and string sizes. It rejects
recognized credential fields with literal values, credentials in concrete
URLs and user-configurable registry fields. Sensitive configuration values
can use `{"secretRef":"local-key-name"}` or exact
`"{{ globals.localCredential }}"` references. Values are resolved on the
device; this backend never stores the referenced credential or executes a
template. Importing an action config with `secretRef` requires a Desktop
adapter that resolves it into the plugin's expected local type.

Generic plugin settings and free text cannot establish the absence of secrets
under arbitrary names or encodings. Authors must remove credentials before
uploading, and moderators must inspect public templates. This validator is
not a credential vault or a replacement for plugin-specific secret schemas.
Uploaded action URLs are data: the backend never fetches them.

## Export, deletion and operations

`GET /api/v2/me/export` returns
`{schemaVersion:2,account,workspaceSync,library}` with a 32 MiB response limit.
Each section has a consistent database snapshot; sections may reflect
different instants. This is an account privacy export, not an atomic restore
across all cloud domains. The existing password-confirmed browser
`DELETE /api/v1/me` cascades workspace records, cursors, feed entries and all
owned library versions. Existing publisher/role blockers and audit retention
still apply. Native/social-only account deletion retains its existing limits.

Apply `cloud-api migrate` explicitly before starting the upgraded API; readiness
checks require the new migration and tables. Older binaries become unready on
a newer schema. Rollback uses a pre-migration backup, not destructive down SQL.
No production database is migrated by this change.

Use `cloud-api run-workers` under a service manager for continuous verification
and publication preparation. It reuses the existing fenced workers, exits on
infrastructure errors for supervisor restart, and stops on SIGINT/SIGTERM.
Interrupted work is reclaimed after its existing lease expires. It requires
object storage and loopback ClamAV. Signature provisioning, independent package
review, candidate verification and `publish-generation` remain explicit
operator actions described in [REGISTRY-OPERATIONS.md](REGISTRY-OPERATIONS.md).
The daemon never signs or promotes snapshots automatically.

`cloud-client::CloudCatalog` provides anonymous discovery. Existing `Session`
adds validated workspace snapshots, change pages, conditional records,
library create/read/update/submit/withdraw and v2 account export. It retains
same-origin transport and redirect refusal. Offline persistence/application
of v2 conflicts in Desktop belongs to the later authorized integration.
