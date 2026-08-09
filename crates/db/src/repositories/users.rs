//! User repository.

use super::RepoError;
use sqlx::PgPool;
use uuid::Uuid;

/// A user row, shaped for auth flows.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub email_lower: String,
    pub password_hash: Option<String>,
    pub role: String,
    pub status: String,
    pub username: Option<String>,
    pub is_verified: bool,
    pub last_login_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone)]
pub struct NewUser<'a> {
    pub email: &'a str,
    pub password_hash: &'a str,
    pub first_name: Option<&'a str>,
    pub last_name: Option<&'a str>,
    pub username: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub struct Users {
    pool: PgPool,
}

impl Users {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Insert a new user in `pending_verification` status. Unique-email
    /// collisions surface as [`RepoError::EmailTaken`].
    pub async fn create(&self, new_user: NewUser<'_>) -> Result<User, RepoError> {
        let row = sqlx::query_as::<_, User>(
            r#"
            INSERT INTO users (email, password_hash, role, status,
                               first_name, last_name, username, is_verified)
            VALUES ($1, $2, 'user', 'pending_verification',
                    $3, $4, $5, false)
            -- email_lower is a generated column; never insert it.
            RETURNING id, email, email_lower, password_hash, role, status,
                      username, is_verified, last_login_at, created_at
            "#,
        )
        .bind(new_user.email)
        .bind(new_user.password_hash)
        .bind(new_user.first_name)
        .bind(new_user.last_name)
        .bind(new_user.username)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| match e {
            sqlx::Error::Database(db) if db.is_unique_violation() => RepoError::EmailTaken,
            other => RepoError::Database(other),
        })?;
        Ok(row)
    }

    pub async fn find_by_email(&self, email: &str) -> Result<Option<User>, RepoError> {
        let row = sqlx::query_as::<_, User>(
            r#"
            SELECT id, email, email_lower, password_hash, role, status,
                   username, is_verified, last_login_at, created_at
            FROM users
            WHERE email_lower = lower($1)
              AND deleted_at IS NULL
            "#,
        )
        .bind(email)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn find_by_id(&self, id: Uuid) -> Result<Option<User>, RepoError> {
        let row = sqlx::query_as::<_, User>(
            r#"
            SELECT id, email, email_lower, password_hash, role, status,
                   username, is_verified, last_login_at, created_at
            FROM users
            WHERE id = $1 AND deleted_at IS NULL
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// Record a successful login (touch `last_login_at`).
    pub async fn record_login(&self, id: Uuid) -> Result<(), RepoError> {
        sqlx::query("UPDATE users SET last_login_at = now() WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Record a failed login attempt (feeds the lockout policy).
    pub async fn record_failed_login(
        &self,
        user_id: Uuid,
        ip: Option<&std::net::IpAddr>,
    ) -> Result<(), RepoError> {
        sqlx::query(
            r#"
            INSERT INTO failed_logins (user_id, ip_address)
            VALUES ($1, $2)
            "#,
        )
        .bind(user_id)
        .bind(ip)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Count failures within the given window (sliding window for lockout).
    pub async fn recent_failure_count(
        &self,
        user_id: Uuid,
        since: chrono::DateTime<chrono::Utc>,
    ) -> Result<u32, RepoError> {
        let count: i64 = sqlx::query_scalar(
            r#"
            SELECT count(*) FROM failed_logins
            WHERE user_id = $1 AND attempted_at >= $2
            "#,
        )
        .bind(user_id)
        .bind(since)
        .fetch_one(&self.pool)
        .await?;
        Ok(count as u32)
    }

    pub async fn set_status(&self, id: Uuid, status: &str) -> Result<(), RepoError> {
        sqlx::query("UPDATE users SET status = $2 WHERE id = $1")
            .bind(id)
            .bind(status)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}
