-- Existing orphan memberships must be investigated before enabling this constraint.
-- RESTRICT forces an explicit membership/role removal before Better Auth can delete
-- the user. It also serializes correctly with concurrent inserts through FK locks.
ALTER TABLE cloud_publisher_memberships
    ADD CONSTRAINT cloud_publisher_memberships_user_fk
    FOREIGN KEY (user_id) REFERENCES "user"(id) ON DELETE RESTRICT;

-- Account role changes must pass the audited operator path, including its
-- last-admin check; a user deletion must not bypass it through cascading deletes.
ALTER TABLE cloud_account_roles DROP CONSTRAINT cloud_account_roles_user_id_fkey;
ALTER TABLE cloud_account_roles
    ADD CONSTRAINT cloud_account_roles_user_fk
    FOREIGN KEY (user_id) REFERENCES "user"(id) ON DELETE RESTRICT;
