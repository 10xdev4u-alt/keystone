-- 0002_content.sql — Content core: one canonical spine, no duplicate concepts
--
-- The four HrX look-alikes (posts, discussions, q&a, polls) collapse into a
-- single `posts` spine with a `kind` column. Real version history, series,
-- tags, nested comments, reactions, bookmarks, reports, moderation, and ONE
-- consolidated reviews table keyed by (entity_type, entity_id).
--
-- Counters: comment/reaction/bookmark counts are DERIVED via a maintained
-- view (`post_counts`), never drifted columns. `view_count` is the only
-- column counter — it is incremented transactionally on view, not derivable.
--
-- Soft delete: `deleted_at` on every user-generated row; repositories filter
-- it. Slug uniqueness is scoped to live rows (partial unique index),
-- matching users_username_key.
--
-- Note: comment nesting cannot enforce "parent belongs to the same post"
-- with a plain CHECK; the repository enforces it.

-- ── Posts (the spine) ───────────────────────────────────────────────────────
CREATE TABLE posts (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    author_id    UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    kind         TEXT NOT NULL CHECK (kind IN ('article', 'post', 'question', 'poll')),
    title        TEXT,
    slug         TEXT NOT NULL,
    body         TEXT NOT NULL,
    summary      TEXT,
    status       TEXT NOT NULL DEFAULT 'published'
                 CHECK (status IN ('draft', 'published', 'archived', 'deleted')),
    visibility   TEXT NOT NULL DEFAULT 'public'
                 CHECK (visibility IN ('public', 'unlisted', 'private')),
    view_count   BIGINT NOT NULL DEFAULT 0 CHECK (view_count >= 0),
    published_at TIMESTAMPTZ,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at   TIMESTAMPTZ
);

CREATE UNIQUE INDEX posts_slug_key ON posts (slug) WHERE deleted_at IS NULL;
CREATE INDEX posts_author_idx ON posts (author_id, created_at DESC);
CREATE INDEX posts_kind_status_idx ON posts (kind, status, created_at DESC);

