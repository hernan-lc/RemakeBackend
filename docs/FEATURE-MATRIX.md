# TikTools Cloud — Scoped Feature Matrix

Status vocabulary: **stable** means the contract is covered by maintained tests
with no open functional caveats; **experimental** means it works and is tested
but carries the listed limitations; **absent** means named roadmap scope with no
implementation. Nothing here is production-ready; see the linked design docs.
All browser `POST`/`PUT`/`DELETE` routes require a Better Auth session cookie
and a trusted Origin. Dedicated per-user rate limits cover uploads, release
submission, moderation, publication requests, sync, handoff, publisher creation,
and account export; membership, transfer, key, and admin mutations rely on
per-resource caps and row locks instead.

## Health and web

| Surface | Status | Limits |
|---|---|---|
| `GET /`, `/desktop/login`, `/desktop/login.js`, `/desktop/login.css` | Experimental | Consent page only; resumes after the upstream Google callback. No real-browser or Desktop verification. See [DESKTOP-AUTH.md](DESKTOP-AUTH.md). |
| `GET /health/live` | Stable | Process liveness only. |
| `GET /health/ready` | Experimental | Checks database access, migration versions and checksums, and required columns and privileges. Does not prove indexes, constraints, data, providers, or operations healthy. Older binaries report unready on newer databases. |

## Identity and account

| Surface | Status | Limits |
|---|---|---|
| `GET /api/v1/me`, `GET /api/v1/native/me`, `POST /api/v1/native/logout` | Experimental | Session resolution works; account deletion needs Better Auth re-authentication that social-only users lack. Keychain adapters exist but use platform backends directly without verified OS integration. |
| Email sign-up/sign-in, `POST /api/auth/username/set`, `POST /api/auth/sign-in/username`, `POST /api/auth/is-username-available` | Experimental | Email+password and username+password login work; usernames are lowercase-normalized, unique, and set after email sign-up. Google OAuth is intentionally last and remains unconfigured. |
| `GET /api/v1/me/deletion-blockers`, `GET /api/v1/me/export`, `DELETE /api/v1/me`, `DELETE /api/v1/me/roles/{role}` (+ `native` variants), `DELETE /api/v1/publishers/{id}/membership` | Experimental | Lists blockers, exports bounded account data including recorded yanks, advisories, and validated sync records, revokes own roles, leaves memberships. `DELETE /api/v1/me` removes the account after blockers clear and a password re-check: the auth identity, sessions, credentials, sync records, and cursor are deleted; audit events and shared pipeline rows (uploads, keys, releases, reviews, yanks, advisories, transfers) are retained under the opaque user id. Social-only accounts have no password to re-check and cannot self-delete yet; no `native` deletion route. |
| `POST /api/v1/desktop/handoff`, `POST /api/v1/native/handoff/redeem` | Experimental | Loopback PKCE handoff with single-use short-lived codes. Redemption is rate-limited per trusted client IP behind allowlisted proxies, per socket peer otherwise. No real Desktop, browser, or keychain verification. See [DESKTOP-AUTH.md](DESKTOP-AUTH.md). |
| `POST /api/v1/admin/publishers/{id}/approve`, `grant-role` / `revoke-role` | Experimental | Operator-run with self-supplied labels; role bootstrap is not production-hardened. Deleting a user can still remove the last admin. |
| `GET /api/v1/admin/overview`, `GET /api/v1/admin/publishers`, `GET /api/v1/admin/audit` | Experimental | Cloud-admin-gated read-only projections: counts, keyset pages, redacted audit (opaque IDs only). Bounded limits, `deny_unknown_fields`, `no-store`. See [ADMIN-WEB-DEPLOYMENT.md](ADMIN-WEB-DEPLOYMENT.md). |
| `web/admin` dashboard (login, review, moderation, portal, audit, account) | Experimental | Cookie sessions only, no tokens in storage, key enrollment rendered disabled. Real-browser E2E and production proxy deployment outstanding. See [ADMIN-WEB-PLAN.md](ADMIN-WEB-PLAN.md). |

