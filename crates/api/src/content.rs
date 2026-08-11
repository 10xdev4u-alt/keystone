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
use chrono::{DateTime, Utc};
use keystone_db::repositories::bookmarks::Bookmarks;
use keystone_db::repositories::comments::{Comments, NewComment};
use keystone_db::repositories::posts::{NewPost, PostUpdate, Posts};
use keystone_db::repositories::reactions::Reactions;
use keystone_db::repositories::tags::Tags;
use serde::Deserialize;
use serde_json::{json, Value};
use std::convert::Infallible;
use utoipa::ToSchema;
use uuid::Uuid;

// ── Validation limits (content abuse controls) ─────────────────────────────

const TITLE_MAX: usize = 200;
const SUMMARY_MAX: usize = 500;
const BODY_MAX: usize = 50_000;
/// Cover art URLs ride as opaque metadata; 2048 keeps space for long
/// signed-object URLs (S3 presigned paths can run several hundred chars).
const COVER_URL_MAX: usize = 2048;
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
pub(crate) fn slugify(title: &str) -> String {
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
    cover_image_url: Option<&str>,
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
                cover_image_url,
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

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreatePostRequest {
    pub kind: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub summary: Option<String>,
    /// Cover art URL — optional presentation metadata. Not an inline
    /// resource: readers always load it with `referrerpolicy` + lazy
    /// loading, and the value is opaque to the renderer.
    #[serde(default)]
    pub cover_image_url: Option<String>,
    #[serde(default = "default_visibility")]
    pub visibility: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

fn default_visibility() -> String {
    "public".to_owned()
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdatePostRequest {
    #[serde(default)]
    pub title: Option<String>,
    pub body: String,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub cover_image_url: Option<String>,
    #[serde(default)]
    pub change_note: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateCommentRequest {
    pub body: String,
    #[serde(default)]
    pub parent_id: Option<Uuid>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SetReactionRequest {
    pub kind: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateReportRequest {
    pub entity_type: String,
    pub entity_id: Uuid,
    pub reason: String,
    #[serde(default)]
    pub detail: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ResolveReportRequest {
    pub resolution_note: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpsertReviewRequest {
    pub entity_type: String,
    pub entity_id: Uuid,
    pub rating: i16,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PostQuery {
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub author: Option<Uuid>,
    #[serde(default = "default_limit")]
    pub limit: i64,
    /// Keyset cursor — opaque to clients, emitted as `next_cursor` by the
    /// previous page. Replaces OFFSET, which degrades and duplicates under
    /// concurrent inserts.
    #[serde(default)]
    pub before: Option<String>,
}

/// `{created_at_micros}:{id}` — microseconds match Postgres `timestamptz`
/// exactly, and the UUID's hyphens never contain `:`, so splitting on the
/// LAST `:` is unambiguous. The cursor is URL-safe (no `+`/`/`/`=`), so
/// clients can pass it back in a query string verbatim.
fn parse_cursor(raw: &str) -> Result<(DateTime<Utc>, Uuid), ApiError> {
    let (ts, id) = raw.rsplit_once(':').ok_or_else(|| {
        ApiError::BadRequest("invalid pagination cursor: expected micros:uuid".into())
    })?;
    let micros = ts
        .parse::<i64>()
        .map_err(|_| ApiError::BadRequest("invalid pagination cursor: bad timestamp".into()))?;
    let created_at = DateTime::from_timestamp_micros(micros)
        .ok_or_else(|| ApiError::BadRequest("invalid pagination cursor: bad timestamp".into()))?
        .with_timezone(&Utc);
    let id = Uuid::parse_str(id)
        .map_err(|_| ApiError::BadRequest("invalid pagination cursor: bad id".into()))?;
    Ok((created_at, id))
}

fn default_limit() -> i64 {
    20
}

#[derive(Debug, Deserialize, ToSchema)]
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

// ── Post view (no counters for single read; counts come from the view) ─────/// Full post — the reader view contract.
#[derive(Debug, serde::Serialize, ToSchema)]
pub struct PostView {
    pub id: String,
    pub author_id: String,
    pub kind: String,
    pub title: String,
    pub slug: String,
    pub body: String,
    /// Markdown rendered to sanitized HTML server-side (see `keystone_db::markdown`).
    pub body_html: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Cover art URL — optional presentation metadata, loaded lazily and
    /// without referrer by the reader.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cover_image_url: Option<String>,
    pub status: String,
    pub visibility: String,
    pub view_count: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub published_at: Option<String>,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

fn post_view(post: &keystone_db::repositories::posts::Post) -> PostView {
    PostView {
        id: post.id.to_string(),
        author_id: post.author_id.to_string(),
        kind: post.kind.clone(),
        title: post.title.clone().unwrap_or_default(),
        slug: post.slug.clone(),
        body: post.body.clone(),
        body_html: keystone_db::markdown::render(&post.body),
        summary: post.summary.clone(),
        cover_image_url: post.cover_image_url.clone(),
        status: post.status.clone(),
        visibility: post.visibility.clone(),
        view_count: post.view_count,
        published_at: post.published_at.map(|t| t.to_rfc3339()),

        created_at: post.created_at.to_rfc3339(),
        updated_at: Some(post.updated_at.to_rfc3339()),
    }
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

#[tracing::instrument(skip(state, auth_user), fields(actor = %auth_user.user_id, kind = %req.kind))]
/// Create a post (article / post / question / poll / discussion).
#[utoipa::path(
    post,
    path = "/api/v1/posts",
    request_body = CreatePostRequest,
    responses(
        (status = 201, description = "Post created", body = Value),
        (status = 400, description = "Invalid kind, title or body"),
        (status = 401, description = "Missing or invalid access token"),
    ),
    security(("bearer_auth" = [])),
    tag = "content"
)]
pub async fn create_post(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(req): Json<CreatePostRequest>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    if !matches!(
        req.kind.as_str(),
        "article" | "post" | "question" | "poll" | "discussion"
    ) {
        return Err(ApiError::BadRequest("unknown post kind".into()));
    }
    if !matches!(req.visibility.as_str(), "public" | "unlisted" | "private") {
        return Err(ApiError::BadRequest("unknown visibility".into()));
    }
    validate_optional(req.title.as_deref(), "title", TITLE_MAX)?;
    validate_optional(req.summary.as_deref(), "summary", SUMMARY_MAX)?;
    validate_optional(
        req.cover_image_url.as_deref(),
        "cover image URL",
        COVER_URL_MAX,
    )?;
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
        req.cover_image_url.as_deref(),
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

    tracing::info!(post_id = %post.id, slug = %post.slug, "post created");
    Ok((
        StatusCode::CREATED,
        Json(json!({ "post": post_view(&post) })),
    ))
}

/// Fetch a post by slug or UUID. Unlisted/private posts require the author
/// or staff.
#[utoipa::path(
    get,
    path = "/api/v1/posts/{id}",
    params(("id" = String, Path, description = "Post slug or UUID")),
    responses(
        (status = 200, description = "Post with author, tags and stats", body = PostDetailResponse),
        (status = 404, description = "Post not found or not visible"),
    ),
    tag = "content"
)]
pub async fn get_post(
    State(state): State<AppState>,
    maybe: MaybeUser,
    Path(key): Path<String>,
) -> ApiResult<Json<PostDetailResponse>> {
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
    Ok(Json(PostDetailResponse {
        post: post_view(&post),
    }))
}

/// A related-reading card: same tag family, ranked by overlap then recency.
#[derive(Debug, serde::Serialize, ToSchema)]
pub struct RelatedPostView {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub slug: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub published_at: Option<String>,
}

/// Wrapper for the related-posts endpoint.
#[derive(Debug, serde::Serialize, ToSchema)]
pub struct RelatedPosts {
    pub posts: Vec<RelatedPostView>,
}

/// Related reading for a post — posts sharing tags, ranked by overlap.
#[utoipa::path(
    get,
    path = "/api/v1/posts/{id}/related",
    params(("id" = Uuid, Path, description = "Post id")),
    responses(
        (status = 200, description = "Related posts", body = RelatedPosts),
        (status = 404, description = "Post not found"),
    ),
    tag = "content"
)]
pub async fn get_related(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<RelatedPosts>> {
    let posts = Posts::new(state.pool.clone());
    let rows = posts.related(id, 6).await.map_err(map_repo_error)?;
    Ok(Json(RelatedPosts {
        posts: rows
            .into_iter()
            .map(|p| RelatedPostView {
                id: p.id.to_string(),
                kind: p.kind,
                title: p.title.unwrap_or_default(),
                slug: p.slug,
                summary: p.summary,
                published_at: p.published_at.map(|t| t.to_rfc3339()),
            })
            .collect(),
    }))
}

/// Wrapper for the single-post endpoint.
#[derive(Debug, serde::Serialize, ToSchema)]
pub struct PostDetailResponse {
    pub post: PostView,
}

/// A single comment in a thread.
#[derive(Debug, serde::Serialize, ToSchema)]
pub struct CommentView {
    pub id: String,
    pub post_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    pub author_id: String,
    pub body: String,
    pub created_at: String,
}

/// Comment thread for a post.
#[derive(Debug, serde::Serialize, ToSchema)]
pub struct CommentList {
    pub comments: Vec<CommentView>,
}

/// Single-comment create response.
#[derive(Debug, serde::Serialize, ToSchema)]
pub struct CommentResponse {
    pub comment: CommentView,
}

fn comment_view(c: &keystone_db::repositories::comments::Comment) -> CommentView {
    CommentView {
        id: c.id.to_string(),
        post_id: c.post_id.to_string(),
        parent_id: c.parent_id.map(|p| p.to_string()),
        author_id: c.author_id.to_string(),
        body: c.body.clone(),
        created_at: c.created_at.to_rfc3339(),
    }
}

/// A single post in a list page — the feed card contract.
#[derive(Debug, serde::Serialize, ToSchema)]
pub struct PostListItem {
    pub id: String,
    pub author_id: String,
    pub kind: String,
    pub title: String,
    pub slug: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub visibility: String,
    pub view_count: i64,
    pub comment_count: i64,
    pub reaction_count: i64,
    pub bookmark_count: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub published_at: Option<String>,
    pub created_at: String,
}

/// Keyset page of posts.
#[derive(Debug, serde::Serialize, ToSchema)]
pub struct PostListPage {
    pub posts: Vec<PostListItem>,
    pub limit: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// List posts with keyset pagination (before cursor from `next_cursor`).
#[utoipa::path(
    get,
    path = "/api/v1/posts",
    params(
        ("kind" = Option<String>, Query, description = "Filter by post kind"),
        ("author" = Option<Uuid>, Query, description = "Filter by author"),
        ("limit" = Option<i64>, Query, description = "Page size (1..=50)"),
        ("before" = Option<String>, Query, description = "Keyset cursor from the previous page"),
    ),
    responses(
        (status = 200, description = "Page of posts + next_cursor", body = PostListPage),
    ),
    tag = "content"
)]
pub async fn list_posts(
    State(state): State<AppState>,
    Query(query): Query<PostQuery>,
) -> ApiResult<Json<Value>> {
    let limit = query.limit.clamp(1, 50);
    let before = match query.before.as_deref() {
        None => None,
        Some(raw) => Some(parse_cursor(raw)?),
    };
    let posts = Posts::new(state.pool.clone());
    let page = posts
        .list(query.kind.as_deref(), query.author, limit, before)
        .await
        .map_err(map_repo_error)?;
    let next_cursor = page
        .next_cursor
        .map(|(ts, id)| format!("{}:{}", ts.timestamp_micros(), id));
    let items: Vec<Value> = page
        .posts
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
        json!({ "posts": items, "limit": limit, "next_cursor": next_cursor }),
    ))
}

#[tracing::instrument(skip(state, auth_user), fields(actor = %auth_user.user_id, post_id = %id))]
/// Update a post. Owner or staff only; every edit writes a version row.
#[utoipa::path(
    patch,
    path = "/api/v1/posts/{id}",
    request_body = UpdatePostRequest,
    params(("id" = Uuid, Path, description = "Post id")),
    responses(
        (status = 200, description = "Updated post", body = Value),
        (status = 401, description = "Missing or invalid access token"),
        (status = 403, description = "Not the author and not staff"),
        (status = 404, description = "Post not found"),
    ),
    security(("bearer_auth" = [])),
    tag = "content"
)]
pub async fn update_post(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdatePostRequest>,
) -> ApiResult<Json<Value>> {
    validate_optional(req.title.as_deref(), "title", TITLE_MAX)?;
    validate_optional(req.summary.as_deref(), "summary", SUMMARY_MAX)?;
    validate_optional(
        req.cover_image_url.as_deref(),
        "cover image URL",
        COVER_URL_MAX,
    )?;
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
                cover_image_url: req.cover_image_url.as_deref(),
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

    tracing::info!(post_id = %id, "post updated");
    Ok(Json(json!({ "post": post_view(&post) })))
}

#[tracing::instrument(skip(state, auth_user), fields(actor = %auth_user.user_id, post_id = %id))]
/// Soft-delete a post. Owner or staff only.
#[utoipa::path(
    delete,
    path = "/api/v1/posts/{id}",
    params(("id" = Uuid, Path, description = "Post id")),
    responses(
        (status = 204, description = "Post deleted"),
        (status = 401, description = "Missing or invalid access token"),
        (status = 403, description = "Not the author and not staff"),
        (status = 404, description = "Post not found"),
    ),
    security(("bearer_auth" = [])),
    tag = "content"
)]
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
    tracing::info!(post_id = %id, "post deleted");
    Ok(StatusCode::NO_CONTENT)
}

