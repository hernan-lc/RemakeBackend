# Database upgrade and restore evidence

## Maintained v2 upgrade and archive drill

The ignored test
`schema_v2_recovery_tests::populated_pre_v2_upgrade_and_real_archive_restore_preserve_all_domains`
now maintains a PostgreSQL-only recovery drill through the latest Cloud v2 and
library-digest-repair migrations. Its implementation is in
[`schema_v2_recovery_tests.rs`](../crates/cloud-api/src/schema_v2_recovery_tests.rs).
It passed locally on 2026-11-08 against disposable native PostgreSQL and pgrust
fixtures. The earlier smoke evidence remains below as historical context.

| Current local check | Result |
| --- | --- |
| Populated migration `20261102000024` through v2 and digest-repair, followed by actual SQL archive restore on native PostgreSQL | Passed, 17.31 s |
| Same maintained recovery drill on pgrust | Passed, 18.61 s |
| Pure fixture-URL guard tests | 2 passed; no database or child process required |
| Focused v2 database cases | 5 passed on each SQL runtime |
| Earlier PostgreSQL regression suite | 67 passed on native PostgreSQL |

The separate authenticated rejected-Origin admin browser checks also passed
locally against native PostgreSQL and pgrust (two browser cases), and admin
build/Playwright coverage was added to CI configuration. GitHub Actions itself
was not run. Separate current-source checks passed on Rust 1.95 for the
workspace and Rust 1.89 for the SDK, including all targets and features; the
archive drill itself does not establish compiler or platform compatibility.

The Rust test CI job is pinned to Ubuntu 24.04 and explicitly installs
`postgresql-client-18` from the official PGDG `noble-pgdg` repository, following
the [PostgreSQL Ubuntu instructions](https://www.postgresql.org/download/linux/ubuntu/).
It puts `/usr/lib/postgresql/18/bin` first on subsequent steps' `PATH` and
prints both archive-tool versions before running the ignored suite against
the PostgreSQL 18.6 pgrust service. This avoids using Ubuntu 24.04's older
default client. The provisioning is configured and statically checked; its
hosted GitHub Actions execution has not been verified locally.

Run it serially against a dedicated **loopback test database** with a name
starting `cloud_test` or `test_`. The authority must be a literal loopback IP
(for example `127.0.0.1` or `[::1]`), not a DNS name. Before any SQLx connection,
the fixture rejects connection-target query overrides, duplicate/unknown
parameters, incoming `options`, decoded database-name slashes and names over
63 bytes. The fixture owns its search path and UTC timezone. The role needs
`CREATEDB`; matching `pg_dump`/`pg_restore` clients must be on `PATH`:

```sh
CLOUD_TEST_DATABASE_URL=postgres://... \
  cargo test --locked -p cloud-api \
  schema_v2_recovery_tests::populated_pre_v2_upgrade_and_real_archive_restore_preserve_all_domains \
  -- --ignored --exact --test-threads=1
```

The fixture creates a random isolated source schema, applies Better Auth and
Cloud migrations through `20261102000024`, and populates accounts, publisher
ownership, roles, audit, signing-key metadata, an active worker lease, blocked
promotion/trust metadata and v1 live/tombstone records with their cursor/feed.
It checks that upgrading to v2 preserves every old table's rows and starts the
separate v2 stores empty. It then adds v2 source/history, a public revision and
newer private draft, workspace live/tombstone records and account cursor/feed.
The digest-repair migration must not rewrite those rows; an explicit reviewed
repair changes only the derived digest and adds its metadata audit receipt.

The test runs actual custom-format `pg_dump --no-owner --no-acl` and restores
only that fixture schema into a newly created random database with
`pg_restore --single-transaction --exit-on-error`. Database connection secrets
go through libpq environment settings using the same helper as local backup;
neither a password nor the credential-bearing URL appears in process arguments.
Exact restored table contents include migration checksums, original review and
publication metadata, tombstones, cursors, lease state and repair history.
Negative checks cover incompatible migration readiness, restored account-writer
locking, publisher Owner deletion protection and deferred public-revision FKs.

Cleanup removes only the schema and database whose random names this fixture
created, including a best-effort cleanup after assertion failure. It never
restores into or drops the configured test database. Keep this opt-in test away
from production regardless of its safety checks.

This is a database-only proof with synthetic data. Worker lease metadata is
preserved; live process/advisory locks are not backup contents. The drill does
not provision signing keys, verify real package trust, restore object-store
versions/credentials, rehearse production traffic or establish a production
backup retention policy. Those remain separate coordinated recovery gates.

## Earlier upgrade and restore evidence

Run migrations explicitly after backing up the database and stopping incompatible
writers. Migration 15 rejects orphan publisher memberships; migration 16 rejects
publishers with multiple owners. Investigate and resolve those records before
retrying. The integration test
`upgrade_guards_preserve_existing_data_and_reject_inconsistent_state` exercises an
upgrade from migration 14 with both negative cases and preserved valid records in an
isolated PostgreSQL schema. Migrations 17 and 18 add the promotion-check, yank,
and advisory tables without backfills or new guards; migration 19 pins the
candidate inventory hash on each promotion check with a backfill from the
referenced candidates. The integration test
`migration_upgrade_preserves_data_through_revocation_tables` upgrades a populated
database from migration 16 through 20, verifies every existing row survives, checks the
backfilled inventory hash and confirms that legacy checks have no manifest hash,
and exercises the new tables against the preserved
releases and candidates.

A local PostgreSQL 18 drill used a custom-format `pg_dump --no-owner --no-acl` of a
disposable test database and `pg_restore --no-owner --no-acl --exit-on-error` into a
new database. User, publisher, membership, audit and successful migration counts
matched; the restored membership foreign key still refused deletion of an owner.
This was a database-only smoke drill with synthetic data. It did not rehearse a live
cutover, point-in-time recovery, object-store versions, external key material,
secrets, or backup retention. Those systems must be restored and reconciled together
before a production recovery can be claimed. No destructive downgrade migration is
provided; a rollback requires a tested, coordinated restore of compatible state.
