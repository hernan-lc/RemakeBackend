# Publisher approval and Cloud roles (experimental)

A namespace request starts `pending`. An authenticated Cloud admin may call `POST /api/v1/admin/publishers/{id}/approve`; an admin who is a member of that publisher is refused. Approval changes the status and writes an audit event in the same transaction. It does not register a public key, sign a registry delegation, or permit package publication. A separate signing-key lifecycle is implemented, but enrollment and approval remain disabled until session-bound step-up is available; no registry delegation or publication is performed.

Cloud account roles are separate from Better Auth identity roles. The explicit database operator commands are `cloud-api grant-role USER_ID admin OPERATOR` and `cloud-api revoke-role USER_ID admin OPERATOR`; `moderator` is the reviewer role: with `admin` it lists the moderation queue and records decisions, yanks, advisories, and inspections, independent of publisher membership (see [MODERATION.md](MODERATION.md)). Only `admin` approves publishers or reads the admin APIs. Run migrations first. A successful change writes an audit event, a repeated no-op does not, and the command refuses to revoke the final admin. The operator label is supplied on the command line and is **not authenticated evidence of who ran it**: restrict database and shell access, and correlate it with independently authenticated operator logs. No HTTP role-management endpoint exists. This bootstrap mechanism is not a substitute for production access-control review.

Auth-user deletion is blocked while Cloud roles or publisher memberships remain; roles can be self-revoked with `DELETE /api/v1/me/roles/{role}` or revoked through the operator command before calling `DELETE /api/v1/me`. Audit events are retained under the opaque user id. Approval is serialized with role revocation and membership changes, but it is not a replacement for independent security review or a complete approval UI.

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
cannot be accepted.

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
