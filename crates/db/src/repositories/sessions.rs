//! Session repository — refresh tokens, rotation chains, revocation.

use super::RepoError;
use sqlx::PgPool;
use uuid::Uuid;

/// A session row. `refresh_token_hash` is SHA-256 of the opaque token.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct Session {
    pub id: Uuid,
    pub user_id: Uuid,
    pub refresh_token_hash: String,
    pub user_agent: Option<String>,
    pub ip_address: Option<sqlx::types::ipnetwork::IpNetwork>,
    pub expires_at: chrono::DateTime<chrono::Utc>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub revoked_at: Option<chrono::DateTime<chrono::Utc>>,
    pub replaced_by_session_id: Option<Uuid>,
}

#[derive(Debug, Clone)]
pub struct NewSession<'a> {
    pub user_id: Uuid,
    pub refresh_token_hash: &'a str,
    pub expires_at: chrono::DateTime<chrono::Utc>,
    pub user_agent: Option<&'a str>,
    pub ip_address: Option<std::net::IpAddr>,
}

#[derive(Debug, Clone)]
pub struct Sessions {
    pool: PgPool,
}

impl Sessions {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, new_session: NewSession<'_>) -> Result<Session, RepoError> {
        let row = sqlx::query_as::<_, Session>(
            r#"
            INSERT INTO sessions (user_id, refresh_token_hash, user_agent,
                                  ip_address, expires_at)
            VALUES ($1, $2, $3, $4, $5)
            RETURNING id, user_id, refresh_token_hash, user_agent, ip_address,
                      expires_at, created_at, revoked_at, replaced_by_session_id
            "#,
        )
        .bind(new_session.user_id)
        .bind(new_session.refresh_token_hash)
        .bind(new_session.user_agent)
        .bind(new_session.ip_address)
        .bind(new_session.expires_at)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    /// Find a live (not revoked, not expired) session by its token hash.
    pub async fn find_live_by_hash(&self, hash: &str) -> Result<Option<Session>, RepoError> {
        let row = sqlx::query_as::<_, Session>(
            r#"
            SELECT id, user_id, refresh_token_hash, user_agent, ip_address,
                   expires_at, created_at, revoked_at, replaced_by_session_id
            FROM sessions
            WHERE refresh_token_hash = $1
              AND revoked_at IS NULL
              AND expires_at > now()
            "#,
        )
        .bind(hash)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// All live sessions for a user (session list / revoke-all).
    pub async fn live_for_user(&self, user_id: Uuid) -> Result<Vec<Session>, RepoError> {
        let rows = sqlx::query_as::<_, Session>(
            r#"
            SELECT id, user_id, refresh_token_hash, user_agent, ip_address,
                   expires_at, created_at, revoked_at, replaced_by_session_id
            FROM sessions
            WHERE user_id = $1 AND revoked_at IS NULL AND expires_at > now()
            ORDER BY created_at DESC
            "#,
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Hash chain of the tokens this session rotated away from, newest first.
    /// Used by the reuse detector: presenting any of these means the token
    /// was stolen and the whole family must be revoked.
    pub async fn ancestor_hashes(&self, session_id: Uuid) -> Result<Vec<String>, RepoError> {
        let mut hashes = Vec::new();
        let mut current: Option<Uuid> = Some(session_id);
        while let Some(id) = current {
            let prev: Option<(Uuid, String)> = sqlx::query_as(
                r#"
                SELECT id, refresh_token_hash FROM sessions
                WHERE replaced_by_session_id = $1
                "#,
            )
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
            match prev {
                Some((prev_id, hash)) => {
                    hashes.push(hash);
                    current = Some(prev_id);
                }
                None => current = None,
            }
        }
        Ok(hashes)
    }

    /// Rotate: mark `old_id` as replaced by `new_id` (revoke old).
    pub async fn rotate(&self, old_id: Uuid, new_id: Uuid) -> Result<(), RepoError> {
        sqlx::query(
            r#"
            UPDATE sessions
            SET revoked_at = now(), replaced_by_session_id = $2
            WHERE id = $1 AND revoked_at IS NULL
            "#,
        )
        .bind(old_id)
        .bind(new_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Revoke the entire family: the session and every ancestor in its chain.
    pub async fn revoke_family(&self, session_id: Uuid) -> Result<(), RepoError> {
        let ancestors = self.ancestor_hashes(session_id).await?;
        let mut ids = Vec::with_capacity(ancestors.len() + 1);
        ids.push(session_id);
        for hash in ancestors {
            if let Some(s) = self.find_any_by_hash(&hash).await? {
                ids.push(s.id);
            }
        }
        for id in ids {
            self.revoke(id).await?;
        }
        Ok(())
    }

    pub async fn revoke(&self, id: Uuid) -> Result<(), RepoError> {
        sqlx::query("UPDATE sessions SET revoked_at = now() WHERE id = $1 AND revoked_at IS NULL")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn revoke_all_for_user(&self, user_id: Uuid) -> Result<(), RepoError> {
        sqlx::query(
            "UPDATE sessions SET revoked_at = now() WHERE user_id = $1 AND revoked_at IS NULL",
        )
        .bind(user_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Find ANY session (revoked or not) by hash — for reuse detection lookups.
    pub async fn find_any_by_hash(&self, hash: &str) -> Result<Option<Session>, RepoError> {
        let row = sqlx::query_as::<_, Session>(
            r#"
            SELECT id, user_id, refresh_token_hash, user_agent, ip_address,
                   expires_at, created_at, revoked_at, replaced_by_session_id
            FROM sessions
            WHERE refresh_token_hash = $1
            "#,
        )
        .bind(hash)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }
}
