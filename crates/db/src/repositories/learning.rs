//! Learning repository — courses/modules/lessons, enrollment, lesson
//! progress (one row per user+lesson), completion certificates, and
//! learning paths.
//!
//! "Progress that cannot lie":
//!   - enrollment is idempotent (PK is (course_id, user_id))
//!   - completion is COMPUTED, never claimed: marking a lesson complete
//!     re-derives course progress from `lesson_progress` rows inside the
//!     same transaction; when every lesson is complete, the certificate is
//!     issued atomically (unique per user+course, so it cannot double-issue)
//!   - certificates store only the token HASH — verification is hash-compare

use super::RepoError;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct Course {
    pub id: Uuid,
    pub author_id: Uuid,
    pub title: String,
    pub slug: String,
    pub description: Option<String>,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct CourseModule {
    pub id: Uuid,
    pub course_id: Uuid,
    pub position: i32,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct Lesson {
    pub id: Uuid,
    pub module_id: Uuid,
    pub position: i32,
    pub title: String,
    pub content: String,
    pub duration_seconds: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct LessonProgress {
    pub lesson_id: Uuid,
    pub user_id: Uuid,
    pub completed: bool,
    pub progress_percent: i32,
    pub completed_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct Certificate {
    pub id: Uuid,
    pub user_id: Uuid,
    pub course_id: Uuid,
    pub token_hash: String,
    pub issued_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewCourse<'a> {
    pub author_id: Uuid,
    pub title: &'a str,
    pub slug: &'a str,
    pub description: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub struct Learning {
    pool: PgPool,
}

impl Learning {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    // ── Courses ──────────────────────────────────────────────────────────

    pub async fn create_course(&self, new_course: NewCourse<'_>) -> Result<Course, RepoError> {
        let course = sqlx::query_as::<_, Course>(
            r#"
            INSERT INTO courses (author_id, title, slug, description)
            VALUES ($1, $2, $3, $4)
            RETURNING id, author_id, title, slug, description, status,
                      created_at, updated_at
            "#,
        )
        .bind(new_course.author_id)
        .bind(new_course.title)
        .bind(new_course.slug)
        .bind(new_course.description)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| match e {
            sqlx::Error::Database(db) if db.is_unique_violation() => {
                RepoError::UniqueViolation(db.constraint().unwrap_or("unknown").to_string())
            }
            other => RepoError::Database(other),
        })?;
        Ok(course)
    }

    pub async fn get_course(&self, id: Uuid) -> Result<Option<Course>, RepoError> {
        let course = sqlx::query_as::<_, Course>(
            r#"
            SELECT id, author_id, title, slug, description, status, created_at, updated_at
            FROM courses WHERE id = $1 AND deleted_at IS NULL
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(course)
    }

    pub async fn get_course_by_slug(&self, slug: &str) -> Result<Option<Course>, RepoError> {
        let course = sqlx::query_as::<_, Course>(
            r#"
            SELECT id, author_id, title, slug, description, status, created_at, updated_at
            FROM courses WHERE slug = $1 AND deleted_at IS NULL
            "#,
        )
        .bind(slug)
        .fetch_optional(&self.pool)
        .await?;
        Ok(course)
    }

    /// Publish a course (draft → published).
    pub async fn publish_course(&self, id: Uuid, author_id: Uuid) -> Result<bool, RepoError> {
        let result = sqlx::query(
            r#"
            UPDATE courses SET status = 'published'
            WHERE id = $1 AND author_id = $2 AND deleted_at IS NULL
            "#,
        )
        .bind(id)
        .bind(author_id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn published_courses(
        &self,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<Course>, RepoError> {
        let rows = sqlx::query_as::<_, Course>(
            r#"
            SELECT id, author_id, title, slug, description, status, created_at, updated_at
            FROM courses
            WHERE status = 'published' AND deleted_at IS NULL
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

    // ── Modules & lessons ────────────────────────────────────────────────

    pub async fn add_module(
        &self,
        course_id: Uuid,
        position: i32,
        title: &str,
    ) -> Result<CourseModule, RepoError> {
        let module = sqlx::query_as::<_, CourseModule>(
            r#"
            INSERT INTO course_modules (course_id, position, title)
            VALUES ($1, $2, $3)
            RETURNING id, course_id, position, title
            "#,
        )
        .bind(course_id)
        .bind(position)
        .bind(title)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| match e {
            sqlx::Error::Database(db) if db.is_unique_violation() => {
                RepoError::UniqueViolation(db.constraint().unwrap_or("unknown").to_string())
            }
            other => RepoError::Database(other),
        })?;
        Ok(module)
    }

    pub async fn modules(&self, course_id: Uuid) -> Result<Vec<CourseModule>, RepoError> {
        let rows = sqlx::query_as::<_, CourseModule>(
            r#"
            SELECT id, course_id, position, title
            FROM course_modules WHERE course_id = $1
            ORDER BY position
            "#,
        )
        .bind(course_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn add_lesson(
        &self,
        module_id: Uuid,
        position: i32,
        title: &str,
        content: &str,
        duration_seconds: Option<i32>,
    ) -> Result<Lesson, RepoError> {
        let lesson = sqlx::query_as::<_, Lesson>(
            r#"
            INSERT INTO lessons (module_id, position, title, content, duration_seconds)
            VALUES ($1, $2, $3, $4, $5)
            RETURNING id, module_id, position, title, content, duration_seconds
            "#,
        )
        .bind(module_id)
        .bind(position)
        .bind(title)
        .bind(content)
        .bind(duration_seconds)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| match e {
            sqlx::Error::Database(db) if db.is_unique_violation() => {
                RepoError::UniqueViolation(db.constraint().unwrap_or("unknown").to_string())
            }
            other => RepoError::Database(other),
        })?;
        Ok(lesson)
    }

    pub async fn lessons(&self, module_id: Uuid) -> Result<Vec<Lesson>, RepoError> {
        let rows = sqlx::query_as::<_, Lesson>(
            r#"
            SELECT id, module_id, position, title, content, duration_seconds
            FROM lessons WHERE module_id = $1
            ORDER BY position
            "#,
        )
        .bind(module_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// All lessons of a course (via its modules), for progress derivation.
    pub async fn course_lesson_ids(&self, course_id: Uuid) -> Result<Vec<Uuid>, RepoError> {
        let rows = sqlx::query_scalar::<_, Uuid>(
            r#"
            SELECT l.id FROM lessons l
            JOIN course_modules m ON m.id = l.module_id
            WHERE m.course_id = $1
            ORDER BY m.position, l.position
            "#,
        )
        .bind(course_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    // ── Enrollment & progress ────────────────────────────────────────────

    /// Idempotent enrollment — the PK makes re-enrolling a no-op.
    pub async fn enroll(&self, course_id: Uuid, user_id: Uuid) -> Result<(), RepoError> {
        sqlx::query(
            r#"
            INSERT INTO enrollments (course_id, user_id)
            VALUES ($1, $2)
            ON CONFLICT (course_id, user_id) DO NOTHING
            "#,
        )
        .bind(course_id)
        .bind(user_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn is_enrolled(&self, course_id: Uuid, user_id: Uuid) -> Result<bool, RepoError> {
        let enrolled = sqlx::query_scalar::<_, bool>(
            r#"
            SELECT EXISTS (
                SELECT 1 FROM enrollments WHERE course_id = $1 AND user_id = $2
            )
            "#,
        )
        .bind(course_id)
        .bind(user_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(enrolled)
    }

    /// Mark a lesson complete (transactional with certificate issuance).
    ///
    /// Inside one transaction: upsert the user's progress row for this
    /// lesson, then re-derive course completion from `lesson_progress`.
    /// If every lesson of the course is now complete and no certificate
    /// exists yet, one is issued atomically (unique (user, course) makes a
    /// double-issue impossible). Returns the certificate when issued.
    pub async fn complete_lesson(
        &self,
        course_id: Uuid,
        lesson_id: Uuid,
        user_id: Uuid,
        certificate_token_hash: &str,
    ) -> Result<Option<Certificate>, RepoError> {
        let mut tx = self.pool.begin().await?;

        // The lesson must belong to this course.
        let owned: Option<Uuid> = sqlx::query_scalar(
            r#"
            SELECT m.course_id FROM lessons l
            JOIN course_modules m ON m.id = l.module_id
            WHERE l.id = $1
            "#,
        )
        .bind(lesson_id)
        .fetch_optional(&mut *tx)
        .await?;
        if owned != Some(course_id) {
            tx.rollback().await?;
            return Err(RepoError::InvalidInput(
                "lesson does not belong to course".into(),
            ));
        }

        sqlx::query(
            r#"
            INSERT INTO lesson_progress (lesson_id, user_id, completed, progress_percent, completed_at)
            VALUES ($1, $2, true, 100, now())
            ON CONFLICT (lesson_id, user_id) DO UPDATE
            SET completed = true, progress_percent = 100, completed_at = now(), updated_at = now()
            "#,
        )
        .bind(lesson_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;

        // Progress that cannot lie: completion is re-derived, not stored.
        let all_complete: bool = sqlx::query_scalar(
            r#"
            SELECT NOT EXISTS (
                SELECT 1
                FROM lessons l
                JOIN course_modules m ON m.id = l.module_id
                WHERE m.course_id = $1
                  AND NOT EXISTS (
                      SELECT 1 FROM lesson_progress lp
                      WHERE lp.lesson_id = l.id AND lp.user_id = $2 AND lp.completed
                  )
            )
            "#,
        )
        .bind(course_id)
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await?;

        let mut certificate = None;
        if all_complete {
            certificate = sqlx::query_as::<_, Certificate>(
                r#"
                INSERT INTO certificates (user_id, course_id, token_hash)
                VALUES ($1, $2, $3)
                ON CONFLICT (user_id, course_id) DO NOTHING
                RETURNING id, user_id, course_id, token_hash, issued_at
                "#,
            )
            .bind(user_id)
            .bind(course_id)
            .bind(certificate_token_hash)
            .fetch_optional(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(certificate)
    }

    pub async fn progress_for(
        &self,
        user_id: Uuid,
        course_id: Uuid,
    ) -> Result<Vec<LessonProgress>, RepoError> {
        let rows = sqlx::query_as::<_, LessonProgress>(
            r#"
            SELECT lp.lesson_id, lp.user_id, lp.completed, lp.progress_percent,
                   lp.completed_at, lp.updated_at
            FROM lesson_progress lp
            JOIN lessons l ON l.id = lp.lesson_id
            JOIN course_modules m ON m.id = l.module_id
            WHERE lp.user_id = $1 AND m.course_id = $2
            ORDER BY m.position, l.position
            "#,
        )
        .bind(user_id)
        .bind(course_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    // ── Certificates ─────────────────────────────────────────────────────

    /// Verify a certificate token — hash-compare, answers whether a live
    /// certificate for the user+course carries this hash.
    pub async fn verify_certificate(
        &self,
        user_id: Uuid,
        course_id: Uuid,
        token_hash: &str,
    ) -> Result<bool, RepoError> {
        let matches = sqlx::query_scalar::<_, bool>(
            r#"
            SELECT EXISTS (
                SELECT 1 FROM certificates
                WHERE user_id = $1 AND course_id = $2 AND token_hash = $3
            )
            "#,
        )
        .bind(user_id)
        .bind(course_id)
        .bind(token_hash)
        .fetch_one(&self.pool)
        .await?;
        Ok(matches)
    }

    pub async fn certificates_for_user(
        &self,
        user_id: Uuid,
    ) -> Result<Vec<Certificate>, RepoError> {
        let rows = sqlx::query_as::<_, Certificate>(
            r#"
            SELECT id, user_id, course_id, token_hash, issued_at
            FROM certificates WHERE user_id = $1
            ORDER BY issued_at DESC
            "#,
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    // ── Learning paths ───────────────────────────────────────────────────

    pub async fn create_path(
        &self,
        title: &str,
        description: Option<&str>,
    ) -> Result<Uuid, RepoError> {
        let id = sqlx::query_scalar::<_, Uuid>(
            r#"
            INSERT INTO learning_paths (title, description)
            VALUES ($1, $2) RETURNING id
            "#,
        )
        .bind(title)
        .bind(description)
        .fetch_one(&self.pool)
        .await?;
        Ok(id)
    }

    pub async fn add_path_course(
        &self,
        path_id: Uuid,
        course_id: Uuid,
        position: i32,
    ) -> Result<(), RepoError> {
        sqlx::query(
            r#"
            INSERT INTO learning_path_courses (path_id, course_id, position)
            VALUES ($1, $2, $3)
            ON CONFLICT (path_id, course_id) DO UPDATE SET position = EXCLUDED.position
            "#,
        )
        .bind(path_id)
        .bind(course_id)
        .bind(position)
        .execute(&self.pool)
        .await
        .map_err(|e| match e {
            sqlx::Error::Database(db) if db.is_unique_violation() => {
                RepoError::UniqueViolation(db.constraint().unwrap_or("unknown").to_string())
            }
            other => RepoError::Database(other),
        })?;
        Ok(())
    }

    /// Ordered course ids in a path, with per-user progress percent.
    pub async fn path_courses(
        &self,
        path_id: Uuid,
        user_id: Uuid,
    ) -> Result<Vec<(Uuid, i32, i64)>, RepoError> {
        let rows: Vec<(Uuid, i32, i64)> = sqlx::query_as(
            r#"
            SELECT lpc.course_id, lpc.position,
                   (
                       SELECT count(*) FROM lesson_progress lp
                       JOIN lessons l ON l.id = lp.lesson_id
                       JOIN course_modules m ON m.id = l.module_id
                       WHERE m.course_id = lpc.course_id AND lp.user_id = $2 AND lp.completed
                   ) AS completed_lessons
            FROM learning_path_courses lpc
            WHERE lpc.path_id = $1
            ORDER BY lpc.position
            "#,
        )
        .bind(path_id)
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }
}
