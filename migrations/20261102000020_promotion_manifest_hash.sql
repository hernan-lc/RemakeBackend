-- Older decisions did not record a file-manifest digest and remain distinguishable.
ALTER TABLE cloud_snapshot_promotion_checks
    ADD COLUMN manifest_sha256 TEXT
    CHECK (manifest_sha256 IS NULL OR manifest_sha256 ~ '^[0-9a-f]{64}$');
