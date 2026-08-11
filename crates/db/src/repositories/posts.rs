//! Post repository — the content spine.
//!
//! Every create/update writes a `post_versions` snapshot (version 1 on
//! create), so history is real. Reads filter `deleted_at IS NULL` and join
//! the maintained `post_counts` view — counters never drift.

use super::RepoError;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

/// A post row (no counters).
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct Post {
    pub id: Uuid,
    pub author_id: Uuid,
    pub kind: String,
    pub title: Option<String>,
    pub slug: String,
    pub body: String,
    pub summary: Option<String>,
    pub cover_image_url: Option<String>,
    pub status: String,
    pub visibility: String,
    pub view_count: i64,
    pub published_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A post joined with its derived counters.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct PostWithCounts {
    #[sqlx(flatten)]
    pub post: Post,
    pub comment_count: i64,
    pub reaction_count: i64,
    pub bookmark_count: i64,
}

/// One page of the feed. The cursor is the `(created_at, id)` of the last
/// returned row — pass it as `before` for the next page. Keyset pagination
/// is O(page) and stable under concurrent inserts (OFFSET skips duplicates).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostPage {
    pub posts: Vec<PostWithCounts>,
    pub next_cursor: Option<(DateTime<Utc>, Uuid)>,
}

/// One row of real version history.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct PostVersion {
    pub id: Uuid,
    pub post_id: Uuid,
    pub title: Option<String>,
    pub body: String,
    pub summary: Option<String>,
    pub editor_id: Uuid,
    pub change_note: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewPost<'a> {
    pub author_id: Uuid,
    pub kind: &'a str,
    pub title: Option<&'a str>,
    pub slug: &'a str,
    pub body: &'a str,
    pub summary: Option<&'a str>,
    pub cover_image_url: Option<&'a str>,
    pub visibility: &'a str,
}

#[derive(Debug, Clone)]
pub struct PostUpdate<'a> {
    pub title: Option<&'a str>,
    pub body: &'a str,
    pub summary: Option<&'a str>,
    pub cover_image_url: Option<&'a str>,
    pub change_note: Option<&'a str>,
    pub editor_id: Uuid,
}

#[derive(Debug, Clone)]
pub struct Posts {
    pool: PgPool,
}

impl Posts {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Create a post plus its initial version snapshot, atomically.
    /// Slug collisions surface as [`RepoError::UniqueViolation`] with the
    /// `posts_slug_key` constraint — callers retry with a suffixed slug.
    pub async fn create(&self, new_post: NewPost<'_>) -> Result<Post, RepoError> {
        let mut tx = self.pool.begin().await?;
        let post = sqlx::query_as::<_, Post>(
            r#"
            INSERT INTO posts (author_id, kind, title, slug, body, summary,
                               cover_image_url, status, visibility, published_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, 'published', $8, now())
            RETURNING id, author_id, kind, title, slug, body, summary,
                      cover_image_url, status, visibility, view_count,
                      published_at, created_at, updated_at
            "#,
        )
        .bind(new_post.author_id)
        .bind(new_post.kind)
        .bind(new_post.title)
        .bind(new_post.slug)
        .bind(new_post.body)
        .bind(new_post.summary)
        .bind(new_post.cover_image_url)
        .bind(new_post.visibility)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| match e {
            sqlx::Error::Database(db) if db.is_unique_violation() => {
                RepoError::UniqueViolation(db.constraint().unwrap_or("unknown").to_string())
            }
            other => RepoError::Database(other),
        })?;

