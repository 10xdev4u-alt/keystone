//! Communities repository — role-based membership and curated discussions.
//!
//! Role invariants that the schema cannot CHECK are enforced here:
//!   - exactly one `owner` per community (promoting another to owner first
//!     demotes the current owner inside the same transaction)
//!   - the owner cannot be demoted or removed
//!   - `moderator`/`admin`/`owner` transitions require a staff actor of
//!     equal-or-higher rank (caller checks; the repo only refuses owner loss)

use super::RepoError;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct Community {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub visibility: String,
    pub created_by: Uuid,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct CommunityMember {
    pub community_id: Uuid,
    pub user_id: Uuid,
    pub role: String,
    pub joined_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewCommunity<'a> {
    pub name: &'a str,
    pub slug: &'a str,
    pub description: Option<&'a str>,
    pub visibility: &'a str,
    pub created_by: Uuid,
}

#[derive(Debug, Clone)]
pub struct Communities {
    pool: PgPool,
}

impl Communities {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Create a community; the creator becomes its sole owner, atomically.
    pub async fn create(&self, new_community: NewCommunity<'_>) -> Result<Community, RepoError> {
        let mut tx = self.pool.begin().await?;
        let community = sqlx::query_as::<_, Community>(
            r#"
            INSERT INTO communities (name, slug, description, visibility, created_by)
            VALUES ($1, $2, $3, $4, $5)
            RETURNING id, name, slug, description, visibility, created_by,
                      created_at, updated_at
            "#,
        )
        .bind(new_community.name)
        .bind(new_community.slug)
        .bind(new_community.description)
        .bind(new_community.visibility)
        .bind(new_community.created_by)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| match e {
            sqlx::Error::Database(db) if db.is_unique_violation() => {
                RepoError::UniqueViolation(db.constraint().unwrap_or("unknown").to_string())
            }
            other => RepoError::Database(other),
        })?;

        sqlx::query(
            r#"
            INSERT INTO community_members (community_id, user_id, role)
            VALUES ($1, $2, 'owner')
            "#,
        )
        .bind(community.id)
        .bind(new_community.created_by)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(community)
    }

    pub async fn get_by_slug(&self, slug: &str) -> Result<Option<Community>, RepoError> {
        let community = sqlx::query_as::<_, Community>(
            r#"
            SELECT id, name, slug, description, visibility, created_by, created_at, updated_at
            FROM communities
            WHERE slug = $1 AND deleted_at IS NULL
            "#,
        )
        .bind(slug)
        .fetch_optional(&self.pool)
        .await?;
        Ok(community)
    }

    pub async fn get_by_id(&self, id: Uuid) -> Result<Option<Community>, RepoError> {
        let community = sqlx::query_as::<_, Community>(
            r#"
            SELECT id, name, slug, description, visibility, created_by, created_at, updated_at
            FROM communities
            WHERE id = $1 AND deleted_at IS NULL
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(community)
    }

    /// Live communities, newest first.
    pub async fn list(&self, limit: i64, offset: i64) -> Result<Vec<Community>, RepoError> {
        let rows = sqlx::query_as::<_, Community>(
            r#"
            SELECT id, name, slug, description, visibility, created_by, created_at, updated_at
            FROM communities
            WHERE deleted_at IS NULL
            ORDER BY created_at DESC
            LIMIT $1 OFFSET $2
            "#,
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Join as a member; already a member is a no-op.
    pub async fn join(&self, community_id: Uuid, user_id: Uuid) -> Result<(), RepoError> {
        sqlx::query(
            r#"
            INSERT INTO community_members (community_id, user_id, role)
            VALUES ($1, $2, 'member')
            ON CONFLICT (community_id, user_id) DO NOTHING
            "#,
        )
        .bind(community_id)
        .bind(user_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Leave; the owner may not leave (must transfer ownership first).
    pub async fn leave(&self, community_id: Uuid, user_id: Uuid) -> Result<bool, RepoError> {
        let role: Option<String> = sqlx::query_scalar(
            "SELECT role FROM community_members WHERE community_id = $1 AND user_id = $2",
        )
        .bind(community_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;
        match role.as_deref() {
            Some("owner") => Err(RepoError::InvalidInput(
                "the owner must transfer ownership before leaving".into(),
            )),
            Some(_) => {
                let result = sqlx::query(
                    "DELETE FROM community_members WHERE community_id = $1 AND user_id = $2",
                )
                .bind(community_id)
                .bind(user_id)
                .execute(&self.pool)
                .await?;
                Ok(result.rows_affected() == 1)
            }
            None => Ok(false),
        }
    }

    pub async fn role_of(
        &self,
        community_id: Uuid,
        user_id: Uuid,
    ) -> Result<Option<String>, RepoError> {
        let role = sqlx::query_scalar(
            "SELECT role FROM community_members WHERE community_id = $1 AND user_id = $2",
        )
        .bind(community_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(role)
    }

    /// Set a member's role. Promoting someone to `owner` transfers ownership:
    /// the current owner is demoted to `admin` in the same transaction, so
    /// the single-owner invariant never breaks. Demoting the current owner
    /// to anything else is refused.
    pub async fn set_role(
        &self,
        community_id: Uuid,
        user_id: Uuid,
        role: &str,
    ) -> Result<(), RepoError> {
        if !matches!(role, "member" | "moderator" | "admin" | "owner") {
            return Err(RepoError::InvalidInput("unknown member role".into()));
        }
        let mut tx = self.pool.begin().await?;

        let current: Option<String> = sqlx::query_scalar(
            "SELECT role FROM community_members WHERE community_id = $1 AND user_id = $2",
        )
        .bind(community_id)
        .bind(user_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(current) = current else {
            return Err(RepoError::InvalidInput("user is not a member".into()));
        };
        if current == "owner" && role != "owner" {
            return Err(RepoError::InvalidInput(
                "the owner must transfer ownership before being demoted".into(),
            ));
        }
        if role == "owner" && current != "owner" {
            // Transfer: demote the current owner to admin inside this tx.
            sqlx::query("UPDATE community_members SET role = 'admin' WHERE community_id = $1 AND role = 'owner'")
                .bind(community_id)
                .execute(&mut *tx)
                .await?;
        }

        sqlx::query(
            "UPDATE community_members SET role = $3 WHERE community_id = $1 AND user_id = $2",
        )
        .bind(community_id)
        .bind(user_id)
        .bind(role)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(())
    }

    /// Members with roles, newest joins last.
    pub async fn members(&self, community_id: Uuid) -> Result<Vec<CommunityMember>, RepoError> {
        let rows = sqlx::query_as::<_, CommunityMember>(
            r#"
            SELECT community_id, user_id, role, joined_at
            FROM community_members
            WHERE community_id = $1
            ORDER BY joined_at ASC
            "#,
        )
        .bind(community_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }
}
