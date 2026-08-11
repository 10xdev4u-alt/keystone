//! Profiles repository — profile settings, education, experience, skills.
//!
//! Visibility model: a profile row is `public`, `connections`, or `private`.
//! Readers decide what they can see by combining this flag with the social
//! graph (see the API layer); the repo stores and enforces the vocabulary.

use super::RepoError;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct UserProfile {
    pub user_id: Uuid,
    pub bio: Option<String>,
    pub location: Option<String>,
    pub visibility: String,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct Education {
    pub id: Uuid,
    pub user_id: Uuid,
    pub school: String,
    pub degree: Option<String>,
    pub field: Option<String>,
    pub start_year: i32,
    pub end_year: Option<i32>,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct Experience {
    pub id: Uuid,
    pub user_id: Uuid,
    pub organization_id: Option<Uuid>,
    pub title: String,
    pub company: Option<String>,
    pub start_date: chrono::NaiveDate,
    pub end_date: Option<chrono::NaiveDate>,
    pub current: bool,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct UserSkill {
    pub user_id: Uuid,
    pub skill: String,
    pub level: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewEducation<'a> {
    pub school: &'a str,
    pub degree: Option<&'a str>,
    pub field: Option<&'a str>,
    pub start_year: i32,
    pub end_year: Option<i32>,
    pub description: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewExperience<'a> {
    pub organization_id: Option<Uuid>,
    pub title: &'a str,
    pub company: Option<&'a str>,
    pub start_date: chrono::NaiveDate,
    pub end_date: Option<chrono::NaiveDate>,
    pub current: bool,
    pub description: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub struct Profiles {
    pool: PgPool,
}

impl Profiles {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Upsert the profile settings row.
    pub async fn set(
        &self,
        user_id: Uuid,
        bio: Option<&str>,
        location: Option<&str>,
        visibility: &str,
    ) -> Result<UserProfile, RepoError> {
        if !matches!(visibility, "public" | "connections" | "private") {
            return Err(RepoError::InvalidInput(format!(
                "unknown visibility: {visibility}"
            )));
        }
        let profile = sqlx::query_as::<_, UserProfile>(
            r#"
            INSERT INTO user_profiles (user_id, bio, location, visibility)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (user_id) DO UPDATE
            SET bio = EXCLUDED.bio,
                location = EXCLUDED.location,
                visibility = EXCLUDED.visibility,
                updated_at = now()
            RETURNING user_id, bio, location, visibility, updated_at
            "#,
        )
        .bind(user_id)
        .bind(bio)
        .bind(location)
        .bind(visibility)
        .fetch_one(&self.pool)
        .await?;
        Ok(profile)
    }

    pub async fn get(&self, user_id: Uuid) -> Result<Option<UserProfile>, RepoError> {
        let profile = sqlx::query_as::<_, UserProfile>(
            r#"
            SELECT user_id, bio, location, visibility, updated_at
            FROM user_profiles WHERE user_id = $1
            "#,
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(profile)
    }

    // ── Education ──────────────────────────────────────────────────────────

    pub async fn add_education(
        &self,
        user_id: Uuid,
        education: NewEducation<'_>,
    ) -> Result<Education, RepoError> {
        let row = sqlx::query_as::<_, Education>(
            r#"
            INSERT INTO user_education
                   (user_id, school, degree, field, start_year, end_year, description)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            RETURNING id, user_id, school, degree, field, start_year, end_year,
                      description, created_at
            "#,
        )
        .bind(user_id)
        .bind(education.school)
        .bind(education.degree)
        .bind(education.field)
        .bind(education.start_year)
        .bind(education.end_year)
        .bind(education.description)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn education(&self, user_id: Uuid) -> Result<Vec<Education>, RepoError> {
        let rows = sqlx::query_as::<_, Education>(
            r#"
            SELECT id, user_id, school, degree, field, start_year, end_year,
                   description, created_at
            FROM user_education WHERE user_id = $1
            ORDER BY start_year DESC, created_at DESC
            "#,
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Remove an education row; answers whether it belonged to the user.
    pub async fn remove_education(
        &self,
        user_id: Uuid,
        education_id: Uuid,
    ) -> Result<bool, RepoError> {
        let result = sqlx::query("DELETE FROM user_education WHERE id = $1 AND user_id = $2")
            .bind(education_id)
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() == 1)
    }

    // ── Experience ─────────────────────────────────────────────────────────

    pub async fn add_experience(
        &self,
        user_id: Uuid,
        experience: NewExperience<'_>,
    ) -> Result<Experience, RepoError> {
        let row = sqlx::query_as::<_, Experience>(
            r#"
            INSERT INTO user_experience
                   (user_id, organization_id, title, company, start_date,
                    end_date, current, description)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            RETURNING id, user_id, organization_id, title, company, start_date,
                      end_date, current, description, created_at
            "#,
        )
        .bind(user_id)
        .bind(experience.organization_id)
        .bind(experience.title)
        .bind(experience.company)
        .bind(experience.start_date)
        .bind(experience.end_date)
        .bind(experience.current)
        .bind(experience.description)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn experience(&self, user_id: Uuid) -> Result<Vec<Experience>, RepoError> {
        let rows = sqlx::query_as::<_, Experience>(
            r#"
            SELECT id, user_id, organization_id, title, company, start_date,
                   end_date, current, description, created_at
            FROM user_experience WHERE user_id = $1
            ORDER BY start_date DESC, created_at DESC
            "#,
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn remove_experience(
        &self,
        user_id: Uuid,
        experience_id: Uuid,
    ) -> Result<bool, RepoError> {
        let result = sqlx::query("DELETE FROM user_experience WHERE id = $1 AND user_id = $2")
            .bind(experience_id)
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() == 1)
    }

    // ── Skills ─────────────────────────────────────────────────────────────

    pub async fn add_skill(
        &self,
        user_id: Uuid,
        skill: &str,
        level: &str,
    ) -> Result<(), RepoError> {
        if !matches!(level, "beginner" | "intermediate" | "advanced" | "expert") {
            return Err(RepoError::InvalidInput(format!(
                "unknown skill level: {level}"
            )));
        }
        sqlx::query(
            r#"
            INSERT INTO user_skills (user_id, skill, level)
            VALUES ($1, $2, $3)
            ON CONFLICT (user_id, skill) DO UPDATE SET level = EXCLUDED.level
            "#,
        )
        .bind(user_id)
        .bind(skill)
        .bind(level)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn skills(&self, user_id: Uuid) -> Result<Vec<UserSkill>, RepoError> {
        let rows = sqlx::query_as::<_, UserSkill>(
            r#"
            SELECT user_id, skill, level, created_at
            FROM user_skills WHERE user_id = $1
            ORDER BY created_at DESC
            "#,
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn remove_skill(&self, user_id: Uuid, skill: &str) -> Result<bool, RepoError> {
        let result = sqlx::query("DELETE FROM user_skills WHERE user_id = $1 AND skill = $2")
            .bind(user_id)
            .bind(skill)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() == 1)
    }
}