## Publishers, members, keys

| Surface | Status | Limits |
|---|---|---|
| `POST /api/v1/publishers`, `GET /api/v1/publishers`, `GET /api/v1/publishers/{id}/members`, `PUT/DELETE .../members/{user_id}`, transfers, `DELETE .../membership` | Experimental | Pending requests grant no authority; approval is a manual operator step. Ownership transfers by recipient-accepted expiring offer; a non-owner can leave. No transfer notification UI. |
| `POST .../keys/challenges`, `POST .../keys`, `GET .../keys`, `POST .../keys/{key_id}/revoke` (+ admin list/approve/revoke) | Experimental | Challenge-based proof of possession and revocation work; enrollment and approval **require a session-bound step-up grant** (fresh password verification, or a fresh Google authentication for social-only users). See [PUBLISHER-KEYS.md](PUBLISHER-KEYS.md). |

## Upload, review, publication

| Surface | Status | Limits |
|---|---|---|
| `POST /api/v1/plugins/uploads`, `POST .../uploads/{id}/complete` | Experimental | Constrained reservations (10 pending per publisher) into private quarantine with version capture and digest checks. Moto/MinIO evidence only; no proof of real S3/IAM behavior. See [OBJECT-STORAGE.md](OBJECT-STORAGE.md). |
| `POST /api/v1/plugins/releases`, `GET .../releases/{id}`, `GET .../releases/{id}/review`, `GET .../releases/{id}/revocation` | Experimental | Submissions bind upload, key, and namespace metadata. The revocation read serves release identity with yank, advisory, and key-revocation state to unauthenticated registry clients; tenant, upload, and key identifiers stay hidden. Release and review reads expose yank and advisory state. |
| `POST /api/v1/moderation/releases`, `POST .../{id}/decision`, `POST .../{id}/inspection` | Experimental | Independent audited decisions with conflict-of-interest refusal; 60-second version-bound inspection URLs with no isolated review environment, refused for yanked releases. No appeal or reversal. See [MODERATION.md](MODERATION.md). |
| `POST .../{id}/yank`, `POST .../{id}/advisory` | Experimental | Permanent yanks block publication, preparation, approval, and candidacy; advisories inform and propagate into candidates. No un-yank, no reviewer appeal. See [MODERATION.md](MODERATION.md). |
| `POST /api/v1/plugins/releases/{id}/publish`, `prepare-publication-once`, `retry-publication` | Experimental | Queues an audited request and prepares a read-back-verified private object with fenced leases. The response reports end-to-end publication as unavailable. See [REGISTRY-OPERATIONS.md](REGISTRY-OPERATIONS.md). |

## Registry candidates

| Surface | Status | Limits |
|---|---|---|
| `build-initial-candidate`, `export-candidate`, `verify-candidate`, `check-promotion-readiness` | Experimental | Builds and checks private candidates using the pinned writer. Readiness records do not deploy; promotion is a separate command that repeats the checks. See [REGISTRY-OPERATIONS.md](REGISTRY-OPERATIONS.md). |
| `migrate`, `verify-once`, `prepare-publication-once`, `build-initial-candidate`, `export-candidate`, `verify-candidate`, `check-promotion-readiness`, `list-promotion-checks`, `retry-verification`, `retry-publication` | Supported | Explicit migrations with no auto-migration on startup and no tested rollback beyond restoring a backup. Workers are one-shot commands, not daemons. Recording a promotion check never deploys. See [REGISTRY-OPERATIONS.md](REGISTRY-OPERATIONS.md) and [DATABASE-RECOVERY.md](DATABASE-RECOVERY.md). |

## Sync and client library

| Surface | Status | Limits |
|---|---|---|
| V1 browser and native sync record, changes, export, and head routes | Experimental | Plugin selections only, 120/min shared writes, bounded reads/exports, tombstones, and a 10,000-change feed with cursor rebuilds. No full schema, no Desktop integration, no multi-device E2E. See [SYNC.md](SYNC.md). |
| `cloud-client` opt-in library | Experimental | Conflict handling, offline persistence, cursor rebuilds, and keychain adapters tested with mocks and DOM stand-ins. Not integrated into TikTools Desktop. |

