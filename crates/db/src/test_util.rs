//! Test harness for integration tests that need a real PostgreSQL.
//!
//! Enabled by the `test-util` feature. Tests must tolerate the database being
//! absent: every helper returns `Option`, and a test that finds `None` should
//! skip itself so unit-only environments (and quick local `cargo test`
//! without a database) stay green. CI always provides Postgres and runs with
//! `--all-features`, so the migration dry-run and repository tests DO run
//! there.

use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

/// Tables owned by the application (schema version bookkeeping excluded).
/// Keep in dependency order so TRUNCATE ... CASCADE can be avoided.
pub const APP_TABLES: &[&str] = &[
    "users",
    "posts",
    "post_versions",
    "series",
    "series_posts",
    "tags",
    "post_tags",
    "comments",
    "reactions",
    "bookmarks",
    "reports",
    "moderation_actions",
    "reviews",
    "communities",
    "community_members",
    "community_posts",
    "poll_options",
    "poll_votes",
    "answers",
    "answer_votes",
    "bounties",
    "organizations",
    "organization_members",
    "organization_claims",
    "user_links",
    "user_profiles",
    "user_education",
    "user_experience",
    "user_skills",
    "salary_benchmarks",
    "vendor_listings",
    "compliance_alerts",
    "career_paths",
    "career_path_steps",
    "self_assessments",
    "courses",
    "course_modules",
    "lessons",
    "enrollments",
    "lesson_progress",
    "certificates",
    "assessments",
    "assessment_questions",
    "assessment_attempts",
    "assessment_answers",
    "credit_ledger",
    "learning_paths",
    "learning_path_courses",
    "mentorship_profiles",
    "mentorship_requests",
    "mentorship_sessions",
    "mentorship_feedback",
    "mentorship_goals",
    "events",
    "event_registrations",
    "event_speakers",
    "sessions",
    "email_verifications",
    "password_resets",
    "failed_logins",
    "audit_logs",
];

/// Connect to the test database, or `None` if `TEST_DATABASE_URL` is unset
/// (or unreachable). The pool is small — tests run serially enough.
pub async fn test_pool() -> Option<PgPool> {
    let url = std::env::var("TEST_DATABASE_URL").ok()?;
    PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .connect(&url)
        .await
        .ok()
}

/// Connect to a FRESH random schema with migrations applied — full isolation
/// from every other test running concurrently against the same database
/// (parallel-safe, unlike [`test_pool`] which shares `public`). Returns `None`
/// when the database is unavailable.
pub async fn test_pool_isolated() -> Option<PgPool> {
    let url = std::env::var("TEST_DATABASE_URL").ok()?;
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .connect(&url)
        .await
        .ok()?;

    // 32 lowercase hex chars — a safe, unquoted identifier.
    let schema = format!("test_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .ok()?;
    drop(admin);

    let owned = schema.clone();
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .after_connect(move |connection, _| {
            let schema = owned.clone();
            Box::pin(async move {
                sqlx::query(&format!("SET search_path TO {schema}"))
                    .execute(connection)
                    .await
                    .map(|_| ())
            })
        })
        .connect(&url)
        .await
        .ok()?;

    migrate(&pool).await.ok()?;
    Some(pool)
}

/// Run migrations on the test database. Caller must hold a pool from
/// [`test_pool`].
pub async fn migrate(pool: &PgPool) -> Result<(), sqlx::Error> {
    crate::migrate(pool)
        .await
        .map_err(|e| sqlx::Error::Configuration(e.to_string().into()))
}

/// Wipe application tables between tests so each test starts from a clean
/// schema. Fails loudly if the wipe fails — silent cross-test contamination
/// is worse than a red test.
pub async fn truncate_all(pool: &PgPool) -> Result<(), sqlx::Error> {
    let tables = APP_TABLES.join(", ");
    let sql = format!("TRUNCATE {tables} RESTART IDENTITY");
    sqlx::query(&sql).execute(pool).await.map(|_| ())
}

/// Full reset: migrate then truncate, in one call.
pub async fn setup(pool: &PgPool) -> Result<(), sqlx::Error> {
    migrate(pool).await?;
    truncate_all(pool).await
}
