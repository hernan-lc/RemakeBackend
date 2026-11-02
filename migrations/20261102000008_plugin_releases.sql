-- Composite references ensure a release cannot link another publisher's upload or key.
ALTER TABLE cloud_upload_sessions ADD CONSTRAINT cloud_upload_publisher_unique UNIQUE (id, publisher_id);
ALTER TABLE cloud_publisher_keys ADD CONSTRAINT cloud_key_publisher_unique UNIQUE (id, publisher_id);
CREATE TABLE cloud_plugin_releases (
    id UUID PRIMARY KEY,
    publisher_id UUID NOT NULL REFERENCES cloud_publishers(id),
    upload_id UUID NOT NULL UNIQUE,
    publisher_key_id UUID NOT NULL,
    plugin_id TEXT NOT NULL,
    version TEXT NOT NULL,
    target TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('awaiting_verification', 'verified', 'rejected')),
    requested_by TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    FOREIGN KEY (upload_id, publisher_id) REFERENCES cloud_upload_sessions(id, publisher_id),
    FOREIGN KEY (publisher_key_id, publisher_id) REFERENCES cloud_publisher_keys(id, publisher_id),
    UNIQUE (plugin_id, version, target)
);
ALTER TABLE cloud_verification_jobs ADD COLUMN release_id UUID UNIQUE REFERENCES cloud_plugin_releases(id);
