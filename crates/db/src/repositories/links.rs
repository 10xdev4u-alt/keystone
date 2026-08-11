//! Social graph repository — one `user_links` table, kind enum, state machine.
//!
//!   follow  → accepted immediately; idempotent re-follow is a no-op
//!   connect → pending until the target accepts (or the requester cancels)
//!   block   → a block in EITHER direction excludes BOTH users from each
//!             other's visibility and messaging (checked at read time via
//!             [`UserLinks::are_blocked`])
//!
//! The PK `(requester_id, target_id, kind)` makes a connection one-way and
//! unique; a `CHECK (requester_id <> target_id)` forbids self-links.

use super::RepoError;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct UserLink {
    pub requester_id: Uuid,
    pub target_id: Uuid,
    pub kind: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct UserLinks {
    pool: PgPool,
}

impl UserLinks {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Follow a user — accepted immediately; re-following is a no-op.
    pub async fn follow(&self, requester_id: Uuid, target_id: Uuid) -> Result<(), RepoError> {
        if requester_id == target_id {
            return Err(RepoError::InvalidInput("cannot follow yourself".into()));
        }
        sqlx::query(
            r#"
            INSERT INTO user_links (requester_id, target_id, kind, status)
            VALUES ($1, $2, 'follow', 'accepted')
            ON CONFLICT (requester_id, target_id, kind)
            DO UPDATE SET status = 'accepted', updated_at = now()
            "#,
        )
        .bind(requester_id)
        .bind(target_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Unfollow / cancel a pending connect / lift a block — one delete.
    pub async fn remove(
        &self,
        requester_id: Uuid,
        target_id: Uuid,
        kind: &str,
    ) -> Result<bool, RepoError> {
        let result = sqlx::query(
            r#"
            DELETE FROM user_links
            WHERE requester_id = $1 AND target_id = $2 AND kind = $3
            "#,
        )
        .bind(requester_id)
        .bind(target_id)
        .bind(kind)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    /// Request a connection — pending until the target accepts.
    /// An existing accepted connection stays accepted (idempotent).
    pub async fn connect(&self, requester_id: Uuid, target_id: Uuid) -> Result<(), RepoError> {
        if requester_id == target_id {
            return Err(RepoError::InvalidInput("cannot connect to yourself".into()));
        }
        sqlx::query(
            r#"
            INSERT INTO user_links (requester_id, target_id, kind, status)
            VALUES ($1, $2, 'connect', 'pending')
            ON CONFLICT (requester_id, target_id, kind) DO NOTHING
            "#,
        )
        .bind(requester_id)
        .bind(target_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Accept an incoming pending connection. Answers whether a pending
    /// link existed and was accepted.
    pub async fn accept(&self, target_id: Uuid, requester_id: Uuid) -> Result<bool, RepoError> {
        let result = sqlx::query(
            r#"
            UPDATE user_links SET status = 'accepted', updated_at = now()
            WHERE requester_id = $2 AND target_id = $1 AND kind = 'connect'
              AND status = 'pending'
            "#,
        )
        .bind(target_id)
        .bind(requester_id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    /// Reject an incoming pending connection — removes the link entirely.
    pub async fn reject(&self, target_id: Uuid, requester_id: Uuid) -> Result<bool, RepoError> {
        let result = sqlx::query(
            r#"
            DELETE FROM user_links
            WHERE requester_id = $2 AND target_id = $1 AND kind = 'connect'
              AND status = 'pending'
            "#,
        )
        .bind(target_id)
        .bind(requester_id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    /// Block a user. A block is one-way stored but mutually enforced: once
    /// either side blocks, both are excluded (see [`Self::are_blocked`]).
    pub async fn block(&self, blocker_id: Uuid, blocked_id: Uuid) -> Result<(), RepoError> {
        if blocker_id == blocked_id {
            return Err(RepoError::InvalidInput("cannot block yourself".into()));
        }
        sqlx::query(
            r#"
            INSERT INTO user_links (requester_id, target_id, kind, status)
            VALUES ($1, $2, 'block', 'blocked')
            ON CONFLICT (requester_id, target_id, kind)
            DO UPDATE SET status = 'blocked', updated_at = now()
            "#,
        )
        .bind(blocker_id)
        .bind(blocked_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Whether a block separates two users in EITHER direction. This is the
    /// single read-time gate for visibility and messaging.
    pub async fn are_blocked(&self, a: Uuid, b: Uuid) -> Result<bool, RepoError> {
        let blocked = sqlx::query_scalar::<_, bool>(
            r#"
            SELECT EXISTS (
                SELECT 1 FROM user_links
                WHERE kind = 'block' AND status = 'blocked'
                  AND ((requester_id = $1 AND target_id = $2)
                    OR (requester_id = $2 AND target_id = $1))
            )
            "#,
        )
        .bind(a)
        .bind(b)
        .fetch_one(&self.pool)
        .await?;
        Ok(blocked)
    }

    /// The link state between two users, if any (latest relevant kind).
    pub async fn between(&self, a: Uuid, b: Uuid) -> Result<Option<UserLink>, RepoError> {
        let link = sqlx::query_as::<_, UserLink>(
            r#"
            SELECT requester_id, target_id, kind, status, created_at, updated_at
            FROM user_links
            WHERE requester_id = $1 AND target_id = $2
            ORDER BY updated_at DESC
            LIMIT 1
            "#,
        )
        .bind(a)
        .bind(b)
        .fetch_optional(&self.pool)
        .await?;
        Ok(link)
    }

    /// Users a person follows, newest first.
    pub async fn following(&self, user_id: Uuid) -> Result<Vec<Uuid>, RepoError> {
        let rows = sqlx::query_scalar::<_, Uuid>(
            r#"
            SELECT target_id FROM user_links
            WHERE requester_id = $1 AND kind = 'follow' AND status = 'accepted'
            ORDER BY created_at DESC
            "#,
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Accepted connections (both directions), newest first.
    pub async fn connections(&self, user_id: Uuid) -> Result<Vec<Uuid>, RepoError> {
        let rows = sqlx::query_scalar::<_, Uuid>(
            r#"
            SELECT CASE WHEN requester_id = $1 THEN target_id ELSE requester_id END
            FROM user_links
            WHERE kind = 'connect' AND status = 'accepted'
              AND (requester_id = $1 OR target_id = $1)
            ORDER BY updated_at DESC
            "#,
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }
}
