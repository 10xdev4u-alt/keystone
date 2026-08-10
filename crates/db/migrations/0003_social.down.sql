-- 0003_social.down.sql — reverse the Month-4 schema.
--
-- Drop dependents first; 0002's `set_updated_at()` trigger function stays.
-- `posts.locked_at` and the reports CHECK constraint return to their 0002
-- shapes.

DROP INDEX IF EXISTS posts_author_feed_idx;
DROP INDEX IF EXISTS posts_feed_idx;

ALTER TABLE reports DROP CONSTRAINT reports_entity_type_check;
ALTER TABLE reports ADD CONSTRAINT reports_entity_type_check
    CHECK (entity_type IN ('post', 'comment', 'user', 'review'));

DROP TABLE IF EXISTS bounties;
DROP TABLE IF EXISTS answer_votes;
DROP TABLE IF EXISTS answers;
DROP TABLE IF EXISTS poll_votes;
DROP TABLE IF EXISTS poll_options;
DROP TABLE IF EXISTS community_posts;
DROP TABLE IF EXISTS community_members;
DROP TABLE IF EXISTS communities;

ALTER TABLE posts DROP COLUMN IF EXISTS locked_at;

ALTER TABLE posts DROP CONSTRAINT posts_kind_check;
ALTER TABLE posts ADD CONSTRAINT posts_kind_check
    CHECK (kind IN ('article', 'post', 'question', 'poll'));
