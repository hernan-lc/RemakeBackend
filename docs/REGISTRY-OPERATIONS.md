# Registry bootstrap and operations

The planned official registry uses the v1 static format and tooling from TikTools PR #43, pinned here for integration checks at `983a375c5b702681a791061ca863cea9c542528d`. The implementation and protocol are upstream in `crates/tiktools-plugin-registry`, `crates/tiktools-plugin-devkit`, and `docs/PLUGIN_REGISTRY_PROTOCOL.md`. Do not introduce a second signing format or let a Cloud login bypass client verification. An official domain, root public key, deployment environment, and authorized operators have not been selected.

## Test fixture

On a trusted development machine, check out that exact TikTools revision and run `scripts/build-registry-fixture.sh /path/to/TikTools-app /path/to/new-output`. The script builds the upstream `tiktools-plugin` CLI, creates disposable test keys in a temporary directory, uses the CLI to initialize, delegate, sign and publish a sample package, and copies only the resulting public registry tree to the output. A separate public root pin is saved as `<output>.root-pin.json`. It refuses an existing output directory and tracked changes to the checkout. These test keys and the generated root are never official trust anchors. Run `scripts/verify-registry-fixture.sh /path/to/TikTools-app /path/to/output /path/to/output.root-pin.json` to use the pinned TikTools registry client against this exact tree. It checks the independent root pin, signed index, downloaded archive digest, publisher signature and extraction, and rejects a wrong root, wrong digest and tampered index. CI runs that verification, upstream client and CLI tests, and uploads the disposable tree and public pin. This does not exercise a complete Desktop install.

## Production key lifecycle (not yet provisioned)

- Generate root keys offline with an audited ceremony, record public fingerprints through an independent channel, protect and back up the private keys offline, and pin the approved public root in Desktop releases. `root.json` is unsigned and cannot establish trust by itself. Never put a root private key in CI, Cloud, a CDN, logs, or a repository.
- Use root-signed `keys.json` to delegate index and namespace-scoped publisher public keys. Review namespace ownership, key identity, scopes and validity before delegation. A Cloud account or publisher membership alone confers no signing authority. Delegation changes require separate approval and the offline root ceremony.
- Keep the index key in a restricted signing environment backed by KMS/HSM where supported; the upstream CLI currently accepts a local key file, so a secure production signing integration remains to be built and audited. Limit who can invoke it, require approval, log signer identity and artifact digest, and rotate it via a root-signed delegation before expiration. Revoke compromised delegated keys and affected digests promptly; publish a new signed snapshot and notify clients.
- Root rotation requires a Desktop update with an independently authenticated new pin, an overlap and recovery plan, and coordinated retirement of the old root. Do not replace `root.json` and expect existing pinned clients to trust it. Test recovery and compromise procedures before launch.

## Snapshot publication and rollback (design; not implemented)

Build the entire snapshot in an isolated staging location with the upstream CLI. Check all signatures, delegated scopes, package digests and install behavior using a real client pinned to the intended root. Publish immutable versioned objects first; verify that every object and detached signature is reachable. Promote the complete tree atomically through infrastructure that guarantees the client sees a consistent snapshot, accounting for CDN cache behavior. The protocol has no built-in snapshot pointer or cross-document atomicity guarantee; the deployment mechanism and its proof still need implementation. Do not upload files one by one over a live tree and call that atomic.

Retain previous immutable snapshots and record their hashes and approvals. Roll back only to a fully verified snapshot compatible with current pins and security state; never restore a compromised key, revoked digest, yanked release, or stale advisory merely to recover availability. Rebuild and sign a corrected snapshot if necessary. Test promotion, cache behavior, rollback, signing credential rotation and restore in staging before production use. The fixture workflow is not a production publication workflow.

## Publication requests

An independent Cloud moderator or admin can request publication of a reviewed release
with `POST /api/v1/plugins/releases/{releaseUuid}/publish`. The request checks current
publisher and signing-key approval, an antivirus-passed verification job, an approved
review, and the requester's role and conflicts of interest. It records one audited,
idempotent queue item and returns `202` with `status: "queued"` and
`processingAvailable: false`. Concurrent retries return the same job ID and current
preparation status. This flag means end-to-end snapshot publication is unavailable;
a preparation worker is available as described below. The release remains
`review_approved` and private. There is no public release status yet.

Run one preparation job in an isolated, resource-limited process with
`cargo run --locked -p cloud-api -- prepare-publication-once`, using the same database,
versioned quarantine storage and `CLAMAV_ADDR` settings as the verification worker.
It rechecks the requester's role and conflicts, publisher, key, review and verification
state; fetches the captured object version; repeats package verification and scanning;
then writes a fresh private `prepared/{publisherUuid}/{leaseUuid}` object with a
conditional PUT. It reads back the exact returned version and checks its size and
SHA-256 before recording `prepared`. It rechecks authorization and the input metadata
under locks after network and scanning work. Ten-minute leases fence stale workers;
transient failures retry at most three times, while revoked authorization blocks the
job. A failed, blocked or superseded attempt can leave an unreferenced object version;
cleanup and operator retry are not implemented. The bucket must remain private and
its policies must preserve referenced versions through publication.

`prepared` is **not published** and does not imply that a snapshot exists. A future
promotion process must recheck current authorization and scanner policy, fetch and verify
the recorded prepared version, independently verify a signed snapshot, and atomically
promote it without mixing snapshot generations. The initial-candidate worker does not
implement that promotion. A queued or prepared
request is not authorization to skip these checks; revocation or suspension after
preparation must prevent publication. The integration test uses simulated S3 and ClamAV
servers; real provider signatures, checksums, IAM and failure behavior remain unverified.