/// List a post's edit history (survives soft delete; owner or staff only).
#[utoipa::path(
    get,
    path = "/api/v1/posts/{id}/versions",
    params(("id" = Uuid, Path, description = "Post id")),
    responses(
        (status = 200, description = "Version history", body = Value),
        (status = 401, description = "Missing or invalid access token"),
        (status = 403, description = "Not the author and not staff"),
        (status = 404, description = "Post not found"),
    ),
    security(("bearer_auth" = [])),
    tag = "content"
)]
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

/// Record a read for analytics. Best-effort; anonymous views count too.
#[utoipa::path(
    post,
    path = "/api/v1/posts/{id}/view",
    params(("id" = Uuid, Path, description = "Post id")),
    responses(
        (status = 204, description = "View recorded"),
    ),
    tag = "content"
)]
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

#[tracing::instrument(skip(state, auth_user), fields(actor = %auth_user.user_id, post_id = %post_id))]
/// Add a comment to a post (threaded via `parent_id`). Locked posts refuse.
#[utoipa::path(
    post,
    path = "/api/v1/posts/{id}/comments",
    request_body = CreateCommentRequest,
    params(("id" = Uuid, Path, description = "Post id")),
    responses(
        (status = 201, description = "Comment created", body = CommentResponse),
        (status = 401, description = "Missing or invalid access token"),
        (status = 423, description = "Post is locked"),
        (status = 404, description = "Post not found"),
    ),
    security(("bearer_auth" = [])),
    tag = "content"
)]
pub async fn create_comment(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(post_id): Path<Uuid>,
    Json(req): Json<CreateCommentRequest>,
) -> ApiResult<(StatusCode, Json<CommentResponse>)> {
    validate_text(&req.body, "comment body", COMMENT_MAX)?;
    // Locked discussions refuse new comments (423 Locked).
    let posts = Posts::new(state.pool.clone());
    if posts.is_locked(post_id).await.map_err(map_repo_error)? {
        return Err(ApiError::Locked);
    }
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
    // Activity feed: notify the post author (not for self-comments).
    if let Ok(Some(post)) = posts.get_by_id(post_id).await {
        if post.author_id != auth_user.user_id {
            crate::realtime::notify(
                &state.pool,
                &state.realtime,
                crate::realtime::Notify {
                    user_id: post.author_id,
                    kind: "comment",
                    actor_id: Some(auth_user.user_id),
                    entity_type: "post",
                    entity_id: Some(post_id),
                    payload: serde_json::json!({
                        "post_id": post_id.to_string(),
                        "comment_id": comment.id.to_string(),
                        "preview": comment.body.chars().take(120).collect::<String>(),
                    }),
                },
            )
            .await;
        }
    }
    tracing::info!(comment_id = %comment.id, post_id = %post_id, "comment created");
    Ok((
        StatusCode::CREATED,
        Json(CommentResponse {
            comment: comment_view(&comment),
        }),
    ))
}

