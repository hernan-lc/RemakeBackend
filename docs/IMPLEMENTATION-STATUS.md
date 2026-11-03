# Implementation status

The plan remains the source of scope; the project is not ready for deployment.

Foundation in this repository: Axum server, configuration validation, PostgreSQL adapter and secondary storage, explicit Better Auth migration command, health endpoints, Better Auth routes, cookie-protected `/api/v1/me`, optional Google provider, versioned Cloud migrations, pending publisher namespace requests and membership-scoped listing with an audit event, local PostgreSQL Compose file, and basic CI. Members may list their publisher membership; only an owner can add, change, or remove non-owner roles, with changes recorded in the audit table. Publisher namespace approval and a signing-key lifecycle are implemented, but key enrollment and approval are disabled until session-bound step-up is available; ownership transfer uses a time-limited offer accepted by an existing member. The pinned auth library has sign-in second-factor challenges; a session-bound step-up integration for key enrollment remains unverified. Better Auth is pinned in the manifest and lockfile. Google provider token retention is not enabled.

Local checks on 2026-11-02: `cargo check --workspace`, `cargo test --workspace --locked` (one config test), and `cargo clippy --workspace --all-targets --locked -- -D warnings` passed. A fresh local PostgreSQL 18 database accepted `cloud-api migrate`; the running API returned 200 for liveness/readiness and 401 for anonymous `/me`. An additional PostgreSQL integration test exercises a local OAuth fixture through the Cloud router: PKCE initiation, state replay and unverified-email refusal, session restoration after reconstructing the app, CSRF rejection, cross-instance logout/revocation with and without cached cookies, and non-retention of provider tokens. It does not cover a real Google callback, an OS process restart, or the desktop flow. Docker Compose could not be exercised because the Compose plugin was unavailable. Better Auth declares Rust 1.89, but the pinned TikTools package crates require Rust 1.95; neither minimum has been tested here.

Still required include real Google OAuth E2E, process restart and native desktop client integration; static registry integration and signing; publisher authorization and audits; package upload, validation, moderation and publishing; opt-in sync and desktop integration; security/load testing, recovery drills, and independent review. See PLAN.md and ROADMAP.md for the full acceptance criteria. Migrations have no tested upgrade or rollback procedure beyond restoring a backup.

Publisher foundation verification: PostgreSQL integration tests passed for two users, including anonymous and cross-origin rejection, invalid and duplicate namespaces, membership isolation, pending status, and audit insertion. A fresh PostgreSQL database accepted the explicit Cloud migration. Key enrollment and approval are disabled pending step-up; complete deletion handling remains unimplemented.

Membership verification: PostgreSQL integration tests exercised non-member isolation, owner-only role changes, cross-origin refusal, unknown users, owner protection, removal and redacted audit role transitions; a repeat role assignment produced no extra audit event. The test database was upgraded from the earlier publisher migration. Concurrent membership operations and account deletion have not been independently stress-tested.

Registry bootstrap verification: checked out TikTools PR #43 at `983a375c5b702681a791061ca863cea9c542528d`; its signed registry client round-trip, tampered/unknown-root rejection, and CLI scaffold/sign/publish tests passed locally. A separate disposable registry was generated using the pinned upstream CLI, and its output contained public registry documents and an archive, with no private key files. CI is configured to repeat these checks and generate a test artifact, but CI itself has not been run here. The generated artifact is now verified with the pinned TikTools registry client and its separately generated root pin, including signed index, archive digest, publisher signature, extraction, wrong-root, wrong-digest and tampered-index checks. A complete Desktop install has not been exercised. Official root provisioning, production signing, snapshot promotion and CDN rollback remain unimplemented; see REGISTRY-OPERATIONS.md.

OIDC verification: a local provider using Better Auth’s test-only HS256 validator exercised the Cloud callback over PostgreSQL. It accepted a valid ID token and rejected absent tokens, invalid signatures, missing or forged nonces, wrong audience, expired tokens, and state replay. This is not a live Google/JWKS or Desktop test. Google JWKS policy is configurable within bounded settings; a changed policy triggers a synchronous reload because the upstream API replaces its cache. Live refresh and outage behavior for this Cloud configuration remain untested.