## Product API v2

| Surface | Status | Limits |
| --- | --- | --- |
| `/api/v2/capabilities` | Experimental | Single-instance product contract; removing Desktop registry selectors is a separate authorized integration. |
| `/api/v2/catalog/plugins` and plugin detail | Experimental | Published inventory, generation pinning, semantic versions, current yanks/revocations. Signed metadata remains authoritative. Search covers IDs. |
| `/api/v2/library/documents` and revision routes | Experimental | Private immutable revisions, quotas, account isolation, conditional edits and deletion. No Desktop v2 UI integration. |
| `/api/v2/catalog/documents` and moderation routes | Experimental | Independent approval and withdrawal; moderators use cookie sessions. Generic content still needs review for secrets and action behavior. |
| `POST /api/v2/catalog/documents/{id}/fork` | Experimental | Copies a publicly visible template/profile revision into the caller's library as a private draft; provenance (`forkOf`) is row metadata, never in the portable value. 409 if the caller already has that document id; library quotas and 60/min write limit apply. No branches or parent ids in profiles, by design. Clients built before it fail on `forkOf`. |
| `/api/v2/sync/*` | Experimental | Separate v2 storage, five portable collections plus the credential-free `variables` collection (announced in `sync.optionalCollections`), ETags/conflicts, tombstones, bounded feed/snapshots/quotas. No automatic local application or v1 migration. `variables` records are rejected by clients built before it existed, so enable writing only after every synced device has the new contracts. |
| `/api/v2/me/export` | Experimental | Includes legacy account, v2 workspace and all own library revisions. Existing password-confirmed browser deletion cascades both new stores; social-only/native deletion limits remain. |
| `cloud-contracts` and v2 `cloud-client` adapters | Experimental | Shared validation and same-origin SDK transport; offline Desktop application remains separate. |
| Signed registry generations and rollback | Experimental | Existing backend implementation; operator trust provisioning and production infrastructure remain deployment gates. |
| `run-workers` | Experimental | Continuous verification/preparation with fenced leases; explicit operator signing and promotion remain required. |

## Product API v3: media, overlays, packages

| Surface | Status | Limits |
| --- | --- | --- |
| `/api/v3/assets` (upload, keyset list, read, `PATCH` label, `DELETE`) | Experimental | PNG/JPEG/WebP up to 512 KiB, 200 assets and 32 MiB per account, labels up to 120 characters. Delete is refused (`conflict`) while any overlay revision references the asset. Rasters are validated by header only, not decoded. Assets of deleted accounts are not removed (no foreign key to the user table). |
| `/api/v3/overlays` (create, keyset list, read, revise, revision list and read, `DELETE`) | Experimental | 50 overlays, 100 revisions each and 16 MiB per account; delete refused while a package pins the overlay; private, never in the v2 review. No rendering in the backend and no revision pruning. |
| `/api/v3/packages` (create, read, preflight, `DELETE`) | Experimental | Pins by version/revision; applied only by the client. Deleting a package releases its pinned overlays. |

All v3 additions are covered by PostgreSQL integration tests that ran on PostgreSQL 18.6
(not pgrust; see [implementation status](IMPLEMENTATION-STATUS.md)). Official registry
root-key ceremony, signed generation publication/promotion and the CDN are still external
operator work and are not part of this delivery.

Full routes, quotas and compatibility rules: [CLOUD-API-V2.md](CLOUD-API-V2.md) and
[CLOUD-API-V3.md](CLOUD-API-V3.md).

## Outstanding scope

Desktop v2 integration and removal of end-user registry controls, social-only
and native account deletion, live provider/OS keychain verification, KMS/HSM
signing, production CDN behavior, load/abuse testing and independent production
security review remain outside the verified local backend delivery. Existing
v1 limits above still apply to v1. Public registry serving, promotion and
rollback exist in code; older missing-feature descriptions were stale.
