-- Managed integrations. The first provider is a signed inbound webhook. Its
-- secret is sealed with AES-256-GCM under the server key; the row keeps no
-- plaintext. Deliveries are stored with their JSON payload, bounded per connection.

CREATE TABLE cloud_integration_connections (
    id UUID PRIMARY KEY,
    owner_user_id TEXT NOT NULL CHECK (length(owner_user_id) BETWEEN 1 AND 256),
    provider TEXT NOT NULL CHECK (provider IN ('webhook')),
    label TEXT NOT NULL CHECK (length(label) BETWEEN 1 AND 120),
    secret_sealed BYTEA CHECK (secret_sealed IS NULL OR octet_length(secret_sealed) BETWEEN 29 AND 1024),
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'revoked')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    revoked_at TIMESTAMPTZ,
    CHECK ((status = 'active') = (secret_sealed IS NOT NULL))
);
CREATE INDEX cloud_integration_connections_owner_idx ON cloud_integration_connections (owner_user_id, id);

CREATE TABLE cloud_integration_events (
    connection_id UUID NOT NULL REFERENCES cloud_integration_connections (id) ON DELETE CASCADE,
    delivery_id TEXT NOT NULL CHECK (length(delivery_id) BETWEEN 1 AND 128),
    payload JSONB NOT NULL CHECK (jsonb_typeof(payload) = 'object'),
    received_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (connection_id, delivery_id)
);
CREATE INDEX cloud_integration_events_recent_idx ON cloud_integration_events (connection_id, received_at);
