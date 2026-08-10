//! Comment repository — one table, optional parent for nesting.
//!
//! The schema cannot CHECK that a parent comment belongs to the same post, so
//! this repository enforces it before insert.

use super::RepoError;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct Comment {
    pub id: Uuid,
    pub post_id: Uuid,
    pub author_id: Uuid,
    pub parent_id: Option<Uuid>,
    pub body: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewComment<'a> {
    pub post_id: Uuid,
    pub author_id: Uuid,
    pub parent_id: Option<Uuid>,
    pub body: &'a str,
}

#[derive(Debug, Clone)]
pub struct Comments {
    pool: PgPool,
}

impl Comments {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Insert a comment. A `parent_id` must reference a VISIBLE comment on the
    /// SAME post — otherwise the tree would cross posts.
    pub async fn create(&self, new_comment: NewComment<'_>) -> Result<Comment, RepoError> {
        if let Some(parent_id) = new_comment.parent_id {
            let parent_post: Option<Uuid> = sqlx::query_scalar(
                "SELECT post_id FROM comments WHERE id = $1 AND deleted_at IS NULL",
            )
            .bind(parent_id)
            .fetch_optional(&self.pool)
            .await?;
            match parent_post {
                Some(post_id) if post_id == new_comment.post_id => {}
                Some(_) => {
                    return Err(RepoError::InvalidInput(
                        "parent comment belongs to a different post".into(),
                    ))
                }
                None => return Err(RepoError::InvalidInput("parent comment not found".into())),
            }
        }

        let comment = sqlx::query_as::<_, Comment>(
            r#"
            INSERT INTO comments (post_id, author_id, parent_id, body)
            VALUES ($1, $2, $3, $4)
            RETURNING id, post_id, author_id, parent_id, body, status, created_at, updated_at
            "#,
        )
        .bind(new_comment.post_id)
        .bind(new_comment.author_id)
        .bind(new_comment.parent_id)
        .bind(new_comment.body)
        .fetch_one(&self.pool)
        .await?;
        Ok(comment)
    }

    /// All visible comments on a post, oldest first (clients build the tree).
    pub async fn list_by_post(&self, post_id: Uuid) -> Result<Vec<Comment>, RepoError> {
        let rows = sqlx::query_as::<_, Comment>(
            r#"
            SELECT id, post_id, author_id, parent_id, body, status, created_at, updated_at
            FROM comments
            WHERE post_id = $1 AND deleted_at IS NULL AND status = 'visible'
            ORDER BY created_at ASC
            "#,
        )
        .bind(post_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn get_by_id(&self, id: Uuid) -> Result<Option<Comment>, RepoError> {
        let comment = sqlx::query_as::<_, Comment>(
            r#"
            SELECT id, post_id, author_id, parent_id, body, status, created_at, updated_at
            FROM comments
            WHERE id = $1 AND deleted_at IS NULL
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(comment)
    }

    /// Soft delete (status + deleted_at). Ownership is the caller's job.
    pub async fn soft_delete(&self, id: Uuid) -> Result<Option<()>, RepoError> {
        let result = sqlx::query(
            r#"
            UPDATE comments SET status = 'deleted', deleted_at = now()
            WHERE id = $1 AND deleted_at IS NULL
            "#,
        )
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok((result.rows_affected() == 1).then_some(()))
    }
}
