//! Reviews repository — ONE table for every reviewed entity type
//! (employer, vendor, organization, course, mentor) keyed by
//! `(author_id, entity_type, entity_id)`. A user's review is an upsert:
//! editing replaces it, soft-deleting hides it, reviewing again resurrects it.

use super::RepoError;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct Review {
    pub id: Uuid,
    pub author_id: Uuid,
    pub entity_type: String,
    pub entity_id: Uuid,
    pub rating: i16,
    pub title: Option<String>,
    pub body: Option<String>,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewReview<'a> {
    pub author_id: Uuid,
    pub entity_type: &'a str,
    pub entity_id: Uuid,
    pub rating: i16,
    pub title: Option<&'a str>,
    pub body: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub struct Reviews {
    pool: PgPool,
}

impl Reviews {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Create or replace the author's review of an entity. The DB CHECK keeps
    /// the rating in 1..=5 and the entity type in the allowed set.
    pub async fn upsert(&self, review: NewReview<'_>) -> Result<Review, RepoError> {
        let saved = sqlx::query_as::<_, Review>(
            r#"
            INSERT INTO reviews (author_id, entity_type, entity_id, rating, title, body)
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT (author_id, entity_type, entity_id)
            DO UPDATE SET rating = EXCLUDED.rating, title = EXCLUDED.title,
                          body = EXCLUDED.body, status = 'published', deleted_at = NULL
            RETURNING id, author_id, entity_type, entity_id, rating, title, body,
                      status, created_at, updated_at
            "#,
        )
        .bind(review.author_id)
        .bind(review.entity_type)
        .bind(review.entity_id)
        .bind(review.rating)
        .bind(review.title)
        .bind(review.body)
        .fetch_one(&self.pool)
        .await?;
        Ok(saved)
    }

    /// Published, live reviews of an entity, newest first.
    pub async fn list_by_entity(
        &self,
        entity_type: &str,
        entity_id: Uuid,
    ) -> Result<Vec<Review>, RepoError> {
        let rows = sqlx::query_as::<_, Review>(
            r#"
            SELECT id, author_id, entity_type, entity_id, rating, title, body,
                   status, created_at, updated_at
            FROM reviews
            WHERE entity_type = $1 AND entity_id = $2
              AND status = 'published' AND deleted_at IS NULL
            ORDER BY created_at DESC
            "#,
        )
        .bind(entity_type)
        .bind(entity_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// The author's review of an entity, if it is live.
    pub async fn get(
        &self,
        author_id: Uuid,
        entity_type: &str,
        entity_id: Uuid,
    ) -> Result<Option<Review>, RepoError> {
        let review = sqlx::query_as::<_, Review>(
            r#"
            SELECT id, author_id, entity_type, entity_id, rating, title, body,
                   status, created_at, updated_at
            FROM reviews
            WHERE author_id = $1 AND entity_type = $2 AND entity_id = $3
              AND deleted_at IS NULL
            "#,
        )
        .bind(author_id)
        .bind(entity_type)
        .bind(entity_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(review)
    }

    /// Soft delete — hidden from listings, resurrected by a later upsert.
    pub async fn soft_delete(&self, id: Uuid) -> Result<Option<()>, RepoError> {
        let result = sqlx::query(
            "UPDATE reviews SET status = 'deleted', deleted_at = now() WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok((result.rows_affected() == 1).then_some(()))
    }
}
