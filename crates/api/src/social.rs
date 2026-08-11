//! Social API — Month 4: communities with role-based membership, polls with
//! single-vote invariants, and discussion locking.
//!
//! Authorization model:
//!   - community staff = `moderator`/`admin`/`owner` OF THAT COMMUNITY
//!   - role changes require the community owner
//!   - adding posts to a community requires membership
//!   - pinning/removing community posts requires community staff
//!   - poll edits require the post owner or platform staff; voting any
//!     authenticated user
//!   - locking a discussion requires the post owner or platform staff

use crate::auth::{audit, map_repo_error, AuthUser};
use crate::content::{slugify, MaybeUser};
use crate::error::{ApiError, ApiResult};
use crate::AppState;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use keystone_db::repositories::communities::{Communities, NewCommunity};
use keystone_db::repositories::community_posts::CommunityPosts;
use keystone_db::repositories::polls::Polls;
use keystone_db::repositories::posts::Posts;
use serde::Deserialize;
use serde_json::{json, Value};
use utoipa::ToSchema;
use uuid::Uuid;

const NAME_MAX: usize = 100;
const DESCRIPTION_MAX: usize = 2_000;
const OPTION_TEXT_MAX: usize = 200;

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

fn is_staff(role: &str) -> bool {
    matches!(role, "moderator" | "admin" | "super_admin")
}

/// The actor's community role if it grants staff powers in this community.
async fn community_staff_role(
    pool: &sqlx::PgPool,
    community_id: Uuid,
    actor: Uuid,
) -> Result<Option<String>, ApiError> {
    let communities = Communities::new(pool.clone());
    let role = communities
        .role_of(community_id, actor)
        .await
        .map_err(map_repo_error)?;
    Ok(role.filter(|r| matches!(r.as_str(), "moderator" | "admin" | "owner")))
}

// ── Request / response types ───────────────────────────────────────────────

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateCommunityRequest {
    pub name: String,
    #[serde(default = "default_visibility")]
    pub visibility: String,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_visibility() -> String {
    "public".to_owned()
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AddCommunityPostRequest {
    pub post_id: Uuid,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SetMemberRoleRequest {
    pub role: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AddPollOptionRequest {
    pub text: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct VotePollRequest {
    pub option_id: Uuid,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PageQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
}

fn default_limit() -> i64 {
    20
}

fn community_view(c: &keystone_db::repositories::communities::Community) -> Value {
    json!({
        "id": c.id.to_string(),
        "name": c.name,
        "slug": c.slug,
        "description": c.description,
        "visibility": c.visibility,
        "created_by": c.created_by.to_string(),
        "created_at": c.created_at,
    })
}

// ── Communities ────────────────────────────────────────────────────────────

/// Create a community. The creator becomes the owner.
#[utoipa::path(
    post,
    path = "/api/v1/communities",
    request_body = CreateCommunityRequest,
    responses(
        (status = 201, description = "Community created", body = Value),
        (status = 401, description = "Missing or invalid access token"),
    ),
    security(("bearer_auth" = [])),
    tag = "social"
)]
pub async fn create_community(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(req): Json<CreateCommunityRequest>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    validate_text(&req.name, "name", NAME_MAX)?;
    if let Some(description) = &req.description {
        validate_text(description, "description", DESCRIPTION_MAX)?;
    }
    if !matches!(req.visibility.as_str(), "public" | "private") {
        return Err(ApiError::BadRequest("unknown visibility".into()));
    }
    let slug = slugify(&req.name);
    if slug.is_empty() {
        return Err(ApiError::BadRequest(
            "name must produce a usable slug".into(),
        ));
    }

    let communities = Communities::new(state.pool.clone());
    let community = communities
        .create(NewCommunity {
            name: &req.name,
            slug: &slug,
            description: req.description.as_deref(),
            visibility: &req.visibility,
            created_by: auth_user.user_id,
        })
        .await
        .map_err(|e| match e {
            keystone_db::repositories::RepoError::UniqueViolation(_) => {
                ApiError::Conflict("community slug already exists".into())
            }
            other => map_repo_error(other),
        })?;

    audit(
        &state.pool,
        auth_user.user_id,
        "community_created",
        "community",
        &community.id.to_string(),
        None,
    )
    .await;
    tracing::info!(community_id = %community.id, slug = %community.slug, "community created");
    Ok((
        StatusCode::CREATED,
        Json(json!({ "community": community_view(&community) })),
    ))
}

/// Fetch a community by slug.
#[utoipa::path(
    get,
    path = "/api/v1/communities/{slug}",
    params(("slug" = String, Path, description = "Community slug")),
    responses(
        (status = 200, description = "Community", body = Value),
        (status = 404, description = "Community not found"),
    ),
    tag = "social"
)]
pub async fn get_community(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> ApiResult<Json<Value>> {
    let communities = Communities::new(state.pool.clone());
    let community = communities
        .get_by_slug(&slug)
        .await
        .map_err(map_repo_error)?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(json!({ "community": community_view(&community) })))
}

/// List communities (paged).
#[utoipa::path(
    get,
    path = "/api/v1/communities",
    params(
        ("limit" = Option<i64>, Query, description = "Page size"),
        ("offset" = Option<i64>, Query, description = "Page offset"),
    ),
    responses(
        (status = 200, description = "Communities page", body = Value),
    ),
    tag = "social"
)]
pub async fn list_communities(
    State(state): State<AppState>,
    Query(query): Query<PageQuery>,
) -> ApiResult<Json<Value>> {
    let communities = Communities::new(state.pool.clone());
    let rows = communities
        .list(query.limit.clamp(1, 50), query.offset.max(0))
        .await
        .map_err(map_repo_error)?;
    let items: Vec<Value> = rows.iter().map(community_view).collect();
    Ok(Json(json!({ "communities": items })))
}

