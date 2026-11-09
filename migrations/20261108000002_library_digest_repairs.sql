-- Explicit operator repairs only. Existing content, review and publication
-- metadata are never changed by schema migration or by an ordinary read.
CREATE TABLE cloud_library_digest_repairs (
    id UUID PRIMARY KEY,
    document_id UUID NOT NULL,
    revision BIGINT NOT NULL CHECK (revision > 0),
    old_sha256 TEXT NOT NULL CHECK (old_sha256 ~ '^[0-9a-f]{64}$'),
    normalized_sha256 TEXT NOT NULL CHECK (normalized_sha256 ~ '^[0-9a-f]{64}$'),
    operator TEXT NOT NULL CHECK (octet_length(operator) BETWEEN 1 AND 200),
    repaired_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (old_sha256 <> normalized_sha256)
);
-- Retain metadata-only evidence after account/document deletion, consistently
-- with the existing audit log. No document or account content is copied here.
CREATE INDEX cloud_library_digest_repairs_document
    ON cloud_library_digest_repairs(document_id, revision, repaired_at);