        // Version 1 is the initial content — history starts on day one.
        sqlx::query(
            r#"
            INSERT INTO post_versions (post_id, title, body, summary, editor_id)
            VALUES ($1, $2, $3, $4, $5)
            "#,
        )
        .bind(post.id)
        .bind(new_post.title)
        .bind(new_post.body)
        .bind(new_post.summary)
        .bind(new_post.author_id)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(post)
    }

    /// Fetch a live post by slug (canonical URL lookup).
    pub async fn get_by_slug(&self, slug: &str) -> Result<Option<Post>, RepoError> {
        let post = sqlx::query_as::<_, Post>(
            r#"
            SELECT id, author_id, kind, title, slug, body, summary,
                   cover_image_url, status, visibility, view_count, published_at,
                   created_at, updated_at
            FROM posts
            WHERE slug = $1 AND deleted_at IS NULL
            "#,
        )
        .bind(slug)
        .fetch_optional(&self.pool)
        .await?;
        Ok(post)
    }

    /// Fetch a live post by id.
    pub async fn get_by_id(&self, id: Uuid) -> Result<Option<Post>, RepoError> {
        let post = sqlx::query_as::<_, Post>(
            r#"
            SELECT id, author_id, kind, title, slug, body, summary,
                   cover_image_url, status, visibility, view_count, published_at,
                   created_at, updated_at
            FROM posts
            WHERE id = $1 AND deleted_at IS NULL
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(post)
    }

    /// Published posts, newest first, with derived counters.
    ///
    /// Keyset pagination: pass the previous page's `next_cursor` as `before`
    /// to fetch strictly older rows. `limit + 1` rows are fetched so the page
    /// knows whether more exist. The `(created_at, id)` tuple ordering makes
    /// the sort total — no two posts share the same cursor position.
    pub async fn list(
        &self,
        kind: Option<&str>,
        author_id: Option<Uuid>,
        limit: i64,
        before: Option<(DateTime<Utc>, Uuid)>,
    ) -> Result<PostPage, RepoError> {
        let mut sql = String::from(
            r#"
            SELECT p.id, p.author_id, p.kind, p.title, p.slug, p.body, p.summary,
                   p.cover_image_url, p.status, p.visibility, p.view_count,
                   p.published_at, p.created_at, p.updated_at,
                   pc.comment_count, pc.reaction_count, pc.bookmark_count
            FROM posts p
            JOIN post_counts pc ON pc.post_id = p.id
            WHERE p.deleted_at IS NULL AND p.status = 'published'
            "#,
        );
        let mut next = 1usize;
        if kind.is_some() {
            sql.push_str(&format!(" AND p.kind = ${next}"));
            next += 1;
        }
        if author_id.is_some() {
            sql.push_str(&format!(" AND p.author_id = ${next}"));
            next += 1;
        }
        if before.is_some() {
            sql.push_str(&format!(
                " AND (p.created_at, p.id) < (${next}, ${})",
                next + 1
            ));
            next += 2;
        }
        sql.push_str(&format!(
            " ORDER BY p.created_at DESC, p.id DESC LIMIT ${next}"
        ));

        let mut query = sqlx::query_as::<_, PostWithCounts>(&sql);
        if let Some(k) = kind {
            query = query.bind(k);
        }
        if let Some(a) = author_id {
            query = query.bind(a);
        }
        if let Some((ts, id)) = before {
            query = query.bind(ts).bind(id);
        }
        let mut rows = query.bind(limit + 1).fetch_all(&self.pool).await?;

        let has_more = rows.len() > limit as usize;
        rows.truncate(limit as usize);
        let next_cursor = has_more.then(|| {
            let last = rows.last().expect("has_more implies non-empty page");
            (last.post.created_at, last.post.id)
        });
        Ok(PostPage {
            posts: rows,
            next_cursor,
        })
    }

    /// Lock a discussion: new comments are refused by callers while
    /// `locked_at` is set. Answers whether a live post was locked.
    /// Find published, public posts (excluding `id`) that share at least one
    /// tag, ranked by shared-tag count then recency. Used for the editorial
    /// "related reading" rail.
    ///
    /// Grouped (not `DISTINCT ON`) on purpose: `DISTINCT ON` forces the
    /// grouping column to be the leftmost `ORDER BY` key, which would sort the
    /// whole result by post id and make the ranking arbitrary.
    pub async fn related(&self, id: Uuid, limit: i64) -> Result<Vec<Post>, RepoError> {
        let rows = sqlx::query_as::<_, Post>(
            r#"
            SELECT p.id, p.author_id, p.kind, p.title, p.slug, p.body, p.summary,
                   p.cover_image_url, p.status, p.visibility, p.view_count,
                   p.published_at, p.created_at, p.updated_at
            FROM posts p
            JOIN post_tags pt ON pt.post_id = p.id
            WHERE p.id <> $1
              AND p.status = 'published'
              AND p.visibility = 'public'
              AND p.deleted_at IS NULL
              AND pt.tag_id IN (
                  SELECT tag_id FROM post_tags WHERE post_id = $1
              )
            GROUP BY p.id
            ORDER BY count(*) DESC, p.created_at DESC, p.id
            LIMIT $2
            "#,
        )
        .bind(id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(RepoError::from)?;
        Ok(rows)
    }

    pub async fn lock(&self, id: Uuid) -> Result<bool, RepoError> {
        let result =
            sqlx::query("UPDATE posts SET locked_at = now() WHERE id = $1 AND deleted_at IS NULL")
                .bind(id)
                .execute(&self.pool)
                .await?;
        Ok(result.rows_affected() == 1)
    }

    /// Unlock a discussion. Answers whether a live post was unlocked.
    pub async fn unlock(&self, id: Uuid) -> Result<bool, RepoError> {
        let result =
            sqlx::query("UPDATE posts SET locked_at = NULL WHERE id = $1 AND deleted_at IS NULL")
                .bind(id)
                .execute(&self.pool)
                .await?;
        Ok(result.rows_affected() == 1)
    }

    /// Whether the post is currently locked for new comments.
    pub async fn is_locked(&self, id: Uuid) -> Result<bool, RepoError> {
        let locked = sqlx::query_scalar::<_, Option<chrono::DateTime<chrono::Utc>>>(
            "SELECT locked_at FROM posts WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(locked.flatten().is_some())
    }

    /// Soft delete: `deleted_at` + status flip. Hard deletes are not exposed.
    pub async fn soft_delete(&self, id: Uuid) -> Result<Option<()>, RepoError> {
        let result = sqlx::query(
            r#"
            UPDATE posts SET status = 'deleted', deleted_at = now()
            WHERE id = $1 AND deleted_at IS NULL
            "#,
        )
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok((result.rows_affected() == 1).then_some(()))
    }

    /// Update content and append a version snapshot, atomically.
    pub async fn update(
        &self,
        id: Uuid,
        update: PostUpdate<'_>,
    ) -> Result<Option<Post>, RepoError> {
        let mut tx = self.pool.begin().await?;
        let post = sqlx::query_as::<_, Post>(
            r#"
            UPDATE posts
            SET title = $2, body = $3, summary = $4, cover_image_url = $5
            WHERE id = $1 AND deleted_at IS NULL
            RETURNING id, author_id, kind, title, slug, body, summary,
                      cover_image_url, status, visibility, view_count,
                      published_at, created_at, updated_at
            "#,
        )
        .bind(id)
        .bind(update.title)
        .bind(update.body)
        .bind(update.summary)
        .bind(update.cover_image_url)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(post) = post else {
            return Ok(None);
        };

        sqlx::query(
            r#"
            INSERT INTO post_versions (post_id, title, body, summary, editor_id, change_note)
            VALUES ($1, $2, $3, $4, $5, $6)
            "#,
        )
        .bind(id)
        .bind(update.title)
        .bind(update.body)
        .bind(update.summary)
        .bind(update.editor_id)
        .bind(update.change_note)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(Some(post))
    }

    /// Transactional view counter increment — the one counter that is not
    /// derivable and is therefore a column, incremented here and nowhere else.
    pub async fn increment_view(&self, id: Uuid) -> Result<(), RepoError> {
        sqlx::query("UPDATE posts SET view_count = view_count + 1 WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Author of a post even when soft-deleted — history and audit access
    /// must survive deletion, so ownership checks use this, not [`get_by_id`].
    pub async fn author_of(&self, id: Uuid) -> Result<Option<Uuid>, RepoError> {
        let author = sqlx::query_scalar("SELECT author_id FROM posts WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(author)
    }

    /// Full version history, newest first.
    pub async fn versions(&self, post_id: Uuid) -> Result<Vec<PostVersion>, RepoError> {
        let rows = sqlx::query_as::<_, PostVersion>(
            r#"
            SELECT id, post_id, title, body, summary, editor_id, change_note, created_at
            FROM post_versions
            WHERE post_id = $1
            ORDER BY created_at DESC
            "#,
        )
        .bind(post_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }
}