/// Join a community.
#[utoipa::path(
    post,
    path = "/api/v1/communities/{slug}/join",
    params(("slug" = String, Path, description = "Community slug")),
    responses(
        (status = 204, description = "Joined"),
        (status = 401, description = "Missing or invalid access token"),
        (status = 404, description = "Community not found"),
    ),
    security(("bearer_auth" = [])),
    tag = "social"
)]
pub async fn join_community(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(slug): Path<String>,
) -> ApiResult<StatusCode> {
    let communities = Communities::new(state.pool.clone());
    let community = communities
        .get_by_slug(&slug)
        .await
        .map_err(map_repo_error)?
        .ok_or(ApiError::NotFound)?;
    communities
        .join(community.id, auth_user.user_id)
        .await
        .map_err(map_repo_error)?;
    audit(
        &state.pool,
        auth_user.user_id,
        "community_joined",
        "community",
        &community.id.to_string(),
        None,
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

/// Leave a community.
#[utoipa::path(
    delete,
    path = "/api/v1/communities/{slug}/leave",
    params(("slug" = String, Path, description = "Community slug")),
    responses(
        (status = 204, description = "Left"),
        (status = 401, description = "Missing or invalid access token"),
    ),
    security(("bearer_auth" = [])),
    tag = "social"
)]
pub async fn leave_community(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(slug): Path<String>,
) -> ApiResult<StatusCode> {
    let communities = Communities::new(state.pool.clone());
    let community = communities
        .get_by_slug(&slug)
        .await
        .map_err(map_repo_error)?
        .ok_or(ApiError::NotFound)?;
    communities
        .leave(community.id, auth_user.user_id)
        .await
        .map_err(|e| match e {
            keystone_db::repositories::RepoError::InvalidInput(msg) => ApiError::BadRequest(msg),
            other => map_repo_error(other),
        })?;
    audit(
        &state.pool,
        auth_user.user_id,
        "community_left",
        "community",
        &community.id.to_string(),
        None,
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

/// List a community's members.
#[utoipa::path(
    get,
    path = "/api/v1/communities/{slug}/members",
    params(("slug" = String, Path, description = "Community slug")),
    responses(
        (status = 200, description = "Members", body = Value),
        (status = 404, description = "Community not found"),
    ),
    tag = "social"
)]
pub async fn list_members(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> ApiResult<Json<Value>> {
    let communities = Communities::new(state.pool.clone());
    let community = communities
        .get_by_slug(&slug)
        .await
        .map_err(map_repo_error)?
        .ok_or(ApiError::NotFound)?;
    let members = communities
        .members(community.id)
        .await
        .map_err(map_repo_error)?;
    let items: Vec<Value> = members
        .iter()
        .map(|m| {
            json!({
                "user_id": m.user_id.to_string(),
                "role": m.role,
                "joined_at": m.joined_at,
            })
        })
        .collect();
    Ok(Json(json!({ "members": items })))
}

/// Change a member's role. Community staff only.
#[utoipa::path(
    patch,
    path = "/api/v1/communities/{slug}/members/{member_id}",
    request_body = SetMemberRoleRequest,
    params(
        ("slug" = String, Path, description = "Community slug"),
        ("member_id" = Uuid, Path, description = "Member user id"),
    ),
    responses(
        (status = 200, description = "Updated member", body = Value),
        (status = 401, description = "Missing or invalid access token"),
        (status = 403, description = "Insufficient role"),
    ),
    security(("bearer_auth" = [])),
    tag = "social"
)]
pub async fn set_member_role(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((slug, member_id)): Path<(String, Uuid)>,
    Json(req): Json<SetMemberRoleRequest>,
) -> ApiResult<Json<Value>> {
    let communities = Communities::new(state.pool.clone());
    let community = communities
        .get_by_slug(&slug)
        .await
        .map_err(map_repo_error)?
        .ok_or(ApiError::NotFound)?;

    // Only the community owner may change roles.
    let actor_role = communities
        .role_of(community.id, auth_user.user_id)
        .await
        .map_err(map_repo_error)?
        .ok_or(ApiError::Forbidden)?;
    if actor_role != "owner" {
        return Err(ApiError::Forbidden);
    }

    communities
        .set_role(community.id, member_id, &req.role)
        .await
        .map_err(|e| match e {
            keystone_db::repositories::RepoError::InvalidInput(msg) => ApiError::BadRequest(msg),
            other => map_repo_error(other),
        })?;

    audit(
        &state.pool,
        auth_user.user_id,
        "community_role_changed",
        "community",
        &community.id.to_string(),
        None,
    )
    .await;
    tracing::info!(community_id = %community.id, member = %member_id, role = %req.role, "member role changed");
    Ok(Json(
        json!({ "member": { "user_id": member_id.to_string(), "role": req.role } }),
    ))
}

