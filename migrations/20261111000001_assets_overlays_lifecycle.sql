-- Asset labels and keyset listing for assets and overlays.
--
-- cloud_assets.created_at already exists (NOT NULL DEFAULT now(), added with the
-- table), so every existing asset already has a real upload time and nothing is
-- backfilled or rewritten. The label is optional; existing rows keep NULL.
-- The label's content rules (no control characters) are enforced by the API;
-- the database only bounds its length.
ALTER TABLE cloud_assets ADD COLUMN label TEXT;
ALTER TABLE cloud_assets
    ADD CONSTRAINT cloud_assets_label_check
    CHECK (label IS NULL OR char_length(label) BETWEEN 1 AND 120);

-- Newest-first keyset pages: ORDER BY created_at DESC, id DESC per owner.
CREATE INDEX cloud_assets_owner_recent_idx
    ON cloud_assets (owner_user_id, created_at DESC, id DESC);

-- Overlays and packages live in cloud_library_documents; the same page order
-- for the owner's overlays. Overlay revisions are already retained one row each
-- in cloud_library_versions, so no history migration is needed.
CREATE INDEX cloud_library_documents_owner_recent_idx
    ON cloud_library_documents (owner_user_id, kind, created_at DESC, id DESC);
