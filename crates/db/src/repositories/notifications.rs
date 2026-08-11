//! Notification repository — feed, read state, preferences, digest batching.
//!
//! The feed is id-sequenced per user (`BIGSERIAL`), so SSE gap recovery is
//! `WHERE id > $cursor ORDER BY id` — the same cursor powers the read state:
//! a single per-user `read_cursor` means "everything with id <= cursor is
//! read". Both operations are single atomic statements, so unread counts and
//! mark-read stay consistent under concurrency without row locks.
//!
//! Delivery tracking lives in `notification_deliveries` (per channel), which
//! makes digest batching idempotent: a notification is batched exactly once.

use super::RepoError;
use chrono::{DateTime, Utc};
use serde_json::Value as JsonValue;
use sqlx::PgPool;
use uuid::Uuid;

/// A single notification as stored.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Notification {
    pub id: i64,
    pub user_id: Uuid,
    pub kind: String,
    pub actor_id: Option<Uuid>,
    pub entity_type: String,
    pub entity_id: Option<Uuid>,
    pub payload: JsonValue,
    pub created_at: DateTime<Utc>,
}

/// Input for creating a notification (feed + in-app delivery row).
#[derive(Debug, Clone)]
pub struct NewNotification<'a> {
    pub user_id: Uuid,
    pub kind: &'a str,
    pub actor_id: Option<Uuid>,
    pub entity_type: &'a str,
    pub entity_id: Option<Uuid>,
    pub payload: JsonValue,
}

/// A digest batch: one user's undelivered notifications older than the cut.
#[derive(Debug, Clone)]
pub struct DigestBatch {
    pub user_id: Uuid,
    pub notifications: Vec<Notification>,
}

/// Per-user notification preferences.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct NotificationPreferences {
    pub user_id: Uuid,
    pub in_app: bool,
    pub digest: bool,
    pub email: bool,
    pub muted_kinds: Vec<String>,
    pub quiet_hours_start: Option<i16>,
    pub quiet_hours_end: Option<i16>,
    pub updated_at: DateTime<Utc>,
}

/// Editable preference fields.
#[derive(Debug, Clone, Default)]
pub struct PreferenceUpdate {
    pub in_app: Option<bool>,
    pub digest: Option<bool>,
    pub email: Option<bool>,
    pub muted_kinds: Option<Vec<String>>,
    pub quiet_hours_start: Option<i16>,
    pub quiet_hours_end: Option<i16>,
}

#[derive(Debug, Clone)]
pub struct Notifications {
    pool: PgPool,
}

impl Notifications {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Insert a notification plus its in-app delivery row atomically.
    pub async fn create(&self, n: &NewNotification<'_>) -> Result<Notification, RepoError> {
        let mut tx = self.pool.begin().await?;
        let notification = sqlx::query_as::<_, Notification>(
            r#"
            INSERT INTO notifications
                (user_id, kind, actor_id, entity_type, entity_id, payload)
            VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING id, user_id, kind, actor_id, entity_type, entity_id, payload, created_at
            "#,
        )
        .bind(n.user_id)
        .bind(n.kind)
        .bind(n.actor_id)
        .bind(n.entity_type)
        .bind(n.entity_id)
        .bind(&n.payload)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO notification_deliveries (notification_id, user_id, channel) VALUES ($1, $2, 'in_app')",
        )
        .bind(notification.id)
        .bind(n.user_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(notification)
    }

    /// Cursor-paged feed, newest first.
    pub async fn list(
        &self,
        user_id: Uuid,
        before: Option<i64>,
        limit: i64,
    ) -> Result<Vec<Notification>, RepoError> {
        let limit = limit.clamp(1, 100);
        sqlx::query_as::<_, Notification>(
            r#"
            SELECT id, user_id, kind, actor_id, entity_type, entity_id, payload, created_at
            FROM notifications
            WHERE user_id = $1 AND ($2::bigint IS NULL OR id < $2)
            ORDER BY id DESC
            LIMIT $3
            "#,
        )
        .bind(user_id)
        .bind(before)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(Into::into)
    }

    /// Everything after `after` in ascending order — SSE gap recovery.
    pub async fn list_after(
        &self,
        user_id: Uuid,
        after: i64,
    ) -> Result<Vec<Notification>, RepoError> {
        sqlx::query_as::<_, Notification>(
            r#"
            SELECT id, user_id, kind, actor_id, entity_type, entity_id, payload, created_at
            FROM notifications
            WHERE user_id = $1 AND id > $2
            ORDER BY id
            "#,
        )
        .bind(user_id)
        .bind(after)
        .fetch_all(&self.pool)
        .await
        .map_err(Into::into)
    }

    /// Unread count: notifications with id above the read cursor.
    pub async fn unread_count(&self, user_id: Uuid) -> Result<i64, RepoError> {
        let count = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT count(*)
            FROM notifications
            WHERE user_id = $1
              AND id > COALESCE(
                  (SELECT read_cursor FROM notification_states WHERE user_id = $1),
                  0)
            "#,
        )
        .bind(user_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(count)
    }