// ── Community posts ────────────────────────────────────────────────────────

/// Attach an existing post to a community.
#[utoipa::path(
    post,
    path = "/api/v1/communities/{slug}/posts",
    request_body = AddCommunityPostRequest,
    params(("slug" = String, Path, description = "Community slug")),
    responses(
        (status = 204, description = "Post attached"),
        (status = 401, description = "Missing or invalid access token"),
        (status = 403, description = "Not a community member"),
    ),
    security(("bearer_auth" = [])),
    tag = "social"
)]
pub async fn add_community_post(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(slug): Path<String>,
    Json(req): Json<AddCommunityPostRequest>,
) -> ApiResult<StatusCode> {
    let communities = Communities::new(state.pool.clone());
    let community = communities
        .get_by_slug(&slug)
        .await
        .map_err(map_repo_error)?
        .ok_or(ApiError::NotFound)?;

    // Membership required to contribute.
    if communities
        .role_of(community.id, auth_user.user_id)
        .await
        .map_err(map_repo_error)?
        .is_none()
    {
        return Err(ApiError::Forbidden);
    }
    let posts = Posts::new(state.pool.clone());
    if posts
        .get_by_id(req.post_id)
        .await
        .map_err(map_repo_error)?
        .is_none()
    {
        return Err(ApiError::NotFound);
    }

    let community_posts = CommunityPosts::new(state.pool.clone());
    community_posts
        .add(community.id, req.post_id, auth_user.user_id)
        .await
        .map_err(map_repo_error)?;
    audit(
        &state.pool,
        auth_user.user_id,
        "community_post_added",
        "post",
        &req.post_id.to_string(),
        None,
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

/// List a community's posts (paged).
#[utoipa::path(
    get,
    path = "/api/v1/communities/{slug}/posts",
    params(
        ("slug" = String, Path, description = "Community slug"),
        ("limit" = Option<i64>, Query, description = "Page size"),
        ("offset" = Option<i64>, Query, description = "Page offset"),
    ),
    responses(
        (status = 200, description = "Posts page", body = Value),
        (status = 404, description = "Community not found"),
    ),
    tag = "social"
)]
pub async fn list_community_posts(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(query): Query<PageQuery>,
) -> ApiResult<Json<Value>> {
    let communities = Communities::new(state.pool.clone());
    let community = communities
        .get_by_slug(&slug)
        .await
        .map_err(map_repo_error)?
        .ok_or(ApiError::NotFound)?;
    let community_posts = CommunityPosts::new(state.pool.clone());
    let rows = community_posts
        .list(community.id, query.limit.clamp(1, 50), query.offset.max(0))
        .await
        .map_err(map_repo_error)?;
    let items: Vec<Value> = rows
        .iter()
        .map(|cp| {
            json!({
                "post_id": cp.post_id.to_string(),
                "pinned": cp.pinned,
                "added_by": cp.added_by.to_string(),
                "added_at": cp.created_at,
            })
        })
        .collect();
    Ok(Json(json!({ "posts": items })))
}

