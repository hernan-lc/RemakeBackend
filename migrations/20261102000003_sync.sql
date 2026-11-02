-- Per-user cursor allocation is serialized in the same transaction as each change.
CREATE TABLE cloud_sync_cursors (
    user_id TEXT PRIMARY KEY,
    cursor BIGINT NOT NULL DEFAULT 0 CHECK (cursor >= 0)
);
CREATE TABLE cloud_sync_records (
    user_id TEXT NOT NULL,
    collection TEXT NOT NULL,
    key TEXT NOT NULL,
    revision BIGINT NOT NULL CHECK (revision > 0),
    value JSONB,
    deleted BOOLEAN NOT NULL,
    PRIMARY KEY (user_id, collection, key),
    CHECK ((deleted AND value IS NULL) OR (NOT deleted AND value IS NOT NULL))
);
CREATE TABLE cloud_sync_changes (
    user_id TEXT NOT NULL,
    cursor BIGINT NOT NULL,
    collection TEXT NOT NULL,
    key TEXT NOT NULL,
    value JSONB,
    deleted BOOLEAN NOT NULL,
    PRIMARY KEY (user_id, cursor),
    CHECK ((deleted AND value IS NULL) OR (NOT deleted AND value IS NOT NULL))
);
