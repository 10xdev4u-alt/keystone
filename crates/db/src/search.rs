//! Unified search over posts/users/communities/courses.
//!
//! [`SearchBackend`] is the seam an external engine (Elasticsearch) can
//! replace behind without touching handlers. The Postgres implementation uses
//! FTS (`tsvector` + GIN) with:
//!   - prefix-matched lexemes for as-you-type behavior,
//!   - `ts_rank` weighted (title A / body B),
//!   - a recency decay so fresh results float,
//!   - `pg_trgm` similarity as a typo-tolerance floor (ranked below exact
//!     lexeme matches),
//!   - `ts_headline` snippets with the matched terms highlighted.

use async_trait::async_trait;
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::repositories::RepoError;

/// One ranked hit across the searchable entities.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct SearchHit {
    pub entity_type: String,
    pub entity_id: Uuid,
    pub title: String,
    pub snippet: String,
    pub score: f64,
}

/// Search contract — a real engine can implement this later.
#[async_trait]
pub trait SearchBackend: Send + Sync {
    async fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchHit>, RepoError>;
}

/// Postgres FTS backend.
pub struct PostgresSearch {
    pool: PgPool,
}

impl PostgresSearch {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// Build a forgiving `to_tsquery` string: every word is prefix-matched and
/// lower-cased; non-alphanumerics stripped so `to_tsquery` never raises on
/// punctuation-heavy input. Empty for a query with no usable words.
pub fn build_tsquery(query: &str) -> String {
    let words: Vec<String> = query
        .split_whitespace()
        .filter_map(|w| {
            let clean: String = w
                .chars()
                .filter(|c| c.is_alphanumeric())
                .collect::<String>()
                .to_lowercase();
            (!clean.is_empty()).then_some(clean)
        })
        .collect();
    if words.is_empty() {
        return String::new();
    }
    words
        .iter()
        .map(|w| format!("{w}:*"))
        .collect::<Vec<_>>()
        .join(" & ")
}

#[async_trait]
impl SearchBackend for PostgresSearch {
    async fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchHit>, RepoError> {
        let tsquery = build_tsquery(query);
        if tsquery.is_empty() {
            return Ok(Vec::new());
        }
        let limit = limit.clamp(1, 50) as i32;
        sqlx::query_as::<_, SearchHit>(
            r#"
            WITH q AS (SELECT to_tsquery('english', $1) AS query)
            SELECT entity_type, entity_id, title, snippet, score FROM (
                -- Posts: published, public, not deleted.
                SELECT 'post'::text AS entity_type,
                       p.id AS entity_id,
                       coalesce(p.title, p.slug) AS title,
                       ts_headline('english', p.body, q.query) AS snippet,
                       (ts_rank(p.search_doc, q.query)
                        / (1.0 + EXTRACT(EPOCH FROM (now() - coalesce(p.published_at, p.created_at))) / 86400.0)
                        ) AS score
                FROM posts p, q
                WHERE p.search_doc @@ q.query
                  AND p.status = 'published' AND p.visibility = 'public'
                  AND p.deleted_at IS NULL
                UNION ALL
                -- Users: active only.
                SELECT 'user'::text,
                       u.id,
                       coalesce(u.username, concat_ws(' ', u.first_name, u.last_name)) AS title,
                       ts_headline('english', coalesce(u.headline, u.username, ''), q.query) AS snippet,
                       (ts_rank(u.search_doc, q.query)
                        / (1.0 + EXTRACT(EPOCH FROM (now() - u.created_at)) / 86400.0)
                        ) AS score
                FROM users u, q
                WHERE u.search_doc @@ q.query
                  AND u.status = 'active' AND u.deleted_at IS NULL
                UNION ALL
                -- Communities.
                SELECT 'community'::text,
                       c.id,
                       c.name AS title,
                       ts_headline('english', coalesce(c.description, ''), q.query) AS snippet,
                       (ts_rank(c.search_doc, q.query)
                        / (1.0 + EXTRACT(EPOCH FROM (now() - c.created_at)) / 86400.0)
                        ) AS score
                FROM communities c, q
                WHERE c.search_doc @@ q.query AND c.deleted_at IS NULL
                UNION ALL
                -- Courses: published only.
                SELECT 'course'::text,
                       cr.id,
                       cr.title,
                       ts_headline('english', coalesce(cr.description, ''), q.query) AS snippet,
                       (ts_rank(cr.search_doc, q.query)
                        / (1.0 + EXTRACT(EPOCH FROM (now() - cr.created_at)) / 86400.0)
                        ) AS score
                FROM courses cr, q
                WHERE cr.search_doc @@ q.query
                  AND cr.status = 'published' AND cr.deleted_at IS NULL
            ) hits
            ORDER BY score DESC
            LIMIT $2
            "#,
        )
        .bind(&tsquery)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(Into::into)
    }
}

/// Typo tolerance: pg_trgm `word_similarity` — the query string is compared
/// against the BEST matching word inside the title, so a one-character typo
/// against any title word still surfaces the document.
pub async fn typo_fallback(
    pool: &PgPool,
    query: &str,
    limit: usize,
) -> Result<Vec<SearchHit>, RepoError> {
    let limit = limit.clamp(1, 50) as i32;
    sqlx::query_as::<_, SearchHit>(
        r#"
        SELECT entity_type, entity_id, title, snippet, score FROM (
            SELECT 'post'::text AS entity_type,
                   p.id AS entity_id,
                   coalesce(p.title, p.slug) AS title,
                   left(p.body, 300) AS snippet,
                   word_similarity($1, coalesce(p.title, ''))::float8 AS score
            FROM posts p
            WHERE word_similarity($1, coalesce(p.title, '')) > 0.4
              AND p.status = 'published' AND p.visibility = 'public'
              AND p.deleted_at IS NULL
        ) hits
        ORDER BY score DESC
        LIMIT $2
        "#,
    )
    .bind(query)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}
