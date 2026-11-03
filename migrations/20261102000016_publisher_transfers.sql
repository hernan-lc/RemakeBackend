-- One outstanding, expiring, recipient-accepted ownership transfer per publisher.
CREATE TABLE cloud_publisher_transfers (
    id UUID PRIMARY KEY,
    publisher_id UUID NOT NULL UNIQUE REFERENCES cloud_publishers(id),
    offered_by TEXT NOT NULL,
    recipient TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL DEFAULT now() + interval '24 hours',
    CHECK (offered_by <> recipient),
    FOREIGN KEY (publisher_id, offered_by) REFERENCES cloud_publisher_memberships(publisher_id, user_id) ON DELETE CASCADE,
    FOREIGN KEY (publisher_id, recipient) REFERENCES cloud_publisher_memberships(publisher_id, user_id) ON DELETE CASCADE
);
-- Fail migration if prior data has multiple owners; each transfer preserves one.
CREATE UNIQUE INDEX cloud_publisher_single_owner ON cloud_publisher_memberships(publisher_id) WHERE role='owner';
