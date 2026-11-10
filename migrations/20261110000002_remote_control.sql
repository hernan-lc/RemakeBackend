-- Remote control: teams (hosts), device identities with proof of possession,
-- single-use invitations, join requests, grants with explicit revisions,
-- sessions, single-use tickets, idempotent command receipts and audit.
-- The host keeps its own journal and remains the authority for effects; these
-- rows route, bound and record what the Cloud relay observed.

CREATE TABLE cloud_remote_hosts (
    id UUID PRIMARY KEY,
    owner_user_id TEXT NOT NULL CHECK (length(owner_user_id) BETWEEN 1 AND 256),
    name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 120),
    protocol_version INTEGER NOT NULL CHECK (protocol_version BETWEEN 1 AND 100),
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'disabled')),
    revision BIGINT NOT NULL DEFAULT 1 CHECK (revision >= 1),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX cloud_remote_hosts_owner_idx ON cloud_remote_hosts (owner_user_id, id);

CREATE TABLE cloud_remote_devices (
    id UUID PRIMARY KEY,
    owner_user_id TEXT NOT NULL CHECK (length(owner_user_id) BETWEEN 1 AND 256),
    label TEXT NOT NULL CHECK (length(label) BETWEEN 1 AND 120),
    kind TEXT NOT NULL CHECK (kind IN ('host', 'controller')),
    host_id UUID REFERENCES cloud_remote_hosts (id),
    public_key BYTEA NOT NULL UNIQUE CHECK (octet_length(public_key) = 32),
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'revoked')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    revoked_at TIMESTAMPTZ,
    CHECK ((kind = 'host' AND host_id IS NOT NULL) OR (kind = 'controller' AND host_id IS NULL))
);
CREATE INDEX cloud_remote_devices_owner_idx ON cloud_remote_devices (owner_user_id, id);

-- Challenges prove possession of a device key for one purpose and one context.
-- The signed message is rebuilt from the stored row, never from client input.
CREATE TABLE cloud_remote_challenges (
    id UUID PRIMARY KEY,
    owner_user_id TEXT NOT NULL CHECK (length(owner_user_id) BETWEEN 1 AND 256),
    purpose TEXT NOT NULL CHECK (purpose IN ('enroll', 'ticket')),
    nonce BYTEA NOT NULL CHECK (octet_length(nonce) = 32),
    binding_sha256 TEXT NOT NULL CHECK (binding_sha256 ~ '^[0-9a-f]{64}$'),
    expires_at TIMESTAMPTZ NOT NULL,
    consumed_at TIMESTAMPTZ
);

CREATE TABLE cloud_remote_invitations (
    id UUID PRIMARY KEY,
    host_id UUID NOT NULL REFERENCES cloud_remote_hosts (id),
    created_by TEXT NOT NULL CHECK (length(created_by) BETWEEN 1 AND 256),
    token_sha256 TEXT NOT NULL UNIQUE CHECK (token_sha256 ~ '^[0-9a-f]{64}$'),
    requested_role TEXT NOT NULL CHECK (requested_role IN ('observer', 'moderator', 'automationEditor')),
    status TEXT NOT NULL DEFAULT 'open' CHECK (status IN ('open', 'redeemed', 'revoked')),
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    redeemed_at TIMESTAMPTZ
);
CREATE INDEX cloud_remote_invitations_host_idx ON cloud_remote_invitations (host_id, id);

CREATE TABLE cloud_remote_requests (
    id UUID PRIMARY KEY,
    host_id UUID NOT NULL REFERENCES cloud_remote_hosts (id),
    invitation_id UUID NOT NULL UNIQUE REFERENCES cloud_remote_invitations (id),
    requester_user_id TEXT NOT NULL CHECK (length(requester_user_id) BETWEEN 1 AND 256),
    device_id UUID NOT NULL REFERENCES cloud_remote_devices (id),
    requested_role TEXT NOT NULL CHECK (requested_role IN ('observer', 'moderator', 'automationEditor')),
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'approved', 'denied')),
    decided_by TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    decided_at TIMESTAMPTZ
);
CREATE INDEX cloud_remote_requests_host_idx ON cloud_remote_requests (host_id, status, id);

