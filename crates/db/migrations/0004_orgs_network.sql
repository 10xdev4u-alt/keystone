-- 0004_orgs_network.sql — Month 5: organizations, network, careers
--
-- Theme: professional identity, connected correctly.
--   - organizations + membership roles + claim flow (email-verified domains)
--   - user_links: ONE social-graph table with a kind enum and a connection
--     state machine (pending/accepted/blocked)
--   - profiles: education / experience / skills as real tables + visibility
--   - salary benchmarks: aggregate-only rows (NO user_id — anonymity is
--     structural, not a policy), integer minor units + currency
--   - vendor listings, compliance alerts, career paths, self-assessments
--
-- Repository-enforced invariants (documented at each repo):
--   - org ownership transfer; one owner per org
--   - block semantics: a block in either direction excludes BOTH users from
--     each other's visibility and messaging (checked at read time)
--   - salary anonymization: rows only merge once ≥ MIN_SOURCE_COUNT
--   - education/experience date ranges (start <= end)
--
-- Note: `users.headline` already exists on the users row; user_profiles holds
-- only the fields that are genuinely profile-scoped (bio, location,
-- visibility) so there is no duplicated headline column to drift.

-- ── Organizations ──────────────────────────────────────────────────────────
CREATE TABLE organizations (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name        TEXT NOT NULL,
    slug        TEXT NOT NULL,
    description TEXT,
    website     TEXT,
    industry    TEXT,
    created_by  UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at  TIMESTAMPTZ
);

CREATE UNIQUE INDEX organizations_slug_key
    ON organizations (slug) WHERE deleted_at IS NULL;

CREATE TRIGGER organizations_set_updated_at
    BEFORE UPDATE ON organizations
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- Org membership with roles. One owner per org is a repository invariant
-- (checked inside the ownership-transfer transaction).
CREATE TABLE organization_members (
    organization_id UUID NOT NULL REFERENCES organizations (id) ON DELETE CASCADE,
    user_id         UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    role            TEXT NOT NULL DEFAULT 'member'
                    CHECK (role IN ('member', 'admin', 'owner')),
    joined_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (organization_id, user_id)
);

CREATE INDEX organization_members_user_idx ON organization_members (user_id);

