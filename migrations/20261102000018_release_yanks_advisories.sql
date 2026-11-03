-- Moderator-recorded release yanks and security advisories. A yank is
-- permanent and blocks publication, preparation, and snapshot candidacy for
-- the release; it never deletes the release row or its audit history. At most
-- one yank and one advisory exist per release; posting an advisory again
-- replaces the previous one.
CREATE TABLE cloud_release_yanks (
    release_id UUID PRIMARY KEY REFERENCES cloud_plugin_releases(id),
    yanked_by TEXT NOT NULL CHECK (length(yanked_by) BETWEEN 1 AND 80),
    reason TEXT NOT NULL CHECK (length(reason) BETWEEN 1 AND 1024),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE cloud_release_advisories (
    release_id UUID PRIMARY KEY REFERENCES cloud_plugin_releases(id),
    severity TEXT NOT NULL CHECK (severity IN ('low', 'moderate', 'high', 'critical')),
    message TEXT NOT NULL CHECK (length(message) BETWEEN 1 AND 1024),
    url TEXT CHECK (url IS NULL OR length(url) BETWEEN 9 AND 2048),
    created_by TEXT NOT NULL CHECK (length(created_by) BETWEEN 1 AND 80),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
