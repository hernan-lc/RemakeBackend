-- Cloud authorization is separate from Better Auth's identity and session tables.
CREATE TABLE cloud_account_roles (
    user_id TEXT NOT NULL REFERENCES "user"(id) ON DELETE CASCADE,
    role TEXT NOT NULL CHECK (role IN ('admin', 'moderator')),
    PRIMARY KEY (user_id, role)
);
