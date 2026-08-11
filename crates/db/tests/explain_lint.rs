//! EXPLAIN query-plan lint (Month-4 acceptance criterion).
//!
//! The keyset-paginated feed must be PROVEN index-backed, not assumed:
//! on a seeded table, the planner must answer the feed queries with an
//! index scan on `posts` and no Sort node. A sequential scan on `posts`
//! means the feed index is missing or mis-shaped — fail the build.
//!
//! Self-skips when TEST_DATABASE_URL is unset.

use sqlx::PgPool;

async fn test_pool() -> Option<PgPool> {
    keystone_db::test_util::test_pool_isolated().await
}

/// Runs `EXPLAIN <sql>` and returns the full plan text.
async fn explain(pool: &PgPool, sql: &str) -> String {
    let rows: Vec<(String,)> = sqlx::query_as(&format!("EXPLAIN {sql}"))
        .fetch_all(pool)
        .await
        .expect("EXPLAIN must run");
    rows.iter()
        .map(|(line,)| line.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn assert_index_backed(plan: &str, label: &str) {
    assert!(
        !plan.contains("Seq Scan on posts"),
        "{label}: feed must not sequential-scan posts\n{plan}"
    );
    assert!(
        !plan.contains("Sort"),
        "{label}: feed must not sort — the index should serve the ORDER BY\n{plan}"
    );
    assert!(
        plan.contains("Index Scan") || plan.contains("Index Only Scan"),
        "{label}: feed must use an index scan\n{plan}"
    );
}

#[tokio::test]
async fn feed_queries_are_index_backed() {
    let Some(pool) = test_pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };

    // Seed a realistic feed so the planner makes real cost decisions
    // (an empty table would let it pick any access path).
    sqlx::query(
        r#"
        INSERT INTO users (email, password_hash, status)
        VALUES ('feed-probe@example.com', 'x', 'active')
        "#,
    )
    .execute(&pool)
    .await
    .expect("seed user");

    sqlx::query(
        r#"
        INSERT INTO posts (author_id, kind, slug, body, status, published_at)
        SELECT u.id,
               CASE WHEN i % 5 = 0 THEN 'article' ELSE 'post' END,
               'feed-probe-' || i,
               repeat('x', 40),
               'published',
               now() - (i || ' seconds')::interval
        FROM users u, generate_series(1, 2000) AS i
        WHERE u.email = 'feed-probe@example.com'
        "#,
    )
    .execute(&pool)
    .await
    .expect("seed posts");

    // Fresh statistics — the planner must see the real row count.
    sqlx::query("ANALYZE posts")
        .execute(&pool)
        .await
        .expect("ANALYZE");

    // The exact keyset feed shape (no cursor: the base query).
    let global = explain(
        &pool,
        r#"
        SELECT p.id
        FROM posts p
        JOIN post_counts pc ON pc.post_id = p.id
        WHERE p.deleted_at IS NULL AND p.status = 'published'
        ORDER BY p.created_at DESC, p.id DESC
        LIMIT 20
        "#,
    )
    .await;
    assert_index_backed(&global, "unfiltered feed");

    // Kind-filtered feed must use the (kind, status, created_at, id) index.
    let by_kind = explain(
        &pool,
        r#"
        SELECT p.id
        FROM posts p
        JOIN post_counts pc ON pc.post_id = p.id
        WHERE p.deleted_at IS NULL AND p.status = 'published' AND p.kind = 'article'
        ORDER BY p.created_at DESC, p.id DESC
        LIMIT 20
        "#,
    )
    .await;
    assert_index_backed(&by_kind, "kind-filtered feed");

    // Author-filtered feed rides posts_author_idx.
    let by_author = explain(
        &pool,
        r#"
        SELECT p.id
        FROM posts p
        JOIN post_counts pc ON pc.post_id = p.id
        WHERE p.deleted_at IS NULL AND p.status = 'published'
          AND p.author_id = (SELECT id FROM users WHERE email = 'feed-probe@example.com')
        ORDER BY p.created_at DESC, p.id DESC
        LIMIT 20
        "#,
    )
    .await;
    assert_index_backed(&by_author, "author-filtered feed");
}
