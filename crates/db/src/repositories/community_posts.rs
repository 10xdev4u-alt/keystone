//! Community posts repository — discussions inside a community, with a
//! moderator-controlled pinned slot and keyset-ready feed ordering.

use super::RepoError;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct CommunityPost {
    pub community_id: Uuid,
    pub post_id: Uuid,
    pub pinned: bool,
    pub added_by: Uuid,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CommunityPosts {
    pool: PgPool,
}

impl CommunityPosts {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Add an existing post to a community. Idempotent per (community, post).
    pub async fn add(
        &self,
        community_id: Uuid,
        post_id: Uuid,
        added_by: Uuid,
    ) -> Result<(), RepoError> {
        sqlx::query(
            r#"
            INSERT INTO community_posts (community_id, post_id, added_by)
            VALUES ($1, $2, $3)
            ON CONFLICT (community_id, post_id) DO NOTHING
            "#,
        )
        .bind(community_id)
        .bind(post_id)
        .bind(added_by)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn remove(&self, community_id: Uuid, post_id: Uuid) -> Result<bool, RepoError> {
        let result =
            sqlx::query("DELETE FROM community_posts WHERE community_id = $1 AND post_id = $2")
                .bind(community_id)
                .bind(post_id)
                .execute(&self.pool)
                .await?;
        Ok(result.rows_affected() == 1)
    }

    /// Pin or unpin. Only moderators call this (checked at the API layer).
    pub async fn set_pinned(
        &self,
        community_id: Uuid,
        post_id: Uuid,
        pinned: bool,
    ) -> Result<bool, RepoError> {
        let result = sqlx::query(
            "UPDATE community_posts SET pinned = $3 WHERE community_id = $1 AND post_id = $2",
        )
        .bind(community_id)
        .bind(post_id)
        .bind(pinned)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    /// Feed: pinned first, then newest, only live published posts.
    pub async fn list(
        &self,
        community_id: Uuid,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<CommunityPost>, RepoError> {
        let rows = sqlx::query_as::<_, CommunityPost>(
            r#"
            SELECT cp.community_id, cp.post_id, cp.pinned, cp.added_by, cp.created_at
            FROM community_posts cp
            JOIN posts p ON p.id = cp.post_id
            WHERE cp.community_id = $1 AND p.deleted_at IS NULL AND p.status = 'published'
            ORDER BY cp.pinned DESC, cp.created_at DESC, cp.post_id DESC
            LIMIT $2 OFFSET $3
            "#,
        )
        .bind(community_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }
}
