//! Credits repository — immutable append-only ledger.
//!
//! The balance is ALWAYS `SUM(delta)` over the user's ledger rows. There is
//! no UPDATE or DELETE path in this module: earning inserts a positive
//! delta, redemption inserts a negative one.
//!
//! Double-spend defense: redemption runs at SERIALIZABLE isolation and
//! re-reads the balance inside the same transaction; on a serialization
//! conflict (two concurrent redemptions racing the same balance) the
//! transaction aborts and the caller receives an explicit conflict error —
//! the balance can never go negative.

use super::RepoError;
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct LedgerEntry {
    pub id: Uuid,
    pub user_id: Uuid,
    pub delta: i32,
    pub reason: String,
    pub reference_type: Option<String>,
    pub reference_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct Credits {
    pool: PgPool,
}

impl Credits {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn balance(&self, user_id: Uuid) -> Result<i64, RepoError> {
        let balance = sqlx::query_scalar::<_, Option<i64>>(
            "SELECT SUM(delta) FROM credit_ledger WHERE user_id = $1",
        )
        .bind(user_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(balance.unwrap_or(0))
    }

    /// Append a ledger entry. The only write path — no UPDATE/DELETE exists.
    pub async fn append(
        &self,
        user_id: Uuid,
        delta: i32,
        reason: &str,
        reference_type: Option<&str>,
        reference_id: Option<Uuid>,
    ) -> Result<LedgerEntry, RepoError> {
        if delta == 0 {
            return Err(RepoError::InvalidInput("delta must be non-zero".into()));
        }
        let entry = sqlx::query_as::<_, LedgerEntry>(
            r#"
            INSERT INTO credit_ledger (user_id, delta, reason, reference_type, reference_id)
            VALUES ($1, $2, $3, $4, $5)
            RETURNING id, user_id, delta, reason, reference_type, reference_id, created_at
            "#,
        )
        .bind(user_id)
        .bind(delta)
        .bind(reason)
        .bind(reference_type)
        .bind(reference_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(entry)
    }

    /// Redeem `amount` credits. Runs at SERIALIZABLE isolation: the balance
    /// is re-read inside the transaction, so two racing redemptions cannot
    /// both succeed against the same balance.
    pub async fn redeem(
        &self,
        user_id: Uuid,
        amount: i32,
        reason: &str,
        reference_type: Option<&str>,
        reference_id: Option<Uuid>,
    ) -> Result<LedgerEntry, RepoError> {
        if amount <= 0 {
            return Err(RepoError::InvalidInput("amount must be positive".into()));
        }
        let mut tx = self.pool.begin().await?;
        // Serializable isolation: concurrent redemptions abort instead of
        // both reading the same balance (double-spend defense).
        sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
            .execute(&mut *tx)
            .await?;
        let balance = self.balance_in(&mut tx, user_id).await?;
        if balance < amount as i64 {
            tx.rollback().await?;
            return Err(RepoError::InvalidInput("insufficient credits".into()));
        }
        let entry = self
            .append_in(
                &mut tx,
                user_id,
                -amount,
                reason,
                reference_type,
                reference_id,
            )
            .await;
        match entry {
            Ok(entry) => {
                tx.commit().await?;
                Ok(entry)
            }
            Err(e) => {
                tx.rollback().await?;
                Err(e)
            }
        }
    }

    async fn balance_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        user_id: Uuid,
    ) -> Result<i64, RepoError> {
        let balance = sqlx::query_scalar::<_, Option<i64>>(
            "SELECT SUM(delta) FROM credit_ledger WHERE user_id = $1 FOR UPDATE",
        )
        .bind(user_id)
        .fetch_one(&mut **tx)
        .await?;
        Ok(balance.unwrap_or(0))
    }

    async fn append_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        user_id: Uuid,
        delta: i32,
        reason: &str,
        reference_type: Option<&str>,
        reference_id: Option<Uuid>,
    ) -> Result<LedgerEntry, RepoError> {
        let entry = sqlx::query_as::<_, LedgerEntry>(
            r#"
            INSERT INTO credit_ledger (user_id, delta, reason, reference_type, reference_id)
            VALUES ($1, $2, $3, $4, $5)
            RETURNING id, user_id, delta, reason, reference_type, reference_id, created_at
            "#,
        )
        .bind(user_id)
        .bind(delta)
        .bind(reason)
        .bind(reference_type)
        .bind(reference_id)
        .fetch_one(&mut **tx)
        .await?;
        Ok(entry)
    }

    /// Full ledger for a user (newest first) — audit view, append-only proof.
    pub async fn ledger(&self, user_id: Uuid) -> Result<Vec<LedgerEntry>, RepoError> {
        let rows = sqlx::query_as::<_, LedgerEntry>(
            r#"
            SELECT id, user_id, delta, reason, reference_type, reference_id, created_at
            FROM credit_ledger WHERE user_id = $1
            ORDER BY created_at DESC
            "#,
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }
}
