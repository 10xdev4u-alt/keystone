-- 0003_social.sql — Month 4: communities, discussions, polls, Q&A
--
-- Everything stacks on the Month-3 content spine:
--   - discussions / questions / polls are `posts` rows (kind column)
--   - comments are already shared; `posts.locked_at` stops new ones
--   - polls vote once per user per poll (PK-enforced)
--   - Q&A: answers + one vote per user per answer + bounty lifecycle with
--     transactional award invariants
--
-- Invariants that CANNOT be expressed as constraints live in repositories
-- (documented at each spot): single owner per community, answer-bounty
-- binding, award-only-once.

-- ── Communities ────────────────────────────────────────────────────────────
CREATE TABLE communities (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name        TEXT NOT NULL,
    slug        TEXT NOT NULL,
    description TEXT,
    visibility  TEXT NOT NULL DEFAULT 'public'
                CHECK (visibility IN ('public', 'private')),
    created_by  UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at  TIMESTAMPTZ
);

CREATE UNIQUE INDEX communities_slug_key ON communities (slug) WHERE deleted_at IS NULL;

CREATE TRIGGER communities_set_updated_at
    BEFORE UPDATE ON communities
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- Membership with roles. One owner per community is a repository invariant
-- (a partial unique index on owner can't be written portably here); the
-- role CHECK keeps the vocabulary honest.
CREATE TABLE community_members (
    community_id UUID NOT NULL REFERENCES communities (id) ON DELETE CASCADE,
    user_id      UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    role         TEXT NOT NULL DEFAULT 'member'
                 CHECK (role IN ('member', 'moderator', 'admin', 'owner')),
    joined_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (community_id, user_id)
);

CREATE INDEX community_members_user_idx ON community_members (user_id);

-- Discussions belong to a community through this join; `pinned` gives
-- moderators a curated top-of-feed slot.
CREATE TABLE community_posts (
    community_id UUID NOT NULL REFERENCES communities (id) ON DELETE CASCADE,
    post_id      UUID NOT NULL REFERENCES posts (id) ON DELETE CASCADE,
    pinned       BOOLEAN NOT NULL DEFAULT false,
    added_by     UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (community_id, post_id)
);

CREATE INDEX community_posts_feed_idx
    ON community_posts (community_id, pinned DESC, created_at DESC);

-- Locked discussions stop accepting new comments (repository enforces).
ALTER TABLE posts ADD COLUMN locked_at TIMESTAMPTZ;

-- Discussions are posts too — widen the spine's kind vocabulary.
ALTER TABLE posts DROP CONSTRAINT posts_kind_check;
ALTER TABLE posts ADD CONSTRAINT posts_kind_check
    CHECK (kind IN ('article', 'post', 'question', 'poll', 'discussion'));

-- ── Polls ──────────────────────────────────────────────────────────────────
CREATE TABLE poll_options (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    post_id    UUID NOT NULL REFERENCES posts (id) ON DELETE CASCADE,
    text       TEXT NOT NULL,
    position   INTEGER NOT NULL CHECK (position >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (post_id, position)
);

-- One vote per user per poll: the PK is (post_id, user_id). Switching the
-- vote is an upsert that moves the vote to another option in one statement.
CREATE TABLE poll_votes (
    post_id    UUID NOT NULL REFERENCES posts (id) ON DELETE CASCADE,
    option_id  UUID NOT NULL REFERENCES poll_options (id) ON DELETE CASCADE,
    user_id    UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (post_id, user_id)
);

CREATE INDEX poll_votes_option_idx ON poll_votes (option_id);

-- ── Q&A: answers + votes + bounties ────────────────────────────────────────
CREATE TABLE answers (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    question_id UUID NOT NULL REFERENCES posts (id) ON DELETE CASCADE,
    author_id   UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    body        TEXT NOT NULL,
    accepted_at TIMESTAMPTZ,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at  TIMESTAMPTZ
);

CREATE INDEX answers_question_idx ON answers (question_id, created_at);

CREATE TRIGGER answers_set_updated_at
    BEFORE UPDATE ON answers
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- One vote per user per answer; -1/+1 direction.
CREATE TABLE answer_votes (
    answer_id UUID NOT NULL REFERENCES answers (id) ON DELETE CASCADE,
    user_id   UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    vote      SMALLINT NOT NULL CHECK (vote IN (-1, 1)),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (answer_id, user_id)
);

CREATE TABLE bounties (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    question_id       UUID NOT NULL REFERENCES posts (id) ON DELETE CASCADE,
    amount            INTEGER NOT NULL CHECK (amount > 0),
    status            TEXT NOT NULL DEFAULT 'open'
                      CHECK (status IN ('open', 'awarded', 'expired')),
    expires_at        TIMESTAMPTZ NOT NULL,
    awarded_answer_id UUID REFERENCES answers (id) ON DELETE SET NULL,
    awarded_at        TIMESTAMPTZ,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (question_id)
);

-- ── Reports vocabulary grows with the new entity kinds ─────────────────────
ALTER TABLE reports DROP CONSTRAINT reports_entity_type_check;
ALTER TABLE reports ADD CONSTRAINT reports_entity_type_check
    CHECK (entity_type IN ('post', 'comment', 'user', 'review', 'community', 'answer'));

-- ── Feed indexes (keyset-ready: (created_at DESC, id DESC)) ────────────────
CREATE INDEX posts_feed_idx
    ON posts (created_at DESC, id DESC)
    WHERE deleted_at IS NULL AND status = 'published';
CREATE INDEX posts_author_feed_idx
    ON posts (author_id, created_at DESC, id DESC)
    WHERE deleted_at IS NULL AND status = 'published';