/// List a post's comments (threaded).
#[utoipa::path(
    get,
    path = "/api/v1/posts/{id}/comments",
    params(("id" = Uuid, Path, description = "Post id")),
    responses(
        (status = 200, description = "Comment tree", body = CommentList),
    ),
    tag = "content"
)]
pub async fn list_comments(
    State(state): State<AppState>,
    Path(post_id): Path<Uuid>,
) -> ApiResult<Json<CommentList>> {
    let comments = Comments::new(state.pool.clone());
    let rows = comments
        .list_by_post(post_id)
        .await
        .map_err(map_repo_error)?;
    let items: Vec<CommentView> = rows.iter().map(comment_view).collect();
    Ok(Json(CommentList { comments: items }))
}

#[tracing::instrument(skip(state, auth_user), fields(actor = %auth_user.user_id, comment_id = %id))]
/// Delete a comment. Owner or staff only.
#[utoipa::path(
    delete,
    path = "/api/v1/comments/{id}",
    params(("id" = Uuid, Path, description = "Comment id")),
    responses(
        (status = 204, description = "Comment deleted"),
        (status = 401, description = "Missing or invalid access token"),
        (status = 403, description = "Not the author and not staff"),
        (status = 404, description = "Comment not found"),
    ),
    security(("bearer_auth" = [])),
    tag = "content"
)]
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
    tracing::info!(comment_id = %id, "comment deleted");
    Ok(StatusCode::NO_CONTENT)
}

