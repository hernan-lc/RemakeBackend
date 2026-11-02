-- Cloud data uses opaque Better Auth user IDs. There is intentionally no cross-schema FK:
-- account deletion and retention coordination must be implemented before production.
CREATE TABLE cloud_publishers (
    id UUID PRIMARY KEY,
    namespace TEXT NOT NULL UNIQUE,
    status TEXT NOT NULL CHECK (status IN ('pending', 'approved', 'suspended')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (namespace ~ '^[a-z][a-z0-9-]{1,62}$' AND namespace !~ '-$')
);
CREATE TABLE cloud_publisher_memberships (
    publisher_id UUID NOT NULL REFERENCES cloud_publishers(id),
    user_id TEXT NOT NULL,
    role TEXT NOT NULL CHECK (role IN ('owner', 'admin', 'publisher', 'viewer')),
    PRIMARY KEY (publisher_id, user_id)
);
CREATE INDEX cloud_memberships_user_idx ON cloud_publisher_memberships (user_id);
CREATE TABLE cloud_audit_events (
    id UUID PRIMARY KEY,
    actor_user_id TEXT NOT NULL,
    publisher_id UUID REFERENCES cloud_publishers(id),
    action TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