-- A grant is authoritative only when 'active'. 'pendingHost' waits until the
-- host confirms the exact revision it persisted.
CREATE TABLE cloud_remote_grants (
    id UUID PRIMARY KEY,
    host_id UUID NOT NULL REFERENCES cloud_remote_hosts (id),
    device_id UUID NOT NULL REFERENCES cloud_remote_devices (id),
    requester_user_id TEXT NOT NULL CHECK (length(requester_user_id) BETWEEN 1 AND 256),
    request_id UUID REFERENCES cloud_remote_requests (id),
    role TEXT NOT NULL CHECK (role IN ('observer', 'moderator', 'automationEditor', 'custom')),
    policy JSONB NOT NULL DEFAULT '{}' CHECK (jsonb_typeof(policy) = 'object' AND octet_length(policy::text) <= 16384),
    status TEXT NOT NULL CHECK (status IN ('pendingHost', 'active', 'revoked')),
    revision BIGINT NOT NULL DEFAULT 1 CHECK (revision >= 1),
    confirmed_revision BIGINT CHECK (confirmed_revision IS NULL OR confirmed_revision >= 1),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (status <> 'active' OR confirmed_revision = revision)
);
CREATE UNIQUE INDEX cloud_remote_grants_live_device_idx ON cloud_remote_grants (host_id, device_id) WHERE status <> 'revoked';
CREATE INDEX cloud_remote_grants_host_idx ON cloud_remote_grants (host_id, id);

CREATE TABLE cloud_remote_sessions (
    id UUID PRIMARY KEY,
    host_id UUID NOT NULL REFERENCES cloud_remote_hosts (id),
    grant_id UUID NOT NULL REFERENCES cloud_remote_grants (id),
    device_id UUID NOT NULL REFERENCES cloud_remote_devices (id),
    grant_revision BIGINT NOT NULL CHECK (grant_revision >= 1),
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'closed', 'revoked')),
    opened_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    closed_at TIMESTAMPTZ
);
CREATE INDEX cloud_remote_sessions_host_idx ON cloud_remote_sessions (host_id, status, id);

-- Tickets authenticate one WebSocket connection. Only the hash is stored and
-- the row is consumed atomically on first use.
CREATE TABLE cloud_remote_tickets (
    id UUID PRIMARY KEY,
    token_sha256 TEXT NOT NULL UNIQUE CHECK (token_sha256 ~ '^[0-9a-f]{64}$'),
    role TEXT NOT NULL CHECK (role IN ('host', 'controller')),
    owner_user_id TEXT NOT NULL CHECK (length(owner_user_id) BETWEEN 1 AND 256),
    host_id UUID NOT NULL REFERENCES cloud_remote_hosts (id),
    device_id UUID NOT NULL REFERENCES cloud_remote_devices (id),
    grant_id UUID REFERENCES cloud_remote_grants (id),
    grant_revision BIGINT,
    expires_at TIMESTAMPTZ NOT NULL,
    consumed_at TIMESTAMPTZ,
    CHECK ((role = 'controller') = (grant_id IS NOT NULL))
);

-- Command receipts are keyed per host so a command id cannot be replayed with
-- another body. A command is never queued for an offline host.
CREATE TABLE cloud_remote_commands (
    host_id UUID NOT NULL REFERENCES cloud_remote_hosts (id),
    command_id TEXT NOT NULL CHECK (length(command_id) BETWEEN 8 AND 128),
    session_id UUID NOT NULL REFERENCES cloud_remote_sessions (id),
    operation TEXT NOT NULL CHECK (length(operation) BETWEEN 1 AND 64),
    args_sha256 TEXT NOT NULL CHECK (args_sha256 ~ '^[0-9a-f]{64}$'),
    status TEXT NOT NULL CHECK (status IN ('accepted', 'running', 'completed', 'failed', 'expired', 'cancelled', 'unknown', 'rejected')),
    code TEXT,
    result JSONB,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (host_id, command_id)
);
CREATE INDEX cloud_remote_commands_open_idx ON cloud_remote_commands (host_id, status) WHERE status IN ('accepted', 'running');

CREATE TABLE cloud_remote_audit (
    id UUID PRIMARY KEY,
    actor_user_id TEXT NOT NULL CHECK (length(actor_user_id) BETWEEN 1 AND 256),
    host_id UUID REFERENCES cloud_remote_hosts (id),
    action TEXT NOT NULL CHECK (length(action) BETWEEN 1 AND 64),
    subject_id TEXT NOT NULL CHECK (length(subject_id) BETWEEN 1 AND 128),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX cloud_remote_audit_host_idx ON cloud_remote_audit (host_id, created_at);
