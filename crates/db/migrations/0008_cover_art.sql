-- Cover art for posts.
--
-- `cover_image_url` points at an object stored in our own S3 bucket (or a
-- same-origin path in dev). It is plain presentation metadata: the HTML
-- renderer treats it as opaque and always loads it with `referrerpolicy`
-- and `loading="lazy"`, never as an inline resource.

ALTER TABLE posts ADD COLUMN cover_image_url TEXT;

-- Editors may want to search/filter by whether a post has cover art.
CREATE INDEX posts_cover_idx ON posts (cover_image_url) WHERE cover_image_url IS NOT NULL;