Sync foundation: an experimental plugin-selection-only schema has per-user records, strong revision preconditions, tombstones, paged listing and a per-user ordered changes feed. PostgreSQL tests covered concurrent conflicting writes, stale edits, tombstones, account isolation and rejection of an unknown token field. Sync now limits records and retains at most the latest 10,000 changes after each write; expired cursors require a documented rebuild. Foreign keys delete sync state with the Better Auth user row. Desktop integration, full schema allowlists, complete Cloud account deletion/export, production abuse controls, secret-leakage review and the multi-device offline workflow remain outstanding; see SYNC.md.

Native transport foundation: `/api/v1/native/me` and native sync routes use Better Auth `NativeAuthUser` to accept bearer sessions without ambient cookies; browser sync remains cookie-only. PostgreSQL tests verified a native write, forged bearer rejection, separation of cookie and bearer routes, and bearer invalidation after account deletion. The backend now exposes a Better Auth single-use PKCE handoff; no Desktop UI, keychain integration, or actual Desktop client is yet wired.

Desktop handoff verification: local PostgreSQL and a Better Auth loopback receiver exercised CSRF and redirect refusal, wrong state, PKCE, single-use redemption, expiration and rate limiting. The socket peer IP is used for redemption limits; proxy deployment, real-browser testing, Google sign-in, real keychain use and independent security review remain outstanding. See DESKTOP-AUTH.md.

Readiness verification: the PostgreSQL integration test checks missing, failed, changed and unknown Cloud migration records, absent auth/Cloud columns, and a closed pool. Liveness stays available. Readiness also checks required table privileges, but it does not verify column types, indexes, constraints, external services or successful application behavior; older binaries report unready when the database has newer Cloud migrations.

Publisher approval: Cloud roles can be granted or revoked by an explicit database operator command, with audited role changes. A non-member Cloud admin can approve a pending publisher, and the decision is audited; this does not confer signing trust. PostgreSQL tests covered self-approval refusal, CSRF, role revocation and audit events. Operator identity is a supplied label, role bootstrap is not production-hardened, and deleting a user can still remove the last admin. See PUBLISHER-APPROVAL.md.

Package verification: the standalone `cloud-verifier` crate copies and checks signed TikTools archives against independently supplied release metadata and a publisher key. Local tests exercised a valid upstream-generated package, forged or revoked/expired keys, digest/identity mismatch, content tampering, traversal and high expansion, plus the disposable CLI fixture. The one-shot worker now connects it to version-pinned uploads and approved-key records; an enforced worker sandbox and publication remain absent; the ClamAV adapter has simulated-protocol tests and a real-daemon smoke test with an artificial signature. See PACKAGE-VERIFICATION.md.

Step-up integration finding: in the pinned Better Auth revision, two-factor enrollment requires a password and the OAuth path refuses users with an enabled second factor rather than completing an OAuth second-factor challenge. Social-only users therefore cannot safely use this flow as-is; publisher key enrollment remains closed pending a suitable upstream integration.

Native sync client: the opt-in `cloud-client` library stages local plugin-selection edits, binds serialized state to origin and account, preserves pending edits on network errors and conflicts, supports cursor rebuilds and rejects HTTP redirects. Tests exercised two sessions against the Cloud API, conflict/rebase/delete, expired-cursor rebuild, offline preservation, account/origin isolation and token exclusion from serialized state. This is a reusable library, not a Desktop integration; keychain and local persistence adapters now exist, while UI integration, actual offline app behavior and broader schemas remain outstanding.

Local sync persistence: `StateStore` adds versioned, generation-checked state files with advisory locking, atomic replacement and Unix file/directory syncing. Tests covered stale local writers, invalid files, private directory handling and a local API reconnect after persisting an offline edit. This does not integrate with Desktop or an OS keychain; Windows ACLs and crash recovery remain unverified.

