-- Existing bearer invitations keep their behavior; new invitations may bind
-- redemption to a selected account without retaining its email.
ALTER TABLE cloud_remote_invitations
  ADD COLUMN recipient_user_id TEXT REFERENCES "user"(id) ON DELETE CASCADE;

CREATE INDEX cloud_remote_people_nickname ON "user"
  (lower(split_part(email, '@', 1)) text_pattern_ops);
