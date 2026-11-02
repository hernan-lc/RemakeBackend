-- Sync data is Cloud-owned but its lifetime follows Better Auth's user row.
-- Validating these foreign keys intentionally fails if an old deployment has orphan rows;
-- operators must investigate those rows before migration rather than silently discard them.
ALTER TABLE cloud_sync_cursors ADD CONSTRAINT cloud_sync_cursors_user_fk FOREIGN KEY (user_id) REFERENCES "user"(id) ON DELETE CASCADE;
ALTER TABLE cloud_sync_records ADD CONSTRAINT cloud_sync_records_user_fk FOREIGN KEY (user_id) REFERENCES "user"(id) ON DELETE CASCADE;
ALTER TABLE cloud_sync_changes ADD CONSTRAINT cloud_sync_changes_user_fk FOREIGN KEY (user_id) REFERENCES "user"(id) ON DELETE CASCADE;
