-- 0006_realtime.sql — Month 7: realtime & notifications
--
-- Theme: push without the plumbing debt.
--   - notifications: id-sequenced per-user feed (BIGSERIAL) so SSE gap
--     recovery is a single `WHERE id > $cursor` query; the read state is a
--     per-user cursor (not per-row flags), so unread counts and mark-read are
--     atomic single-statement operations — consistent under concurrency
--   - notification_deliveries: per-channel delivery tracking (in_app,
--     digest, email) so digest batching is idempotent
--   - notification_preferences: per-user channel toggles + muted kinds +
--     quiet hours, defaults = in-app only
--   - conversations: direct (unique unordered pair, DB-enforced) and group;
--     memberships are the authorization primitive — every read/write is
--     gated on membership, never on conversation id alone
--   - messages: persisted through the normal write path; WS is a thin
--     transport (delivery acks, typing, presence), never the source of truth
--   - presence: durable last_seen + status; visibility is restricted to
--     conversation members at the repo/API layer

-- ── Notifications ──────────────────────────────────────────────────────────
CREATE TABLE notification_preferences (
    user_id         UUID PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    in_app          BOOLEAN NOT NULL DEFAULT true,
    digest          BOOLEAN NOT NULL DEFAULT false,
    email           BOOLEAN NOT NULL DEFAULT false,
    muted_kinds     TEXT[]  NOT NULL DEFAULT '{}',
    quiet_hours_start SMALLINT,
    quiet_hours_end   SMALLINT,
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE notifications (
    id          BIGSERIAL PRIMARY KEY,
    user_id     UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    kind        TEXT NOT NULL,
    actor_id    UUID REFERENCES users (id) ON DELETE SET NULL,
    entity_type TEXT NOT NULL,
    entity_id   UUID,
    payload     JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Feed ordering + gap recovery: `(user_id, id)` covers both
-- `WHERE user_id = $1 AND id > $2 ORDER BY id` and cursor paging.
CREATE INDEX notifications_user_id_idx ON notifications (user_id, id DESC);

-- Read state is a single per-user cursor: everything with id <= cursor is
-- read. Upsert is atomic, so concurrent mark-reads never race.
CREATE TABLE notification_states (
    user_id     UUID PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    read_cursor BIGINT NOT NULL DEFAULT 0,
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE notification_deliveries (
    id              BIGSERIAL PRIMARY KEY,
    notification_id BIGINT NOT NULL REFERENCES notifications (id) ON DELETE CASCADE,
    user_id         UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    channel         TEXT NOT NULL CHECK (channel IN ('in_app', 'digest', 'email')),
    delivered_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (notification_id, channel)
);

-- ── Conversations & messages ───────────────────────────────────────────────
CREATE TABLE conversations (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    kind            TEXT NOT NULL CHECK (kind IN ('direct', 'group')),
    title           TEXT,
    user_a          UUID REFERENCES users (id) ON DELETE CASCADE,
    user_b          UUID REFERENCES users (id) ON DELETE CASCADE,
    created_by      UUID REFERENCES users (id) ON DELETE SET NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_message_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- At most one direct conversation per unordered pair, enforced by the DB.
CREATE UNIQUE INDEX conversations_direct_pair_key
    ON conversations (LEAST(user_a, user_b), GREATEST(user_a, user_b))
    WHERE kind = 'direct';

CREATE TABLE conversation_members (
    conversation_id UUID NOT NULL REFERENCES conversations (id) ON DELETE CASCADE,
    user_id         UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    joined_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_read_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (conversation_id, user_id)
);

CREATE TABLE messages (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    conversation_id UUID NOT NULL REFERENCES conversations (id) ON DELETE CASCADE,
    sender_id       UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    body            TEXT NOT NULL CHECK (char_length(body) BETWEEN 1 AND 4000),
    sent_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    edited_at       TIMESTAMPTZ,
    delivered_at    TIMESTAMPTZ,
    read_at         TIMESTAMPTZ
);

CREATE INDEX messages_conversation_sent_idx ON messages (conversation_id, sent_at);

-- ── Presence ───────────────────────────────────────────────────────────────
CREATE TABLE presence (
    user_id     UUID PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    status      TEXT NOT NULL DEFAULT 'offline'
                CHECK (status IN ('online', 'away', 'offline')),
    last_seen_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
