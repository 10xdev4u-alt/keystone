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