/// Pin a post to the top of a community. Community staff only.
#[utoipa::path(
    post,
    path = "/api/v1/communities/{slug}/posts/{post_id}/pin",
    params(
        ("slug" = String, Path, description = "Community slug"),
        ("post_id" = Uuid, Path, description = "Post id"),
    ),
    responses(
        (status = 204, description = "Pinned"),
        (status = 401, description = "Missing or invalid access token"),
        (status = 403, description = "Insufficient role"),
    ),
    security(("bearer_auth" = [])),
    tag = "social"
)]
pub async fn pin_community_post(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((slug, post_id)): Path<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    set_pin(state, auth_user, slug, post_id, true).await
}

/// Unpin a post. Community staff only.
#[utoipa::path(
    delete,
    path = "/api/v1/communities/{slug}/posts/{post_id}/pin",
    params(
        ("slug" = String, Path, description = "Community slug"),
        ("post_id" = Uuid, Path, description = "Post id"),
    ),
    responses(
        (status = 204, description = "Unpinned"),
        (status = 401, description = "Missing or invalid access token"),
        (status = 403, description = "Insufficient role"),
    ),
    security(("bearer_auth" = [])),
    tag = "social"
)]
pub async fn unpin_community_post(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((slug, post_id)): Path<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    set_pin(state, auth_user, slug, post_id, false).await
}

