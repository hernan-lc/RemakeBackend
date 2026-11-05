-- Preserve the original publisher identity when an unused namespace is removed.
ALTER TABLE cloud_audit_events ADD COLUMN deleted_publisher_id UUID;
CREATE INDEX cloud_audit_deleted_publisher_idx ON cloud_audit_events(deleted_publisher_id)
    WHERE deleted_publisher_id IS NOT NULL;
