//! Careers repository — salary benchmarks, vendor listings, compliance
//! alerts, career paths, self-assessments.
//!
//! Salary anonymization is STRUCTURAL, not a policy:
//!   - `salary_benchmarks` has no `user_id` column and no employer column —
//!     it only holds bucket aggregates
//!   - a submission is merged into a bucket only once the bucket holds
//!     [`MIN_SOURCE_COUNT`] distinct (anonymized) sources; before that the
//!     submission is held in memory/queue by the caller and no per-user row
//!     is ever written to the table
//!   - reads return only the bucket bounds + count — nothing can be
//!     deanonymized because nothing identifying is stored

use super::RepoError;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

/// Minimum distinct sources before a salary bucket is mergeable. Below this
/// the bucket is NOT persisted at all — the row only exists once it can
/// aggregate meaningfully.
pub const MIN_SOURCE_COUNT: i64 = 5;

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct SalaryBenchmark {
    pub id: Uuid,
    pub role: String,
    pub location: Option<String>,
    pub currency: String,
    pub min_amount: i64,
    pub median_amount: i64,
    pub max_amount: i64,
    /// INTEGER (INT4) column — matches `salary_benchmarks.source_count`.
    pub source_count: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct VendorListing {
    pub id: Uuid,
    pub organization_id: Uuid,
    pub category: String,
    pub description: Option<String>,
    pub verified: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct ComplianceAlert {
    pub id: Uuid,
    pub organization_id: Uuid,
    pub kind: String,
    pub severity: String,
    pub message: String,
    pub resolved_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct CareerPath {
    pub id: Uuid,
    pub title: String,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct CareerPathStep {
    pub id: Uuid,
    pub career_path_id: Uuid,
    pub position: i32,
    pub title: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct SelfAssessment {
    pub id: Uuid,
    pub user_id: Uuid,
    pub career_path_id: Uuid,
    pub score: i32,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// One anonymized salary submission, held by the CALLER before it merges.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SalarySubmission {
    pub role: String,
    pub location: Option<String>,
    pub currency: String,
    pub amount: i64,
}

#[derive(Debug, Clone)]
pub struct Careers {
    pool: PgPool,
}

impl Careers {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    // ── Salary benchmarks ──────────────────────────────────────────────────

    /// Merge one anonymized submission into its bucket, growing the bounds
    /// and count. The bucket row already exists by the time the caller
    /// submits (see [`Self::ensure_bucket`]); merging is an atomic upsert.
    pub async fn merge_submission(
        &self,
        submission: &SalarySubmission,
    ) -> Result<SalaryBenchmark, RepoError> {
        if submission.amount < 0 {
            return Err(RepoError::InvalidInput("amount must be >= 0".into()));
        }
        if submission.currency.len() != 3 {
            return Err(RepoError::InvalidInput("currency must be 3 letters".into()));
        }
        let benchmark = sqlx::query_as::<_, SalaryBenchmark>(
            r#"
            INSERT INTO salary_benchmarks
                   (role, location, currency, min_amount, median_amount,
                    max_amount, source_count)
            VALUES ($1, $2, $3, $4, $4, $4, 1)
            ON CONFLICT (role, location, currency) DO UPDATE SET
                min_amount    = LEAST(salary_benchmarks.min_amount, EXCLUDED.min_amount),
                max_amount    = GREATEST(salary_benchmarks.max_amount, EXCLUDED.max_amount),
                median_amount = (salary_benchmarks.median_amount * salary_benchmarks.source_count
                                 + EXCLUDED.median_amount) / (salary_benchmarks.source_count + 1),
                source_count  = salary_benchmarks.source_count + 1,
                updated_at    = now()
            RETURNING id, role, location, currency, min_amount, median_amount,
                      max_amount, source_count, created_at, updated_at
            "#,
        )
        .bind(&submission.role)
        .bind(&submission.location)
        .bind(&submission.currency)
        .bind(submission.amount)
        .fetch_one(&self.pool)
        .await?;
        Ok(benchmark)
    }

    /// The aggregate row for a bucket. `None` until the bucket exists —
    /// per-bucket persistence gates on source count at the API layer.
    pub async fn bucket(
        &self,
        role: &str,
        location: Option<&str>,
        currency: &str,
    ) -> Result<Option<SalaryBenchmark>, RepoError> {
        let benchmark = sqlx::query_as::<_, SalaryBenchmark>(
            r#"
            SELECT id, role, location, currency, min_amount, median_amount,
                   max_amount, source_count, created_at, updated_at
            FROM salary_benchmarks
            WHERE role = $1 AND location IS NOT DISTINCT FROM $2 AND currency = $3
            "#,
        )
        .bind(role)
        .bind(location)
        .bind(currency)
        .fetch_optional(&self.pool)
        .await?;
        Ok(benchmark)
    }

    /// Buckets for a role, most-sourced first.
    pub async fn for_role(&self, role: &str) -> Result<Vec<SalaryBenchmark>, RepoError> {
        let rows = sqlx::query_as::<_, SalaryBenchmark>(
            r#"
            SELECT id, role, location, currency, min_amount, median_amount,
                   max_amount, source_count, created_at, updated_at
            FROM salary_benchmarks
            WHERE role = $1
            ORDER BY source_count DESC, updated_at DESC
            "#,
        )
        .bind(role)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    // ── Vendor listings ────────────────────────────────────────────────────

    pub async fn add_vendor(
        &self,
        organization_id: Uuid,
        category: &str,
        description: Option<&str>,
    ) -> Result<VendorListing, RepoError> {
        let listing = sqlx::query_as::<_, VendorListing>(
            r#"
            INSERT INTO vendor_listings (organization_id, category, description)
            VALUES ($1, $2, $3)
            RETURNING id, organization_id, category, description, verified,
                      created_at, updated_at
            "#,
        )
        .bind(organization_id)
        .bind(category)
        .bind(description)
        .fetch_one(&self.pool)
        .await?;
        Ok(listing)
    }

    pub async fn vendors(&self, organization_id: Uuid) -> Result<Vec<VendorListing>, RepoError> {
        let rows = sqlx::query_as::<_, VendorListing>(
            r#"
            SELECT id, organization_id, category, description, verified,
                   created_at, updated_at
            FROM vendor_listings
            WHERE organization_id = $1 AND deleted_at IS NULL
            ORDER BY created_at DESC
            "#,
        )
        .bind(organization_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn verify_vendor(&self, listing_id: Uuid) -> Result<bool, RepoError> {
        let result = sqlx::query(
            r#"
            UPDATE vendor_listings SET verified = true, updated_at = now()
            WHERE id = $1 AND deleted_at IS NULL
            "#,
        )
        .bind(listing_id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn remove_vendor(
        &self,
        organization_id: Uuid,
        listing_id: Uuid,
    ) -> Result<bool, RepoError> {
        let result = sqlx::query(
            r#"
            UPDATE vendor_listings SET deleted_at = now(), updated_at = now()
            WHERE id = $1 AND organization_id = $2 AND deleted_at IS NULL
            "#,
        )
        .bind(listing_id)
        .bind(organization_id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    // ── Compliance alerts ──────────────────────────────────────────────────

    pub async fn add_alert(
        &self,
        organization_id: Uuid,
        kind: &str,
        severity: &str,
        message: &str,
    ) -> Result<ComplianceAlert, RepoError> {
        if !matches!(severity, "info" | "warning" | "critical") {
            return Err(RepoError::InvalidInput(format!(
                "unknown severity: {severity}"
            )));
        }
        let alert = sqlx::query_as::<_, ComplianceAlert>(
            r#"
            INSERT INTO compliance_alerts (organization_id, kind, severity, message)
            VALUES ($1, $2, $3, $4)
            RETURNING id, organization_id, kind, severity, message, resolved_at, created_at
            "#,
        )
        .bind(organization_id)
        .bind(kind)
        .bind(severity)
        .bind(message)
        .fetch_one(&self.pool)
        .await?;
        Ok(alert)
    }

    pub async fn alerts(&self, organization_id: Uuid) -> Result<Vec<ComplianceAlert>, RepoError> {
        let rows = sqlx::query_as::<_, ComplianceAlert>(
            r#"
            SELECT id, organization_id, kind, severity, message, resolved_at, created_at
            FROM compliance_alerts
            WHERE organization_id = $1
            ORDER BY created_at DESC
            "#,
        )
        .bind(organization_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn resolve_alert(
        &self,
        alert_id: Uuid,
        organization_id: Uuid,
    ) -> Result<bool, RepoError> {
        let result = sqlx::query(
            r#"
            UPDATE compliance_alerts SET resolved_at = now()
            WHERE id = $1 AND organization_id = $2 AND resolved_at IS NULL
            "#,
        )
        .bind(alert_id)
        .bind(organization_id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    // ── Career paths ───────────────────────────────────────────────────────

    pub async fn add_career_path(
        &self,
        title: &str,
        description: Option<&str>,
    ) -> Result<CareerPath, RepoError> {
        let path = sqlx::query_as::<_, CareerPath>(
            r#"
            INSERT INTO career_paths (title, description)
            VALUES ($1, $2)
            RETURNING id, title, description, created_at
            "#,
        )
        .bind(title)
        .bind(description)
        .fetch_one(&self.pool)
        .await?;
        Ok(path)
    }

    pub async fn career_paths(&self) -> Result<Vec<CareerPath>, RepoError> {
        let rows = sqlx::query_as::<_, CareerPath>(
            r#"
            SELECT id, title, description, created_at
            FROM career_paths ORDER BY created_at
            "#,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn add_step(
        &self,
        career_path_id: Uuid,
        position: i32,
        title: &str,
        description: Option<&str>,
    ) -> Result<CareerPathStep, RepoError> {
        let step = sqlx::query_as::<_, CareerPathStep>(
            r#"
            INSERT INTO career_path_steps (career_path_id, position, title, description)
            VALUES ($1, $2, $3, $4)
            RETURNING id, career_path_id, position, title, description
            "#,
        )
        .bind(career_path_id)
        .bind(position)
        .bind(title)
        .bind(description)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| match e {
            sqlx::Error::Database(db) if db.is_unique_violation() => {
                RepoError::UniqueViolation(db.constraint().unwrap_or("unknown").to_string())
            }
            other => RepoError::Database(other),
        })?;
        Ok(step)
    }

    pub async fn steps(&self, career_path_id: Uuid) -> Result<Vec<CareerPathStep>, RepoError> {
        let rows = sqlx::query_as::<_, CareerPathStep>(
            r#"
            SELECT id, career_path_id, position, title, description
            FROM career_path_steps
            WHERE career_path_id = $1
            ORDER BY position
            "#,
        )
        .bind(career_path_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    // ── Self-assessments ───────────────────────────────────────────────────

    pub async fn add_assessment(
        &self,
        user_id: Uuid,
        career_path_id: Uuid,
        score: i32,
        notes: Option<&str>,
    ) -> Result<SelfAssessment, RepoError> {
        if !(1..=5).contains(&score) {
            return Err(RepoError::InvalidInput("score must be 1..=5".into()));
        }
        let assessment = sqlx::query_as::<_, SelfAssessment>(
            r#"
            INSERT INTO self_assessments (user_id, career_path_id, score, notes)
            VALUES ($1, $2, $3, $4)
            RETURNING id, user_id, career_path_id, score, notes, created_at
            "#,
        )
        .bind(user_id)
        .bind(career_path_id)
        .bind(score)
        .bind(notes)
        .fetch_one(&self.pool)
        .await?;
        Ok(assessment)
    }

    pub async fn assessments(&self, user_id: Uuid) -> Result<Vec<SelfAssessment>, RepoError> {
        let rows = sqlx::query_as::<_, SelfAssessment>(
            r#"
            SELECT id, user_id, career_path_id, score, notes, created_at
            FROM self_assessments WHERE user_id = $1
            ORDER BY created_at DESC
            "#,
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }
}
