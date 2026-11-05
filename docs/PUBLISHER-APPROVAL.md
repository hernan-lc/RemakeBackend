# Publisher approval and Cloud roles (experimental)

A namespace request starts `pending`. An authenticated Cloud admin may call `POST /api/v1/admin/publishers/{id}/approve`; an admin who is a member of that publisher is refused. Approval changes the status and writes an audit event in the same transaction. It does not register a public key, sign a registry delegation, or permit package publication. A separate signing-key lifecycle is implemented; enrollment and approval require session-bound step-up grants and no registry delegation or publication is performed.

Cloud account roles are separate from Better Auth identity roles. The explicit database operator commands are `cloud-api grant-role USER_ID admin OPERATOR` and `cloud-api revoke-role USER_ID admin OPERATOR`; `moderator` is the reviewer role: with `admin` it lists the moderation queue and records decisions, yanks, advisories, and inspections, independent of publisher membership (see [MODERATION.md](MODERATION.md)). Only `admin` approves publishers or reads the admin APIs. Run migrations first. A successful change writes an audit event, a repeated no-op does not, and the command refuses to revoke the final admin. The operator label is supplied on the command line and is **not authenticated evidence of who ran it**: restrict database and shell access, and correlate it with independently authenticated operator logs. No HTTP role-management endpoint exists. This bootstrap mechanism is not a substitute for production access-control review.

## Independent review workflow

The reviewer must be a different trusted person with a Cloud `admin` role and
must not be a member of the publisher, regardless of their membership role.
Under the normal review path, an admin who owns the namespace cannot approve it or review its keys. Have the
reviewer sign up or sign in, obtain their user ID through their account, and ask
an authorized database operator to grant that ID the admin role using the command
above. The reviewer signs in through the browser and approves the pending
namespace. The owner then enrolls a signing key with fresh step-up authentication;
the reviewer independently checks its fingerprint and approves the key with a
fresh step-up grant. Approving a namespace does not approve any key. Release
moderation has separate independent-review requirements.

The admin publisher page checks the current account's memberships and explains
when another reviewer is needed. Its check is advisory; the API rechecks the role
and membership at the time of the action. A 403 can also mean that the admin
role or membership changed, so refresh the page and verify the account before
retrying. Do not use a second account controlled by the requester as a substitute
for an independent reviewer.

Auth-user deletion is blocked while Cloud roles or publisher memberships remain; roles can be self-revoked with `DELETE /api/v1/me/roles/{role}` or revoked through the operator command before calling `DELETE /api/v1/me`. Audit events are retained under the opaque user id. Approval is serialized with role revocation and membership changes, but it is not a replacement for independent security review or a complete approval UI.

## Temporary Railway log-code override

For a temporary single-operator deployment, an operator can explicitly set
`TEMPORARY_LOG_APPROVALS=true` in the service environment and redeploy. It is
disabled by default; omit the setting or set it to `false` to turn it off.
When enabled, a signed-in Cloud admin who belongs to a publisher can select
**Approve with log code**. Generate a code, find the matching challenge ID and
eight-digit code in the service logs, and enter it in the same browser session.
For a key, first compare the complete fingerprint shown in the dialog with the
publisher's key. A separate code is required for each namespace or key.

Codes expire after five minutes, are single-use, and are bound to the issuing
account, session, and target. Attempts and issuance are rate limited. The
approval and code consumption are atomic and use distinct audit actions
`publisher.temporary_operator_approved` and
`publisher_key.temporary_operator_approved`. This bypasses independent review
and key-approval step-up; log readers who also control the signed-in admin
session can approve. Restrict log access and retention, and disable the flag
once it is no longer needed. Ensure logging includes warnings. All replicas
must share the database and authentication secret. Only code digests are stored
in the database; generated plaintext codes are intentionally written to logs.

## Ownership transfer

The current owner calls `POST /api/v1/publishers/{id}/transfer` with
`{"recipientUserId":"..."}` naming an existing non-owner member. The offer expires
after 24 hours; a new offer replaces the previous one. The current owner or named
recipient can read it with `GET` on the same path; the owner may cancel with `DELETE`.
The named recipient calls `POST /api/v1/publishers/{id}/transfer/{offerId}/accept`.
The former owner becomes an admin, the recipient becomes owner, and the offer is
consumed in one transaction with audit events. Any non-owner, including the former owner, may leave using
`DELETE /api/v1/publishers/{id}/membership`. Leaving records an audit event and
invalidates an outstanding offer to that member. An owner must first transfer ownership. Replaced, expired, removed-member and replayed offers
cannot be accepted. An owner can instead delete an unused namespace using
`DELETE /api/v1/publishers/{id}` with a browser or native session. Deletion returns
204 on success, 403 for a non-owner member, and 409 if another member, key,
key challenge, upload, release, transfer, or step-up record exists. Historical
audit records retain the deleted publisher ID in `deleted_publisher_id`; the
deletion itself is also audited. The namespace becomes available for reuse.

Migration `20261102000016_publisher_transfers.sql` rejects existing multiple-owner
records and adds the single-owner index; investigate conflicts before applying it.
The flow currently relies on existing sessions and CSRF protections, without
session-bound step-up or recipient notifications. Do not treat it as independently
security-reviewed.

Namespace requests are limited to 20 validly shaped requests per account per
minute using Better Auth secondary storage. Creation also refuses new requests while
the account owns ten pending namespaces; the count check is serialized with other
creations by that account. Approval or ownership transfer can change that count.
Publishers are limited to 1,000 memberships when adding new members; existing
members can still have their role changed or be removed. These controls limit routine
per-account abuse, not automated account creation or all concurrent ownership
transfers. The secondary store must be available; edge and deployment-wide abuse
controls remain necessary.
