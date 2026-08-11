-- 0005_learning.sql — Month 6: learning, mentorship, events
--
-- Theme: progress that cannot lie.
--   - courses / modules / lessons with enrollment + lesson progress (ONE row
--     per user+lesson, PK-enforced — progress is transactional by design)
--   - certificates: verifiable token (hash stored; unique per user+course)
--   - assessments: question banks, attempts with scoring + pass/fail rules;
--     attempt caps + time limits enforced at the repo (anti-cheat basics)
--   - credits: IMMUTABLE append-only ledger; balance is SUM(delta);
--     redemption re-checks the balance inside a transaction
--   - learning paths: ordered courses, progress derived from course progress
--   - mentorship: profiles, requests (state machine), sessions, feedback,
--     goals
--   - events: idempotent registrations (PK (event_id, user_id)), waitlists,
--     speakers, capacity limits
--
-- Repository-enforced invariants (documented at each repo):
--   - enrollment is idempotent; completion is computed, never claimed
--   - certificate issuance is atomic with course completion and unique
--   - credit redemption is serializable — no double-spend under concurrency
--   - event registration checks capacity and promotes waitlists in order
--   - assessment attempts are capped and time-limited

-- ── Courses ────────────────────────────────────────────────────────────────
CREATE TABLE courses (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    author_id   UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    title       TEXT NOT NULL,
    slug        TEXT NOT NULL,
    description TEXT,
    status      TEXT NOT NULL DEFAULT 'draft'
                CHECK (status IN ('draft', 'published', 'archived')),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at  TIMESTAMPTZ
);

CREATE UNIQUE INDEX courses_slug_key ON courses (slug) WHERE deleted_at IS NULL;

CREATE TRIGGER courses_set_updated_at
    BEFORE UPDATE ON courses
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE TABLE course_modules (
    id        UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    course_id UUID NOT NULL REFERENCES courses (id) ON DELETE CASCADE,
    position  INTEGER NOT NULL CHECK (position >= 0),
    title     TEXT NOT NULL,
    UNIQUE (course_id, position)
);

CREATE INDEX course_modules_course_idx ON course_modules (course_id);

CREATE TABLE lessons (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    module_id         UUID NOT NULL REFERENCES course_modules (id) ON DELETE CASCADE,
    position          INTEGER NOT NULL CHECK (position >= 0),
    title             TEXT NOT NULL,
    content           TEXT NOT NULL DEFAULT '',
    duration_seconds  INTEGER CHECK (duration_seconds IS NULL OR duration_seconds >= 0),
    UNIQUE (module_id, position)
);

CREATE INDEX lessons_module_idx ON lessons (module_id);

-- ── Enrollment & progress ───────────────────────────────────────────────────
CREATE TABLE enrollments (
    course_id    UUID NOT NULL REFERENCES courses (id) ON DELETE CASCADE,
    user_id      UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    enrolled_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (course_id, user_id)
);

-- One row per user+lesson. Progress is a single upsert — the PK makes
-- concurrent double-marking idempotent at the schema level.
CREATE TABLE lesson_progress (
    lesson_id        UUID NOT NULL REFERENCES lessons (id) ON DELETE CASCADE,
    user_id          UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    completed        BOOLEAN NOT NULL DEFAULT false,
    progress_percent INTEGER NOT NULL DEFAULT 0 CHECK (progress_percent BETWEEN 0 AND 100),
    completed_at     TIMESTAMPTZ,
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (lesson_id, user_id)
);

CREATE INDEX lesson_progress_user_idx ON lesson_progress (user_id, completed);

-- ── Certificates (verifiable token, not a forged-able PDF) ─────────────────
CREATE TABLE certificates (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id    UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    course_id  UUID NOT NULL REFERENCES courses (id) ON DELETE CASCADE,
    token_hash TEXT NOT NULL,
    issued_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (user_id, course_id)
);

CREATE INDEX certificates_course_idx ON certificates (course_id);

