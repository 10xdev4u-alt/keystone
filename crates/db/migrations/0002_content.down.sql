-- 0002_content.down.sql — reverse the content-core schema.
--
-- Drop order matters: dependents first, then their bases. Triggers and
-- indexes drop with their tables; `set_updated_at()` is owned by 0001 and
-- stays.

DROP VIEW IF EXISTS post_counts;

DROP TABLE IF EXISTS reviews;
DROP TABLE IF EXISTS moderation_actions;
DROP TABLE IF EXISTS reports;
DROP TABLE IF EXISTS bookmarks;
DROP TABLE IF EXISTS reactions;
DROP TABLE IF EXISTS comments;
DROP TABLE IF EXISTS post_tags;
DROP TABLE IF EXISTS tags;
DROP TABLE IF EXISTS series_posts;
DROP TABLE IF EXISTS series;
DROP TABLE IF EXISTS post_versions;
DROP TABLE IF EXISTS posts;
