//! Mentorship repository — profiles, request state machine, sessions,
//! feedback, goals.
//!
//! Request lifecycle (repo-enforced, one transition per call):
//!   pending → accepted | declined (by the mentor)
//!   pending → cancelled (by the mentee)
//! A session can only be scheduled against an ACCEPTED request; feedback
//! is one per (session, author); a request has no more than one accepted
//! state — accepting twice is a no-op, not a double-booking.

use super::RepoError;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct MentorshipProfile {
    pub user_id: Uuid,
    pub bio: Option<String>,
    pub areas: Option<String>,
    pub available: bool,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct MentorshipRequest {
    pub id: Uuid,
    pub mentor_id: Uuid,
    pub mentee_id: Uuid,
    pub status: String,
    pub message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct MentorshipSession {
    pub id: Uuid,
    pub request_id: Uuid,
    pub mentor_id: Uuid,
    pub mentee_id: Uuid,
    pub scheduled_at: DateTime<Utc>,
    pub duration_minutes: i32,
    pub status: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct Feedback {
    pub id: Uuid,
    pub session_id: Uuid,
    pub author_id: Uuid,
    pub rating: i32,
    pub comment: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct Goal {
    pub id: Uuid,
    pub request_id: Uuid,
    pub goal: String,
    pub completed: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct Mentorship {
    pool: PgPool,
}

impl Mentorship {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    // ── Profiles ─────────────────────────────────────────────────────────

    pub async fn set_profile(
        &self,
        user_id: Uuid,
        bio: Option<&str>,
        areas: Option<&str>,
        available: bool,
    ) -> Result<MentorshipProfile, RepoError> {
        let profile = sqlx::query_as::<_, MentorshipProfile>(
            r#"
            INSERT INTO mentorship_profiles (user_id, bio, areas, available)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (user_id) DO UPDATE
            SET bio = EXCLUDED.bio, areas = EXCLUDED.areas,
                available = EXCLUDED.available, updated_at = now()
            RETURNING user_id, bio, areas, available, updated_at
            "#,
        )
        .bind(user_id)
        .bind(bio)
        .bind(areas)
        .bind(available)
        .fetch_one(&self.pool)
        .await?;
        Ok(profile)
    }

    pub async fn profile(&self, user_id: Uuid) -> Result<Option<MentorshipProfile>, RepoError> {
        let profile = sqlx::query_as::<_, MentorshipProfile>(
            r#"
            SELECT user_id, bio, areas, available, updated_at
            FROM mentorship_profiles WHERE user_id = $1
            "#,
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(profile)
    }

    pub async fn available_mentors(&self) -> Result<Vec<MentorshipProfile>, RepoError> {
        let rows = sqlx::query_as::<_, MentorshipProfile>(
            r#"
            SELECT user_id, bio, areas, available, updated_at
            FROM mentorship_profiles WHERE available
            ORDER BY updated_at DESC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    // ── Requests (state machine) ─────────────────────────────────────────

    pub async fn request(
        &self,
        mentor_id: Uuid,
        mentee_id: Uuid,
        message: Option<&str>,
    ) -> Result<MentorshipRequest, RepoError> {
        if mentor_id == mentee_id {
            return Err(RepoError::InvalidInput("cannot mentor yourself".into()));
        }
        let request = sqlx::query_as::<_, MentorshipRequest>(
            r#"
            INSERT INTO mentorship_requests (mentor_id, mentee_id, message)
            VALUES ($1, $2, $3)
            RETURNING id, mentor_id, mentee_id, status, message, created_at, updated_at
            "#,
        )
        .bind(mentor_id)
        .bind(mentee_id)
        .bind(message)
        .fetch_one(&self.pool)
        .await?;
        Ok(request)
    }

    /// Mentor accepts — pending → accepted (idempotent if already accepted).
    pub async fn accept(&self, request_id: Uuid, mentor_id: Uuid) -> Result<bool, RepoError> {
        let result = sqlx::query(
            r#"
            UPDATE mentorship_requests SET status = 'accepted', updated_at = now()
            WHERE id = $1 AND mentor_id = $2 AND status = 'pending'
            "#,
        )
        .bind(request_id)
        .bind(mentor_id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    /// Mentor declines — pending → declined.
    pub async fn decline(&self, request_id: Uuid, mentor_id: Uuid) -> Result<bool, RepoError> {
        let result = sqlx::query(
            r#"
            UPDATE mentorship_requests SET status = 'declined', updated_at = now()
            WHERE id = $1 AND mentor_id = $2 AND status = 'pending'
            "#,
        )
        .bind(request_id)
        .bind(mentor_id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    /// Mentee cancels — pending → cancelled.
    pub async fn cancel(&self, request_id: Uuid, mentee_id: Uuid) -> Result<bool, RepoError> {
        let result = sqlx::query(
            r#"
            UPDATE mentorship_requests SET status = 'cancelled', updated_at = now()
            WHERE id = $1 AND mentee_id = $2 AND status = 'pending'
            "#,
        )
        .bind(request_id)
        .bind(mentee_id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn request_by_id(&self, id: Uuid) -> Result<Option<MentorshipRequest>, RepoError> {
        let request = sqlx::query_as::<_, MentorshipRequest>(
            r#"
            SELECT id, mentor_id, mentee_id, status, message, created_at, updated_at
            FROM mentorship_requests WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(request)
    }

    pub async fn requests_for_mentee(
        &self,
        mentee_id: Uuid,
    ) -> Result<Vec<MentorshipRequest>, RepoError> {
        let rows = sqlx::query_as::<_, MentorshipRequest>(
            r#"
            SELECT id, mentor_id, mentee_id, status, message, created_at, updated_at
            FROM mentorship_requests WHERE mentee_id = $1
            ORDER BY created_at DESC
            "#,
        )
        .bind(mentee_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn requests_for_mentor(
        &self,
        mentor_id: Uuid,
    ) -> Result<Vec<MentorshipRequest>, RepoError> {
        let rows = sqlx::query_as::<_, MentorshipRequest>(
            r#"
            SELECT id, mentor_id, mentee_id, status, message, created_at, updated_at
            FROM mentorship_requests WHERE mentor_id = $1
            ORDER BY created_at DESC
            "#,
        )
        .bind(mentor_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    // ── Sessions, feedback, goals ────────────────────────────────────────

    /// Schedule a session — only on an ACCEPTED request.
    pub async fn schedule_session(
        &self,
        request_id: Uuid,
        scheduled_at: DateTime<Utc>,
        duration_minutes: i32,
    ) -> Result<MentorshipSession, RepoError> {
        let mut tx = self.pool.begin().await?;
        let request = sqlx::query_as::<_, MentorshipRequest>(
            r#"
            SELECT id, mentor_id, mentee_id, status, message, created_at, updated_at
            FROM mentorship_requests WHERE id = $1
            "#,
        )
        .bind(request_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RepoError::InvalidInput("request not found".into()))?;
        if request.status != "accepted" {
            tx.rollback().await?;
            return Err(RepoError::InvalidInput(
                "session requires an accepted request".into(),
            ));
        }
        let session = sqlx::query_as::<_, MentorshipSession>(
            r#"
            INSERT INTO mentorship_sessions
                   (request_id, mentor_id, mentee_id, scheduled_at, duration_minutes)
            VALUES ($1, $2, $3, $4, $5)
            RETURNING id, request_id, mentor_id, mentee_id, scheduled_at,
                      duration_minutes, status, created_at
            "#,
        )
        .bind(request_id)
        .bind(request.mentor_id)
        .bind(request.mentee_id)
        .bind(scheduled_at)
        .bind(duration_minutes)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(session)
    }

    pub async fn complete_session(
        &self,
        session_id: Uuid,
        mentor_id: Uuid,
    ) -> Result<bool, RepoError> {
        let result = sqlx::query(
            r#"
            UPDATE mentorship_sessions SET status = 'completed'
            WHERE id = $1 AND mentor_id = $2 AND status = 'scheduled'
            "#,
        )
        .bind(session_id)
        .bind(mentor_id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    /// One feedback per (session, author) — enforced by the unique index.
    pub async fn add_feedback(
        &self,
        session_id: Uuid,
        author_id: Uuid,
        rating: i32,
        comment: Option<&str>,
    ) -> Result<Feedback, RepoError> {
        if !(1..=5).contains(&rating) {
            return Err(RepoError::InvalidInput("rating must be 1..=5".into()));
        }
        let feedback = sqlx::query_as::<_, Feedback>(
            r#"
            INSERT INTO mentorship_feedback (session_id, author_id, rating, comment)
            VALUES ($1, $2, $3, $4)
            RETURNING id, session_id, author_id, rating, comment, created_at
            "#,
        )
        .bind(session_id)
        .bind(author_id)
        .bind(rating)
        .bind(comment)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| match e {
            sqlx::Error::Database(db) if db.is_unique_violation() => {
                RepoError::InvalidInput("feedback already given for this session".into())
            }
            other => RepoError::Database(other),
        })?;
        Ok(feedback)
    }

    pub async fn add_goal(&self, request_id: Uuid, goal: &str) -> Result<Goal, RepoError> {
        let goal_row = sqlx::query_as::<_, Goal>(
            r#"
            INSERT INTO mentorship_goals (request_id, goal)
            VALUES ($1, $2)
            RETURNING id, request_id, goal, completed, created_at
            "#,
        )
        .bind(request_id)
        .bind(goal)
        .fetch_one(&self.pool)
        .await?;
        Ok(goal_row)
    }

    pub async fn complete_goal(&self, goal_id: Uuid, request_id: Uuid) -> Result<bool, RepoError> {
        let result = sqlx::query(
            r#"
            UPDATE mentorship_goals SET completed = true
            WHERE id = $1 AND request_id = $2
            "#,
        )
        .bind(goal_id)
        .bind(request_id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn goals(&self, request_id: Uuid) -> Result<Vec<Goal>, RepoError> {
        let rows = sqlx::query_as::<_, Goal>(
            r#"
            SELECT id, request_id, goal, completed, created_at
            FROM mentorship_goals WHERE request_id = $1
            ORDER BY created_at
            "#,
        )
        .bind(request_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }
}
