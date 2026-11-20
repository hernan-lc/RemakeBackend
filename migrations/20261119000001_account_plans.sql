-- Account plans. Every account is treated as `free` until a row exists, so
-- introducing this table never changes the limits of existing accounts.
-- Plans are a backend concern: clients cannot raise their own quota.
CREATE TABLE IF NOT EXISTS cloud_account_plan (
    user_id TEXT PRIMARY KEY REFERENCES "user"(id) ON DELETE CASCADE,
    plan TEXT NOT NULL DEFAULT 'free' CHECK (plan IN ('free', 'pro', 'team')),
    updated_at BIGINT NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS cloud_account_plan_plan_idx ON cloud_account_plan (plan);