CREATE TRIGGER posts_set_updated_at
    BEFORE UPDATE ON posts
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- ── Version history (real, not a single "edited at" flag) ──────────────────
CREATE TABLE post_versions (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    post_id     UUID NOT NULL REFERENCES posts (id) ON DELETE CASCADE,
    title       TEXT,
    body        TEXT NOT NULL,
    summary     TEXT,
    editor_id   UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    change_note TEXT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX post_versions_post_idx ON post_versions (post_id, created_at DESC);

-- ── Series ──────────────────────────────────────────────────────────────────
CREATE TABLE series (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    author_id   UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    title       TEXT NOT NULL,
    slug        TEXT NOT NULL,
    description TEXT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at  TIMESTAMPTZ
);

CREATE UNIQUE INDEX series_slug_key ON series (slug) WHERE deleted_at IS NULL;

CREATE TRIGGER series_set_updated_at
    BEFORE UPDATE ON series
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE TABLE series_posts (
    series_id UUID NOT NULL REFERENCES series (id) ON DELETE CASCADE,
    post_id   UUID NOT NULL REFERENCES posts (id) ON DELETE CASCADE,
    position  INTEGER NOT NULL CHECK (position >= 0),
    added_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (series_id, post_id),
    UNIQUE (series_id, position)
);

-- ── Tags ────────────────────────────────────────────────────────────────────
CREATE TABLE tags (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name       TEXT NOT NULL,
    name_lower TEXT GENERATED ALWAYS AS (lower(name)) STORED NOT NULL,
    slug       TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX tags_name_lower_key ON tags (name_lower);
CREATE UNIQUE INDEX tags_slug_key ON tags (slug);

CREATE TABLE post_tags (
    post_id UUID NOT NULL REFERENCES posts (id) ON DELETE CASCADE,
    tag_id  UUID NOT NULL REFERENCES tags (id) ON DELETE CASCADE,
    PRIMARY KEY (post_id, tag_id)
);

-- ── Comments (single table, optional parent for nesting) ───────────────────
CREATE TABLE comments (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    post_id    UUID NOT NULL REFERENCES posts (id) ON DELETE CASCADE,
    author_id  UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    parent_id  UUID REFERENCES comments (id) ON DELETE CASCADE,
    body       TEXT NOT NULL,
    status     TEXT NOT NULL DEFAULT 'visible'
               CHECK (status IN ('visible', 'hidden', 'deleted')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at TIMESTAMPTZ
);

CREATE INDEX comments_post_idx ON comments (post_id, created_at);
CREATE INDEX comments_parent_idx ON comments (parent_id);

CREATE TRIGGER comments_set_updated_at
    BEFORE UPDATE ON comments
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- ── Reactions (one per user per post; changing kind replaces it) ───────────
CREATE TABLE reactions (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    post_id    UUID NOT NULL REFERENCES posts (id) ON DELETE CASCADE,
    user_id    UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    kind       TEXT NOT NULL
               CHECK (kind IN ('like', 'love', 'laugh', 'celebrate', 'insightful', 'curious')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (post_id, user_id)
);

CREATE INDEX reactions_user_idx ON reactions (user_id);

-- ── Bookmarks ───────────────────────────────────────────────────────────────
CREATE TABLE bookmarks (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id    UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    post_id    UUID NOT NULL REFERENCES posts (id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (user_id, post_id)
);

-- ── Reports (generic entity targeting) ─────────────────────────────────────
CREATE TABLE reports (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    reporter_id     UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    entity_type     TEXT NOT NULL CHECK (entity_type IN ('post', 'comment', 'user', 'review')),
    entity_id       UUID NOT NULL,
    reason          TEXT NOT NULL,
    detail          TEXT,
    status          TEXT NOT NULL DEFAULT 'open'
                    CHECK (status IN ('open', 'under_review', 'resolved', 'dismissed')),
    resolved_by     UUID REFERENCES users (id) ON DELETE SET NULL,
    resolved_at     TIMESTAMPTZ,
    resolution_note TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX reports_entity_idx ON reports (entity_type, entity_id);
CREATE INDEX reports_status_idx ON reports (status, created_at);

-- ── Moderation actions (append-only record of moderator decisions) ─────────
CREATE TABLE moderation_actions (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    moderator_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    action       TEXT NOT NULL
                 CHECK (action IN ('hide_comment', 'unhide_comment', 'delete_post',
                                   'restore_post', 'suspend_user', 'warn_user')),
    target_type  TEXT NOT NULL,
    target_id    UUID NOT NULL,
    reason       TEXT,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX moderation_target_idx ON moderation_actions (target_type, target_id);

-- ── Reviews (consolidated: one table for every reviewed entity type) ───────
CREATE TABLE reviews (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    author_id   UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    entity_type TEXT NOT NULL
                CHECK (entity_type IN ('employer', 'vendor', 'organization', 'course', 'mentor')),
    entity_id   UUID NOT NULL,
    rating      SMALLINT NOT NULL CHECK (rating BETWEEN 1 AND 5),
    title       TEXT,
    body        TEXT,
    status      TEXT NOT NULL DEFAULT 'published'
                CHECK (status IN ('published', 'hidden', 'deleted')),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at  TIMESTAMPTZ,
    UNIQUE (author_id, entity_type, entity_id)
);

CREATE INDEX reviews_entity_idx ON reviews (entity_type, entity_id, created_at DESC);

CREATE TRIGGER reviews_set_updated_at
    BEFORE UPDATE ON reviews
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- ── Derived counters (maintained view — never drifted columns) ─────────────
CREATE VIEW post_counts AS
SELECT p.id                                                   AS post_id,
       (SELECT count(*) FROM comments c
         WHERE c.post_id = p.id AND c.status = 'visible' AND c.deleted_at IS NULL)
                                                              AS comment_count,
       (SELECT count(*) FROM reactions r WHERE r.post_id = p.id)
                                                              AS reaction_count,
       (SELECT count(*) FROM bookmarks b WHERE b.post_id = p.id)
                                                              AS bookmark_count
FROM posts p;
