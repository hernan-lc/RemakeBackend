-- Optional write fencing for clients that persist the epoch with each request.
-- The epoch and workspace cursor share one row lock and one account lifetime.
ALTER TABLE cloud_workspace_cursors
    ADD COLUMN write_epoch BIGINT NOT NULL DEFAULT 0 CHECK (write_epoch >= 0);
