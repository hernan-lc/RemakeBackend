ALTER TABLE cloud_audit_events ADD COLUMN subject_id UUID;
CREATE TABLE cloud_publisher_key_challenges (
    id UUID PRIMARY KEY,
    publisher_id UUID NOT NULL REFERENCES cloud_publishers(id),
    actor_user_id TEXT NOT NULL,
    nonce TEXT NOT NULL UNIQUE CHECK (nonce ~ '^[0-9a-f]{64}$'),
    expires_at TIMESTAMPTZ NOT NULL,
    consumed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX cloud_key_challenges_active_idx ON cloud_publisher_key_challenges (publisher_id, actor_user_id, expires_at) WHERE consumed_at IS NULL;
CREATE TABLE cloud_publisher_keys (
    id UUID PRIMARY KEY,
    publisher_id UUID NOT NULL REFERENCES cloud_publishers(id),
    key_id TEXT NOT NULL UNIQUE CHECK (key_id ~ '^[a-z0-9][a-z0-9._-]{1,63}$'),
    public_key TEXT NOT NULL,
    fingerprint TEXT NOT NULL UNIQUE CHECK (fingerprint ~ '^[0-9a-f]{64}$'),
    status TEXT NOT NULL CHECK (status IN ('pending', 'approved', 'revoked')),
    requested_by TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX cloud_publisher_keys_publisher_idx ON cloud_publisher_keys (publisher_id, created_at);
