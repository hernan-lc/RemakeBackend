-- Provenance of a forked library document, kept as row metadata. It is never
-- written into the portable template/profile value. The pair is both set or
-- both NULL. If the source revision (or its document) is ever deleted, the
-- link is cleared rather than left dangling and the fork stays a normal
-- independent document. Existing rows are untouched (both columns NULL).
ALTER TABLE cloud_library_documents
    ADD COLUMN fork_of_document UUID,
    ADD COLUMN fork_of_revision BIGINT;
ALTER TABLE cloud_library_documents
    ADD CONSTRAINT cloud_library_fork_pair
    CHECK ((fork_of_document IS NULL) = (fork_of_revision IS NULL));
ALTER TABLE cloud_library_documents
    ADD CONSTRAINT cloud_library_fork_source_fk
    FOREIGN KEY (fork_of_document, fork_of_revision)
    REFERENCES cloud_library_versions (document_id, revision) ON DELETE SET NULL;
