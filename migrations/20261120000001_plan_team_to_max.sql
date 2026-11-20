-- Rename the `team` account plan to `max`. Limits are unchanged.
-- The old CHECK must be dropped before rows are rewritten to `max`, otherwise
-- the UPDATE is rejected by the constraint that still allows only free/pro/team.
ALTER TABLE cloud_account_plan DROP CONSTRAINT IF EXISTS cloud_account_plan_plan_check;
UPDATE cloud_account_plan SET plan = 'max' WHERE plan = 'team';
ALTER TABLE cloud_account_plan
    ADD CONSTRAINT cloud_account_plan_plan_check CHECK (plan IN ('free', 'pro', 'max'));
