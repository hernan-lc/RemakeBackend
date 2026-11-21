CREATE TABLE IF NOT EXISTS cloud_backups (
    id UUID PRIMARY KEY,
    owner_user_id TEXT NOT NULL REFERENCES "user"(id) ON DELETE CASCADE,
    name TEXT NOT NULL CHECK (char_length(name) BETWEEN 1 AND 120),
    manifest JSONB NOT NULL CHECK (jsonb_typeof(manifest) = 'object'),
    size_bytes BIGINT NOT NULL CHECK (size_bytes >= 0),
    workspace_records INTEGER NOT NULL DEFAULT 0 CHECK (workspace_records >= 0),
    library_documents INTEGER NOT NULL DEFAULT 0 CHECK (library_documents >= 0),
    media_documents INTEGER NOT NULL DEFAULT 0 CHECK (media_documents >= 0),
    assets INTEGER NOT NULL DEFAULT 0 CHECK (assets >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS cloud_backups_owner_recent_idx
    ON cloud_backups (owner_user_id, created_at DESC, id DESC);
