//! Q&A repository — answers, per-answer votes, and the bounty lifecycle.
//!
//! Bounty award invariants (enforced inside one transaction):
//!   - the bounty must be `open` and not past `expires_at`
//!   - the awarded answer must belong to the bounty's question
//!   - awarding is idempotent: a second award call is a no-op (status check)

use super::RepoError;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct Answer {
    pub id: Uuid,
    pub question_id: Uuid,
    pub author_id: Uuid,
    pub body: String,
    pub accepted_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// An answer joined with its derived score and acceptance state.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct AnswerWithScore {
    pub id: Uuid,
    pub question_id: Uuid,
    pub author_id: Uuid,
    pub body: String,
    pub accepted_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub score: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct Bounty {
    pub id: Uuid,
    pub question_id: Uuid,
    pub amount: i32,
    pub status: String,
    pub expires_at: DateTime<Utc>,
    pub awarded_answer_id: Option<Uuid>,
    pub awarded_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewBounty {
    pub question_id: Uuid,
    pub amount: i32,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct Qa {
    pool: PgPool,
}

impl Qa {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    // ── Answers ────────────────────────────────────────────────────────────

    /// Answer a question. The question must be a live `question`-kind post.
    pub async fn create_answer(
        &self,
        question_id: Uuid,
        author_id: Uuid,
        body: &str,
    ) -> Result<Answer, RepoError> {
        if body.trim().is_empty() {
            return Err(RepoError::InvalidInput(
                "answer body must not be empty".into(),
            ));
        }
        let kind: Option<String> =
            sqlx::query_scalar("SELECT kind FROM posts WHERE id = $1 AND deleted_at IS NULL")
                .bind(question_id)
                .fetch_optional(&self.pool)
                .await?;
        match kind.as_deref() {
            Some("question") => {}
            Some(_) => return Err(RepoError::InvalidInput("target is not a question".into())),
            None => return Err(RepoError::InvalidInput("question not found".into())),
        }
        let answer = sqlx::query_as::<_, Answer>(
            r#"
            INSERT INTO answers (question_id, author_id, body)
            VALUES ($1, $2, $3)
            RETURNING id, question_id, author_id, body, accepted_at, created_at, updated_at
            "#,
        )
        .bind(question_id)
        .bind(author_id)
        .bind(body)
        .fetch_one(&self.pool)
        .await?;
        Ok(answer)
    }

    /// Accept an answer (clears any previously accepted one — a question has
    /// exactly one accepted answer). Ownership checked by the caller.
    pub async fn accept_answer(&self, question_id: Uuid, answer_id: Uuid) -> Result<(), RepoError> {
        let mut tx = self.pool.begin().await?;
        let belongs: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM answers WHERE id = $1 AND question_id = $2 AND deleted_at IS NULL",
        )
        .bind(answer_id)
        .bind(question_id)
        .fetch_optional(&mut *tx)
        .await?;
        if belongs.is_none() {
            return Err(RepoError::InvalidInput(
                "answer does not belong to the question".into(),
            ));
        }
        sqlx::query("UPDATE answers SET accepted_at = NULL WHERE question_id = $1 AND accepted_at IS NOT NULL")
            .bind(question_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE answers SET accepted_at = now() WHERE id = $1")
            .bind(answer_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Answers with derived scores, accepted first then newest.
    pub async fn list_answers(&self, question_id: Uuid) -> Result<Vec<AnswerWithScore>, RepoError> {
        let rows = sqlx::query_as::<_, AnswerWithScore>(
            r#"
            SELECT a.id, a.question_id, a.author_id, a.body, a.accepted_at,
                   a.created_at, a.updated_at,
                   COALESCE((SELECT sum(vote) FROM answer_votes v WHERE v.answer_id = a.id), 0) AS score
            FROM answers a
            WHERE a.question_id = $1 AND a.deleted_at IS NULL
            ORDER BY a.accepted_at DESC NULLS LAST, a.created_at ASC
            "#,
        )
        .bind(question_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// One vote per user per answer; upsert moves the direction. Voting 0 is
    /// a removal.
    pub async fn vote_answer(
        &self,
        answer_id: Uuid,
        user_id: Uuid,
        vote: i16,
    ) -> Result<(), RepoError> {
        match vote {
            0 => {
                sqlx::query("DELETE FROM answer_votes WHERE answer_id = $1 AND user_id = $2")
                    .bind(answer_id)
                    .bind(user_id)
                    .execute(&self.pool)
                    .await?;
                Ok(())
            }
            1 | -1 => {
                sqlx::query(
                    r#"
                    INSERT INTO answer_votes (answer_id, user_id, vote)
                    VALUES ($1, $2, $3)
                    ON CONFLICT (answer_id, user_id)
                    DO UPDATE SET vote = EXCLUDED.vote
                    "#,
                )
                .bind(answer_id)
                .bind(user_id)
                .bind(vote)
                .execute(&self.pool)
                .await?;
                Ok(())
            }
            _ => Err(RepoError::InvalidInput("vote must be -1, 0, or 1".into())),
        }
    }

    // ── Bounties ───────────────────────────────────────────────────────────

    /// Open a bounty on a question (one per question — UNIQUE question_id).
    pub async fn create_bounty(&self, bounty: NewBounty) -> Result<Bounty, RepoError> {
        if bounty.amount <= 0 {
            return Err(RepoError::InvalidInput("bounty must be positive".into()));
        }
        let bounty = sqlx::query_as::<_, Bounty>(
            r#"
            INSERT INTO bounties (question_id, amount, expires_at)
            VALUES ($1, $2, $3)
            RETURNING id, question_id, amount, status, expires_at,
                      awarded_answer_id, awarded_at, created_at
            "#,
        )
        .bind(bounty.question_id)
        .bind(bounty.amount)
        .bind(bounty.expires_at)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| match e {
            sqlx::Error::Database(db) if db.is_unique_violation() => {
                RepoError::UniqueViolation("question already has a bounty".into())
            }
            other => RepoError::Database(other),
        })?;
        Ok(bounty)
    }

    pub async fn bounty_for_question_by_id(&self, id: Uuid) -> Result<Option<Bounty>, RepoError> {
        let bounty = sqlx::query_as::<_, Bounty>(
            r#"
            SELECT id, question_id, amount, status, expires_at,
                   awarded_answer_id, awarded_at, created_at
            FROM bounties
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(bounty)
    }

    pub async fn bounty_for_question(
        &self,
        question_id: Uuid,
    ) -> Result<Option<Bounty>, RepoError> {
        let bounty = sqlx::query_as::<_, Bounty>(
            r#"
            SELECT id, question_id, amount, status, expires_at,
                   awarded_answer_id, awarded_at, created_at
            FROM bounties
            WHERE question_id = $1
            "#,
        )
        .bind(question_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(bounty)
    }

    /// Award the bounty to an answer. Transactional invariants:
    /// open + unexpired + answer belongs to the question. Returns the updated
    /// bounty, or `Ok(None)` when the bounty is already awarded/expired
    /// (idempotent by design).
    pub async fn award_bounty(
        &self,
        bounty_id: Uuid,
        answer_id: Uuid,
    ) -> Result<Option<Bounty>, RepoError> {
        let mut tx = self.pool.begin().await?;

        let bounty = sqlx::query_as::<_, Bounty>(
            r#"
            SELECT id, question_id, amount, status, expires_at,
                   awarded_answer_id, awarded_at, created_at
            FROM bounties
            WHERE id = $1
            FOR UPDATE
            "#,
        )
        .bind(bounty_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RepoError::InvalidInput("bounty not found".into()))?;

        if bounty.status != "open" {
            return Ok(None); // already awarded or expired — idempotent
        }
        if bounty.expires_at <= Utc::now() {
            return Ok(None);
        }

        let belongs: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM answers WHERE id = $1 AND question_id = $2 AND deleted_at IS NULL",
        )
        .bind(answer_id)
        .bind(bounty.question_id)
        .fetch_optional(&mut *tx)
        .await?;
        if belongs.is_none() {
            return Err(RepoError::InvalidInput(
                "answer does not belong to the bounty's question".into(),
            ));
        }

        let updated = sqlx::query_as::<_, Bounty>(
            r#"
            UPDATE bounties
            SET status = 'awarded', awarded_answer_id = $2, awarded_at = now()
            WHERE id = $1
            RETURNING id, question_id, amount, status, expires_at,
                      awarded_answer_id, awarded_at, created_at
            "#,
        )
        .bind(bounty_id)
        .bind(answer_id)
        .fetch_one(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(Some(updated))
    }

    /// Mark an open, past-due bounty expired (scheduler job).
    pub async fn expire_overdue(&self) -> Result<u64, RepoError> {
        let result = sqlx::query(
            r#"
            UPDATE bounties
            SET status = 'expired'
            WHERE status = 'open' AND expires_at <= now()
            "#,
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }
}
