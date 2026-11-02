# Publisher signing keys

**Enrollment and approval are disabled in the normal server.** The specification requires session-bound step-up authentication, which the pinned Better Auth integration does not currently provide for Google users. Challenge, key request, and approval endpoints return 503 until that capability is implemented; there is no configuration bypass. The lifecycle below is exercised only by a test harness. Revocation and authorized listing remain available.

Publisher namespaces and signing keys have separate approvals. A namespace owner or
admin can request a key only while the namespace is approved. Only an independent
Cloud administrator who is not a member of that publisher can approve it. A key is
not trusted while pending or after revocation. A publisher owner or admin, or an
independent Cloud administrator, can revoke a pending or approved key; revocation is
irreversible. All transitions are audited with the key record ID. Revocation remains
available to publisher managers when the namespace is suspended.

The authenticated browser API uses trusted Origin checks. All key endpoints return
`Cache-Control: no-store`. Members can list their publisher's keys with
`GET /api/v1/publishers/{publisherUuid}/keys`. An independent administrator can use
`GET /api/v1/admin/publishers/{publisherUuid}/keys`. Key IDs and Ed25519 public keys
use the formats implemented by the pinned TikTools package library. Key IDs and
SHA-256 public-key fingerprints are globally unique and remain reserved after
revocation; use new key material and a new key ID when rotating.

Once step-up integration is implemented, the intended enrollment sequence is:

1. Send `POST /api/v1/publishers/{publisherUuid}/keys/challenges`. The response has
   `challengeId`, `nonce` and `expiresIn`. Challenges expire after five minutes, are
   bound to the publisher and requesting account, and are consumed once.
2. On a trusted local machine with the existing TikTools private key file, run
   `cargo run --locked -p cloud-api --example sign_publisher_key -- KEY_FILE PUBLISHER_UUID CHALLENGE_UUID NONCE`.
   The command prints a JSON request containing the public key and signature; it does
   not transmit or print the private key. Never send the key file to Cloud.
3. Send that JSON to `POST /api/v1/publishers/{publisherUuid}/keys`. The service
   verifies the Ed25519 proof, creates a pending request and returns its record UUID.
4. An independent Cloud admin sends
   `POST /api/v1/admin/publishers/{publisherUuid}/keys/{recordUuid}/approve`.
   Revoke with the corresponding publisher or admin path ending in `/revoke`.

The signed bytes are UTF-8 `tiktools-cloud-publisher-key-v1`, publisher UUID,
challenge UUID, nonce, key ID and canonical standard-base64 public key, joined with
newline characters and no trailing newline. The server enforces the same format.
A challenge is not an authorization decision; it demonstrates possession of the
private key. Operators must independently verify the key fingerprint and ownership
before approval. Cloud does not provide that verification workflow yet.

Migration `20261102000007_publisher_keys.sql` adds key/challenge tables and audit
subject IDs. Run the explicit migration before deploying. Back up the database first;
rollback requires stopping all key and publication operations and restoring a matching
backup. Dropping the new tables loses revocation history and can make previously
revoked IDs reusable, so it is unsafe without preserving and reconciling that history.
The rollback has not been rehearsed. The API records key approval but does not yet
wire approved keys into a verification worker or signed registry publication. Existing
registry snapshots are not automatically changed by revocation.
