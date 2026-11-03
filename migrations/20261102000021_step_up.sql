-- Session-bound step-up authentication grants and one-time OAuth states.
--
-- Grants are minted only after a fresh password or OIDC reauthentication and
-- are consumed atomically with the sensitive mutation they authorize. Both
-- tables are append-then-consume: rows are never updated except to set
-- consumed_at, and nothing here is ever deleted by the application except
-- through operator maintenance. There is no rollback rehearsal beyond
-- restoring a backup: dropping these tables re-enables enrollment without
-- step-up history, so treat them like the key tables.
CREATE TABLE cloud_step_up_grants (
    id UUID PRIMARY KEY,
    user_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    action TEXT NOT NULL CHECK (action IN ('publisher.key.enroll', 'publisher.key.approve')),
    publisher_id UUID NOT NULL REFERENCES cloud_publishers(id),
    key_id UUID NULL,
    method TEXT NOT NULL CHECK (method IN ('password', 'oauth')),
    expires_at TIMESTAMPTZ NOT NULL,
    consumed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK ((action = 'publisher.key.approve' AND key_id IS NOT NULL)
        OR (action = 'publisher.key.enroll' AND key_id IS NULL))
);
CREATE INDEX cloud_step_up_grants_active_idx
    ON cloud_step_up_grants (user_id, session_id, expires_at)
    WHERE consumed_at IS NULL;

-- One-time OAuth step-up states: the state/nonce/PKCE record bound to the
-- authenticated user and session that started the flow. Consumed on callback
-- before the code exchange, so a captured callback URL cannot be replayed.
CREATE TABLE cloud_step_up_oauth_states (
    state TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    action TEXT NOT NULL CHECK (action IN ('publisher.key.enroll', 'publisher.key.approve')),
    publisher_id UUID NOT NULL REFERENCES cloud_publishers(id),
    key_id UUID NULL,
    nonce TEXT NOT NULL,
    code_verifier TEXT NOT NULL,
    redirect_uri TEXT NOT NULL,
    return_to TEXT NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    consumed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK ((action = 'publisher.key.approve' AND key_id IS NOT NULL)
        OR (action = 'publisher.key.enroll' AND key_id IS NULL))
);
