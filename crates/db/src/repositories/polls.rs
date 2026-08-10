//! Polls repository — options + one vote per user per poll.
//!
//! The `(post_id, user_id)` PK enforces single voting at the database; a
//! "change my vote" is an upsert that moves the vote to another option in
//! one statement. Counts are always derived from `poll_votes`, never stored.

use super::RepoError;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct PollOption {
    pub id: Uuid,
    pub post_id: Uuid,
    pub text: String,
    pub position: i32,
    pub created_at: DateTime<Utc>,
}

/// One option with its live vote count.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct OptionResult {
    pub option_id: Uuid,
    pub text: String,
    pub position: i32,
    pub votes: i64,
}

#[derive(Debug, Clone)]
pub struct Polls {
    pool: PgPool,
}

impl Polls {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Append an option at the next position.
    pub async fn add_option(&self, post_id: Uuid, text: &str) -> Result<PollOption, RepoError> {
        if text.trim().is_empty() {
            return Err(RepoError::InvalidInput(
                "option text must not be empty".into(),
            ));
        }
        let option = sqlx::query_as::<_, PollOption>(
            r#"
            INSERT INTO poll_options (post_id, text, position)
            SELECT $1, $2, COALESCE(MAX(position) + 1, 0)
            FROM poll_options
            WHERE post_id = $1
            RETURNING id, post_id, text, position, created_at
            "#,
        )
        .bind(post_id)
        .bind(text)
        .fetch_one(&self.pool)
        .await?;
        Ok(option)
    }

    /// All options of a poll in display order.
    pub async fn options(&self, post_id: Uuid) -> Result<Vec<PollOption>, RepoError> {
        let rows = sqlx::query_as::<_, PollOption>(
            r#"
            SELECT id, post_id, text, position, created_at
            FROM poll_options
            WHERE post_id = $1
            ORDER BY position ASC
            "#,
        )
        .bind(post_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Cast (or switch) the user's vote. Idempotent: voting for the same
    /// option again is a no-op thanks to the upsert.
    pub async fn vote(
        &self,
        post_id: Uuid,
        user_id: Uuid,
        option_id: Uuid,
    ) -> Result<(), RepoError> {
        // The option must belong to this poll — the PK plus a join keep a
        // stray option_id from registering a vote on the wrong poll.
        let owns: Option<Uuid> =
            sqlx::query_scalar("SELECT id FROM poll_options WHERE id = $1 AND post_id = $2")
                .bind(option_id)
                .bind(post_id)
                .fetch_optional(&self.pool)
                .await?;
        if owns.is_none() {
            return Err(RepoError::InvalidInput(
                "option does not belong to this poll".into(),
            ));
        }
        sqlx::query(
            r#"
            INSERT INTO poll_votes (post_id, option_id, user_id)
            VALUES ($1, $2, $3)
            ON CONFLICT (post_id, user_id)
            DO UPDATE SET option_id = EXCLUDED.option_id
            "#,
        )
        .bind(post_id)
        .bind(option_id)
        .bind(user_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Withdraw the user's vote; answers whether one existed.
    pub async fn remove_vote(&self, post_id: Uuid, user_id: Uuid) -> Result<bool, RepoError> {
        let result = sqlx::query("DELETE FROM poll_votes WHERE post_id = $1 AND user_id = $2")
            .bind(post_id)
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() == 1)
    }

    /// The option the user voted for, if any.
    pub async fn voted_option(
        &self,
        post_id: Uuid,
        user_id: Uuid,
    ) -> Result<Option<Uuid>, RepoError> {
        let option = sqlx::query_scalar(
            "SELECT option_id FROM poll_votes WHERE post_id = $1 AND user_id = $2",
        )
        .bind(post_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(option)
    }

    /// Live tallies, derived from votes — never stored, so they cannot drift.
    pub async fn results(&self, post_id: Uuid) -> Result<Vec<OptionResult>, RepoError> {
        let rows = sqlx::query_as::<_, OptionResult>(
            r#"
            SELECT o.id AS option_id, o.text, o.position,
                   (SELECT count(*) FROM poll_votes v WHERE v.option_id = o.id) AS votes
            FROM poll_options o
            WHERE o.post_id = $1
            ORDER BY o.position ASC
            "#,
        )
        .bind(post_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Total votes cast on a poll.
    pub async fn total_votes(&self, post_id: Uuid) -> Result<i64, RepoError> {
        let total = sqlx::query_scalar("SELECT count(*) FROM poll_votes WHERE post_id = $1")
            .bind(post_id)
            .fetch_one(&self.pool)
            .await?;
        Ok(total)
    }
}
