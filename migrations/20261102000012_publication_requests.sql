CREATE TABLE cloud_publication_jobs (
    id UUID PRIMARY KEY,
    release_id UUID NOT NULL UNIQUE REFERENCES cloud_plugin_releases(id),
    requested_by TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status = 'queued'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
