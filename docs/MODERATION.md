# Release moderation

`GET /api/v1/moderation/releases` lists up to 50 releases eligible for review. It
requires a Cloud `moderator` or `admin` account role and returns an optional
`nextCursor`; pass that UUID as `?after=...` to get the next page. Candidates include
release identity, expected archive digest and size, and signing-key ID and
fingerprint. They appear only when the release and verification job both say
`antivirus_passed`, the publisher and key are approved, and the reviewer has no
conflict of interest. Historical integrity-only results are ineligible.

A reviewer may not be a current publisher member or the person who requested the
release, upload, or signing key. The same checks apply when recording a decision.
The API rechecks the account role and locks the publisher and key, so role revocation,
membership changes, publisher suspension and key revocation serialize with review.
Approval requires the publisher and key still be approved. A reviewer may reject an
otherwise eligible release whose publisher or key is no longer approved.

`POST /api/v1/moderation/releases/{releaseUuid}/decision` requires a browser session,
a trusted Origin, and one of these JSON bodies:

- `{"decision":"approve","reasonCode":"policy_reviewed"}`
- `{"decision":"reject","reasonCode":"policy"}`
- `{"decision":"reject","reasonCode":"malware"}`
- `{"decision":"reject","reasonCode":"other"}`

The decision, actor, reason code and audit event are recorded transactionally. One
decision is allowed per release; an identical retry by the same reviewer is
idempotent and a competing or changed decision conflicts. Publisher members can read
the decision and reason code at `GET /api/v1/plugins/releases/{releaseUuid}/review`;
reviewer identity is omitted. The reviewer must perform an independent policy review before asserting
`policy_reviewed`. To obtain the candidate archive, an eligible reviewer can send
`POST /api/v1/moderation/releases/{releaseUuid}/inspection`. It returns a 60-second
bearer URL bound to the captured object version. The service rechecks reviewer,
publisher, key and verification authorization after contacting storage, and audits
each returned capability. The signed object response requests attachment disposition,
`application/octet-stream` and `Cache-Control: no-store`. A URL already returned cannot
be individually revoked before expiry, so protect it from logs and disclosure.

Treat the download as untrusted even after an antivirus pass. Compare its SHA-256 and
size with the queue metadata, and inspect only in an appropriately isolated environment;
do not execute it in a reviewer's ordinary workstation session. This API does not
provide that inspection environment or an evidence-capture workflow. Actual S3 policy,
version handling and response overrides require deployment-specific validation.

`review_approved` means only that a reviewer recorded approval. It does not publish,
create a signed delegation, establish current key authorization indefinitely, or
prove the scanner configuration and signatures were current. Publication must
recheck the review, publisher, key and artifact against current policy, and must
produce and atomically deploy a signed snapshot. A moderator can enqueue a publication request, but processing, an appeal, and a
review-reversal flow are not implemented; see [registry operations](REGISTRY-OPERATIONS.md). The account-role bootstrap is also not yet
production-hardened.

Migration `20261102000011_moderation.sql` adds the decision table and release states.
Run the explicit migration before deploying the API. Back up the database and stop
review and publication work before rollback; restoring a matching backup requires
reconciliation with any external review evidence and artifacts. Dropping the table
loses review history and is unsafe without preserving it. Rollback has not been
rehearsed.
