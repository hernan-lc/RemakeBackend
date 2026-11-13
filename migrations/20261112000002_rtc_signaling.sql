-- Negotiation metadata only; SDP/candidates are not stored in this table.
CREATE TABLE cloud_remote_rtc_negotiations (
    id UUID PRIMARY KEY,
    session_id UUID NOT NULL REFERENCES cloud_remote_sessions(id),
    host_device_id UUID NOT NULL REFERENCES cloud_remote_devices(id),
    controller_endpoint TEXT NOT NULL CHECK (controller_endpoint ~ '^[0-9a-f]{64}$'),
    host_endpoint TEXT NOT NULL CHECK (host_endpoint ~ '^[0-9a-f]{64}$'),
    grant_revision BIGINT NOT NULL CHECK (grant_revision > 0),
    controller_sequence INTEGER NOT NULL CHECK (controller_sequence BETWEEN 1 AND 65),
    host_sequence INTEGER NOT NULL DEFAULT 0 CHECK (host_sequence BETWEEN 0 AND 65),
    answered BOOLEAN NOT NULL DEFAULT FALSE,
    expires_at TIMESTAMPTZ NOT NULL DEFAULT now() + interval '60 seconds'
);
CREATE INDEX cloud_remote_rtc_session_idx ON cloud_remote_rtc_negotiations(session_id);
CREATE INDEX cloud_remote_rtc_expiry_idx ON cloud_remote_rtc_negotiations(expires_at);
