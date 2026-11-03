-- Audited point-in-time promotion readiness decisions for private candidates.
-- These rows never deploy a snapshot; they retain the decision history that a
-- future promotion process must recheck before any deployment.
CREATE TABLE cloud_snapshot_promotion_checks (
    id UUID PRIMARY KEY,
    candidate_id UUID NOT NULL REFERENCES cloud_snapshot_candidates(id),
    decision TEXT NOT NULL CHECK (decision IN ('verified', 'blocked')),
    operator TEXT NOT NULL CHECK (length(operator) BETWEEN 1 AND 80),
    reason TEXT NOT NULL CHECK (length(reason) BETWEEN 1 AND 1024),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX cloud_snapshot_promotion_checks_candidate_idx
    ON cloud_snapshot_promotion_checks(candidate_id);
