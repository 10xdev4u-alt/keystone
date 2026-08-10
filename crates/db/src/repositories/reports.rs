//! Reports repository — generic (entity_type, entity_id) targeting with an
//! open → under_review → resolved/dismissed lifecycle.

use super::RepoError;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct Report {
    pub id: Uuid,
    pub reporter_id: Uuid,
    pub entity_type: String,
    pub entity_id: Uuid,
    pub reason: String,
    pub detail: Option<String>,
    pub status: String,
    pub resolved_by: Option<Uuid>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub resolution_note: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewReport<'a> {
    pub reporter_id: Uuid,
    pub entity_type: &'a str,
    pub entity_id: Uuid,
    pub reason: &'a str,
    pub detail: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub struct Reports {
    pool: PgPool,
}

impl Reports {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// File a report. Reasons are free text but the schema CHECKs the entity
    /// type; a reporter may file many reports against the same target.
    pub async fn create(&self, report: NewReport<'_>) -> Result<Report, RepoError> {
        let created = sqlx::query_as::<_, Report>(
            r#"
            INSERT INTO reports (reporter_id, entity_type, entity_id, reason, detail)
            VALUES ($1, $2, $3, $4, $5)
            RETURNING id, reporter_id, entity_type, entity_id, reason, detail,
                      status, resolved_by, resolved_at, resolution_note, created_at
            "#,
        )
        .bind(report.reporter_id)
        .bind(report.entity_type)
        .bind(report.entity_id)
        .bind(report.reason)
        .bind(report.detail)
        .fetch_one(&self.pool)
        .await?;
        Ok(created)
    }

    /// Unresolved reports, oldest first, capped for the moderation queue.
    pub async fn list_open(&self, limit: i64, offset: i64) -> Result<Vec<Report>, RepoError> {
        let rows = sqlx::query_as::<_, Report>(
            r#"
            SELECT id, reporter_id, entity_type, entity_id, reason, detail,
                   status, resolved_by, resolved_at, resolution_note, created_at
            FROM reports
            WHERE status = 'open'
            ORDER BY created_at ASC
            LIMIT $1 OFFSET $2
            "#,
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Move a report through its lifecycle. `to` must be a valid terminal-or-
    /// intermediate state; the SQL CHECK keeps junk states out.
    pub async fn update_status(
        &self,
        id: Uuid,
        to: &str,
        moderator_id: Uuid,
        resolution_note: Option<&str>,
    ) -> Result<Option<Report>, RepoError> {
        let updated = sqlx::query_as::<_, Report>(
            r#"
            UPDATE reports
            SET status = $2, resolved_by = $3, resolution_note = $4,
                resolved_at = CASE WHEN $2 IN ('resolved', 'dismissed') THEN now() ELSE resolved_at END
            WHERE id = $1
            RETURNING id, reporter_id, entity_type, entity_id, reason, detail,
                      status, resolved_by, resolved_at, resolution_note, created_at
            "#,
        )
        .bind(id)
        .bind(to)
        .bind(moderator_id)
        .bind(resolution_note)
        .fetch_optional(&self.pool)
        .await?;
        Ok(updated)
    }

    pub async fn get_by_id(&self, id: Uuid) -> Result<Option<Report>, RepoError> {
        let report = sqlx::query_as::<_, Report>(
            r#"
            SELECT id, reporter_id, entity_type, entity_id, reason, detail,
                   status, resolved_by, resolved_at, resolution_note, created_at
            FROM reports
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(report)
    }
}