-- Org claim flow: a user proves control of a domain, then moderators verify.
-- Only the token HASH is stored — the raw token is single-use in the email.
CREATE TABLE organization_claims (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    organization_id UUID NOT NULL REFERENCES organizations (id) ON DELETE CASCADE,
    claimant_id     UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    domain          TEXT NOT NULL,
    status          TEXT NOT NULL DEFAULT 'pending'
                    CHECK (status IN ('pending', 'approved', 'rejected')),
    token_hash      TEXT NOT NULL,
    expires_at      TIMESTAMPTZ NOT NULL,
    decided_by      UUID REFERENCES users (id) ON DELETE SET NULL,
    decided_at      TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX organization_claims_org_idx
    ON organization_claims (organization_id, status);

-- ── Social graph: one table, kind enum, state machine ─────────────────────
--   follow   → accepted immediately
--   connect  → pending until the target accepts (status flips)
--   block    → a block in either direction excludes BOTH users (read-time)
CREATE TABLE user_links (
    requester_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    target_id    UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    kind         TEXT NOT NULL CHECK (kind IN ('follow', 'connect', 'block')),
    status       TEXT NOT NULL DEFAULT 'accepted'
                 CHECK (status IN ('pending', 'accepted', 'blocked')),
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (requester_id, target_id, kind),
    CHECK (requester_id <> target_id)
);

CREATE INDEX user_links_target_idx ON user_links (target_id, kind, status);
CREATE INDEX user_links_requester_idx ON user_links (requester_id, kind, status);

-- ── Profiles ───────────────────────────────────────────────────────────────
CREATE TABLE user_profiles (
    user_id    UUID PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    bio        TEXT,
    location   TEXT,
    visibility TEXT NOT NULL DEFAULT 'public'
               CHECK (visibility IN ('public', 'connections', 'private')),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE user_education (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id     UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    school      TEXT NOT NULL,
    degree      TEXT,
    field       TEXT,
    start_year  INTEGER NOT NULL CHECK (start_year >= 1900),
    end_year    INTEGER CHECK (end_year >= start_year),
    description TEXT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX user_education_user_idx ON user_education (user_id);

CREATE TABLE user_experience (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id         UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    organization_id UUID REFERENCES organizations (id) ON DELETE SET NULL,
    title           TEXT NOT NULL,
    company         TEXT,
    start_date      DATE NOT NULL,
    end_date        DATE CHECK (end_date >= start_date),
    current         BOOLEAN NOT NULL DEFAULT false,
    description     TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX user_experience_user_idx ON user_experience (user_id);

CREATE TABLE user_skills (
    user_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    skill   TEXT NOT NULL,
    level   TEXT NOT NULL DEFAULT 'intermediate'
            CHECK (level IN ('beginner', 'intermediate', 'advanced', 'expert')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, skill)
);

-- ── Salary benchmarks (anonymity by construction) ──────────────────────────
-- Aggregates only: no user_id, no employer. Rows are buckets
-- (role × location × currency); the API merges a submission only once a
-- bucket has collected enough sources, and stores only the bounds + count.
CREATE TABLE salary_benchmarks (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    role          TEXT NOT NULL,
    location      TEXT,
    currency      TEXT NOT NULL DEFAULT 'USD'
                  CHECK (char_length(currency) = 3),
    min_amount    BIGINT NOT NULL CHECK (min_amount >= 0),
    median_amount BIGINT NOT NULL CHECK (median_amount >= min_amount),
    max_amount    BIGINT NOT NULL CHECK (max_amount >= median_amount),
    source_count  INTEGER NOT NULL DEFAULT 1 CHECK (source_count >= 1),
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX salary_benchmarks_bucket_key
    ON salary_benchmarks (role, location, currency);

CREATE TRIGGER salary_benchmarks_set_updated_at
    BEFORE UPDATE ON salary_benchmarks
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- ── Vendor listings & compliance ───────────────────────────────────────────
CREATE TABLE vendor_listings (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    organization_id UUID NOT NULL REFERENCES organizations (id) ON DELETE CASCADE,
    category        TEXT NOT NULL,
    description     TEXT,
    verified        BOOLEAN NOT NULL DEFAULT false,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at      TIMESTAMPTZ
);

CREATE INDEX vendor_listings_org_idx ON vendor_listings (organization_id);

CREATE TABLE compliance_alerts (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    organization_id UUID NOT NULL REFERENCES organizations (id) ON DELETE CASCADE,
    kind            TEXT NOT NULL,
    severity        TEXT NOT NULL DEFAULT 'info'
                    CHECK (severity IN ('info', 'warning', 'critical')),
    message         TEXT NOT NULL,
    resolved_at     TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX compliance_alerts_org_idx ON compliance_alerts (organization_id, resolved_at);

-- ── Career paths & self-assessments ────────────────────────────────────────
CREATE TABLE career_paths (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    title       TEXT NOT NULL,
    description TEXT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE career_path_steps (
    id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    career_path_id UUID NOT NULL REFERENCES career_paths (id) ON DELETE CASCADE,
    position       INTEGER NOT NULL CHECK (position >= 0),
    title          TEXT NOT NULL,
    description    TEXT,
    UNIQUE (career_path_id, position)
);

CREATE TABLE self_assessments (
    id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id        UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    career_path_id UUID NOT NULL REFERENCES career_paths (id) ON DELETE CASCADE,
    score          INTEGER NOT NULL CHECK (score BETWEEN 1 AND 5),
    notes          TEXT,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX self_assessments_user_idx ON self_assessments (user_id);