// ── Handlers: reactions ────────────────────────────────────────────────────

#[tracing::instrument(skip(state, auth_user), fields(actor = %auth_user.user_id, post_id = %post_id))]
/// Set (upsert) a reaction on a post.
#[utoipa::path(
    put,
    path = "/api/v1/posts/{id}/reaction",
    request_body = SetReactionRequest,
    params(("id" = Uuid, Path, description = "Post id")),
    responses(
        (status = 200, description = "Reaction counts", body = Value),
        (status = 401, description = "Missing or invalid access token"),
    ),
    security(("bearer_auth" = [])),
    tag = "content"
)]
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
    // Activity feed: notify the post author (not for self-reactions).
    let posts = Posts::new(state.pool.clone());
    if let Ok(Some(post)) = posts.get_by_id(post_id).await {
        if post.author_id != auth_user.user_id {
            crate::realtime::notify(
                &state.pool,
                &state.realtime,
                crate::realtime::Notify {
                    user_id: post.author_id,
                    kind: "reaction",
                    actor_id: Some(auth_user.user_id),
                    entity_type: "post",
                    entity_id: Some(post_id),
                    payload: serde_json::json!({
                        "post_id": post_id.to_string(),
                        "kind": reaction.kind,
                    }),
                },
            )
            .await;
        }
    }
    tracing::info!(post_id = %post_id, kind = %reaction.kind, "reaction set");
    Ok(Json(json!({
        "reaction": {
            "id": reaction.id.to_string(),
            "post_id": reaction.post_id.to_string(),
            "kind": reaction.kind,
        }
    })))
}

