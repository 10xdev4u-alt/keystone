//! Events repository — create/register/cancel with **idempotent
//! registrations**, waitlists, speakers, and capacity limits.
//!
//! The registration PK is `(event_id, user_id)` — the idempotency key is
//! structural, so concurrent duplicate registrations collapse to one row
//! instead of racing. Capacity is enforced inside a transaction: the repo
//! counts registered rows while holding the event row lock, so two racers
//! cannot both claim the last seat; overflow goes to the waitlist in
//! registration order. Cancelling a registered seat promotes the first
//! waitlisted registrant atomically.

use super::RepoError;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct Event {
    pub id: Uuid,
    pub organizer_id: Uuid,
    pub title: String,
    pub slug: String,
    pub description: Option<String>,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub capacity: Option<i32>,
    pub location: Option<String>,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct Registration {
    pub event_id: Uuid,
    pub user_id: Uuid,
    pub status: String,
    pub registered_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewEvent<'a> {
    pub organizer_id: Uuid,
    pub title: &'a str,
    pub slug: &'a str,
    pub description: Option<&'a str>,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub capacity: Option<i32>,
    pub location: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub struct Events {
    pool: PgPool,
}

impl Events {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, new_event: NewEvent<'_>) -> Result<Event, RepoError> {
        if new_event.ends_at <= new_event.starts_at {
            return Err(RepoError::InvalidInput(
                "ends_at must be after starts_at".into(),
            ));
        }
        let event = sqlx::query_as::<_, Event>(
            r#"
            INSERT INTO events
                   (organizer_id, title, slug, description, starts_at, ends_at,
                    capacity, location)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            RETURNING id, organizer_id, title, slug, description, starts_at, ends_at,
                      capacity, location, status, created_at, updated_at
            "#,
        )
        .bind(new_event.organizer_id)
        .bind(new_event.title)
        .bind(new_event.slug)
        .bind(new_event.description)
        .bind(new_event.starts_at)
        .bind(new_event.ends_at)
        .bind(new_event.capacity)
        .bind(new_event.location)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| match e {
            sqlx::Error::Database(db) if db.is_unique_violation() => {
                RepoError::UniqueViolation(db.constraint().unwrap_or("unknown").to_string())
            }
            other => RepoError::Database(other),
        })?;
        Ok(event)
    }

    pub async fn get_by_slug(&self, slug: &str) -> Result<Option<Event>, RepoError> {
        let event = sqlx::query_as::<_, Event>(
            r#"
            SELECT id, organizer_id, title, slug, description, starts_at, ends_at,
                   capacity, location, status, created_at, updated_at
            FROM events WHERE slug = $1 AND deleted_at IS NULL
            "#,
        )
        .bind(slug)
        .fetch_optional(&self.pool)
        .await?;
        Ok(event)
    }

    pub async fn get_by_id(&self, id: Uuid) -> Result<Option<Event>, RepoError> {
        let event = sqlx::query_as::<_, Event>(
            r#"
            SELECT id, organizer_id, title, slug, description, starts_at, ends_at,
                   capacity, location, status, created_at, updated_at
            FROM events WHERE id = $1 AND deleted_at IS NULL
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(event)
    }

    pub async fn published_events(&self, limit: i64, offset: i64) -> Result<Vec<Event>, RepoError> {
        let rows = sqlx::query_as::<_, Event>(
            r#"
            SELECT id, organizer_id, title, slug, description, starts_at, ends_at,
                   capacity, location, status, created_at, updated_at
            FROM events
            WHERE status = 'published' AND deleted_at IS NULL AND ends_at > now()
            ORDER BY starts_at
            LIMIT $1 OFFSET $2
            "#,
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Register (or re-register after cancel). Idempotent by PK; capacity
    /// is enforced under the event-row lock; overflow lands on the waitlist
    /// in registration order. Answers the resulting registration status.
    pub async fn register(&self, event_id: Uuid, user_id: Uuid) -> Result<String, RepoError> {
        let mut tx = self.pool.begin().await?;
        let event = sqlx::query_as::<_, Event>(
            r#"
            SELECT id, organizer_id, title, slug, description, starts_at, ends_at,
                   capacity, location, status, created_at, updated_at
            FROM events WHERE id = $1 AND deleted_at IS NULL
            FOR UPDATE
            "#,
        )
        .bind(event_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RepoError::InvalidInput("event not found".into()))?;
        if event.status == "cancelled" {
            tx.rollback().await?;
            return Err(RepoError::InvalidInput("event is cancelled".into()));
        }

        // Idempotency: an existing row (any status) is the single source of
        // truth — re-registering after cancel flips back to registered if a
        // seat exists, otherwise stays waitlisted.
        let existing: Option<String> = sqlx::query_scalar(
            r#"
            SELECT status FROM event_registrations
            WHERE event_id = $1 AND user_id = $2
            "#,
        )
        .bind(event_id)
        .bind(user_id)
        .fetch_optional(&mut *tx)
        .await?;

        if let Some(status) = existing {
            if status == "registered" {
                tx.rollback().await?;
                return Ok("registered".into()); // already in — no-op
            }
            // cancelled → re-register: check capacity again.
            if status == "waitlisted" {
                tx.rollback().await?;
                return Ok("waitlisted".into()); // still no seat until promoted
            }
        }

        let registered_count: i64 = sqlx::query_scalar(
            r#"
            SELECT count(*) FROM event_registrations
            WHERE event_id = $1 AND status = 'registered'
            "#,
        )
        .bind(event_id)
        .fetch_one(&mut *tx)
        .await?;

        let fits = event
            .capacity
            .is_none_or(|cap| registered_count < cap as i64);
        let status = if fits { "registered" } else { "waitlisted" };

        sqlx::query(
            r#"
            INSERT INTO event_registrations (event_id, user_id, status, registered_at)
            VALUES ($1, $2, $3, now())
            ON CONFLICT (event_id, user_id) DO UPDATE
            SET status = EXCLUDED.status, registered_at = now()
            "#,
        )
        .bind(event_id)
        .bind(user_id)
        .bind(status)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(status.into())
    }

    /// Cancel a registration. If a seat frees, the first waitlisted
    /// registrant is promoted atomically in the same transaction.
    pub async fn cancel_registration(
        &self,
        event_id: Uuid,
        user_id: Uuid,
    ) -> Result<bool, RepoError> {
        let mut tx = self.pool.begin().await?;
        let result = sqlx::query(
            r#"
            UPDATE event_registrations SET status = 'cancelled'
            WHERE event_id = $1 AND user_id = $2 AND status = 'registered'
            "#,
        )
        .bind(event_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
        let cancelled = result.rows_affected() == 1;
        if cancelled {
            // Promote the oldest waitlisted registrant (subquery: UPDATE
            // cannot ORDER BY directly in Postgres).
            let promoted: Option<Uuid> = sqlx::query_scalar(
                r#"
                UPDATE event_registrations SET status = 'registered'
                WHERE event_id = $1 AND status = 'waitlisted'
                  AND user_id = (
                      SELECT user_id FROM event_registrations
                      WHERE event_id = $1 AND status = 'waitlisted'
                      ORDER BY registered_at
                      LIMIT 1
                  )
                RETURNING user_id
                "#,
            )
            .bind(event_id)
            .fetch_optional(&mut *tx)
            .await?;
            if let Some(promoted_user) = promoted {
                tracing::info!(event_id = %event_id, user_id = %promoted_user, "waitlist promoted");
            }
        }
        tx.commit().await?;
        Ok(cancelled)
    }

    pub async fn registration_status(
        &self,
        event_id: Uuid,
        user_id: Uuid,
    ) -> Result<Option<String>, RepoError> {
        let status = sqlx::query_scalar::<_, String>(
            r#"
            SELECT status FROM event_registrations
            WHERE event_id = $1 AND user_id = $2
            "#,
        )
        .bind(event_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(status)
    }

    pub async fn registrations(&self, event_id: Uuid) -> Result<Vec<Registration>, RepoError> {
        let rows = sqlx::query_as::<_, Registration>(
            r#"
            SELECT event_id, user_id, status, registered_at
            FROM event_registrations WHERE event_id = $1
            ORDER BY registered_at
            "#,
        )
        .bind(event_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn add_speaker(&self, event_id: Uuid, user_id: Uuid) -> Result<(), RepoError> {
        sqlx::query(
            r#"
            INSERT INTO event_speakers (event_id, user_id)
            VALUES ($1, $2)
            ON CONFLICT (event_id, user_id) DO NOTHING
            "#,
        )
        .bind(event_id)
        .bind(user_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn speakers(&self, event_id: Uuid) -> Result<Vec<Uuid>, RepoError> {
        let rows =
            sqlx::query_scalar::<_, Uuid>("SELECT user_id FROM event_speakers WHERE event_id = $1")
                .bind(event_id)
                .fetch_all(&self.pool)
                .await?;
        Ok(rows)
    }
}
