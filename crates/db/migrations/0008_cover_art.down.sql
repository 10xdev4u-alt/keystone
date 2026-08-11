DROP INDEX IF EXISTS posts_cover_idx;
ALTER TABLE posts DROP COLUMN IF EXISTS cover_image_url;
