//! Bookmark repository — saved posts per user (UNIQUE (user_id, post_id)).

use super::RepoError;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct Bookmark {
    pub id: Uuid,
    pub user_id: Uuid,
    pub post_id: Uuid,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct Bookmarks {
    pool: PgPool,
}

impl Bookmarks {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Idempotent add (duplicate bookmarks are silently collapsed).
    pub async fn add(&self, user_id: Uuid, post_id: Uuid) -> Result<(), RepoError> {
        sqlx::query(
            r#"
            INSERT INTO bookmarks (user_id, post_id)
            VALUES ($1, $2)
            ON CONFLICT (user_id, post_id) DO NOTHING
            "#,
        )
        .bind(user_id)
        .bind(post_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Remove; answers whether a bookmark existed.
    pub async fn remove(&self, user_id: Uuid, post_id: Uuid) -> Result<bool, RepoError> {
        let result = sqlx::query("DELETE FROM bookmarks WHERE user_id = $1 AND post_id = $2")
            .bind(user_id)
            .bind(post_id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn is_bookmarked(&self, user_id: Uuid, post_id: Uuid) -> Result<bool, RepoError> {
        let row: Option<Uuid> =
            sqlx::query_scalar("SELECT id FROM bookmarks WHERE user_id = $1 AND post_id = $2")
                .bind(user_id)
                .bind(post_id)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.is_some())
    }

    /// Post ids bookmarked by a user, newest bookmark first.
    pub async fn post_ids_for_user(&self, user_id: Uuid) -> Result<Vec<Uuid>, RepoError> {
        let ids = sqlx::query_scalar(
            "SELECT post_id FROM bookmarks WHERE user_id = $1 ORDER BY created_at DESC",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(ids)
    }
}