Desktop login client: the Cloud consent page resumes after the upstream Google callback, and `cloud-client` supplies PKCE generation, Better Auth loopback reception, optional system-browser opening and a bearer logout method. An optional OS keychain adapter uses platform backends directly. Local PostgreSQL tests exercised the native handoff and logout; JavaScript tests use DOM mocks and keychain tests use a mock credential. Real Google, system browser, OS keychains, cross-platform builds, and TikTools Desktop integration are not verified.

Object-storage foundation: `cloud-storage` can generate constrained S3 quarantine upload URLs, require bucket versioning, capture a version and stream it with a digest check. Local unit tests inspect signed headers; a disposable Moto smoke test exercised conditional PUT, version capture and digest checking. An API integration test exercises authorization and concurrent completion against a simulated S3 endpoint. The authenticated API now creates idempotent, publisher-authorized upload reservations and queues immutable versions; a one-shot worker now checks integrity once release metadata is attached; a ClamAV adapter is connected, while publication is not implemented. A moderation API records independent decisions and issues short-lived version-bound inspection URLs; an isolated inspection environment is not provided. Moto is not proof of real S3 signature/IAM behavior. A later disposable MinIO smoke test passed; see OBJECT-STORAGE.md.

Publisher signing-key lifecycle: challenge-based proof of possession, independent approval, scoped listing and irreversible revocation are implemented and exercised by a test-only harness. The normal server refuses enrollment and approval because the required session-bound step-up capability is missing; revocation remains available. The integrity and initial-candidate workers read these records; no public registry is deployed. See PUBLISHER-KEYS.md.

Release submission and integrity worker: an authenticated request binds a quarantined upload to namespace-scoped metadata and an approved publisher key. A one-shot worker claims it with a fenced lease, downloads the recorded object version, verifies digest and package integrity, and rechecks key authorization before recording the result. ClamAV scanning is connected; the full worker is tested with a simulated scanner, and the adapter has a separate real-daemon smoke test; moderation decisions are implemented, but an isolated reviewer environment and publication remain absent, and the process sandbox and real storage still require validation. See RELEASE-VERIFICATION.md.

Moderation: reviewers with a Cloud admin or moderator role can list eligible, antivirus-passed releases and record one independent, audited decision. Approval rechecks publisher and key status; conflicts of interest and competing decisions are refused. Reviewers can request short-lived version-bound inspection URLs; the service does not provide an isolated review environment, appeal or reversal, and does not publish an artifact. See MODERATION.md.

Publication request: an independent moderator or admin can queue a reviewed release after rechecking publisher, key, antivirus and review state. A one-shot preparer now reverifies, rescans, and stores a read-back-verified private object version with fenced leases. The response still reports end-to-end publication as unavailable. An initial-only candidate worker now signs and stages private files, but no public release or deployment worker exists yet; see REGISTRY-OPERATIONS.md.

Snapshot candidate builder: the standalone `cloud-snapshot` library checks independently pinned and root-signed trust material, verifies an explicitly supplied release inventory, and uses the pinned TikTools writer to produce and recheck a private signed candidate. A real TikTools client verified the local candidate in tests. An initial-candidate worker now reads every prepared job, checks its current authorization, rescans and builds a candidate, and records privately staged file versions after inventory rechecks. It does not handle an existing published inventory, guarantee later authorization, or deploy. See REGISTRY-OPERATIONS.md.

Initial candidate worker: an operator command connects prepared jobs to the signed candidate builder, stores versioned private files and records an inventory digest. It is restricted to an empty initial seed, uses point-in-time checks, and has no promotion or cleanup. Simulated-storage tests cover authorization changes and corrupt read-back; real storage, scanner, KMS and deployment remain unverified.

Candidate export: an operator command reconstructs a private candidate from exact stored versions and checks hashes and a stable database manifest before writing a completion marker. Tests verify the reconstructed registry with the pinned TikTools client and reject corrupt data or a changing manifest. Export does not establish current authorization or deploy the candidate.