#[tracing::instrument(skip(state, auth_user), fields(actor = %auth_user.user_id, post_id = %post_id))]
/// Remove the caller's reaction from a post.
#[utoipa::path(
    delete,
    path = "/api/v1/posts/{id}/reaction",
    params(("id" = Uuid, Path, description = "Post id")),
    responses(
        (status = 204, description = "Reaction removed"),
        (status = 401, description = "Missing or invalid access token"),
    ),
    security(("bearer_auth" = [])),
    tag = "content"
)]
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
    tracing::info!(post_id = %post_id, "reaction removed");
    Ok(StatusCode::NO_CONTENT)
}

/// Reaction breakdown for a post.
#[utoipa::path(
    get,
    path = "/api/v1/posts/{id}/reactions",
    params(("id" = Uuid, Path, description = "Post id")),
    responses(
        (status = 200, description = "Reaction counts + caller state", body = Value),
    ),
    tag = "content"
)]
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

#[tracing::instrument(skip(state, auth_user), fields(actor = %auth_user.user_id, post_id = %post_id))]
/// Bookmark a post for the current user.
#[utoipa::path(
    put,
    path = "/api/v1/posts/{id}/bookmark",
    params(("id" = Uuid, Path, description = "Post id")),
    responses(
        (status = 204, description = "Bookmarked"),
        (status = 401, description = "Missing or invalid access token"),
    ),
    security(("bearer_auth" = [])),
    tag = "content"
)]
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
    tracing::info!(post_id = %post_id, "bookmark added");
    Ok(StatusCode::NO_CONTENT)
}

#[tracing::instrument(skip(state, auth_user), fields(actor = %auth_user.user_id, post_id = %post_id))]
/// Remove a bookmark.
#[utoipa::path(
    delete,
    path = "/api/v1/posts/{id}/bookmark",
    params(("id" = Uuid, Path, description = "Post id")),
    responses(
        (status = 204, description = "Bookmark removed"),
        (status = 401, description = "Missing or invalid access token"),
    ),
    security(("bearer_auth" = [])),
    tag = "content"
)]
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
    tracing::info!(post_id = %post_id, "bookmark removed");
    Ok(StatusCode::NO_CONTENT)
}

/// List the current user's bookmarked posts.
#[utoipa::path(
    get,
    path = "/api/v1/me/bookmarks",
    responses(
        (status = 200, description = "Bookmarked posts", body = Value),
        (status = 401, description = "Missing or invalid access token"),
    ),
    security(("bearer_auth" = [])),
    tag = "content"
)]
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
