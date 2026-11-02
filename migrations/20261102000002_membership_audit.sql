ALTER TABLE cloud_audit_events ADD COLUMN target_user_id TEXT;
ALTER TABLE cloud_audit_events ADD COLUMN old_role TEXT;
ALTER TABLE cloud_audit_events ADD COLUMN new_role TEXT;