-- ── Assessments ─────────────────────────────────────────────────────────────
CREATE TABLE assessments (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    course_id         UUID NOT NULL REFERENCES courses (id) ON DELETE CASCADE,
    title             TEXT NOT NULL,
    pass_threshold    INTEGER NOT NULL DEFAULT 70 CHECK (pass_threshold BETWEEN 1 AND 100),
    time_limit_seconds INTEGER CHECK (time_limit_seconds IS NULL OR time_limit_seconds > 0),
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE assessment_questions (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    assessment_id UUID NOT NULL REFERENCES assessments (id) ON DELETE CASCADE,
    position      INTEGER NOT NULL CHECK (position >= 0),
    prompt        TEXT NOT NULL,
    UNIQUE (assessment_id, position)
);

CREATE INDEX assessment_questions_assessment_idx ON assessment_questions (assessment_id);

-- Attempt: score/passed are written at submit time inside a transaction;
-- the repo enforces the attempt cap and the time limit.
CREATE TABLE assessment_attempts (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    assessment_id UUID NOT NULL REFERENCES assessments (id) ON DELETE CASCADE,
    user_id       UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    started_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    submitted_at  TIMESTAMPTZ,
    score         INTEGER CHECK (score IS NULL OR score BETWEEN 0 AND 100),
    passed        BOOLEAN
);

CREATE INDEX assessment_attempts_user_idx
    ON assessment_attempts (user_id, assessment_id);

CREATE TABLE assessment_answers (
    attempt_id  UUID NOT NULL REFERENCES assessment_attempts (id) ON DELETE CASCADE,
    question_id UUID NOT NULL REFERENCES assessment_questions (id) ON DELETE CASCADE,
    response    TEXT NOT NULL,
    correct     BOOLEAN NOT NULL DEFAULT false,
    PRIMARY KEY (attempt_id, question_id)
);

-- ── Credits: immutable append-only ledger ───────────────────────────────────
-- No UPDATE/DELETE path exists for this table — the balance is always
-- SUM(delta). Redemption inserts a NEGATIVE delta only after re-checking the
-- balance inside a transaction (serializable when racing).
CREATE TABLE credit_ledger (
    id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id        UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    delta          INTEGER NOT NULL CHECK (delta <> 0),
    reason         TEXT NOT NULL,
    reference_type TEXT,
    reference_id   UUID,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX credit_ledger_user_idx ON credit_ledger (user_id, created_at);

-- ── Learning paths ──────────────────────────────────────────────────────────
CREATE TABLE learning_paths (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    title       TEXT NOT NULL,
    description TEXT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE learning_path_courses (
    path_id   UUID NOT NULL REFERENCES learning_paths (id) ON DELETE CASCADE,
    course_id UUID NOT NULL REFERENCES courses (id) ON DELETE CASCADE,
    position  INTEGER NOT NULL CHECK (position >= 0),
    PRIMARY KEY (path_id, course_id),
    UNIQUE (path_id, position)
);

-- ── Mentorship ──────────────────────────────────────────────────────────────
CREATE TABLE mentorship_profiles (
    user_id   UUID PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    bio       TEXT,
    areas     TEXT,
    available BOOLEAN NOT NULL DEFAULT true,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Request state machine: pending → accepted | declined | cancelled (repo).
CREATE TABLE mentorship_requests (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    mentor_id  UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    mentee_id  UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    status     TEXT NOT NULL DEFAULT 'pending'
               CHECK (status IN ('pending', 'accepted', 'declined', 'cancelled')),
    message    TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (mentor_id <> mentee_id)
);

CREATE INDEX mentorship_requests_mentee_idx
    ON mentorship_requests (mentee_id, status);

CREATE TABLE mentorship_sessions (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    request_id       UUID NOT NULL REFERENCES mentorship_requests (id) ON DELETE CASCADE,
    mentor_id        UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    mentee_id        UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    scheduled_at     TIMESTAMPTZ NOT NULL,
    duration_minutes INTEGER NOT NULL CHECK (duration_minutes > 0),
    status           TEXT NOT NULL DEFAULT 'scheduled'
                     CHECK (status IN ('scheduled', 'completed', 'cancelled', 'no_show')),
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX mentorship_sessions_mentor_idx ON mentorship_sessions (mentor_id);

CREATE TABLE mentorship_feedback (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    session_id UUID NOT NULL REFERENCES mentorship_sessions (id) ON DELETE CASCADE,
    author_id  UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    rating     INTEGER NOT NULL CHECK (rating BETWEEN 1 AND 5),
    comment    TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (session_id, author_id)
);

CREATE TABLE mentorship_goals (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    request_id UUID NOT NULL REFERENCES mentorship_requests (id) ON DELETE CASCADE,
    goal       TEXT NOT NULL,
    completed  BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- ── Events ──────────────────────────────────────────────────────────────────
CREATE TABLE events (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    organizer_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    title       TEXT NOT NULL,
    slug        TEXT NOT NULL,
    description TEXT,
    starts_at   TIMESTAMPTZ NOT NULL,
    ends_at     TIMESTAMPTZ NOT NULL CHECK (ends_at > starts_at),
    capacity    INTEGER CHECK (capacity IS NULL OR capacity > 0),
    location    TEXT,
    status      TEXT NOT NULL DEFAULT 'published'
                CHECK (status IN ('draft', 'published', 'cancelled')),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at  TIMESTAMPTZ
);

CREATE UNIQUE INDEX events_slug_key ON events (slug) WHERE deleted_at IS NULL;

CREATE TRIGGER events_set_updated_at
    BEFORE UPDATE ON events
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- Idempotent registration: the PK IS the idempotency key — the same
-- (event_id, user_id) can only ever hold one row; re-registering flips a
-- cancelled row back to registered (repo).
CREATE TABLE event_registrations (
    event_id      UUID NOT NULL REFERENCES events (id) ON DELETE CASCADE,
    user_id       UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    status        TEXT NOT NULL DEFAULT 'registered'
                  CHECK (status IN ('registered', 'waitlisted', 'cancelled')),
    registered_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (event_id, user_id)
);

CREATE INDEX event_registrations_user_idx ON event_registrations (user_id, status);

CREATE TABLE event_speakers (
    event_id UUID NOT NULL REFERENCES events (id) ON DELETE CASCADE,
    user_id  UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    PRIMARY KEY (event_id, user_id)
);
