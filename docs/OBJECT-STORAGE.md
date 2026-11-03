# Quarantine object storage (experimental)

`cloud-storage` provides S3-compatible quarantine storage for the authenticated HTTP upload API. A one-shot verification worker and ClamAV protocol adapter are connected; moderation can record decisions, but publication is not yet connected.

The component requires an explicitly configured endpoint, bucket, region and credentials; it does not read a default credential chain. Endpoints must use HTTPS except for a loopback IP used in tests. It accepts only server-generated `quarantine/{publisher UUID}/{upload UUID}` keys. Before presigning or capturing an object it requires bucket versioning to be enabled. Upload URLs last at most five minutes and bind `Content-Length`, `If-None-Match: *`, and the SHA-256 checksum in signed headers; the caller must send all returned headers. The component refuses to return a URL if the SDK does not sign those headers. URLs are bearer capabilities: deliver them only to an authorized user over TLS with `Cache-Control: no-store`, and never log them.

After upload, `capture` checks the expected length and records a non-null S3 version ID. `download_version` streams that exact version to a caller-provided private temporary sink and checks both size and SHA-256. Discard the sink on any error. Never verify or publish the mutable object key. Bucket versioning does not prevent an administrator or lifecycle rule from deleting a version; deployment policy must protect versions through verification and publication.

The API enforces publisher membership, quota, idempotency, expiry and authorization changes, and creates a transactional verification queue record. A future worker and publication system must independently approve publisher keys, scan and validate packages in a resource-limited worker, recheck digest and key authorization at publication, and publish only a reviewed immutable artifact. A compatible store must enforce signed headers, conditional writes and checksums; verify those properties on the actual deployment before use. Storage credentials and rotation, IAM boundaries, encryption, cleanup and recovery procedures are still needed. The component currently accepts static credentials and does not support short-lived role credentials.

Local tests validate endpoint/key handling and inspect the SDK's presigned header list with a minimal HTTP fixture. A manual smoke example was run against Moto with disposable buckets, exercising versioning, conditional PUT, version capture, streamed download and digest mismatch. Moto is an emulator and does not establish that real S3 or MinIO enforces signatures, checksums, IAM, TLS, CORS or lifecycle policy. A later smoke test also passed against a disposable MinIO server; see below. Neither test establishes AWS S3, production IAM, TLS, CORS, or lifecycle behavior.

## Authenticated quarantine reservations

The API optionally enables upload reservations when all five environment variables
`STORAGE_ENDPOINT`, `STORAGE_REGION`, `STORAGE_BUCKET`, `STORAGE_ACCESS_KEY`, and
`STORAGE_SECRET_KEY` are set. Partial configuration fails startup. Use a dedicated,
versioned quarantine bucket and restricted credentials; the server does not configure
bucket policy or versioning. An unavailable or unversioned store fails closed.

`POST /api/v1/plugins/uploads` requires a browser session, a trusted Origin, and an
`Idempotency-Key` of 16–128 ASCII letters, digits, `-`, `_`, `.`, or `~`. Its JSON body
contains `publisherId`, `size`, and lowercase hexadecimal `sha256`. Only an owner,
admin, or publisher of an approved namespace may reserve an upload. The response
contains an upload ID, short-lived bearer URL, and required headers; clients must send
those headers exactly and must protect the URL from disclosure. A retry with the same
key and identical request returns the same reservation. A different request conflicts;
a reservation expires after five minutes and a publisher may have at most ten live
reservations. Signed URLs cannot be revoked individually and can remain usable briefly
after a membership change or session expiry; the API rechecks authorization and
expiry at completion.

`POST /api/v1/plugins/uploads/{id}/complete` captures a version ID from the
quarantine bucket, rechecks authorization, and atomically adds a queued verification
record. Repeated completion is idempotent. Completion only means **quarantined**:
completion itself does not verify the checksum or package signature, and no release is published. A separate integrity worker can process the queue after release metadata is submitted; see RELEASE-VERIFICATION.md. The integrity worker fetches the recorded version and checks the digest and signature; a ClamAV scan follows; moderation and the remaining publication checks are still required.

The API integration test simulates S3 versioning and object metadata; it does not
validate AWS request signatures or demonstrate a real upload. The separate storage
emulator smoke test covers a PUT and versioned download but is not a substitute for
provider-specific S3, IAM, TLS, or policy testing.

Migration `20261102000006_upload_sessions.sql` adds the reservation and queue tables;
apply it using the explicit `migrate` command before deploying this API. Back up the
database and preserve quarantined object versions before any rollback. Rolling back
requires first stopping upload traffic and workers, then restoring a matching database
backup or deliberately dropping the two new tables in dependency order and removing
the matching SQLx migration record. Dropping them loses outstanding reservations and
queue records; object cleanup must be coordinated separately. Test the procedure on a
restored staging copy before production use.

The publication preparer may also write private `prepared/{publisherUuid}/{leaseUuid}`
objects after re-verification. It records only a returned non-null version ID after
reading that exact version back and checking its size and digest. Failed or stale
attempts can leave unreferenced versions; no automatic cleanup exists. Prepared
versions are not public artifacts and are not protected against deletion by this
component alone. See [registry operations](REGISTRY-OPERATIONS.md).


## Disposable MinIO smoke test

The storage example can create uniquely named test buckets when explicitly requested.
Use this only with a disposable server: the buckets and objects are not deleted by
the example. The following local test used the legacy Bitnami MinIO image
`2025.4.22` (MinIO development build 2025-04-22), with no persistent volume:

```sh
docker run -d --rm --name cloud-minio-integration -p 127.0.0.1:39000:9000 \
  -e MINIO_ROOT_USER=test -e MINIO_ROOT_PASSWORD=testsecret \
  bitnamilegacy/minio:2025.4.22
# Wait until the server is ready, then:
CLOUD_TEST_S3_ENDPOINT=http://127.0.0.1:39000 \
CLOUD_TEST_S3_ACCESS_KEY=test CLOUD_TEST_S3_SECRET_KEY=testsecret \
CLOUD_TEST_S3_CREATE_BUCKETS=1 \
  cargo run --locked -p cloud-storage --example emulator_smoke
docker stop cloud-minio-integration
```

The test rejected unversioned storage, a changed payload, a missing signed header,
a repeated conditional upload, a wrong object size and an incorrect digest. It
captured and downloaded the versioned quarantine object, overwrote its mutable key
with a privileged SDK write, and verified that both the replacement version and
the originally captured version remained readable with their respective bytes. It
also staged and re-read a versioned prepared object. The overwrite check passed
against MinIO development build 2025-04-22 using disposable buckets, test root
credentials and HTTP. This is not proof of production provider, TLS, IAM, CORS,
retention, or lifecycle configuration.
