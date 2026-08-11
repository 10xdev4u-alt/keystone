-- 0007_search_storage.sql — Month 8: search & storage
--
-- Theme: findable and durable, actually.
--   - Postgres FTS over posts/users/communities/courses: generated tsvector
--     columns (title weighted A, body B) + GIN indexes, pg_trgm for typo
--     tolerance. Rankings combine ts_rank + recency + trigram similarity.
--   - file_records: metadata + object key ONLY (the bytes live in the object
--     bucket); folders are a metadata tree (parent_id), never paths.
--   - upload_quotas: per-user byte caps; enforcement sums file_records.
--
-- Extensions are idempotent; the migration user owns the database.
-- pg_trgm is installed in `public` deterministically (IF NOT EXISTS skips
-- once it exists anywhere), and isolated test schemas extend their search_path
-- with `public` so the trigram functions are visible everywhere.
CREATE EXTENSION IF NOT EXISTS pg_trgm WITH SCHEMA public;

-- ── Full-text search ────────────────────────────────────────────────────────
-- Posts: title (weight A) + body/summary (weight B); only live public content.
ALTER TABLE posts
    ADD COLUMN search_doc tsvector
    GENERATED ALWAYS AS (
        setweight(to_tsvector('english', coalesce(title, '')), 'A') ||
        setweight(to_tsvector('english', coalesce(body, '')), 'B') ||
        setweight(to_tsvector('english', coalesce(summary, '')), 'B')
    ) STORED;

CREATE INDEX posts_search_idx ON posts USING GIN (search_doc)
    WHERE status = 'published' AND visibility = 'public' AND deleted_at IS NULL;

-- Users: identity fields + headline.
ALTER TABLE users
    ADD COLUMN search_doc tsvector
    GENERATED ALWAYS AS (
        setweight(to_tsvector('english', coalesce(username, '')), 'A') ||
        setweight(to_tsvector('english', coalesce(first_name, '')), 'B') ||
        setweight(to_tsvector('english', coalesce(last_name, '')), 'B') ||
        setweight(to_tsvector('english', coalesce(headline, '')), 'B')
    ) STORED;

CREATE INDEX users_search_idx ON users USING GIN (search_doc)
    WHERE status = 'active' AND deleted_at IS NULL;

-- Communities.
ALTER TABLE communities
    ADD COLUMN search_doc tsvector
    GENERATED ALWAYS AS (
        setweight(to_tsvector('english', coalesce(name, '')), 'A') ||
        setweight(to_tsvector('english', coalesce(description, '')), 'B')
    ) STORED;

CREATE INDEX communities_search_idx ON communities USING GIN (search_doc)
    WHERE deleted_at IS NULL;

-- Courses.
ALTER TABLE courses
    ADD COLUMN search_doc tsvector
    GENERATED ALWAYS AS (
        setweight(to_tsvector('english', coalesce(title, '')), 'A') ||
        setweight(to_tsvector('english', coalesce(description, '')), 'B')
    ) STORED;

CREATE INDEX courses_search_idx ON courses USING GIN (search_doc)
    WHERE status = 'published' AND deleted_at IS NULL;

-- ── Object storage metadata ─────────────────────────────────────────────────
CREATE TABLE file_records (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    owner_id      UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    bucket        TEXT NOT NULL,
    -- Object key ONLY — bytes live in the bucket, never in the DB.
    object_key    TEXT NOT NULL,
    original_name TEXT NOT NULL,
    content_type  TEXT NOT NULL,
    size_bytes    BIGINT NOT NULL CHECK (size_bytes >= 0),
    sha256        TEXT NOT NULL,
    width         INTEGER,
    height        INTEGER,
    -- Metadata tree: folders are parent links, not path strings.
    parent_id     UUID REFERENCES file_records (id) ON DELETE CASCADE,
    is_public     BOOLEAN NOT NULL DEFAULT FALSE,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (owner_id, object_key)
);

CREATE INDEX file_records_owner_idx ON file_records (owner_id, created_at DESC);

-- Per-user upload caps; enforcement sums file_records.size_bytes live.
CREATE TABLE upload_quotas (
    user_id     UUID PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    bytes_limit BIGINT NOT NULL DEFAULT 1073741824 CHECK (bytes_limit > 0),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
