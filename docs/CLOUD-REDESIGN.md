# Cloud integration redesign

RemakeBackend owns cloud contracts, persistence and operations: identity,
plugin discovery and publication, templates, profiles and synchronization.
Desktop consumes one cloud instance. End users do not select registries.
Operators who clone and deploy the project configure their own instance and
trust material at build and deployment time. User requests cannot change it.

## Repository boundaries

This delivery changes RemakeBackend exclusively. Desktop is inspected to
understand wire formats and compatibility, without editing its files.
Integrating Desktop and removing its multiple-registry controls require the
user's subsequent authorization. Native plugin execution, OS access, local
credentials and application of sync conflicts still require a Desktop adapter.
LIVE collaboration and local control currently being edited in Desktop are
outside this delivery.

## Initial state verified in code

- Accounts, native handoff, publishers, RBAC, uploads, antivirus, moderation,
  immutable registry generations, rollback and signed file serving exist.
- `/api/v1/registry/current` and generation files are implemented. Older
  documentation describing them as absent is stale.
- Sync v1 supports only `plugin-selections`. Its client rejects other schemas;
  adding collections to that feed would break released clients.
- There is no cloud template/profile library or product-oriented plugin
  catalog API. Desktop interchange formats are `.tiktemplate.json` and
  `.tikprofile.json` v1.

## Implementation sequence

1. **Instance contract:** advertise capabilities and relative paths for one
   instance, without user-supplied registries or downloaded trust anchors.
2. **Portable contracts:** validate template/profile v1 documents, portable
   preferences and plugin settings on the backend; bound size and depth;
   reject known credential fields and support local secret references.
3. **Library:** private versioned documents, listing and export, submission
   for independent review, approval and public withdrawal. Published revisions
   are immutable; editing a draft does not alter published content.
4. **Plugin catalog:** search and inspect releases from a pinned published
   generation, including targets, digests and current yanks/revocations.
   Downloads retain the existing signed, independently verifiable protocol.
5. **Expanded sync:** isolate v2 data from v1; require revision ETags, retain
   tombstones, paginate changes, support cursor rebuilds, enforce quotas and
   account isolation. Cover selections, templates, profiles, portable settings
   and preferences. Exclude credentials and ephemeral LIVE state.
6. **Operations and delivery:** explicit migrations, readiness, account export
   and deletion, backend SDK, disposable PostgreSQL API tests, deployment
   instructions and a concrete subsequent Desktop integration contract.

## Acceptance criteria

- Users consume this instance's catalog/library without supplying registry URLs.
  Domain, storage and signing keys are operator configuration.
- Accounts cannot read or mutate another account's private records.
- Stale writes conflict; reconnecting never authorizes silent overwrites.
  Expired change cursors require an explicit snapshot rebuild.
- Only an independently approved revision is public. Authors cannot approve
  their own content; withdrawal blocks public reads of that revision.
- v1 retains its original contract. New APIs use v2 and do not depend on
  uncommitted Desktop work.
- Migrations and tests cover the new stores. No production database is changed,
  no deployment is performed and no official signing keys are generated.

## Subsequent Desktop integration

After backend validation, the handoff covers the v2 client, cloud catalog and
library, import/sync adapters, build-time instance configuration and removal
of end-user registry selectors. That work waits for the user's authorization
so it does not conflict with current Desktop changes.

The proposed Desktop edit surface for that later review is:

- `crates/tiktools-cloud-auth/src/origin.rs`: pin the release instance at build
  time and keep development overrides outside final-user workflows.
- `crates/tiktools-core/src/plugin_packages.rs` and
  `control/plugin_packages.rs`: bind discovery/install/update to that instance
  while preserving package verification and local installation safety.
- `crates/tiktools-control-api/src/modules/registry.rs` and
  `src/web/features/manage-plugin-sources/`: remove source selection/addition
  from the final-user API and settings UI.
- The Desktop cloud client and sync coordinator: consume the backend-owned v2
  contract, migrate local selections explicitly, and preserve pending edits
  through offline operation and account changes.
- `src/web/entities/rule-template/` and the template dialog: consume the cloud
  library and import reviewed interchange documents through existing local
  validation and user consent.

Before editing any of those paths, inspect the user's then-current worktree
again. Owner-definition, approval, proposal and LIVE control changes are not
part of the authorization requested for cloud integration.
