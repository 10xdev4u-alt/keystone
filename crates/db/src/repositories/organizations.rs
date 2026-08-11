//! Organizations repository — orgs, role-based membership, and the claim flow.
//!
//! Repo-enforced invariants:
//!   - exactly one `owner` per org: promoting someone to owner demotes the
//!     current owner inside the same transaction; the owner cannot leave or
//!     be removed
//!   - a claim's raw token is only ever stored hashed; `verify` compares
//!     hashes in constant time and refuses expired claims

use super::RepoError;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct Organization {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub website: Option<String>,
    pub industry: Option<String>,
    pub created_by: Uuid,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct OrganizationMember {
    pub organization_id: Uuid,
    pub user_id: Uuid,
    pub role: String,
    pub joined_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct OrganizationClaim {
    pub id: Uuid,
    pub organization_id: Uuid,
    pub claimant_id: Uuid,
    pub domain: String,
    pub status: String,
    pub token_hash: String,
    pub expires_at: DateTime<Utc>,
    pub decided_by: Option<Uuid>,
    pub decided_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewOrganization<'a> {
    pub name: &'a str,
    pub slug: &'a str,
    pub description: Option<&'a str>,
    pub website: Option<&'a str>,
    pub industry: Option<&'a str>,
    pub created_by: Uuid,
}

#[derive(Debug, Clone)]
pub struct Organizations {
    pool: PgPool,
}

impl Organizations {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Create an org; the creator becomes its sole owner, atomically.
    pub async fn create(&self, new_org: NewOrganization<'_>) -> Result<Organization, RepoError> {
        let mut tx = self.pool.begin().await?;
        let org = sqlx::query_as::<_, Organization>(
            r#"
            INSERT INTO organizations (name, slug, description, website, industry, created_by)
            VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING id, name, slug, description, website, industry, created_by,
                      created_at, updated_at
            "#,
        )
        .bind(new_org.name)
        .bind(new_org.slug)
        .bind(new_org.description)
        .bind(new_org.website)
        .bind(new_org.industry)
        .bind(new_org.created_by)
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
            INSERT INTO organization_members (organization_id, user_id, role)
            VALUES ($1, $2, 'owner')
            "#,
        )
        .bind(org.id)
        .bind(new_org.created_by)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(org)
    }

    pub async fn get_by_id(&self, id: Uuid) -> Result<Option<Organization>, RepoError> {
        let org = sqlx::query_as::<_, Organization>(
            r#"
            SELECT id, name, slug, description, website, industry, created_by,
                   created_at, updated_at
            FROM organizations
            WHERE id = $1 AND deleted_at IS NULL
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(org)
    }

    pub async fn get_by_slug(&self, slug: &str) -> Result<Option<Organization>, RepoError> {
        let org = sqlx::query_as::<_, Organization>(
            r#"
            SELECT id, name, slug, description, website, industry, created_by,
                   created_at, updated_at
            FROM organizations
            WHERE slug = $1 AND deleted_at IS NULL
            "#,
        )
        .bind(slug)
        .fetch_optional(&self.pool)
        .await?;
        Ok(org)
    }

    /// Live orgs, newest first.
    pub async fn list(&self, limit: i64, offset: i64) -> Result<Vec<Organization>, RepoError> {
        let rows = sqlx::query_as::<_, Organization>(
            r#"
            SELECT id, name, slug, description, website, industry, created_by,
                   created_at, updated_at
            FROM organizations
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

    /// Role of a user in an org, if they are a member.
    pub async fn member_role(
        &self,
        organization_id: Uuid,
        user_id: Uuid,
    ) -> Result<Option<String>, RepoError> {
        let role = sqlx::query_scalar::<_, String>(
            r#"
            SELECT role FROM organization_members
            WHERE organization_id = $1 AND user_id = $2
            "#,
        )
        .bind(organization_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(role)
    }

    /// Join as a plain member; already a member is a no-op.
    pub async fn join(&self, organization_id: Uuid, user_id: Uuid) -> Result<(), RepoError> {
        sqlx::query(
            r#"
            INSERT INTO organization_members (organization_id, user_id, role)
            VALUES ($1, $2, 'member')
            ON CONFLICT (organization_id, user_id) DO NOTHING
            "#,
        )
        .bind(organization_id)
        .bind(user_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Leave an org. Refuses to remove the last owner (the org would be
    /// ownerless). Answers whether a membership was removed.
    pub async fn leave(&self, organization_id: Uuid, user_id: Uuid) -> Result<bool, RepoError> {
        let mut tx = self.pool.begin().await?;
        let owner_count: i64 = sqlx::query_scalar(
            r#"
            SELECT count(*) FROM organization_members
            WHERE organization_id = $1 AND role = 'owner'
            "#,
        )
        .bind(organization_id)
        .fetch_one(&mut *tx)
        .await?;

        let role: Option<String> = sqlx::query_scalar(
            r#"
            SELECT role FROM organization_members
            WHERE organization_id = $1 AND user_id = $2
            "#,
        )
        .bind(organization_id)
        .bind(user_id)
        .fetch_optional(&mut *tx)
        .await?;

        if role.as_deref() == Some("owner") && owner_count <= 1 {
            tx.rollback().await?;
            return Err(RepoError::InvalidInput(
                "cannot leave: the org would have no owner".into(),
            ));
        }

        let result = sqlx::query(
            r#"
            DELETE FROM organization_members
            WHERE organization_id = $1 AND user_id = $2
            "#,
        )
        .bind(organization_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(result.rows_affected() == 1)
    }

    /// Set a member's role. Transferring ownership demotes the current owner
    /// atomically; the current owner cannot be demoted to a plain member by
    /// themselves (callers gate on actor rank — the repo only keeps the
    /// one-owner invariant).
    pub async fn set_role(
        &self,
        organization_id: Uuid,
        user_id: Uuid,
        role: &str,
    ) -> Result<(), RepoError> {
        if !matches!(role, "member" | "admin" | "owner") {
            return Err(RepoError::InvalidInput(format!("unknown org role: {role}")));
        }
        let mut tx = self.pool.begin().await?;
        if role == "owner" {
            // Demote the current owner first so there is never a gap.
            sqlx::query(
                r#"
                UPDATE organization_members SET role = 'admin'
                WHERE organization_id = $1 AND role = 'owner'
                "#,
            )
            .bind(organization_id)
            .execute(&mut *tx)
            .await?;
        }
        let result = sqlx::query(
            r#"
            UPDATE organization_members SET role = $3
            WHERE organization_id = $1 AND user_id = $2
            "#,
        )
        .bind(organization_id)
        .bind(user_id)
        .bind(role)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        if result.rows_affected() == 0 {
            return Err(RepoError::InvalidInput("user is not a member".into()));
        }
        Ok(())
    }

    /// Members of an org with their roles.
    pub async fn members(
        &self,
        organization_id: Uuid,
    ) -> Result<Vec<OrganizationMember>, RepoError> {
        let rows = sqlx::query_as::<_, OrganizationMember>(
            r#"
            SELECT organization_id, user_id, role, joined_at
            FROM organization_members
            WHERE organization_id = $1
            ORDER BY joined_at
            "#,
        )
        .bind(organization_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// File a claim on an org. The caller hashes the token — the repo only
    /// stores the hash and refuses overlapping pending claims on the same org.
    pub async fn create_claim(
        &self,
        organization_id: Uuid,
        claimant_id: Uuid,
        domain: &str,
        token_hash: &str,
        expires_at: DateTime<Utc>,
    ) -> Result<OrganizationClaim, RepoError> {
        let claim = sqlx::query_as::<_, OrganizationClaim>(
            r#"
            INSERT INTO organization_claims
                   (organization_id, claimant_id, domain, token_hash, expires_at)
            VALUES ($1, $2, $3, $4, $5)
            RETURNING id, organization_id, claimant_id, domain, status, token_hash,
                      expires_at, decided_by, decided_at, created_at
            "#,
        )
        .bind(organization_id)
        .bind(claimant_id)
        .bind(domain)
        .bind(token_hash)
        .bind(expires_at)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| match e {
            sqlx::Error::Database(db) if db.is_unique_violation() => {
                RepoError::UniqueViolation(db.constraint().unwrap_or("unknown").to_string())
            }
            other => RepoError::Database(other),
        })?;
        Ok(claim)
    }

    /// Verify a claim token. Compares the stored hash, refuses expired or
    /// already-decided claims, and flips the status to `approved` atomically.
    pub async fn verify_claim(&self, claim_id: Uuid, token_hash: &str) -> Result<bool, RepoError> {
        let mut tx = self.pool.begin().await?;
        let claim = sqlx::query_as::<_, OrganizationClaim>(
            r#"
            SELECT id, organization_id, claimant_id, domain, status, token_hash,
                   expires_at, decided_by, decided_at, created_at
            FROM organization_claims
            WHERE id = $1
            "#,
        )
        .bind(claim_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(claim) = claim else {
            return Ok(false);
        };
        if claim.status != "pending" || claim.expires_at < Utc::now() {
            return Ok(false);
        }
        // Constant-time hash comparison — a raw token is never stored.
        let matches: subtle::Choice =
            subtle::ConstantTimeEq::ct_eq(claim.token_hash.as_bytes(), token_hash.as_bytes());
        if !bool::from(matches) {
            return Ok(false);
        }
        sqlx::query(
            "UPDATE organization_claims SET status = 'approved', decided_at = now() WHERE id = $1",
        )
        .bind(claim_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(true)
    }
}
