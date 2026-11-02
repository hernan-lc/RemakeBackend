CREATE TABLE cloud_upload_sessions (
    id UUID PRIMARY KEY,
    publisher_id UUID NOT NULL REFERENCES cloud_publishers(id),
    requested_by TEXT NOT NULL,
    idempotency_key TEXT NOT NULL CHECK (idempotency_key ~ '^[A-Za-z0-9._~-]{16,128}$'),
    expected_size BIGINT NOT NULL CHECK (expected_size BETWEEN 1 AND 134217728),
    expected_sha256 TEXT NOT NULL CHECK (expected_sha256 ~ '^[0-9a-f]{64}$'),
    object_key TEXT NOT NULL UNIQUE,
    status TEXT NOT NULL CHECK (status IN ('pending', 'quarantined')),
    expires_at TIMESTAMPTZ NOT NULL,
    version_id TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (publisher_id, idempotency_key),
    CHECK ((status = 'pending' AND version_id IS NULL) OR (status = 'quarantined' AND version_id IS NOT NULL AND version_id <> ''))
);
CREATE INDEX cloud_upload_sessions_active_idx ON cloud_upload_sessions (publisher_id, expires_at) WHERE status = 'pending';
CREATE TABLE cloud_verification_jobs (
    upload_id UUID PRIMARY KEY REFERENCES cloud_upload_sessions(id),
    status TEXT NOT NULL CHECK (status = 'queued'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