    /// Mark everything up to `up_to` read. Idempotent and atomic.
    pub async fn mark_read(&self, user_id: Uuid, up_to: i64) -> Result<(), RepoError> {
        sqlx::query(
            r#"
            INSERT INTO notification_states (user_id, read_cursor)
            VALUES ($1, $2)
            ON CONFLICT (user_id)
            DO UPDATE SET read_cursor = GREATEST(notification_states.read_cursor, EXCLUDED.read_cursor),
                          updated_at = now()
            "#,
        )
        .bind(user_id)
        .bind(up_to)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Mark everything read; returns the new cursor (the latest notification id).
    pub async fn mark_all_read(&self, user_id: Uuid) -> Result<i64, RepoError> {
        let latest = sqlx::query_scalar::<_, Option<i64>>(
            "SELECT max(id) FROM notifications WHERE user_id = $1",
        )
        .bind(user_id)
        .fetch_one(&self.pool)
        .await?;
        let cursor = latest.unwrap_or(0);
        self.mark_read(user_id, cursor).await?;
        Ok(cursor)
    }

    /// Upsert preferences (defaults: in-app only).
    pub async fn upsert_preferences(
        &self,
        user_id: Uuid,
        update: &PreferenceUpdate,
    ) -> Result<NotificationPreferences, RepoError> {
        // Insert-with-defaults first so partial updates merge onto a row.
        sqlx::query(
            r#"
            INSERT INTO notification_preferences (user_id)
            VALUES ($1)
            ON CONFLICT (user_id) DO NOTHING
            "#,
        )
        .bind(user_id)
        .execute(&self.pool)
        .await?;
        sqlx::query_as::<_, NotificationPreferences>(
            r#"
            UPDATE notification_preferences
            SET in_app            = COALESCE($2, in_app),
                digest            = COALESCE($3, digest),
                email             = COALESCE($4, email),
                muted_kinds       = COALESCE($5, muted_kinds),
                quiet_hours_start = COALESCE($6, quiet_hours_start),
                quiet_hours_end   = COALESCE($7, quiet_hours_end),
                updated_at        = now()
            WHERE user_id = $1
            RETURNING user_id, in_app, digest, email, muted_kinds,
                      quiet_hours_start, quiet_hours_end, updated_at
            "#,
        )
        .bind(user_id)
        .bind(update.in_app)
        .bind(update.digest)
        .bind(update.email)
        .bind(&update.muted_kinds)
        .bind(update.quiet_hours_start)
        .bind(update.quiet_hours_end)
        .fetch_one(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn get_preferences(
        &self,
        user_id: Uuid,
    ) -> Result<NotificationPreferences, RepoError> {
        sqlx::query_as::<_, NotificationPreferences>(
            r#"
            SELECT user_id, in_app, digest, email, muted_kinds,
                   quiet_hours_start, quiet_hours_end, updated_at
            FROM notification_preferences
            WHERE user_id = $1
            "#,
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?
        .map(Ok)
        .unwrap_or_else(|| {
            // No row yet → defaults (in-app only).
            Ok(NotificationPreferences {
                user_id,
                in_app: true,
                digest: false,
                email: false,
                muted_kinds: Vec::new(),
                quiet_hours_start: None,
                quiet_hours_end: None,
                updated_at: chrono::Utc::now(),
            })
        })
    }

    /// Whether a given kind is muted for the user.
    pub async fn is_muted(&self, user_id: Uuid, kind: &str) -> Result<bool, RepoError> {
        let muted = sqlx::query_scalar::<_, Option<Vec<String>>>(
            "SELECT muted_kinds FROM notification_preferences WHERE user_id = $1",
        )
        .bind(user_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(muted
            .map(|kinds| kinds.iter().any(|k| k == kind))
            .unwrap_or(false))
    }

    /// Collect one digest batch: undelivered (channel `digest`) notifications
    /// older than `before` for users with digest enabled, oldest first, up to
    /// `limit` total. Marks each batched notification as delivered to `digest`
    /// so the next call never re-batches it (idempotent).
    pub async fn digest_batch(
        &self,
        before: DateTime<Utc>,
        limit: usize,
    ) -> Result<Vec<DigestBatch>, RepoError> {
        let limit = limit.clamp(1, 500) as i64;
        let rows = sqlx::query_as::<_, Notification>(
            r#"
            SELECT n.id, n.user_id, n.kind, n.actor_id, n.entity_type, n.entity_id,
                   n.payload, n.created_at
            FROM notifications n
            JOIN notification_preferences p ON p.user_id = n.user_id AND p.digest
            WHERE n.created_at < $1
              AND NOT EXISTS (
                  SELECT 1 FROM notification_deliveries d
                  WHERE d.notification_id = n.id AND d.channel = 'digest')
            ORDER BY n.id
            LIMIT $2
            "#,
        )
        .bind(before)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        if rows.is_empty() {
            return Ok(Vec::new());
        }

        let mut tx = self.pool.begin().await?;
        for n in &rows {
            sqlx::query(
                "INSERT INTO notification_deliveries (notification_id, user_id, channel) VALUES ($1, $2, 'digest')",
            )
            .bind(n.id)
            .bind(n.user_id)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;

        // Group by user, preserving id order.
        let mut batches: Vec<DigestBatch> = Vec::new();
        for n in rows {
            match batches.last_mut() {
                Some(b) if b.user_id == n.user_id => b.notifications.push(n),
                _ => batches.push(DigestBatch {
                    user_id: n.user_id,
                    notifications: vec![n],
                }),
            }
        }
        Ok(batches)
    }
}
