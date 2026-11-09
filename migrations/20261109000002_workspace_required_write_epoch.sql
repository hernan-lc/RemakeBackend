-- Once explicitly enabled for an account, legacy unfenced v2 writers must
-- not bypass recovery fencing. This is separate from ordinary barriers.
ALTER TABLE cloud_workspace_cursors
    ADD COLUMN write_epoch_required BOOLEAN NOT NULL DEFAULT FALSE;
