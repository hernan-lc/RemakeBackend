-- Fencing generations must not be reused after a lease row is removed.
CREATE SEQUENCE cloud_relay_fencing_seq AS BIGINT;
SELECT setval('cloud_relay_fencing_seq', GREATEST(COALESCE(MAX(fencing_token),0)+1,1), FALSE)
FROM cloud_relay_leases;

-- Opaque TLS streams only: no certificates, ciphertext or business commands.
CREATE TABLE cloud_remote_relay_tunnels (
    id UUID PRIMARY KEY,
    session_id UUID NOT NULL REFERENCES cloud_remote_sessions(id) ON DELETE CASCADE,
    host_device_id UUID NOT NULL REFERENCES cloud_remote_devices(id),
    controller_endpoint TEXT NOT NULL CHECK (controller_endpoint ~ '^[0-9a-f]{64}$'),
    host_endpoint TEXT NOT NULL CHECK (host_endpoint ~ '^[0-9a-f]{64}$'),
    grant_revision BIGINT NOT NULL CHECK (grant_revision > 0),
    host_node TEXT NOT NULL,
    controller_node TEXT NOT NULL,
    host_fencing BIGINT NOT NULL,
    controller_fencing BIGINT NOT NULL,
    controller_sequence BIGINT NOT NULL DEFAULT 0 CHECK (controller_sequence >= 0),
    host_sequence BIGINT NOT NULL DEFAULT 0 CHECK (host_sequence >= 0),
    answered BOOLEAN NOT NULL DEFAULT FALSE,
    closed BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL DEFAULT now() + interval '60 seconds'
);
CREATE INDEX cloud_remote_relay_tunnels_session_idx ON cloud_remote_relay_tunnels(session_id);
CREATE INDEX cloud_remote_relay_tunnels_expiry_idx ON cloud_remote_relay_tunnels(expires_at);
