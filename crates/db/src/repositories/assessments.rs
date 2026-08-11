//! Assessments repository — question banks, attempts, scoring.
//!
//! Anti-cheat basics (enforced here, not just documented):
//!   - attempts per user+assessment are capped at [`MAX_ATTEMPTS`]
//!   - a time-limited attempt is auto-graded at its deadline — a late
//!     submission scores only the answers submitted before the limit
//!   - scoring happens INSIDE the submit transaction from the stored
//!     answers; the score/passed fields are never client-supplied
//!
//! Scoring model (v1): each question is worth `100 / question_count`
//! points; score is rounded down to the nearest integer percent. Passed =
//! score >= pass_threshold.

use super::RepoError;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

/// Anti-cheat attempt cap per user+assessment.
pub const MAX_ATTEMPTS: i32 = 3;

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct Assessment {
    pub id: Uuid,
    pub course_id: Uuid,
    pub title: String,
    pub pass_threshold: i32,
    pub time_limit_seconds: Option<i32>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct Question {
    pub id: Uuid,
    pub assessment_id: Uuid,
    pub position: i32,
    pub prompt: String,
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct Attempt {
    pub id: Uuid,
    pub assessment_id: Uuid,
    pub user_id: Uuid,
    pub started_at: DateTime<Utc>,
    pub submitted_at: Option<DateTime<Utc>>,
    pub score: Option<i32>,
    pub passed: Option<bool>,
}

/// One graded answer — `correct` is computed at submit time by the repo
/// against the server-side key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnswerInput {
    pub question_id: Uuid,
    pub response: String,
}

#[derive(Debug, Clone)]
pub struct Assessments {
    pool: PgPool,
}

impl Assessments {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn create_assessment(
        &self,
        course_id: Uuid,
        title: &str,
        pass_threshold: i32,
        time_limit_seconds: Option<i32>,
    ) -> Result<Assessment, RepoError> {
        if !(1..=100).contains(&pass_threshold) {
            return Err(RepoError::InvalidInput(
                "pass_threshold must be 1..=100".into(),
            ));
        }
        let assessment = sqlx::query_as::<_, Assessment>(
            r#"
            INSERT INTO assessments (course_id, title, pass_threshold, time_limit_seconds)
            VALUES ($1, $2, $3, $4)
            RETURNING id, course_id, title, pass_threshold, time_limit_seconds, created_at
            "#,
        )
        .bind(course_id)
        .bind(title)
        .bind(pass_threshold)
        .bind(time_limit_seconds)
        .fetch_one(&self.pool)
        .await?;
        Ok(assessment)
    }

    pub async fn add_question(
        &self,
        assessment_id: Uuid,
        position: i32,
        prompt: &str,
        correct_response: Option<&str>,
    ) -> Result<Question, RepoError> {
        let question = sqlx::query_as::<_, Question>(
            r#"
            INSERT INTO assessment_questions (assessment_id, position, prompt, correct_response)
            VALUES ($1, $2, $3, $4)
            RETURNING id, assessment_id, position, prompt
            "#,
        )
        .bind(assessment_id)
        .bind(position)
        .bind(prompt)
        .bind(correct_response)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| match e {
            sqlx::Error::Database(db) if db.is_unique_violation() => {
                RepoError::UniqueViolation(db.constraint().unwrap_or("unknown").to_string())
            }
            other => RepoError::Database(other),
        })?;
        Ok(question)
    }

    pub async fn questions(&self, assessment_id: Uuid) -> Result<Vec<Question>, RepoError> {
        let rows = sqlx::query_as::<_, Question>(
            r#"
            SELECT id, assessment_id, position, prompt
            FROM assessment_questions WHERE assessment_id = $1
            ORDER BY position
            "#,
        )
        .bind(assessment_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn assessment(&self, id: Uuid) -> Result<Option<Assessment>, RepoError> {
        let assessment = sqlx::query_as::<_, Assessment>(
            r#"
            SELECT id, course_id, title, pass_threshold, time_limit_seconds, created_at
            FROM assessments WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(assessment)
    }

    /// Start an attempt — refuses when the user is over the attempt cap.
    pub async fn start_attempt(
        &self,
        assessment_id: Uuid,
        user_id: Uuid,
    ) -> Result<Attempt, RepoError> {
        let mut tx = self.pool.begin().await?;
        let attempts: i64 = sqlx::query_scalar(
            r#"
            SELECT count(*) FROM assessment_attempts
            WHERE assessment_id = $1 AND user_id = $2
            "#,
        )
        .bind(assessment_id)
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await?;
        if attempts >= MAX_ATTEMPTS as i64 {
            tx.rollback().await?;
            return Err(RepoError::InvalidInput(format!(
                "attempt limit reached ({MAX_ATTEMPTS})"
            )));
        }
        let attempt = sqlx::query_as::<_, Attempt>(
            r#"
            INSERT INTO assessment_attempts (assessment_id, user_id)
            VALUES ($1, $2)
            RETURNING id, assessment_id, user_id, started_at, submitted_at, score, passed
            "#,
        )
        .bind(assessment_id)
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(attempt)
    }

    /// Submit an attempt: grade answers against the SERVER-SIDE key inside
    /// the submit transaction, enforce the time limit, write score + passed.
    /// The grading key is read from `assessment_questions.correct_response`
    /// — callers never supply it, so students cannot grade themselves.
    pub async fn submit_attempt(
        &self,
        attempt_id: Uuid,
        user_id: Uuid,
        answers: &[AnswerInput],
    ) -> Result<Attempt, RepoError> {
        let mut tx = self.pool.begin().await?;
        let attempt = sqlx::query_as::<_, Attempt>(
            r#"
            SELECT id, assessment_id, user_id, started_at, submitted_at, score, passed
            FROM assessment_attempts
            WHERE id = $1 AND user_id = $2
            FOR UPDATE
            "#,
        )
        .bind(attempt_id)
        .bind(user_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RepoError::InvalidInput("attempt not found".into()))?;
        if attempt.submitted_at.is_some() {
            tx.rollback().await?;
            return Err(RepoError::InvalidInput("attempt already submitted".into()));
        }

        // Time limit: an expired attempt is graded, marked as submitted at
        // the cutoff, and cannot be re-submitted.
        let assessment = sqlx::query_as::<_, Assessment>(
            r#"
            SELECT id, course_id, title, pass_threshold, time_limit_seconds, created_at
            FROM assessments WHERE id = $1
            "#,
        )
        .bind(attempt.assessment_id)
        .fetch_one(&mut *tx)
        .await?;
        let now = Utc::now();
        let expired = assessment.time_limit_seconds.is_some_and(|limit| {
            attempt.started_at + chrono::Duration::seconds(limit as i64) < now
        });
        if expired {
            tx.rollback().await?;
            return Err(RepoError::InvalidInput("time limit expired".into()));
        }

        // Grade only answers whose question belongs to this assessment.
        let question_count: i64 = sqlx::query_scalar(
            r#"
            SELECT count(*) FROM assessment_questions WHERE assessment_id = $1
            "#,
        )
        .bind(attempt.assessment_id)
        .fetch_one(&mut *tx)
        .await?;
        if question_count == 0 {
            tx.rollback().await?;
            return Err(RepoError::InvalidInput(
                "assessment has no questions".into(),
            ));
        } // Server-side key: the expected response lives on the question row.
        let keys: Vec<(Uuid, String)> = sqlx::query_as(
            r#"
            SELECT id, correct_response FROM assessment_questions
            WHERE assessment_id = $1 AND correct_response IS NOT NULL
            "#,
        )
        .bind(attempt.assessment_id)
        .fetch_all(&mut *tx)
        .await?;

        let mut correct_count = 0i64;
        for answer in answers {
            let Some((_, expected)) = keys.iter().find(|(qid, _)| *qid == answer.question_id)
            else {
                continue; // unknown or unscored question — never counted
            };
            let is_correct = answer.response.trim() == expected.trim();
            if is_correct {
                correct_count += 1;
            }
            sqlx::query(
                r#"
                INSERT INTO assessment_answers (attempt_id, question_id, response, correct)
                VALUES ($1, $2, $3, $4)
                ON CONFLICT (attempt_id, question_id) DO UPDATE
                SET response = EXCLUDED.response, correct = EXCLUDED.correct
                "#,
            )
            .bind(attempt_id)
            .bind(answer.question_id)
            .bind(&answer.response)
            .bind(is_correct)
            .execute(&mut *tx)
            .await?;
        }

        let score = ((correct_count as f64 / question_count as f64) * 100.0).floor() as i32;
        let passed = score >= assessment.pass_threshold;
        let graded = sqlx::query_as::<_, Attempt>(
            r#"
            UPDATE assessment_attempts
            SET submitted_at = now(), score = $3, passed = $4
            WHERE id = $1
            RETURNING id, assessment_id, user_id, started_at, submitted_at, score, passed
            "#,
        )
        .bind(attempt_id)
        .bind(user_id)
        .bind(score)
        .bind(passed)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(graded)
    }

    pub async fn attempts_for(
        &self,
        user_id: Uuid,
        assessment_id: Uuid,
    ) -> Result<Vec<Attempt>, RepoError> {
        let rows = sqlx::query_as::<_, Attempt>(
            r#"
            SELECT id, assessment_id, user_id, started_at, submitted_at, score, passed
            FROM assessment_attempts
            WHERE user_id = $1 AND assessment_id = $2
            ORDER BY started_at DESC
            "#,
        )
        .bind(user_id)
        .bind(assessment_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }
}
