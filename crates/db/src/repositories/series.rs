//! Series repository — ordered collections of posts by one author.
//!
//! Positions are appended (max + 1) in a single statement; the
//! `UNIQUE (series_id, position)` constraint makes racing appends fail safe
//! rather than corrupt the order.

use super::RepoError;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct Series {
    pub id: Uuid,
    pub author_id: Uuid,
    pub title: String,
    pub slug: String,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A post inside a series, with its position.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct SeriesPost {
    pub post_id: Uuid,
    pub position: i32,
    pub added_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewSeries<'a> {
    pub author_id: Uuid,
    pub title: &'a str,
    pub slug: &'a str,
    pub description: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub struct SeriesRepo {
    pool: PgPool,
}

impl SeriesRepo {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Create a series. Slug collisions surface as
    /// [`RepoError::UniqueViolation`] (`series_slug_key`) — callers retry with
    /// a suffixed slug.
    pub async fn create(&self, new_series: NewSeries<'_>) -> Result<Series, RepoError> {
        let series = sqlx::query_as::<_, Series>(
            r#"
            INSERT INTO series (author_id, title, slug, description)
            VALUES ($1, $2, $3, $4)
            RETURNING id, author_id, title, slug, description, created_at, updated_at
            "#,
        )
        .bind(new_series.author_id)
        .bind(new_series.title)
        .bind(new_series.slug)
        .bind(new_series.description)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| match e {
            sqlx::Error::Database(db) if db.is_unique_violation() => {
                RepoError::UniqueViolation(db.constraint().unwrap_or("unknown").to_string())
            }
            other => RepoError::Database(other),
        })?;
        Ok(series)
    }

    /// Fetch a live series by slug.
    pub async fn get_by_slug(&self, slug: &str) -> Result<Option<Series>, RepoError> {
        let series = sqlx::query_as::<_, Series>(
            r#"
            SELECT id, author_id, title, slug, description, created_at, updated_at
            FROM series
            WHERE slug = $1 AND deleted_at IS NULL
            "#,
        )
        .bind(slug)
        .fetch_optional(&self.pool)
        .await?;
        Ok(series)
    }

    /// Append a post to the end of the series. Idempotent per (series, post).
    pub async fn add_post(&self, series_id: Uuid, post_id: Uuid) -> Result<(), RepoError> {
        sqlx::query(
            r#"
            INSERT INTO series_posts (series_id, post_id, position)
            SELECT $1, $2, COALESCE(MAX(position) + 1, 0)
            FROM series_posts
            WHERE series_id = $1
            ON CONFLICT (series_id, post_id) DO NOTHING
            "#,
        )
        .bind(series_id)
        .bind(post_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Posts in a series, in reading order, only live posts.
    pub async fn list_posts(&self, series_id: Uuid) -> Result<Vec<SeriesPost>, RepoError> {
        let rows = sqlx::query_as::<_, SeriesPost>(
            r#"
            SELECT sp.post_id, sp.position, sp.added_at
            FROM series_posts sp
            JOIN posts p ON p.id = sp.post_id
            WHERE sp.series_id = $1 AND p.deleted_at IS NULL AND p.status = 'published'
            ORDER BY sp.position ASC
            "#,
        )
        .bind(series_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Remove a post from a series; answers whether it was present.
    pub async fn remove_post(&self, series_id: Uuid, post_id: Uuid) -> Result<bool, RepoError> {
        let result = sqlx::query("DELETE FROM series_posts WHERE series_id = $1 AND post_id = $2")
            .bind(series_id)
            .bind(post_id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() == 1)
    }

    /// Soft delete; hidden from all reads.
    pub async fn soft_delete(&self, id: Uuid) -> Result<Option<()>, RepoError> {
        let result = sqlx::query(
            "UPDATE series SET deleted_at = now() WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok((result.rows_affected() == 1).then_some(()))
    }
}
