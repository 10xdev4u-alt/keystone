//! Content API — the canonical spine: posts, versions, comments, reactions,
//! bookmarks, tags.
//!
//! Authorization model (mirrors the Month-3 authorization matrix):
//!   - Writes require authentication.
//!   - Own content: the author may edit/delete their own posts, comments.
//!   - Staff (moderator / admin / super_admin) may edit/delete any content.
//!   - Reads respect visibility: `public` for everyone, `unlisted`/`private`
//!     for the author and staff only.
//!   - Every `/{id}` route re-checks ownership — no trusting client IDs.

use crate::auth::{audit, map_repo_error, AuthUser};
use crate::error::{ApiError, ApiResult};
use crate::AppState;
use axum::extract::{FromRequestParts, Path, Query, State};
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::Json;
use keystone_db::repositories::bookmarks::Bookmarks;
use keystone_db::repositories::comments::{Comments, NewComment};
use keystone_db::repositories::posts::{NewPost, PostUpdate, Posts};
use keystone_db::repositories::reactions::Reactions;
use keystone_db::repositories::tags::Tags;
use serde::Deserialize;
use serde_json::{json, Value};
use std::convert::Infallible;
use uuid::Uuid;

// ── Validation limits (content abuse controls) ─────────────────────────────

const TITLE_MAX: usize = 200;
const SUMMARY_MAX: usize = 500;
const BODY_MAX: usize = 50_000;
const COMMENT_MAX: usize = 10_000;
const TAG_MAX: usize = 20;
const SLUG_SUFFIX_TRIES: u32 = 100;

fn validate_text(value: &str, what: &str, max: usize) -> Result<(), ApiError> {
    if value.trim().is_empty() {
        return Err(ApiError::BadRequest(format!("{what} must not be empty")));
    }
    if value.chars().count() > max {
        return Err(ApiError::BadRequest(format!(
            "{what} exceeds {max} characters"
        )));
    }
    Ok(())
}

fn validate_optional(value: Option<&str>, what: &str, max: usize) -> Result<(), ApiError> {
    if let Some(v) = value {
        validate_text(v, what, max)?;
    }
    Ok(())
}

fn is_staff(role: &str) -> bool {
    matches!(role, "moderator" | "admin" | "super_admin")
}

/// Slug from a title: lowercase ASCII alphanumerics and dashes. Empty input
/// (no title) yields an empty string — callers fall back to a random id.
fn slugify(title: &str) -> String {
    let mut out = String::with_capacity(title.len());
    let mut last_dash = false;
    for ch in title.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash && !out.is_empty() {
            out.push('-');
            last_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// Build a unique slug: slugify the title, or fall back to a short random id,
/// then retry with `-2`, `-3`, … on collision until the DB accepts it.
async fn unique_slug(
    posts: &Posts,
    author_id: Uuid,
    kind: &str,
    title: Option<&str>,
    body: &str,
    summary: Option<&str>,
    visibility: &str,
) -> Result<keystone_db::repositories::posts::Post, ApiError> {
    let base = title
        .map(slugify)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| Uuid::new_v4().simple().to_string()[..8].to_owned());
    let mut slug = base.clone();
    for attempt in 1..=SLUG_SUFFIX_TRIES {
        match posts
            .create(NewPost {
                author_id,
                kind,
                title,
                slug: &slug,
                body,
                summary,
                visibility,
            })
            .await
        {
            Ok(post) => return Ok(post),
            Err(keystone_db::repositories::RepoError::UniqueViolation(_)) => {
                slug = format!("{base}-{attempt}");
            }
            Err(e) => return Err(crate::auth::map_repo_error(e)),
        }
    }
    Err(ApiError::Conflict(
        "could not allocate a unique slug".into(),
    ))
}

// ── Request / response types ───────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreatePostRequest {
    pub kind: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default = "default_visibility")]
    pub visibility: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

fn default_visibility() -> String {
    "public".to_owned()
}

