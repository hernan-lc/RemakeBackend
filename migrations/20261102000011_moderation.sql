ALTER TABLE cloud_plugin_releases DROP CONSTRAINT cloud_plugin_release_status;
ALTER TABLE cloud_plugin_releases ADD CONSTRAINT cloud_plugin_release_status CHECK (status IN ('awaiting_verification','integrity_passed','antivirus_passed','review_approved','review_rejected','rejected'));
CREATE TABLE cloud_moderation_events (
    id UUID PRIMARY KEY,
    release_id UUID NOT NULL UNIQUE REFERENCES cloud_plugin_releases(id),
    actor_user_id TEXT NOT NULL,
    decision TEXT NOT NULL CHECK (decision IN ('approve','reject')),
    reason_code TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK ((decision='approve' AND reason_code='policy_reviewed') OR (decision='reject' AND reason_code IN ('policy','malware','other')))
);
CREATE INDEX cloud_release_moderation_queue_idx ON cloud_plugin_releases (id) WHERE status='antivirus_passed';
