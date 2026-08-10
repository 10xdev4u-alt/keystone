//! Moderation actions repository — append-only record of moderator
//! decisions. Actions are never edited or deleted; the audit trail is the
//! point.

use super::RepoError;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct ModerationAction {
    pub id: Uuid,
    pub moderator_id: Uuid,
    pub action: String,
    pub target_type: String,
    pub target_id: Uuid,
    pub reason: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewModerationAction<'a> {
    pub moderator_id: Uuid,
    pub action: &'a str,
    pub target_type: &'a str,
    pub target_id: Uuid,
    pub reason: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub struct Moderation {
    pool: PgPool,
}

impl Moderation {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Record one moderator decision. `action` is CHECK-constrained in the
    /// schema; `target_type` follows the reports vocabulary.
    pub async fn record(
        &self,
        action: NewModerationAction<'_>,
    ) -> Result<ModerationAction, RepoError> {
        let recorded = sqlx::query_as::<_, ModerationAction>(
            r#"
            INSERT INTO moderation_actions (moderator_id, action, target_type, target_id, reason)
            VALUES ($1, $2, $3, $4, $5)
            RETURNING id, moderator_id, action, target_type, target_id, reason, created_at
            "#,
        )
        .bind(action.moderator_id)
        .bind(action.action)
        .bind(action.target_type)
        .bind(action.target_id)
        .bind(action.reason)
        .fetch_one(&self.pool)
        .await?;
        Ok(recorded)
    }

    /// Every decision ever taken against a target, oldest first.
    pub async fn list_by_target(
        &self,
        target_type: &str,
        target_id: Uuid,
    ) -> Result<Vec<ModerationAction>, RepoError> {
        let rows = sqlx::query_as::<_, ModerationAction>(
            r#"
            SELECT id, moderator_id, action, target_type, target_id, reason, created_at
            FROM moderation_actions
            WHERE target_type = $1 AND target_id = $2
            ORDER BY created_at ASC
            "#,
        )
        .bind(target_type)
        .bind(target_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }
}