async fn set_pin(
    state: AppState,
    auth_user: AuthUser,
    slug: String,
    post_id: Uuid,
    pinned: bool,
) -> ApiResult<StatusCode> {
    let communities = Communities::new(state.pool.clone());
    let community = communities
        .get_by_slug(&slug)
        .await
        .map_err(map_repo_error)?
        .ok_or(ApiError::NotFound)?;
    let staff_role = community_staff_role(&state.pool, community.id, auth_user.user_id).await?;
    if staff_role.is_none() {
        return Err(ApiError::Forbidden);
    }
    let community_posts = CommunityPosts::new(state.pool.clone());
    community_posts
        .set_pinned(community.id, post_id, pinned)
        .await
        .map_err(map_repo_error)?;
    audit(
        &state.pool,
        auth_user.user_id,
        if pinned {
            "community_post_pinned"
        } else {
            "community_post_unpinned"
        },
        "post",
        &post_id.to_string(),
        None,
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

/// Remove a post from a community. Community staff only.
#[utoipa::path(
    delete,
    path = "/api/v1/communities/{slug}/posts/{post_id}",
    params(
        ("slug" = String, Path, description = "Community slug"),
        ("post_id" = Uuid, Path, description = "Post id"),
    ),
    responses(
        (status = 204, description = "Post removed"),
        (status = 401, description = "Missing or invalid access token"),
        (status = 403, description = "Insufficient role"),
    ),
    security(("bearer_auth" = [])),
    tag = "social"
)]
pub async fn remove_community_post(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((slug, post_id)): Path<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    let communities = Communities::new(state.pool.clone());
    let community = communities
        .get_by_slug(&slug)
        .await
        .map_err(map_repo_error)?
        .ok_or(ApiError::NotFound)?;
    let staff_role = community_staff_role(&state.pool, community.id, auth_user.user_id).await?;
    if staff_role.is_none() {
        return Err(ApiError::Forbidden);
    }
    let community_posts = CommunityPosts::new(state.pool.clone());
    community_posts
        .remove(community.id, post_id)
        .await
        .map_err(map_repo_error)?;
    audit(
        &state.pool,
        auth_user.user_id,
        "community_post_removed",
        "post",
        &post_id.to_string(),
        None,
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

// ── Polls ──────────────────────────────────────────────────────────────────

/// Add an option to a poll. Post author only.
#[utoipa::path(
    post,
    path = "/api/v1/posts/{id}/poll/options",
    request_body = AddPollOptionRequest,
    params(("id" = Uuid, Path, description = "Post id")),
    responses(
        (status = 201, description = "Option added", body = Value),
        (status = 401, description = "Missing or invalid access token"),
        (status = 403, description = "Not the post author"),
    ),
    security(("bearer_auth" = [])),
    tag = "social"
)]
pub async fn add_poll_option(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(post_id): Path<Uuid>,
    Json(req): Json<AddPollOptionRequest>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    validate_text(&req.text, "option text", OPTION_TEXT_MAX)?;
    let posts = Posts::new(state.pool.clone());
    let post = posts
        .get_by_id(post_id)
        .await
        .map_err(map_repo_error)?
        .ok_or(ApiError::NotFound)?;
    if post.author_id != auth_user.user_id && !is_staff(&auth_user.role) {
        return Err(ApiError::Forbidden);
    }
    let polls = Polls::new(state.pool.clone());
    let option = polls
        .add_option(post_id, &req.text)
        .await
        .map_err(map_repo_error)?;
    tracing::info!(post_id = %post_id, option_id = %option.id, "poll option added");
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "option": {
                "id": option.id.to_string(),
                "text": option.text,
                "position": option.position,
            }
        })),
    ))
}

/// Cast (or change) the caller's poll vote.
#[utoipa::path(
    put,
    path = "/api/v1/posts/{id}/poll/votes",
    request_body = VotePollRequest,
    params(("id" = Uuid, Path, description = "Post id")),
    responses(
        (status = 204, description = "Vote recorded"),
        (status = 401, description = "Missing or invalid access token"),
    ),
    security(("bearer_auth" = [])),
    tag = "social"
)]
pub async fn vote_poll(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(post_id): Path<Uuid>,
    Json(req): Json<VotePollRequest>,
) -> ApiResult<StatusCode> {
    let polls = Polls::new(state.pool.clone());
    polls
        .vote(post_id, auth_user.user_id, req.option_id)
        .await
        .map_err(|e| match e {
            keystone_db::repositories::RepoError::InvalidInput(msg) => ApiError::BadRequest(msg),
            other => map_repo_error(other),
        })?;
    tracing::info!(post_id = %post_id, actor = %auth_user.user_id, "poll vote cast");
    Ok(StatusCode::NO_CONTENT)
}

