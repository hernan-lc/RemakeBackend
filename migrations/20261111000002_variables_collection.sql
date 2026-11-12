-- The credential-free `variables` workspace collection. The CHECK on the table
-- is widened; every existing row already satisfies it. The change feed table
-- has no collection constraint. Validation of keys and values stays in the
-- cloud-contracts crate; secrets are never a variable type.
ALTER TABLE cloud_workspace_records
    DROP CONSTRAINT IF EXISTS cloud_workspace_records_collection_check;
ALTER TABLE cloud_workspace_records
    ADD CONSTRAINT cloud_workspace_records_collection_check
    CHECK (collection IN ('plugin-selections','rule-templates','profiles','plugin-settings','preferences','variables'));
