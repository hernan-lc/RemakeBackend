-- Temporary operator overrides are session- and resource-bound and short-lived.
-- Store only a keyed digest of the code; the code itself is emitted once to logs.
CREATE TABLE cloud_temporary_log_approvals (
    id UUID PRIMARY KEY,
    actor_user_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    publisher_id UUID NOT NULL,
    key_id UUID,
    code_digest TEXT NOT NULL CHECK (code_digest ~ '^[0-9a-f]{64}$'),
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts BETWEEN 0 AND 5),
    expires_at TIMESTAMPTZ NOT NULL,
    consumed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX cloud_temporary_log_approvals_expiry ON cloud_temporary_log_approvals (expires_at);
