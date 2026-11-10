# TikTools Cloud docs index

Start with the root [README.md](../README.md) for setup, then the plan and
status docs below. Every doc states its own scope and limits up front;
“experimental” means tested but limited, “absent” means named scope with no
implementation. The status vocabulary is defined in
[FEATURE-MATRIX.md](FEATURE-MATRIX.md); verified evidence lives in
[IMPLEMENTATION-STATUS.md](IMPLEMENTATION-STATUS.md).

Links between docs are relative (`OTHER.md`), so they work from any
checkout. New docs go in this directory, use `UPPER-KEBAB.md` names, link
from here and from the doc that scopes them, and state limits explicitly
rather than implying production readiness.

## Product, plan, and status

- [CLOUD-REDESIGN.md](CLOUD-REDESIGN.md) — Current single-instance cloud plan and repository boundaries.
- [CLOUD-API-V2.md](CLOUD-API-V2.md) — Product catalog, reviewed templates/profiles, workspace sync, SDK and operations.
- [CLOUD-API-V3.md](CLOUD-API-V3.md) — Visual catalog, remote control relay, device proof, grants and integration status.
- [CLOUD-API-V3-DEPLOYMENT.md](CLOUD-API-V3-DEPLOYMENT.md) — Environment, migrations, relay replicas, verification and recovery for v3.

- [SPECS.md](SPECS.md) — Architecture proposal: accounts, Google login,
  verified publishers, registry publication, opt-in sync.
- [PLAN.md](PLAN.md) — Work breakdown for coding agents; status tracked in
  IMPLEMENTATION-STATUS.md.
- [ROADMAP.md](ROADMAP.md) — Milestones with checked/unchecked exit criteria.
- [FEATURE-MATRIX.md](FEATURE-MATRIX.md) — Per-surface status (stable /
  experimental / absent) with limits.
- [IMPLEMENTATION-STATUS.md](IMPLEMENTATION-STATUS.md) — What was verified,
  how, and what remains outstanding.

## Identity, sessions, and account

- [DESKTOP-AUTH.md](DESKTOP-AUTH.md) — Browser-to-native PKCE handoff,
  loopback rules, rate limits, keychain policy.
- [PUBLISHER-APPROVAL.md](PUBLISHER-APPROVAL.md) — Namespace approval, Cloud
  roles, operator grant/revoke, ownership transfer, deletion blockers.
- [SYNC.md](SYNC.md) — Plugin-selection sync API, preconditions, tombstones,
  cursor rebuilds, and limits.

## Publishers and signing keys

- [PUBLISHER-KEYS.md](PUBLISHER-KEYS.md) — Key lifecycle; enrollment and
  approval stay disabled pending session-bound step-up.
- [PACKAGE-VERIFICATION.md](PACKAGE-VERIFICATION.md) — What `cloud-verifier`
  checks and explicitly does not establish.
- [RELEASE-VERIFICATION.md](RELEASE-VERIFICATION.md) — Release submission
  binding and the integrity worker.

## Review, moderation, and publication

- [MODERATION.md](MODERATION.md) — Review queue, independent decisions,
  inspection URLs, yanks, advisories, audit trail.
- [REGISTRY-OPERATIONS.md](REGISTRY-OPERATIONS.md) — Registry bootstrap,
  fixture scripts, snapshot candidates, promotion checks, publication
  worker. Includes the private candidate builder section.
- [OBJECT-STORAGE.md](OBJECT-STORAGE.md) — Quarantine S3 storage, version
  capture, digest checks, upload API.

## Admin web app

- [ADMIN-WEB-PLAN.md](ADMIN-WEB-PLAN.md) — Web login and dashboard plan
  (PRs A–D), stack, and boundaries. Implemented.
- [ADMIN-WEB-DEPLOYMENT.md](ADMIN-WEB-DEPLOYMENT.md) — DTO contract tests,
  CSRF/cookie/proxy/rate-limit coverage, reverse proxy, CSP, secrets
  policy, deploy/rollback, smoke tests. Example config:
  `../deploy/admin-reverse-proxy.example.conf`.

## Operations and recovery

- [DATABASE-RECOVERY.md](DATABASE-RECOVERY.md) — Upgrade guards, backup
  before migrate, restore evidence.
- [LOCAL-PGRUST.md](LOCAL-PGRUST.md) — Docker-free local database via the
  pinned pgrust submodule.
