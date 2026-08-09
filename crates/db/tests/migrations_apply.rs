//! Migration dry-run against a fresh PostgreSQL.
//!
//! Runs in CI (which provides a Postgres service and sets TEST_DATABASE_URL).
//! Locally it self-skips when no test database is configured, so `cargo test`
//! without Postgres stays green.

use keystone_db::test_util;

#[tokio::test]
async fn migrations_apply_cleanly_and_idempotently() {
    let Some(pool) = test_util::test_pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };

    test_util::migrate(&pool)
        .await
        .expect("first migrate must apply");
    test_util::migrate(&pool)
        .await
        .expect("second migrate must be a no-op (idempotent)");

    // Every recorded migration succeeded.
    let applied: i64 =
        sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations WHERE success = true")
            .fetch_one(&pool)
            .await
            .expect("migrations table must exist");
    assert!(applied >= 1, "expected at least one applied migration");

    // The core tables exist with the enforced constraints.
    let tables: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM information_schema.tables WHERE table_schema = 'public'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        tables > test_util::APP_TABLES.len() as i64, // +1 for _sqlx_migrations
        "expected all core tables, found {tables}"
    );

    // Down migration is reversible: apply down, then re-apply up.
    sqlx::migrate!("./migrations")
        .undo(&pool, 1)
        .await
        .expect("down migration must succeed");
    test_util::migrate(&pool)
        .await
        .expect("re-apply must succeed");
}
