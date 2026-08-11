//! File metadata repository — `file_records` rows + upload quotas.
//!
//! The database stores METADATA ONLY (`object_key` points into the bucket);
//! the bytes never touch Postgres. Quota enforcement sums `size_bytes` live
//! at upload time, inside a transaction, so racing uploads can't overshoot.

use super::RepoError;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct FileRecord {
    pub id: Uuid,
    pub owner_id: Uuid,
    pub bucket: String,
    pub object_key: String,
    pub original_name: String,
    pub content_type: String,
    pub size_bytes: i64,
    pub sha256: String,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub parent_id: Option<Uuid>,
    pub is_public: bool,
    pub created_at: DateTime<Utc>,
}

/// Default per-user quota (1 GiB).
pub const DEFAULT_QUOTA_BYTES: i64 = 1024 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct NewFileRecord<'a> {
    pub owner_id: Uuid,
    pub bucket: &'a str,
    pub object_key: &'a str,
    pub original_name: &'a str,
    pub content_type: &'a str,
    pub size_bytes: i64,
    pub sha256: &'a str,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub parent_id: Option<Uuid>,
    pub is_public: bool,
}

#[derive(Debug, Clone)]
pub struct Files {
    pool: PgPool,
}

impl Files {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// The user's byte cap (defaults to 1 GiB when no row exists).
    pub async fn quota(&self, user_id: Uuid) -> Result<i64, RepoError> {
        let quota = sqlx::query_scalar::<_, Option<i64>>(
            "SELECT bytes_limit FROM upload_quotas WHERE user_id = $1",
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?
        .flatten()
        .unwrap_or(DEFAULT_QUOTA_BYTES);
        Ok(quota)
    }

    /// Set a custom quota (tests + admin).
    pub async fn set_quota(&self, user_id: Uuid, bytes_limit: i64) -> Result<(), RepoError> {
        if bytes_limit <= 0 {
            return Err(RepoError::InvalidInput("quota must be positive".into()));
        }
        sqlx::query(
            r#"
            INSERT INTO upload_quotas (user_id, bytes_limit)
            VALUES ($1, $2)
            ON CONFLICT (user_id) DO UPDATE SET bytes_limit = $2, updated_at = now()
            "#,
        )
        .bind(user_id)
        .bind(bytes_limit)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Bytes currently stored by the user (live sum over file_records).
    pub async fn used_bytes(&self, user_id: Uuid) -> Result<i64, RepoError> {
        let used = sqlx::query_scalar::<_, Option<i64>>(
            "SELECT sum(size_bytes)::bigint FROM file_records WHERE owner_id = $1",
        )
        .bind(user_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(used.unwrap_or(0))
    }

    /// Reserve-and-check: register the metadata inside a transaction so the
    /// quota check and the insert are atomic against concurrent uploads.
    /// Returns the created record.
    pub async fn register(&self, record: &NewFileRecord<'_>) -> Result<FileRecord, RepoError> {
        if record.size_bytes < 0 {
            return Err(RepoError::InvalidInput("size cannot be negative".into()));
        }
        let mut tx = self.pool.begin().await?;
        // Serialize all uploads for this owner. Without this, two concurrent
        // transactions both read the pre-insert sum and overshoot together;
        // a plain transaction does NOT make check-then-insert atomic.
        // The lock is xact-scoped, so it releases at commit/rollback and
        // different users never contend (advisory locks are not schema-scoped,
        // so isolated test schemas still race correctly).
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0)::bigint)")
            .bind(record.owner_id)
            .execute(&mut *tx)
            .await?;
        let used = sqlx::query_scalar::<_, Option<i64>>(
            "SELECT sum(size_bytes)::bigint FROM file_records WHERE owner_id = $1",
        )
        .bind(record.owner_id)
        .fetch_one(&mut *tx)
        .await?;
        let limit = sqlx::query_scalar::<_, Option<i64>>(
            "SELECT bytes_limit FROM upload_quotas WHERE user_id = $1",
        )
        .bind(record.owner_id)
        .fetch_optional(&mut *tx)
        .await?
        .flatten()
        .unwrap_or(DEFAULT_QUOTA_BYTES);
        if used.unwrap_or(0) + record.size_bytes > limit {
            tx.rollback().await?;
            return Err(RepoError::InvalidInput(format!(
                "upload quota exceeded ({} + {} > {})",
                used.unwrap_or(0),
                record.size_bytes,
                limit
            )));
        }
        let row = sqlx::query_as::<_, FileRecord>(
            r#"
            INSERT INTO file_records
                (owner_id, bucket, object_key, original_name, content_type,
                 size_bytes, sha256, width, height, parent_id, is_public)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
            RETURNING id, owner_id, bucket, object_key, original_name, content_type,
                      size_bytes, sha256, width, height, parent_id, is_public, created_at
            "#,
        )
        .bind(record.owner_id)
        .bind(record.bucket)
        .bind(record.object_key)
        .bind(record.original_name)
        .bind(record.content_type)
        .bind(record.size_bytes)
        .bind(record.sha256)
        .bind(record.width)
        .bind(record.height)
        .bind(record.parent_id)
        .bind(record.is_public)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(row)
    }

    pub async fn get(&self, id: Uuid) -> Result<Option<FileRecord>, RepoError> {
        sqlx::query_as::<_, FileRecord>(
            r#"
            SELECT id, owner_id, bucket, object_key, original_name, content_type,
                   size_bytes, sha256, width, height, parent_id, is_public, created_at
            FROM file_records WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn list_for_owner(
        &self,
        owner_id: Uuid,
        before: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<Vec<FileRecord>, RepoError> {
        let limit = limit.clamp(1, 200);
        sqlx::query_as::<_, FileRecord>(
            r#"
            SELECT id, owner_id, bucket, object_key, original_name, content_type,
                   size_bytes, sha256, width, height, parent_id, is_public, created_at
            FROM file_records
            WHERE owner_id = $1 AND ($2::timestamptz IS NULL OR created_at < $2)
            ORDER BY created_at DESC
            LIMIT $3
            "#,
        )
        .bind(owner_id)
        .bind(before)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn delete(&self, id: Uuid, owner_id: Uuid) -> Result<(), RepoError> {
        sqlx::query("DELETE FROM file_records WHERE id = $1 AND owner_id = $2")
            .bind(id)
            .bind(owner_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}