#[derive(Debug, Deserialize)]
pub struct UpdatePostRequest {
    #[serde(default)]
    pub title: Option<String>,
    pub body: String,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub change_note: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct CreateCommentRequest {
    pub body: String,
    #[serde(default)]
    pub parent_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct SetReactionRequest {
    pub kind: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateReportRequest {
    pub entity_type: String,
    pub entity_id: Uuid,
    pub reason: String,
    #[serde(default)]
    pub detail: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ResolveReportRequest {
    pub resolution_note: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpsertReviewRequest {
    pub entity_type: String,
    pub entity_id: Uuid,
    pub rating: i16,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PostQuery {
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub author: Option<Uuid>,
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
}

fn default_limit() -> i64 {
    20
}

#[derive(Debug, Deserialize)]
pub struct ReviewQuery {
    pub entity_type: String,
    pub entity_id: Uuid,
}

/// Optional auth — public reads that include the caller's own state when a
/// token is present. Missing/expired tokens are simply anonymous.
pub struct MaybeUser(pub Option<AuthUser>);

impl FromRequestParts<AppState> for MaybeUser {
    type Rejection = Infallible;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        Ok(MaybeUser(
            AuthUser::from_request_parts(parts, state).await.ok(),
        ))
    }
}

// ── Post view (no counters for single read; counts come from the view) ─────

fn post_view(post: &keystone_db::repositories::posts::Post) -> Value {
    json!({
        "id": post.id.to_string(),
        "author_id": post.author_id.to_string(),
        "kind": post.kind,
        "title": post.title,
        "slug": post.slug,
        "body": post.body,
        "summary": post.summary,
        "status": post.status,
        "visibility": post.visibility,
        "view_count": post.view_count,
        "published_at": post.published_at,
        "created_at": post.created_at,
        "updated_at": post.updated_at,
    })
}

/// The read may only see the post when its visibility allows.
fn can_read(auth: Option<&AuthUser>, post: &keystone_db::repositories::posts::Post) -> bool {
    match post.visibility.as_str() {
        "public" => true,
        "unlisted" | "private" => match auth {
            Some(u) => u.user_id == post.author_id || is_staff(&u.role),
            None => false,
        },
        _ => false,
    }
}

// ── Handlers: posts ────────────────────────────────────────────────────────

pub async fn create_post(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(req): Json<CreatePostRequest>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    if !matches!(req.kind.as_str(), "article" | "post" | "question" | "poll") {
        return Err(ApiError::BadRequest("unknown post kind".into()));
    }
    if !matches!(req.visibility.as_str(), "public" | "unlisted" | "private") {
        return Err(ApiError::BadRequest("unknown visibility".into()));
    }
    validate_optional(req.title.as_deref(), "title", TITLE_MAX)?;
    validate_optional(req.summary.as_deref(), "summary", SUMMARY_MAX)?;
    validate_text(&req.body, "body", BODY_MAX)?;
    if req.tags.len() > TAG_MAX {
        return Err(ApiError::BadRequest(format!(
            "at most {TAG_MAX} tags allowed"
        )));
    }

    let posts = Posts::new(state.pool.clone());
    let post = unique_slug(
        &posts,
        auth_user.user_id,
        &req.kind,
        req.title.as_deref(),
        &req.body,
        req.summary.as_deref(),
        &req.visibility,
    )
    .await?;

    // Attach tags after creation — a bad tag name must not roll back the post.
    let tags = Tags::new(state.pool.clone());
    for name in &req.tags {
        let tag = tags
            .ensure(name, &slugify(name))
            .await
            .map_err(map_repo_error)?;
        tags.attach(post.id, tag.id).await.map_err(map_repo_error)?;
    }

    audit(
        &state.pool,
        auth_user.user_id,
        "post_created",
        "post",
        &post.id.to_string(),
        None,
    )
    .await;

    Ok((
        StatusCode::CREATED,
        Json(json!({ "post": post_view(&post) })),
    ))
}

pub async fn get_post(
    State(state): State<AppState>,
    maybe: MaybeUser,
    Path(key): Path<String>,
) -> ApiResult<Json<Value>> {
    // Canonical URLs use the slug; UUID reads are accepted too. Both ride the
    // same `{id}` path slot so it never conflicts with the UUID routes below.
    let posts = Posts::new(state.pool.clone());
    let post = match Uuid::parse_str(&key) {
        Ok(id) => posts.get_by_id(id).await.map_err(map_repo_error)?,
        Err(_) => posts.get_by_slug(&key).await.map_err(map_repo_error)?,
    }
    .ok_or(ApiError::NotFound)?;
    if !can_read(maybe.0.as_ref(), &post) {
        return Err(ApiError::NotFound); // hide existence of unlisted/private
    }
    Ok(Json(json!({ "post": post_view(&post) })))
}

pub async fn list_posts(
    State(state): State<AppState>,
    Query(query): Query<PostQuery>,
) -> ApiResult<Json<Value>> {
    let limit = query.limit.clamp(1, 50);
    let offset = query.offset.max(0);
    let posts = Posts::new(state.pool.clone());
    let rows = posts
        .list(query.kind.as_deref(), query.author, limit, offset)
        .await
        .map_err(map_repo_error)?;
    let items: Vec<Value> = rows
        .into_iter()
        .map(|row| {
            json!({
                "id": row.post.id.to_string(),
                "author_id": row.post.author_id.to_string(),
                "kind": row.post.kind,
                "title": row.post.title,
                "slug": row.post.slug,
                "summary": row.post.summary,
                "visibility": row.post.visibility,
                "view_count": row.post.view_count,
                "comment_count": row.comment_count,
                "reaction_count": row.reaction_count,
                "bookmark_count": row.bookmark_count,
                "published_at": row.post.published_at,
                "created_at": row.post.created_at,
            })
        })
        .collect();
    Ok(Json(
        json!({ "posts": items, "limit": limit, "offset": offset }),
    ))
}

pub async fn update_post(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdatePostRequest>,
) -> ApiResult<Json<Value>> {
    validate_optional(req.title.as_deref(), "title", TITLE_MAX)?;
    validate_optional(req.summary.as_deref(), "summary", SUMMARY_MAX)?;
    validate_text(&req.body, "body", BODY_MAX)?;

    let posts = Posts::new(state.pool.clone());
    let existing = posts
        .get_by_id(id)
        .await
        .map_err(map_repo_error)?
        .ok_or(ApiError::NotFound)?;
    if existing.author_id != auth_user.user_id && !is_staff(&auth_user.role) {
        return Err(ApiError::Forbidden);
    }

    let post = posts
        .update(
            id,
            PostUpdate {
                title: req.title.as_deref(),
                body: &req.body,
                summary: req.summary.as_deref(),
                change_note: req.change_note.as_deref(),
                editor_id: auth_user.user_id,
            },
        )
        .await
        .map_err(map_repo_error)?
        .ok_or(ApiError::NotFound)?;

    if let Some(tags) = req.tags {
        let tags_repo = Tags::new(state.pool.clone());
        for name in tags {
            if name.trim().is_empty() {
                continue;
            }
            let tag = tags_repo
                .ensure(&name, &slugify(&name))
                .await
                .map_err(map_repo_error)?;
            tags_repo.attach(id, tag.id).await.map_err(map_repo_error)?;
        }
    }

    audit(
        &state.pool,
        auth_user.user_id,
        "post_updated",
        "post",
        &id.to_string(),
        None,
    )
    .await;

    Ok(Json(json!({ "post": post_view(&post) })))
}

pub async fn delete_post(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let posts = Posts::new(state.pool.clone());
    let existing = posts
        .get_by_id(id)
        .await
        .map_err(map_repo_error)?
        .ok_or(ApiError::NotFound)?;
    if existing.author_id != auth_user.user_id && !is_staff(&auth_user.role) {
        return Err(ApiError::Forbidden);
    }
    posts
        .soft_delete(id)
        .await
        .map_err(map_repo_error)?
        .ok_or(ApiError::NotFound)?;
    audit(
        &state.pool,
        auth_user.user_id,
        "post_deleted",
        "post",
        &id.to_string(),
        None,
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn post_versions(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    let posts = Posts::new(state.pool.clone());
    // History survives soft delete by design — the author lookup must be
    // deleted-blind so owners and staff can audit even deleted content.
    let author = posts
        .author_of(id)
        .await
        .map_err(map_repo_error)?
        .ok_or(ApiError::NotFound)?;
    if author != auth_user.user_id && !is_staff(&auth_user.role) {
        return Err(ApiError::Forbidden);
    }
    let versions = posts.versions(id).await.map_err(map_repo_error)?;
    let items: Vec<Value> = versions
        .into_iter()
        .map(|v| {
            json!({
                "id": v.id.to_string(),
                "title": v.title,
                "summary": v.summary,
                "change_note": v.change_note,
                "editor_id": v.editor_id.to_string(),
                "created_at": v.created_at,
            })
        })
        .collect();
    Ok(Json(json!({ "versions": items })))
}

pub async fn record_view(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let posts = Posts::new(state.pool.clone());
    if posts.get_by_id(id).await.map_err(map_repo_error)?.is_none() {
        return Err(ApiError::NotFound);
    }
    posts.increment_view(id).await.map_err(map_repo_error)?;
    Ok(StatusCode::NO_CONTENT)
}

// ── Handlers: comments ─────────────────────────────────────────────────────

pub async fn create_comment(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(post_id): Path<Uuid>,
    Json(req): Json<CreateCommentRequest>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    validate_text(&req.body, "comment body", COMMENT_MAX)?;
    let comments = Comments::new(state.pool.clone());
    let comment = comments
        .create(NewComment {
            post_id,
            author_id: auth_user.user_id,
            parent_id: req.parent_id,
            body: &req.body,
        })
        .await
        .map_err(map_repo_error)?;
    audit(
        &state.pool,
        auth_user.user_id,
        "comment_created",
        "comment",
        &comment.id.to_string(),
        None,
    )
    .await;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "comment": {
                "id": comment.id.to_string(),
                "post_id": comment.post_id.to_string(),
                "parent_id": comment.parent_id.map(|p| p.to_string()),
                "body": comment.body,
                "created_at": comment.created_at,
            }
        })),
    ))
}

pub async fn list_comments(
    State(state): State<AppState>,
    Path(post_id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    let comments = Comments::new(state.pool.clone());
    let rows = comments
        .list_by_post(post_id)
        .await
        .map_err(map_repo_error)?;
    let items: Vec<Value> = rows
        .into_iter()
        .map(|c| {
            json!({
                "id": c.id.to_string(),
                "post_id": c.post_id.to_string(),
                "parent_id": c.parent_id.map(|p| p.to_string()),
                "author_id": c.author_id.to_string(),
                "body": c.body,
                "created_at": c.created_at,
            })
        })
        .collect();
    Ok(Json(json!({ "comments": items })))
}

pub async fn delete_comment(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let comments = Comments::new(state.pool.clone());
    let existing = comments
        .get_by_id(id)
        .await
        .map_err(map_repo_error)?
        .ok_or(ApiError::NotFound)?;
    if existing.author_id != auth_user.user_id && !is_staff(&auth_user.role) {
        return Err(ApiError::Forbidden);
    }
    comments
        .soft_delete(id)
        .await
        .map_err(map_repo_error)?
        .ok_or(ApiError::NotFound)?;
    audit(
        &state.pool,
        auth_user.user_id,
        "comment_deleted",
        "comment",
        &id.to_string(),
        None,
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

// ── Handlers: reactions ────────────────────────────────────────────────────

pub async fn set_reaction(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(post_id): Path<Uuid>,
    Json(req): Json<SetReactionRequest>,
) -> ApiResult<Json<Value>> {
    if !matches!(
        req.kind.as_str(),
        "like" | "love" | "laugh" | "celebrate" | "insightful" | "curious"
    ) {
        return Err(ApiError::BadRequest("unknown reaction kind".into()));
    }
    let reactions = Reactions::new(state.pool.clone());
    let reaction = reactions
        .set(post_id, auth_user.user_id, &req.kind)
        .await
        .map_err(map_repo_error)?;
    Ok(Json(json!({
        "reaction": {
            "id": reaction.id.to_string(),
            "post_id": reaction.post_id.to_string(),
            "kind": reaction.kind,
        }
    })))
}

pub async fn remove_reaction(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(post_id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let reactions = Reactions::new(state.pool.clone());
    reactions
        .remove(post_id, auth_user.user_id)
        .await
        .map_err(map_repo_error)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn get_reactions(
    State(state): State<AppState>,
    maybe: MaybeUser,
    Path(post_id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    let reactions = Reactions::new(state.pool.clone());
    let posts = Posts::new(state.pool.clone());
    if posts
        .get_by_id(post_id)
        .await
        .map_err(map_repo_error)?
        .is_none()
    {
        return Err(ApiError::NotFound);
    }
    // Counts by kind via the repo's row set (small, capped by unique per user).
    let rows = sqlx::query_as::<_, (String, i64)>(
        "SELECT kind, count(*) FROM reactions WHERE post_id = $1 GROUP BY kind",
    )
    .bind(post_id)
    .fetch_all(&state.pool)
    .await
    .map_err(ApiError::from)?;
    let total: i64 = rows.iter().map(|(_, n)| n).sum();
    let mine = match maybe.0 {
        Some(u) => reactions
            .get(post_id, u.user_id)
            .await
            .map_err(map_repo_error)?
            .map(|r| r.kind),
        None => None,
    };
    Ok(Json(
        json!({ "total": total, "by_kind": rows, "mine": mine }),
    ))
}

// ── Handlers: bookmarks ────────────────────────────────────────────────────

pub async fn add_bookmark(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(post_id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let bookmarks = Bookmarks::new(state.pool.clone());
    bookmarks
        .add(auth_user.user_id, post_id)
        .await
        .map_err(map_repo_error)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn remove_bookmark(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(post_id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let bookmarks = Bookmarks::new(state.pool.clone());
    bookmarks
        .remove(auth_user.user_id, post_id)
        .await
        .map_err(map_repo_error)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn my_bookmarks(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> ApiResult<Json<Value>> {
    let bookmarks = Bookmarks::new(state.pool.clone());
    let ids = bookmarks
        .post_ids_for_user(auth_user.user_id)
        .await
        .map_err(map_repo_error)?;
    Ok(Json(json!({
        "post_ids": ids.iter().map(|id| id.to_string()).collect::<Vec<_>>()
    })))
}
