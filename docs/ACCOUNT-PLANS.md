# Account plans and quotas

**Status:** implemented on `master`; unit-tested.

Every account is on the `free` plan until an operator assigns another one. The
Free ceilings are exactly the constants that were hard-coded before this table
existed, so deploying the migration changes no existing account's limits.

## Schema

`migrations/20261119000001_account_plans.sql`:

```sql
CREATE TABLE cloud_account_plan (
    user_id TEXT PRIMARY KEY REFERENCES "user"(id) ON DELETE CASCADE,
    plan TEXT NOT NULL DEFAULT 'free' CHECK (plan IN ('free','pro','max')),
    updated_at BIGINT NOT NULL DEFAULT 0
);
```

`migrations/20261120000001_plan_team_to_max.sql` migrates existing rows to the
`max` plan and updates the `CHECK` constraint; limits are unchanged.

A missing row, an unknown value or a database error all fall back to `free`
(fail closed; a transient error must never grant a higher quota).

## Tiers

| Limit | free (default) | pro | max |
|---|---:|---:|---:|
| workspace records | 1,000 | 10,000 | 50,000 |
| workspace bytes | 8 MiB | 64 MiB | 256 MiB |
| change feed entries | 10,000 | 100,000 | 500,000 |
| change feed bytes | 16 MiB | 128 MiB | 512 MiB |
| assets per owner | 200 | 1,000 | 5,000 |
| asset bytes per owner | 32 MiB | 256 MiB | 1 GiB |
| overlays per owner | 50 | 250 | 1,000 |
| revisions per overlay | 100 | 500 | 2,000 |
| overlay bytes per owner | 16 MiB | 64 MiB | 256 MiB |

The tier table lives only in the backend (`crates/cloud-api/src/plans.rs`).
Clients receive their effective limits read-only; they cannot raise them.

## Endpoints

| Route | Auth | Purpose |
|---|---|---|
| `GET /api/v1/native/plan` | native session | `{ plan, limits }` for the caller |
| `POST /api/v1/admin/account-plan` | browser session, Cloud admin role | `{ userId, plan }` to assign a tier |

The admin endpoint uses the same `cloud_account_roles` gate as every other
administrative route; a native client cannot change its own plan.

## Enforcement points

- `workspace.rs`: record count / bytes on write, change-log retention, snapshot
  export bound.
- `assets.rs`: asset count / bytes on upload.
- `overlays.rs`: overlay count / revisions / bytes on create and revise.

Per-document schema limits (overlay element count, document size, page size,
rate limits) stay constant across tiers: they protect the protocol, not the
commercial tier.

## Relationship to the sync credential

The `plan` is not a sync scope. A `remote.control` or `sync.*` credential
carries no plan information; quota is always read from the backend by user id,
so a scoped credential cannot escape its tier.
