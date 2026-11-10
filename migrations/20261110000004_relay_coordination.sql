-- Multi-replica relay coordination. Each API process is a relay node. A host or
-- controller connection is owned by exactly one node through a lease; frames for
-- a connection owned elsewhere are queued here and delivered by the owner. The
-- fencing token only increases, so a node that lost its lease cannot act on it.

CREATE TABLE cloud_relay_nodes (
    node_id TEXT PRIMARY KEY CHECK (length(node_id) BETWEEN 1 AND 128),
    heartbeat_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE cloud_relay_leases (
    resource_kind TEXT NOT NULL CHECK (resource_kind IN ('host', 'session')),
    resource_id UUID NOT NULL,
    node_id TEXT NOT NULL REFERENCES cloud_relay_nodes (node_id) ON DELETE CASCADE,
    fencing_token BIGINT NOT NULL CHECK (fencing_token >= 1),
    expires_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (resource_kind, resource_id)
);
CREATE INDEX cloud_relay_leases_node_idx ON cloud_relay_leases (node_id);

CREATE TABLE cloud_relay_frames (
    id BIGSERIAL PRIMARY KEY,
    target_node TEXT NOT NULL CHECK (length(target_node) BETWEEN 1 AND 128),
    resource_kind TEXT NOT NULL CHECK (resource_kind IN ('host', 'session')),
    resource_id UUID NOT NULL,
    payload TEXT NOT NULL CHECK (octet_length(payload) <= 131072),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX cloud_relay_frames_target_idx ON cloud_relay_frames (target_node, id);
