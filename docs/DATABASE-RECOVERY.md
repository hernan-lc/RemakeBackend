# Database upgrade and restore evidence

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
database from migration 16 to 19, verifies every existing row survives, checks the
backfilled inventory hash, and exercises the new tables against the preserved
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
