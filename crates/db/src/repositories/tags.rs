//! Tag repository — normalized names (case-insensitive unique), many-to-many
//! attachment via post_tags.

use super::RepoError;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct Tag {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct Tags {
    pool: PgPool,
}

impl Tags {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Find-or-create by name, case-insensitively. Concurrency-safe: a racing
    /// insert collapses into the existing row.
    pub async fn ensure(&self, name: &str, slug: &str) -> Result<Tag, RepoError> {
        let existing = sqlx::query_as::<_, Tag>(
            "SELECT id, name, slug, created_at FROM tags WHERE lower(name) = lower($1)",
        )
        .bind(name)
        .fetch_optional(&self.pool)
        .await?;
        if let Some(tag) = existing {
            return Ok(tag);
        }
        let created = sqlx::query_as::<_, Tag>(
            r#"
            INSERT INTO tags (name, slug)
            VALUES ($1, $2)
            ON CONFLICT (name_lower) DO NOTHING
            RETURNING id, name, slug, created_at
            "#,
        )
        .bind(name)
        .bind(slug)
        .fetch_optional(&self.pool)
        .await?;
        match created {
            Some(tag) => Ok(tag),
            None => {
                // Lost the race — the other writer won.
                let tag = sqlx::query_as::<_, Tag>(
                    "SELECT id, name, slug, created_at FROM tags WHERE lower(name) = lower($1)",
                )
                .bind(name)
                .fetch_one(&self.pool)
                .await?;
                Ok(tag)
            }
        }
    }

    pub async fn attach(&self, post_id: Uuid, tag_id: Uuid) -> Result<(), RepoError> {
        sqlx::query(
            "INSERT INTO post_tags (post_id, tag_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        )
        .bind(post_id)
        .bind(tag_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Tags on a post, alphabetized.
    pub async fn for_post(&self, post_id: Uuid) -> Result<Vec<Tag>, RepoError> {
        let rows = sqlx::query_as::<_, Tag>(
            r#"
            SELECT t.id, t.name, t.slug, t.created_at
            FROM post_tags pt
            JOIN tags t ON t.id = pt.tag_id
            WHERE pt.post_id = $1
            ORDER BY t.name
            "#,
        )
        .bind(post_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn remove(&self, post_id: Uuid, tag_id: Uuid) -> Result<bool, RepoError> {
        let result = sqlx::query("DELETE FROM post_tags WHERE post_id = $1 AND tag_id = $2")
            .bind(post_id)
            .bind(tag_id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() == 1)
    }
}
