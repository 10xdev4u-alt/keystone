//! Reaction repository — one reaction per user per post; changing kind
//! replaces it (UNIQUE (post_id, user_id) + ON CONFLICT upsert).

use super::RepoError;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct Reaction {
    pub id: Uuid,
    pub post_id: Uuid,
    pub user_id: Uuid,
    pub kind: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct Reactions {
    pool: PgPool,
}

impl Reactions {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Set (or change) the user's reaction on a post. Idempotent upsert.
    pub async fn set(
        &self,
        post_id: Uuid,
        user_id: Uuid,
        kind: &str,
    ) -> Result<Reaction, RepoError> {
        let reaction = sqlx::query_as::<_, Reaction>(
            r#"
            INSERT INTO reactions (post_id, user_id, kind)
            VALUES ($1, $2, $3)
            ON CONFLICT (post_id, user_id)
            DO UPDATE SET kind = EXCLUDED.kind
            RETURNING id, post_id, user_id, kind, created_at
            "#,
        )
        .bind(post_id)
        .bind(user_id)
        .bind(kind)
        .fetch_one(&self.pool)
        .await?;
        Ok(reaction)
    }

    /// Remove the user's reaction; answers whether anything was removed.
    pub async fn remove(&self, post_id: Uuid, user_id: Uuid) -> Result<bool, RepoError> {
        let result = sqlx::query("DELETE FROM reactions WHERE post_id = $1 AND user_id = $2")
            .bind(post_id)
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() == 1)
    }

    /// The user's current reaction on a post, if any.
    pub async fn get(&self, post_id: Uuid, user_id: Uuid) -> Result<Option<Reaction>, RepoError> {
        let reaction = sqlx::query_as::<_, Reaction>(
            r#"
            SELECT id, post_id, user_id, kind, created_at
            FROM reactions
            WHERE post_id = $1 AND user_id = $2
            "#,
        )
        .bind(post_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(reaction)
    }
}