Candidate verification: an operator command reconstructs a private candidate and
checks its stored hashes, registry signatures against independently supplied public
pins, release links, current point-in-time authorization, and initial candidate
index and catalog completeness against that inventory. It also compares independently
supplied signed policy and rejects revoked digests. It reverifies package
signatures and rescans candidate artifacts, but does not authorize promotion;
changes can occur after it returns. Scanner integration is tested with a simulated
service, not a production ClamAV deployment. See REGISTRY-OPERATIONS.md.

Real ClamAV adapter smoke test: ClamAV 1.4.6 in a disposable local container,
with an isolated custom signature database, returned clean and detected results over
INSTREAM for the Rust adapter, including detection within a deflated ZIP archive.
This does not validate production signatures, other archive formats, scanner
configuration, or freshness; the test is opt-in.

MinIO smoke test: a disposable MinIO 2025-04-22 server enforced signed upload headers, checksum and conditional writes, and supported version-pinned quarantine and prepared-object readback. Production S3, IAM and retention remain unverified; see OBJECT-STORAGE.md.

Sync export: authenticated browser and native endpoints return a bounded, consistent
snapshot of the current allowlisted sync records and tombstones. PostgreSQL tests
cover account isolation, authentication, cache headers, and fail-closed handling of
unexpected stored fields. It is not a complete account export or deletion workflow.

Account deletion guard: restrictive foreign keys prevent auth-user deletion while
Cloud roles or publisher memberships remain, requiring explicit role and membership
removal. Existing orphan memberships block migration for investigation. This is
not complete account deletion: social-only reauthentication,
audit retention, and user-facing error handling remain unfinished.

Publisher ownership transfer: the owner offers to an existing member, who must
accept a unique expiring offer. Role updates and audits are transactional; replaced,
expired, cancelled and replayed offers fail. This uses existing session/CSRF checks
and does not yet provide session-bound step-up authentication or a notification UI.

Publisher departure: a non-owner can leave their own publisher membership with an
audited, CSRF-protected request; an owner must transfer ownership first. Leaving
invalidates a pending offer to that member.

Deletion blocker visibility: authenticated browser and native endpoints list the
caller’s Cloud roles and paginated publisher memberships. This helps users identify
required role removal, departure or ownership transfer; it does not promise account
deletion or address auth reauthentication and retention.

Combined local regression: the locked workspace unit tests, all-feature workspace
Clippy, all-feature Cloud client tests, desktop-login JavaScript tests, and all 17
PostgreSQL integration tests passed together on a fresh PostgreSQL 18 database.
The optional real-ClamAV test was not enabled in that combined run; its separate
custom-signature smoke test is described above. CI itself was not run here.

Sync write throttling: browser and native writes share a 120-per-minute account
limit backed by Better Auth secondary storage. A PostgreSQL integration test checks
shared limits, account isolation, unchanged cursor on rejection, and preservation of
native pending edits. Separate per-account limits now cover sync reads and export;
deployment-wide abuse controls remain open.

Transient job recovery: operator commands can requeue failed verification and
publication-preparation jobs only when their result is `temporary_failure` and their
release remains in the expected state. Concurrent attempts produce one audit event;
workers recheck authorization and artifact integrity. Tests use PostgreSQL and
simulated storage/scanner services. Blocked, rejected and already prepared jobs are
not recovered by these commands, and the operator label is not authenticated.

Blocked publication recovery: a currently authorized independent moderator can
re-request a job blocked by an authorization change. The API rechecks current
release, upload, key and review state and assigns the new requester transactionally;
the worker rechecks it again. PostgreSQL tests cover revoked roles, revoked keys,
concurrent repeat requests, auditing and renewed worker fencing.

Migration and restore drill: a PostgreSQL 18 integration test applied Cloud
migrations through `20261102000014`, inserted existing memberships, and verified
that the next migrations reject orphan memberships and duplicate owners before
succeeding after correction. A separate disposable `pg_dump`/`pg_restore` drill
restored a test database with the same user, publisher, membership, audit and
migration counts and retained the owner-deletion foreign-key guard. This is a local
database-only drill; production-sized recovery, object versions, secrets, backup
retention and point-in-time recovery remain untested.
