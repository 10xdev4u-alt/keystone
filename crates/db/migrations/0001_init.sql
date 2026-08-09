-- 0001_init.sql — Identity & audit baseline (hardened)
--
-- Principles:
--   * Every relationship has a real FOREIGN KEY with explicit ON DELETE semantics.
--   * Every enum-ish column has a CHECK constraint (mirrors the Rust enums).
--   * Snake_case identifiers only.
--   * Tokens are stored HASHED, never raw (token_hash columns).
--   * audit_logs is append-only, enforced by a trigger.
--   * updated_at is maintained by a trigger, not by application code.

-- ── Users ───────────────────────────────────────────────────────────────────
CREATE TABLE users (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    email         TEXT NOT NULL,
    email_lower   TEXT GENERATED ALWAYS AS (lower(email)) STORED NOT NULL,
    password_hash TEXT,
    role          TEXT NOT NULL DEFAULT 'user'
                  CHECK (role IN ('user', 'moderator', 'admin', 'super_admin')),
    status        TEXT NOT NULL DEFAULT 'pending_verification'
                  CHECK (status IN ('pending_verification', 'active', 'suspended', 'deleted')),
    first_name    TEXT,
    last_name     TEXT,
    username      TEXT,
    headline      TEXT,
    avatar_url    TEXT,
    trust_level   INTEGER NOT NULL DEFAULT 0 CHECK (trust_level >= 0),
    is_verified   BOOLEAN NOT NULL DEFAULT FALSE,
    last_login_at TIMESTAMPTZ,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at    TIMESTAMPTZ
);

-- Case-insensitive email uniqueness without a citext extension.
CREATE UNIQUE INDEX users_email_lower_key ON users (email_lower);
-- Usernames are unique only among live rows; soft-deleted rows free the name.
CREATE UNIQUE INDEX users_username_key ON users (username) WHERE deleted_at IS NULL;
CREATE INDEX users_status_idx ON users (status) WHERE deleted_at IS NULL;

-- ── Sessions (refresh tokens, hashed) ───────────────────────────────────────
CREATE TABLE sessions (
    id                      UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id                 UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    refresh_token_hash      TEXT NOT NULL,
    user_agent              TEXT,
    ip_address              INET,
    expires_at              TIMESTAMPTZ NOT NULL,
    created_at              TIMESTAMPTZ NOT NULL DEFAULT now(),
    revoked_at              TIMESTAMPTZ,
    replaced_by_session_id  UUID REFERENCES sessions (id) ON DELETE SET NULL
);

CREATE UNIQUE INDEX sessions_refresh_token_hash_key ON sessions (refresh_token_hash);
-- Live sessions per user (for list/revoke-all).
CREATE INDEX sessions_user_live_idx ON sessions (user_id) WHERE revoked_at IS NULL;
-- Cleanup jobs scan for expired sessions.
CREATE INDEX sessions_expires_at_idx ON sessions (expires_at) WHERE revoked_at IS NULL;

-- ── Audit log (append-only) ─────────────────────────────────────────────────
CREATE TABLE audit_logs (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    actor_user_id UUID REFERENCES users (id) ON DELETE SET NULL,
    action        TEXT NOT NULL,
    entity_type   TEXT,
    entity_id     TEXT,
    metadata      JSONB NOT NULL DEFAULT '{}'::jsonb,
    ip_address    INET,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX audit_logs_entity_idx ON audit_logs (entity_type, entity_id);
CREATE INDEX audit_logs_actor_idx ON audit_logs (actor_user_id, created_at DESC);
CREATE INDEX audit_logs_created_at_idx ON audit_logs (created_at DESC);

-- Append-only enforcement at the database level.
CREATE OR REPLACE FUNCTION audit_logs_append_only() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'audit_logs is append-only: updates and deletes are forbidden';
END;
$$;

CREATE TRIGGER audit_logs_append_only_trigger
    BEFORE UPDATE OR DELETE ON audit_logs
    FOR EACH ROW EXECUTE FUNCTION audit_logs_append_only();

-- ── Email verification ──────────────────────────────────────────────────────
CREATE TABLE email_verifications (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id    UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    token_hash TEXT NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    used_at    TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX email_verifications_token_hash_key ON email_verifications (token_hash);
CREATE INDEX email_verifications_user_idx ON email_verifications (user_id);

-- ── Password resets ─────────────────────────────────────────────────────────
CREATE TABLE password_resets (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id    UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    token_hash TEXT NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    used_at    TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX password_resets_token_hash_key ON password_resets (token_hash);
CREATE INDEX password_resets_user_idx ON password_resets (user_id);

-- ── Failed logins (lockout bookkeeping) ─────────────────────────────────────
CREATE TABLE failed_logins (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id      UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    ip_address   INET,
    attempted_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX failed_logins_user_idx ON failed_logins (user_id, attempted_at DESC);
CREATE INDEX failed_logins_ip_idx ON failed_logins (ip_address, attempted_at DESC);

-- ── updated_at maintenance ──────────────────────────────────────────────────
CREATE OR REPLACE FUNCTION set_updated_at() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    NEW.updated_at := now();
    RETURN NEW;
END;
$$;

CREATE TRIGGER users_set_updated_at
    BEFORE UPDATE ON users
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
