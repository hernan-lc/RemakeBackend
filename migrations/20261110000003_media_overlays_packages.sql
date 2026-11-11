-- Managed media, overlay and package documents.
-- Assets hold verified raster bytes (PNG, JPEG, WebP) owned by one account.
-- Overlays and packages reuse the private library tables with their own kinds;
-- they are never published through the v2 review pipeline.

CREATE TABLE cloud_assets (
    id UUID PRIMARY KEY,
    owner_user_id TEXT NOT NULL CHECK (length(owner_user_id) BETWEEN 1 AND 256),
    sha256 TEXT NOT NULL CHECK (sha256 ~ '^[0-9a-f]{64}$'),
    mime TEXT NOT NULL CHECK (mime IN ('image/png', 'image/jpeg', 'image/webp')),
    width INTEGER NOT NULL CHECK (width BETWEEN 1 AND 4096),
    height INTEGER NOT NULL CHECK (height BETWEEN 1 AND 4096),
    size INTEGER NOT NULL CHECK (size BETWEEN 1 AND 524288),
    content BYTEA NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (owner_user_id, sha256),
    CHECK (octet_length(content) = size)
);
CREATE INDEX cloud_assets_owner_idx ON cloud_assets (owner_user_id, id);

ALTER TABLE cloud_library_documents
    DROP CONSTRAINT cloud_library_documents_kind_check,
    ADD CONSTRAINT cloud_library_documents_kind_check
        CHECK (kind IN ('template', 'profile', 'overlay', 'package'));
