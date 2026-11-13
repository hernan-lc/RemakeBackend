-- Authenticated, short-lived native endpoint discovery; no business commands.
ALTER TABLE cloud_remote_challenges DROP CONSTRAINT cloud_remote_challenges_purpose_check;
ALTER TABLE cloud_remote_challenges ADD CONSTRAINT cloud_remote_challenges_purpose_check
    CHECK (purpose IN ('enroll', 'ticket', 'peer', 'peer-grant'));

CREATE TABLE cloud_remote_peer_endpoints (
    device_id UUID PRIMARY KEY REFERENCES cloud_remote_devices(id),
    endpoint_id TEXT NOT NULL UNIQUE CHECK (endpoint_id ~ '^[0-9a-f]{64}$'),
    addresses JSONB NOT NULL CHECK (jsonb_typeof(addresses) = 'array' AND jsonb_array_length(addresses) <= 8),
    relay_urls JSONB NOT NULL CHECK (jsonb_typeof(relay_urls) = 'array' AND jsonb_array_length(relay_urls) <= 4),
    revision BIGINT NOT NULL CHECK (revision >= 1),
    expires_at TIMESTAMPTZ NOT NULL,
    relay_authorized_until TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX cloud_remote_peer_endpoints_expiry_idx ON cloud_remote_peer_endpoints(expires_at);
