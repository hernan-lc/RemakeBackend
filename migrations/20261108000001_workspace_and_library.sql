-- Separate v2 data prevents broadening the strict plugin-only v1 contract.
CREATE TABLE cloud_workspace_cursors (
    user_id TEXT PRIMARY KEY REFERENCES "user"(id) ON DELETE CASCADE,
    cursor BIGINT NOT NULL DEFAULT 0 CHECK (cursor >= 0)
);
CREATE TABLE cloud_workspace_records (
    user_id TEXT NOT NULL REFERENCES "user"(id) ON DELETE CASCADE,
    collection TEXT NOT NULL CHECK (collection IN ('plugin-selections','rule-templates','profiles','plugin-settings','preferences')),
    key TEXT NOT NULL CHECK (length(key) BETWEEN 1 AND 128),
    revision BIGINT NOT NULL CHECK (revision > 0),
    value JSONB,
    deleted BOOLEAN NOT NULL,
    PRIMARY KEY (user_id, collection, key),
    CHECK ((deleted AND value IS NULL) OR (NOT deleted AND value IS NOT NULL))
);
CREATE TABLE cloud_workspace_changes (
    user_id TEXT NOT NULL REFERENCES "user"(id) ON DELETE CASCADE,
    cursor BIGINT NOT NULL CHECK (cursor > 0),
    collection TEXT NOT NULL,
    key TEXT NOT NULL,
    value JSONB,
    deleted BOOLEAN NOT NULL,
    PRIMARY KEY (user_id, cursor),
    CHECK ((deleted AND value IS NULL) OR (NOT deleted AND value IS NOT NULL))
);

CREATE TABLE cloud_library_documents (
    id UUID PRIMARY KEY,
    owner_user_id TEXT NOT NULL REFERENCES "user"(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('template','profile')),
    slug TEXT NOT NULL CHECK (length(slug) BETWEEN 2 AND 64),
    revision BIGINT NOT NULL DEFAULT 1 CHECK (revision > 0),
    published_revision BIGINT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (owner_user_id, kind, slug)
);
CREATE TABLE cloud_library_versions (
    document_id UUID NOT NULL REFERENCES cloud_library_documents(id) ON DELETE CASCADE,
    revision BIGINT NOT NULL CHECK (revision > 0),
    title TEXT NOT NULL CHECK (length(title) BETWEEN 1 AND 120),
    document JSONB NOT NULL CHECK (jsonb_typeof(document) = 'object'),
    sha256 TEXT NOT NULL CHECK (sha256 ~ '^[0-9a-f]{64}$'),
    status TEXT NOT NULL DEFAULT 'draft' CHECK (status IN ('draft','pending','published','rejected','withdrawn')),
    reviewed_by TEXT,
    reason TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (document_id, revision)
);
ALTER TABLE cloud_library_documents ADD CONSTRAINT cloud_library_published_version
    FOREIGN KEY (id, published_revision) REFERENCES cloud_library_versions(document_id, revision) DEFERRABLE INITIALLY DEFERRED;
CREATE INDEX cloud_library_public ON cloud_library_versions(status, document_id) WHERE status='published';