Migration `20261102000012_publication_requests.sql` adds this queue;
`20261102000013_publication_preparation.sql` adds preparation leases and versioned
artifact references. Stop request
traffic and workers before rollback and preserve queued jobs and their audit history.
Restore a matching backup and reconcile any snapshot deployment separately; dropping
the table loses requests. The rollback and snapshot deployment remain untested.

## Private candidate builder

The `cloud-snapshot` library builds a temporary candidate tree with the pinned
TikTools v1 `IndexWriter`. Its caller supplies a **complete, independently authorized**
release inventory, current approved publisher keys, an independently obtained root
pin, a root-signed `keys.json`, an index-signed `config.json`, and a delegated index
signing key. The builder verifies the seed signatures and key validity, checks each
publisher delegation against the independently supplied key, re-verifies each package,
rejects duplicate identities, and preserves the caller-supplied yank and advisory
states. The pinned devkit keeps its publish-readiness helper private, so the builder
mirrors its non-changelog metadata checks; this compatibility code must be reviewed
when changing the upstream pin. It checks the generated signatures and artifact digests and returns a
short-lived private directory and per-file SHA-256 map. Candidate construction has
limits of 10,000 artifacts, 6,400 plugin IDs and 16 GiB of compressed artifacts.

The standalone library does **not** establish that the caller's inventory is complete
or still authorized, check scanner freshness, perform an independent review, or
promote/deploy a candidate. The experimental initial-candidate worker described below
connects it to prepared jobs and privately stores the resulting files. It reconstructs
release documents with the upstream writer, so callers must assess changes to
publication timestamps, metadata and distribution tags before use. Treat the file map
as evidence of the candidate at build time and reverify all contents and authorization
before any later use. Index signing currently requires an in-process local key; a
production KMS/HSM integration and key ceremony are still missing. Run builds only in
an isolated, resource-limited environment.

Tests build a two-plugin candidate and use the pinned TikTools client to verify its
root, signed index, package digest and publisher signature. They also exercise yank
and advisory fields, untrusted and expired keys, forged packages, duplicate versions,
tampered seed documents and tampered output. This is a local protocol test, not proof
of a complete inventory, real scanner, remote CDN behavior or atomic publication.

## Initial candidate worker

`cargo run --locked -p cloud-api -- build-initial-candidate` builds an **initial,
private** candidate from every publication job currently marked `prepared`. It refuses
an invalid prepared job rather than silently omitting it, checks current publisher,
key, review, verification and requester authorization, downloads each recorded
prepared version, and repeats the antivirus scan. It passes the complete captured set
to `cloud-snapshot`, compares the inventory again after building and before recording,
revalidates the candidate signatures and hashes, and stores each file as a separate
private versioned object with read-back digest checks. A database record contains the
file paths, hashes, sizes and version IDs and references each included prepared job.
The worker logs the non-secret candidate UUID for operators; the same UUID is recorded
in the audit event. The event uses a service-process label, not an authenticated human
operator identity. No public registry pointer or release status is changed.

Set `SNAPSHOT_SEED_DIR` to an empty initial registry containing the signed seed
documents, `SNAPSHOT_ROOT_PIN_FILE` to an independently authenticated root `TrustedKey`
JSON, and `SNAPSHOT_INDEX_KEY_FILE` to the delegated TikTools index private-key JSON.
The index key file must be a non-symlink regular file with no group or other access on
Unix; this file-based signer is disabled on platforms where those permissions cannot
be checked. The worker also requires the database, versioned private object storage,
and loopback `CLAMAV_ADDR` settings. Protect the signing host and key; production
KMS/HSM integration is not implemented.

This command accepts only a seed with no index, package or catalog tree and zero
catalog pages. Cloud does not yet model an existing published inventory, yanks or
advisories, so it must not be used to update an existing public registry. It assigns
channels from versions and supplies no yanks or advisories. The database comparisons
are point-in-time checks; they do not freeze future changes or make a candidate safe
to deploy. A future promotion process must recheck the entire inventory, current
authorization, trust, scanner policy and all stored object versions after construction
and enforce consistent client access across snapshot generations. Candidate creation
is not idempotent, and failed or raced builds may leave unreferenced private objects;
operator retry and garbage collection are not implemented. The integration test uses
simulated S3 and ClamAV servers, not a real provider or production scanner.

Migration `20261102000014_snapshot_candidates.sql` adds private candidate, release and
file records. Stop candidate builders before rollback and preserve candidate metadata
and referenced object versions. Restoring a database backup requires reconciliation
with private objects; dropping these tables loses candidate references. Rollback has
not been rehearsed.

## Inspecting a private candidate

An operator can reconstruct a recorded candidate with
`cargo run --locked -p cloud-api -- export-candidate CANDIDATE_UUID NEW_OUTPUT_DIR`.
The output directory must not exist. On Unix it is created with private permissions;
other platforms fail closed until equivalent ACL checks are implemented. The command
reads each exact private object version, verifies its stored size and digest, checks
the database manifest again, and writes `.cloud-candidate.json` last. A failed export
may leave an incomplete directory without that marker; it never overwrites an
existing destination. Treat the marker as usable only when the command also exits
successfully. The marker describes the private candidate and does not assert
that it is approved or safe to deploy.

Independently verify the exported signed documents, root pin, package signatures and
current policy before review. The root fingerprint in the marker comes from the
database and is not an independent trust anchor. Export does not recheck current
publisher, reviewer, scanner or inventory authorization and does not promote the
candidate. The test reconstructs a candidate through simulated storage and verifies
its registry documents with the pinned TikTools client; actual provider behavior and
an isolated reviewer environment remain unverified.
