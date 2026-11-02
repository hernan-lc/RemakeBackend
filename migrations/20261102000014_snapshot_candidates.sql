-- These are private candidates, never evidence of publication or promotion.
CREATE TABLE cloud_snapshot_candidates (
    id UUID PRIMARY KEY,
    status TEXT NOT NULL CHECK (status='candidate'),
    root_fingerprint TEXT NOT NULL CHECK (root_fingerprint ~ '^[0-9a-f]{64}$'),
    inventory_sha256 TEXT NOT NULL CHECK (inventory_sha256 ~ '^[0-9a-f]{64}$'),
    release_count INTEGER NOT NULL CHECK (release_count BETWEEN 1 AND 10000),
    file_count INTEGER NOT NULL CHECK (file_count BETWEEN 1 AND 41000),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE cloud_snapshot_candidate_releases (
    candidate_id UUID NOT NULL REFERENCES cloud_snapshot_candidates(id),
    publication_job_id UUID NOT NULL REFERENCES cloud_publication_jobs(id),
    prepared_key TEXT NOT NULL,
    prepared_version_id TEXT NOT NULL,
    sha256 TEXT NOT NULL CHECK (sha256 ~ '^[0-9a-f]{64}$'),
    PRIMARY KEY (candidate_id,publication_job_id)
);
CREATE TABLE cloud_snapshot_candidate_files (
    candidate_id UUID NOT NULL REFERENCES cloud_snapshot_candidates(id),
    path TEXT NOT NULL CHECK (length(path) BETWEEN 1 AND 2048),
    sha256 TEXT NOT NULL CHECK (sha256 ~ '^[0-9a-f]{64}$'),
    size BIGINT NOT NULL CHECK (size BETWEEN 1 AND 134217728),
    object_key TEXT NOT NULL UNIQUE,
    version_id TEXT NOT NULL CHECK (length(version_id) BETWEEN 1 AND 1024 AND version_id<>'null'),
    PRIMARY KEY (candidate_id,path)
);
