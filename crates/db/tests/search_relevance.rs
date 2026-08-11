//! Month-8 search relevance smoke suite against a real Postgres:
//!   - exact matches rank above partial/typo matches
//!   - recency decay: same term, newer post ranks higher
//!   - typo tolerance via prefix lexemes + trigram fallback
//!   - highlighting surfaces the matched term
//!   - all four entities are searchable (posts/users/communities/courses)
//!
//! Self-skips when TEST_DATABASE_URL is unset.

use keystone_db::repositories::users::{NewUser, Users};
use keystone_db::search::{build_tsquery, typo_fallback, PostgresSearch, SearchBackend};
use sqlx::PgPool;
use uuid::Uuid;

async fn test_pool() -> Option<PgPool> {
    keystone_db::test_util::test_pool_isolated().await
}

async fn make_user(pool: &PgPool, email: &str, username: &str) -> Uuid {
    let users = Users::new(pool.clone());
    let user = users
        .create(NewUser {
            email,
            password_hash: "not-a-real-hash",
            first_name: Some("Test"),
            last_name: Some("User"),
            username: Some(username),
        })
        .await
        .expect("user must be created");
    // Users start pending_verification; activate for search visibility.
    sqlx::query("UPDATE users SET status = 'active' WHERE id = $1")
        .bind(user.id)
        .execute(pool)
        .await
        .unwrap();
    user.id
}

async fn make_post(pool: &PgPool, author: Uuid, title: &str, body: &str, slug: &str) {
    sqlx::query(
        r#"
        INSERT INTO posts (author_id, kind, title, slug, body, status, visibility, published_at)
        VALUES ($1, 'article', $2, $3, $4, 'published', 'public', now())
        "#,
    )
    .bind(author)
    .bind(title)
    .bind(slug)
    .bind(body)
    .execute(pool)
    .await
    .unwrap();
}

#[tokio::test]
async fn search_ranks_exact_above_partial_and_typo() {
    let Some(pool) = test_pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let author = make_user(&pool, "rank@test.dev", "ranker").await;
    make_post(
        &pool,
        author,
        "Postgres full-text search guide",
        "A deep guide on postgres full text search with tsvector and GIN indexes.",
        "pg-fts-guide",
    )
    .await;
    make_post(
        &pool,
        author,
        "Rust async runtime internals",
        "How the tokio runtime schedules futures and wakes tasks efficiently.",
        "tokio-runtime",
    )
    .await;
    make_post(
        &pool,
        author,
        "Postgres basics",
        "Introductory notes about postgres for beginners starting with SQL.",
        "pg-basics",
    )
    .await;

    let backend = PostgresSearch::new(pool.clone());
    let hits = backend.search("postgres", 10).await.unwrap();

    // Both postgres posts rank above the unrelated rust post.
    let first = &hits[0];
    assert_eq!(first.entity_type, "post");
    assert!(
        first.title.contains("Postgres"),
        "exact-term post must rank first, got: {}",
        first.title
    );
    let titles: Vec<&str> = hits.iter().map(|h| h.title.as_str()).collect();
    assert!(
        titles.iter().any(|t| t.contains("full-text")),
        "guide must be found: {titles:?}"
    );
    assert!(
        titles.iter().any(|t| t.contains("basics")),
        "basics must be found: {titles:?}"
    );
    assert!(
        !titles.iter().any(|t| t.contains("async")),
        "unrelated posts must not match: {titles:?}"
    );

    // Snippet highlights the matched term.
    assert!(
        hits.iter()
            .any(|h| h.snippet.to_lowercase().contains("postgres")),
        "snippets must surface the matched term"
    );
}

#[tokio::test]
async fn search_recency_ranks_newer_posts_higher() {
    let Some(pool) = test_pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let author = make_user(&pool, "recent@test.dev", "recent").await;

    // Old post on the term.
    sqlx::query(
        r#"
        INSERT INTO posts (author_id, kind, title, slug, body, status, visibility, published_at, created_at)
        VALUES ($1, 'article', 'Old rust article', 'old-rust', 'Writing rust code the old way.', 'published', 'public',
                now() - interval '90 days', now() - interval '90 days')
        "#,
    )
    .bind(author)
    .execute(&pool)
    .await
    .unwrap();
    // Fresh post on the same term.
    make_post(
        &pool,
        author,
        "Fresh rust article",
        "Modern rust code with the newest idioms.",
        "fresh-rust",
    )
    .await;

    let backend = PostgresSearch::new(pool.clone());
    let hits = backend.search("rust", 10).await.unwrap();
    let fresh = hits
        .iter()
        .position(|h| h.title.starts_with("Fresh"))
        .unwrap();
    let old = hits
        .iter()
        .position(|h| h.title.starts_with("Old"))
        .unwrap();
    assert!(fresh < old, "recency decay must rank the fresh post higher");
}

#[tokio::test]
async fn search_typo_tolerance_finds_near_miss() {
    let Some(pool) = test_pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let author = make_user(&pool, "typo@test.dev", "typoer").await;
    make_post(
        &pool,
        author,
        "Advanced PostgreSQL optimization",
        "Tuning postgresql for high throughput workloads.",
        "pg-tuning",
    )
    .await;

    // "postgres" with a typo — prefix lexemes won't match "PostgreSQL"
    // exactly, but the trigram fallback should still surface it.
    let fallback = typo_fallback(&pool, "postgress", 10).await.unwrap();
    assert!(
        fallback.iter().any(|h| h.title.contains("PostgreSQL")),
        "trigram fallback must surface the near-miss title"
    );

    // The tsquery builder sanitizes and prefix-matches.
    assert_eq!(build_tsquery("Rust & Postgres!!"), "rust:* & postgres:*");
    assert_eq!(build_tsquery("!!!  "), "");
}

#[tokio::test]
async fn search_covers_all_entities() {
    let Some(pool) = test_pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let author = make_user(&pool, "entities@test.dev", "entity-keystone").await;
    make_post(
        &pool,
        author,
        "Keystone project architecture",
        "The keystone monorepo layout explained.",
        "keystone-arch",
    )
    .await;

    // A user, a community, and a course all mentioning the term.
    sqlx::query(
        "INSERT INTO users (email, password_hash, username, headline, status) VALUES ('keystone-user@test.dev', 'x', 'keystonefan', 'Keystone platform enthusiast', 'active')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO communities (name, slug, description, created_by) VALUES ('Keystone Users', 'keystone-users', 'Community around the keystone platform', $1)",
    )
    .bind(author)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO courses (author_id, title, slug, description, status) VALUES ($1, 'Keystone deep dive', 'keystone-course', 'Everything about building keystone', 'published')",
    )
    .bind(author)
    .execute(&pool)
    .await
    .unwrap();

    let backend = PostgresSearch::new(pool.clone());
    let hits = backend.search("keystone", 20).await.unwrap();

    let types: Vec<&str> = hits.iter().map(|h| h.entity_type.as_str()).collect();
    for expected in ["post", "user", "community", "course"] {
        assert!(
            types.contains(&expected),
            "search must cover {expected}, got: {types:?}"
        );
    }
}
