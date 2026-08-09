//! Database crate for keystone: connection pool, migrations, and the foundation
//! repositories will build on. All SQL goes through sqlx (compile-time checked
//! via `query!` once offline metadata is wired); dynamic SQL is banned.
#![forbid(unsafe_code)]

pub use sqlx;

use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::time::Duration;

/// Open a PostgreSQL connection pool with sane defaults.
pub async fn connect(
    database_url: &str,
    max_connections: u32,
    acquire_timeout: Duration,
) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(max_connections.max(1))
        .acquire_timeout(acquire_timeout)
        .connect(database_url)
        .await
}

/// Apply all pending migrations (versioned, checksummed, transactional).
pub async fn migrate(pool: &PgPool) -> Result<(), sqlx::migrate::MigrateError> {
    sqlx::migrate!("./migrations").run(pool).await
}

/// Readiness probe: answers only if the database is reachable.
pub async fn ping(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT 1").execute(pool).await.map(|_| ())
}

/// Typed repositories — the only place SQL lives.
pub mod repositories;

/// Integration-test helpers (feature-gated; never shipped).
#[cfg(feature = "test-util")]
pub mod test_util;