/// Remove the caller's poll vote.
#[utoipa::path(
    delete,
    path = "/api/v1/posts/{id}/poll/votes",
    params(("id" = Uuid, Path, description = "Post id")),
    responses(
        (status = 204, description = "Vote removed"),
        (status = 401, description = "Missing or invalid access token"),
    ),
    security(("bearer_auth" = [])),
    tag = "social"
)]
pub async fn remove_poll_vote(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(post_id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let polls = Polls::new(state.pool.clone());
    polls
        .remove_vote(post_id, auth_user.user_id)
        .await
        .map_err(map_repo_error)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Poll state for a post (options + tallies + caller vote).
#[utoipa::path(
    get,
    path = "/api/v1/posts/{id}/poll",
    params(("id" = Uuid, Path, description = "Post id")),
    responses(
        (status = 200, description = "Poll with tallies", body = Value),
        (status = 404, description = "Post or poll not found"),
    ),
    tag = "social"
)]
pub async fn get_poll(
    State(state): State<AppState>,
    maybe: MaybeUser,
    Path(post_id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    let polls = Polls::new(state.pool.clone());
    let results = polls.results(post_id).await.map_err(map_repo_error)?;
    let total = polls.total_votes(post_id).await.map_err(map_repo_error)?;
    let my_vote = match maybe.0 {
        Some(u) => polls
            .voted_option(post_id, u.user_id)
            .await
            .map_err(map_repo_error)?
            .map(|id| id.to_string()),
        None => None,
    };
    let options: Vec<Value> = results
        .iter()
        .map(|r| {
            json!({
                "id": r.option_id.to_string(),
                "text": r.text,
                "position": r.position,
                "votes": r.votes,
            })
        })
        .collect();
    Ok(Json(
        json!({ "options": options, "total_votes": total, "my_vote": my_vote }),
    ))
}

// ── Discussion locking ─────────────────────────────────────────────────────

/// Lock a post (refuses new comments). Author or staff only.
#[utoipa::path(
    post,
    path = "/api/v1/posts/{id}/lock",
    params(("id" = Uuid, Path, description = "Post id")),
    responses(
        (status = 204, description = "Locked"),
        (status = 401, description = "Missing or invalid access token"),
        (status = 403, description = "Not the author and not staff"),
    ),
    security(("bearer_auth" = [])),
    tag = "social"
)]
pub async fn lock_post(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(post_id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    set_lock(state, auth_user, post_id, true).await
}

/// Unlock a post. Author or staff only.
#[utoipa::path(
    delete,
    path = "/api/v1/posts/{id}/lock",
    params(("id" = Uuid, Path, description = "Post id")),
    responses(
        (status = 204, description = "Unlocked"),
        (status = 401, description = "Missing or invalid access token"),
        (status = 403, description = "Not the author and not staff"),
    ),
    security(("bearer_auth" = [])),
    tag = "social"
)]
pub async fn unlock_post(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(post_id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    set_lock(state, auth_user, post_id, false).await
}

async fn set_lock(
    state: AppState,
    auth_user: AuthUser,
    post_id: Uuid,
    lock: bool,
) -> ApiResult<StatusCode> {
    let posts = Posts::new(state.pool.clone());
    let post = posts
        .get_by_id(post_id)
        .await
        .map_err(map_repo_error)?
        .ok_or(ApiError::NotFound)?;
    if post.author_id != auth_user.user_id && !is_staff(&auth_user.role) {
        return Err(ApiError::Forbidden);
    }
    let applied = if lock {
        posts.lock(post_id).await
    } else {
        posts.unlock(post_id).await
    }
    .map_err(map_repo_error)?;
    if !applied {
        return Err(ApiError::NotFound);
    }
    audit(
        &state.pool,
        auth_user.user_id,
        if lock { "post_locked" } else { "post_unlocked" },
        "post",
        &post_id.to_string(),
        None,
    )
    .await;
    tracing::info!(post_id = %post_id, locked = lock, "discussion lock changed");
    Ok(StatusCode::NO_CONTENT)
}
