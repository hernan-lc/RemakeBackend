-- Pin the exact candidate inventory each promotion-readiness decision was
-- recorded against. Every check references a live candidate row through its
-- foreign key, so the backfill below resolves for all existing rows; a check
-- without a candidate would stop this upgrade for investigation like the
-- earlier membership guards.
ALTER TABLE cloud_snapshot_promotion_checks
    ADD COLUMN inventory_sha256 TEXT;
UPDATE cloud_snapshot_promotion_checks AS c
    SET inventory_sha256 =
        (SELECT inventory_sha256 FROM cloud_snapshot_candidates WHERE id = c.candidate_id);
ALTER TABLE cloud_snapshot_promotion_checks
    ALTER COLUMN inventory_sha256 SET NOT NULL;
ALTER TABLE cloud_snapshot_promotion_checks
    ADD CONSTRAINT cloud_snapshot_promotion_checks_inventory_sha256_check
    CHECK (inventory_sha256 ~ '^[0-9a-f]{64}$');
