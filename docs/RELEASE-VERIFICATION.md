# Release submission and verification worker

An authenticated publisher owner, admin or publisher can submit a completed quarantine
upload with `POST /api/v1/plugins/releases`. The JSON fields are `uploadId`,
`publisherKeyId` (the Cloud key record UUID), `pluginId`, `version` and `target`.
The publisher must be approved, the key must be approved for that same publisher,
and the plugin ID must be inside its namespace. The API uses the pinned TikTools
manifest rules, requires a canonical semantic version, and reserves each plugin ID,
version and target only once. Repeating the same request for the same upload returns
the original release; changing its metadata conflicts. A release and its upload are
bound to the same publisher by database constraints. Members can inspect the
unpublished status at `GET /api/v1/plugins/releases/{id}`; it is not public.

Completion of an upload creates a queue row, but a worker will claim it only after a
release submission attaches the expected metadata. Run one job with
`cargo run --locked -p cloud-api -- verify-once` using the same database and storage
environment configuration as the API. The process downloads the recorded immutable
object version to a private temporary directory, checks its size and SHA-256, then
uses `cloud-verifier` and the independently approved publisher key to check package
identity, signature, checksums, archive limits and safe extraction. It rechecks the
publisher and key under database locks before recording the outcome. The worker also
streams the exact verified private copy to ClamAV and recomputes its size and SHA-256
while streaming. Set `CLAMAV_ADDR` to a numeric loopback IP and port, such as
`127.0.0.1:3310`; `verify-once` refuses to run without it. The scanner connection is
local and unauthenticated, so isolate the daemon and its loopback network namespace.
The verified copy from this worker is temporary. A separate publication preparer
repeats verification and scanning and stores a private versioned staging copy; neither
worker publishes an artifact.

A job has a ten-minute lease with a unique fencing token. Workers claim with
`SKIP LOCKED`; an expired lease can be retried, and a stale worker cannot commit a
result after losing its lease. Transient failures are retried at most three times and
then recorded as `failed`; an operator can explicitly requeue an eligible transient failure after addressing its cause. Invalid
packages, revoked keys and antivirus alerts are rejected. A scanner error, malformed
reply, timeout, or mismatch in streamed bytes is a transient failure. New successful
jobs are `antivirus_passed`, meaning the configured scanner returned a clean result
for the verified bytes; it is not proof that an artifact is harmless, moderation
approval, an immutable publication artifact, or authorization to publish. An independent
reviewer can record a decision through the [moderation API](MODERATION.md). Historical
`integrity_passed` records predate scanning and must not be treated as scanned. No
general rescan command is implemented. Key and publisher
status must be checked again when later decisions or publication occur.

Run the worker as a separate unprivileged process with externally enforced CPU,
memory, disk, process-count, network and wall-time limits, and restrict its database
and object-store credentials. The code does not currently create or enforce that
sandbox, ensure antivirus configuration or signature freshness, or produce signed
registry snapshots. Moderation records decisions and provides version-bound inspection links, but does
not supply an isolated inspection environment. Do not treat this pipeline as production-ready. The package integration test uses simulated S3 and ClamAV protocol servers. A separate
opt-in adapter smoke test exercises a real ClamAV daemon with an artificial signature;
neither test proves production ClamAV configuration, real S3, IAM, or sandbox behavior. Key enrollment itself remains disabled until the
required session-bound step-up authentication exists.

Migrations `20261102000008_plugin_releases.sql` and
`20261102000009_verification_leases.sql` add release records, queue links and leases.
`20261102000010_antivirus.sql` adds separate antivirus result states without upgrading
historical integrity-only results.
Apply them explicitly before running the new API or worker. Back up the database and
stop all workers before rollback; restoring a matching database backup also requires
reconciling quarantine versions and releases. Dropping these tables or columns loses
release history and queued work. Rollback has not been rehearsed.

## ClamAV deployment requirements

Configure and validate the daemon independently: keep signatures current, enable
archive scanning and alerts for scan limits, and size `StreamMaxLength`, `MaxFileSize`,
`MaxScanSize`, recursion and file-count limits for the accepted package policy. A
misconfigured scanner can return clean without examining every relevant payload; the
worker cannot inspect or prove daemon configuration or database freshness. Run
integration tests against the actual daemon and safely isolated test signatures before
use. ClamAV's [protocol documentation](https://docs.clamav.net/manual/Usage/ClamdProtocol.html)
describes the streaming transport and its limits; the upstream
[sample configuration](https://github.com/Cisco-Talos/clamav/blob/main/etc/clamd.conf.sample)
describes scan-limit and alert options.

## Local ClamAV adapter integration test

The adapter has an opt-in test against a real daemon. It checks a clean stream and
an artificial byte pattern detected by a private test signature, both directly and
inside a deflated ZIP archive. It does not test production signature freshness,
other archive formats, or deployment configuration. When
`CLOUD_CLAMAV_TEST_ADDR` is unset the test returns without contacting a daemon.

For example, on a Linux Docker host, create a private test database directory and
run an isolated daemon with only this test signature:

```sh
mkdir -p /tmp/cloud-clamav-test-db
printf '%s\n' 'Cloud.Test.Signature:0:*:636c6f75642d61762d696e746567726174696f6e2d746573742d7369676e61747572652d32303236' > /tmp/cloud-clamav-test-db/cloud-test.ndb
cat > /tmp/cloud-clamav-test-db/clamd.conf <<'CONF'
Foreground yes
TCPSocket 33317
TCPAddr 127.0.0.1
DatabaseDirectory /test-db
StreamMaxLength 134217728
MaxFileSize 134217728
MaxScanSize 268435456
ScanArchive yes
AlertExceedsMax yes
CONF
docker run -d --rm --name cloud-clamav-integration --network host \
  -v /tmp/cloud-clamav-test-db:/test-db:ro --entrypoint /usr/sbin/clamd \
  clamav/clamav:1.4 -c /test-db/clamd.conf
# Wait for clamd to load the signature database, then:
CLOUD_CLAMAV_TEST_ADDR=127.0.0.1:33317 cargo test -p cloud-api scanner::tests::real_clamav_custom_signature
docker stop cloud-clamav-integration
```

This daemon has **only the test signature** and must never be used for real package
verification. The host-network example is Linux-specific and reserves port 33317.

## Recovering a transient verification failure

After investigating the cause, an operator may run
`cargo run --locked -p cloud-api -- retry-verification UPLOAD_UUID OPERATOR`.
Only a `failed` job with `temporary_failure`, no lease, an awaiting-verification
release and a quarantined upload is reset to `queued` with three fresh attempts.
A competing or repeated retry fails without a second audit event. Running, passed,
rejected, malware-detected and otherwise ineligible jobs are never reset by this
command. The worker rechecks authorization and re-downloads and re-scans the
recorded version; retry does not grant trust or approval. The operator label is
self-supplied, so restrict command and database access and correlate external logs.
