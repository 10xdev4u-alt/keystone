//! Network API — Month 5: organizations with role-based membership, the
//! user_links social graph, and profiles with a strict visibility matrix.
//!
//! Authorization model:
//!   - org creation → creator is sole owner; role changes require the owner
//!   - claims: any org MEMBER may file (prevents drive-by domain takeover
//!     attempts); verification is token-based (hash-compared in the repo)
//!   - follow/connect/block: any authenticated user; block is mutual and
//!     enforced at read time (see [`profile_visible`])
//!   - profile reads apply the visibility matrix: public → everyone,
//!     connections → accepted connections only, private → self only;
//!     a block in either direction hides the profile entirely (404)

use crate::auth::{audit, map_repo_error, AuthUser};
use crate::content::{slugify, MaybeUser};
use crate::error::{ApiError, ApiResult};
use crate::social::PageQuery;
use crate::AppState;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::{Duration, Utc};
use keystone_db::repositories::links::UserLinks;
use keystone_db::repositories::organizations::{NewOrganization, Organizations};
use keystone_db::repositories::profiles::{NewEducation, NewExperience, Profiles};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::Digest;
use uuid::Uuid;

const NAME_MAX: usize = 100;
const TEXT_MAX: usize = 4_000;
const SKILL_MAX: usize = 60;

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

// ── Organizations ───────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct CreateOrgRequest {
    pub name: String,
    pub description: Option<String>,
    pub website: Option<String>,
    pub industry: Option<String>,
}

#[derive(Deserialize)]
pub struct SetRoleRequest {
    pub role: String,
}

#[derive(Deserialize)]
pub struct ClaimRequest {
    pub domain: String,
}

#[derive(Deserialize)]
pub struct VerifyClaimRequest {
    pub token: String,
}

pub async fn create_org(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(req): Json<CreateOrgRequest>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    validate_text(&req.name, "name", NAME_MAX)?;
    let slug = slugify(&req.name);
    let orgs = Organizations::new(state.pool.clone());
    let org = orgs
        .create(NewOrganization {
            name: &req.name,
            slug: &slug,
            description: req.description.as_deref(),
            website: req.website.as_deref(),
            industry: req.industry.as_deref(),
            created_by: auth_user.user_id,
        })
        .await
        .map_err(map_repo_error)?;
    audit(
        &state.pool,
        auth_user.user_id,
        "org_created",
        "organization",
        &org.id.to_string(),
        None,
    )
    .await;
    tracing::info!(org_id = %org.id, slug = %org.slug, "organization created");
    Ok((
        StatusCode::CREATED,
        Json(json!({ "organization": org_json(&org) })),
    ))
}

pub async fn list_orgs(
    State(state): State<AppState>,
    Query(query): Query<PageQuery>,
) -> ApiResult<Json<Value>> {
    let orgs = Organizations::new(state.pool.clone());
    let rows = orgs
        .list(query.limit.clamp(1, 50), query.offset.max(0))
        .await
        .map_err(map_repo_error)?;
    let items: Vec<Value> = rows.iter().map(org_json).collect();
    Ok(Json(json!({ "organizations": items })))
}

pub async fn get_org(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> ApiResult<Json<Value>> {
    let orgs = Organizations::new(state.pool.clone());
    let org = orgs
        .get_by_slug(&slug)
        .await
        .map_err(map_repo_error)?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(json!({ "organization": org_json(&org) })))
}

pub async fn join_org(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(slug): Path<String>,
) -> ApiResult<StatusCode> {
    let orgs = Organizations::new(state.pool.clone());
    let org = orgs
        .get_by_slug(&slug)
        .await
        .map_err(map_repo_error)?
        .ok_or(ApiError::NotFound)?;
    orgs.join(org.id, auth_user.user_id)
        .await
        .map_err(map_repo_error)?;
    audit(
        &state.pool,
        auth_user.user_id,
        "org_joined",
        "organization",
        &org.id.to_string(),
        None,
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn leave_org(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(slug): Path<String>,
) -> ApiResult<StatusCode> {
    let orgs = Organizations::new(state.pool.clone());
    let org = orgs
        .get_by_slug(&slug)
        .await
        .map_err(map_repo_error)?
        .ok_or(ApiError::NotFound)?;
    orgs.leave(org.id, auth_user.user_id)
        .await
        .map_err(map_repo_error)?;
    audit(
        &state.pool,
        auth_user.user_id,
        "org_left",
        "organization",
        &org.id.to_string(),
        None,
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_members(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> ApiResult<Json<Value>> {
    let orgs = Organizations::new(state.pool.clone());
    let org = orgs
        .get_by_slug(&slug)
        .await
        .map_err(map_repo_error)?
        .ok_or(ApiError::NotFound)?;
    let members = orgs.members(org.id).await.map_err(map_repo_error)?;
    let items: Vec<Value> = members
        .into_iter()
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

/// Role changes require the org owner. Ownership transfer demotes the old
/// owner atomically (repo-level invariant).
pub async fn set_member_role(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((slug, member_id)): Path<(String, Uuid)>,
    Json(req): Json<SetRoleRequest>,
) -> ApiResult<StatusCode> {
    let orgs = Organizations::new(state.pool.clone());
    let org = orgs
        .get_by_slug(&slug)
        .await
        .map_err(map_repo_error)?
        .ok_or(ApiError::NotFound)?;
    let actor_role = orgs
        .member_role(org.id, auth_user.user_id)
        .await
        .map_err(map_repo_error)?
        .ok_or(ApiError::Forbidden)?;
    if actor_role != "owner" {
        return Err(ApiError::Forbidden);
    }
    orgs.set_role(org.id, member_id, &req.role)
        .await
        .map_err(map_repo_error)?;
    audit(
        &state.pool,
        auth_user.user_id,
        "org_role_changed",
        "organization",
        &org.id.to_string(),
        None,
    )
    .await;
    tracing::info!(org_id = %org.id, member = %member_id, role = %req.role, "org role changed");
    Ok(StatusCode::NO_CONTENT)
}

/// File an org claim. The claimant must already be a member (prevents
/// drive-by domain takeover attempts on orgs you have no stake in). Only
/// the token HASH is persisted.
pub async fn file_claim(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(slug): Path<String>,
    Json(req): Json<ClaimRequest>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    validate_text(&req.domain, "domain", 255)?;
    let orgs = Organizations::new(state.pool.clone());
    let org = orgs
        .get_by_slug(&slug)
        .await
        .map_err(map_repo_error)?
        .ok_or(ApiError::NotFound)?;
    if orgs
        .member_role(org.id, auth_user.user_id)
        .await
        .map_err(map_repo_error)?
        .is_none()
    {
        return Err(ApiError::Forbidden);
    }
    // Token is a random 32-byte secret; the client presents it base64-encoded
    // and only the SHA-256 hash of THAT string is stored (verify hashes the
    // same string, so the comparison is exact).
    let mut token = [0u8; 32];
    getrandom::fill(&mut token).map_err(|_| ApiError::Internal)?;
    let token_b64 =
        base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, token);
    let token_hash = hex::encode(sha2::Sha256::digest(token_b64.as_bytes()));
    let claim = orgs
        .create_claim(
            org.id,
            auth_user.user_id,
            &req.domain,
            &token_hash,
            Utc::now() + Duration::hours(24),
        )
        .await
        .map_err(map_repo_error)?;
    audit(
        &state.pool,
        auth_user.user_id,
        "org_claim_filed",
        "organization",
        &org.id.to_string(),
        None,
    )
    .await;
    // The raw token is returned ONCE to the claimant (it doubles as the
    // email proof); it is never logged or stored.
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "claim_id": claim.id.to_string(),
            "token": token_b64,
            "expires_at": claim.expires_at,
        })),
    ))
}

/// Verify a claim with the token. The token is hashed before comparison.
pub async fn verify_claim(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((slug, claim_id)): Path<(String, Uuid)>,
    Json(req): Json<VerifyClaimRequest>,
) -> ApiResult<Json<Value>> {
    let orgs = Organizations::new(state.pool.clone());
    let org = orgs
        .get_by_slug(&slug)
        .await
        .map_err(map_repo_error)?
        .ok_or(ApiError::NotFound)?;
    let token_hash = hex::encode(sha2::Sha256::digest(req.token.as_bytes()));
    let approved = orgs
        .verify_claim(claim_id, &token_hash)
        .await
        .map_err(map_repo_error)?;
    if !approved {
        return Err(ApiError::BadRequest(
            "invalid, expired, or already-used claim token".into(),
        ));
    }
    audit(
        &state.pool,
        auth_user.user_id,
        "org_claim_approved",
        "organization",
        &org.id.to_string(),
        None,
    )
    .await;
    Ok(Json(json!({ "status": "approved" })))
}

fn org_json(org: &keystone_db::repositories::organizations::Organization) -> Value {
    json!({
        "id": org.id.to_string(),
        "name": org.name,
        "slug": org.slug,
        "description": org.description,
        "website": org.website,
        "industry": org.industry,
        "created_by": org.created_by.to_string(),
        "created_at": org.created_at,
    })
}

/// Shared guard: fetch a live org by slug or 404. Used by network and
/// careers routes so org existence is checked once, consistently.
pub(crate) async fn org_by_slug_or_404(
    pool: &sqlx::PgPool,
    slug: &str,
) -> Result<keystone_db::repositories::organizations::Organization, ApiError> {
    let orgs = Organizations::new(pool.clone());
    orgs.get_by_slug(slug)
        .await
        .map_err(map_repo_error)?
        .ok_or(ApiError::NotFound)
}

// ── Social graph ────────────────────────────────────────────────────────────

pub async fn follow(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(target): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let links = UserLinks::new(state.pool.clone());
    links
        .follow(auth_user.user_id, target)
        .await
        .map_err(map_repo_error)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn unfollow(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(target): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let links = UserLinks::new(state.pool.clone());
    links
        .remove(auth_user.user_id, target, "follow")
        .await
        .map_err(map_repo_error)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn connect(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(target): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let links = UserLinks::new(state.pool.clone());
    links
        .connect(auth_user.user_id, target)
        .await
        .map_err(map_repo_error)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn cancel_connect(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(target): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let links = UserLinks::new(state.pool.clone());
    links
        .remove(auth_user.user_id, target, "connect")
        .await
        .map_err(map_repo_error)?;
    Ok(StatusCode::NO_CONTENT)
}

/// The TARGET accepts the requester's pending connection.
pub async fn accept_connection(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(requester): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let links = UserLinks::new(state.pool.clone());
    let accepted = links
        .accept(auth_user.user_id, requester)
        .await
        .map_err(map_repo_error)?;
    if !accepted {
        return Err(ApiError::BadRequest(
            "no pending connection to accept".into(),
        ));
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn reject_connection(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(requester): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let links = UserLinks::new(state.pool.clone());
    let rejected = links
        .reject(auth_user.user_id, requester)
        .await
        .map_err(map_repo_error)?;
    if !rejected {
        return Err(ApiError::BadRequest(
            "no pending connection to reject".into(),
        ));
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn block(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(target): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let links = UserLinks::new(state.pool.clone());
    links
        .block(auth_user.user_id, target)
        .await
        .map_err(map_repo_error)?;
    audit(
        &state.pool,
        auth_user.user_id,
        "user_blocked",
        "user",
        &target.to_string(),
        None,
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn unblock(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(target): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let links = UserLinks::new(state.pool.clone());
    links
        .remove(auth_user.user_id, target, "block")
        .await
        .map_err(map_repo_error)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn my_following(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> ApiResult<Json<Value>> {
    let links = UserLinks::new(state.pool.clone());
    let rows = links
        .following(auth_user.user_id)
        .await
        .map_err(map_repo_error)?;
    Ok(Json(json!({
        "following": rows.iter().map(|u| u.to_string()).collect::<Vec<_>>()
    })))
}

pub async fn my_connections(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> ApiResult<Json<Value>> {
    let links = UserLinks::new(state.pool.clone());
    let rows = links
        .connections(auth_user.user_id)
        .await
        .map_err(map_repo_error)?;
    Ok(Json(json!({
        "connections": rows.iter().map(|u| u.to_string()).collect::<Vec<_>>()
    })))
}

// ── Profiles (visibility matrix) ────────────────────────────────────────────

/// The Month-5 visibility matrix: public → everyone, connections → accepted
/// connections only, private → self only; a block in EITHER direction hides
/// the profile completely (callers 404 to avoid confirming existence).
async fn profile_visible(
    pool: &sqlx::PgPool,
    viewer: Option<&AuthUser>,
    owner_id: Uuid,
) -> Result<bool, ApiError> {
    let profiles = Profiles::new(pool.clone());
    let links = UserLinks::new(pool.clone());
    let Some(profile) = profiles.get(owner_id).await.map_err(map_repo_error)? else {
        return Ok(false);
    };
    if let Some(viewer) = viewer {
        if links
            .are_blocked(viewer.user_id, owner_id)
            .await
            .map_err(map_repo_error)?
        {
            return Ok(false);
        }
    }
    match profile.visibility.as_str() {
        "public" => Ok(true),
        "private" => Ok(viewer.is_some_and(|v| v.user_id == owner_id)),
        "connections" => {
            let Some(viewer) = viewer else {
                return Ok(false);
            };
            if viewer.user_id == owner_id {
                return Ok(true);
            }
            let connections = links.connections(owner_id).await.map_err(map_repo_error)?;
            Ok(connections.contains(&viewer.user_id))
        }
        _ => Ok(false),
    }
}

#[derive(Deserialize)]
pub struct SetProfileRequest {
    pub bio: Option<String>,
    pub location: Option<String>,
    pub visibility: Option<String>,
}

pub async fn set_profile(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(req): Json<SetProfileRequest>,
) -> ApiResult<Json<Value>> {
    let bio = req.bio.as_deref();
    if let Some(bio) = bio {
        validate_text(bio, "bio", TEXT_MAX)?;
    }
    let profiles = Profiles::new(state.pool.clone());
    let profile = profiles
        .set(
            auth_user.user_id,
            bio,
            req.location.as_deref(),
            req.visibility.as_deref().unwrap_or("public"),
        )
        .await
        .map_err(map_repo_error)?;
    Ok(Json(json!({
        "profile": {
            "user_id": profile.user_id.to_string(),
            "bio": profile.bio,
            "location": profile.location,
            "visibility": profile.visibility,
        }
    })))
}

pub async fn get_profile(
    State(state): State<AppState>,
    maybe: MaybeUser,
    Path(user_id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    if !profile_visible(&state.pool, maybe.0.as_ref(), user_id).await? {
        return Err(ApiError::NotFound); // hide existence, never confirm it
    }
    let profiles = Profiles::new(state.pool.clone());
    let profile = profiles
        .get(user_id)
        .await
        .map_err(map_repo_error)?
        .ok_or(ApiError::NotFound)?;
    let education = profiles.education(user_id).await.map_err(map_repo_error)?;
    let experience = profiles.experience(user_id).await.map_err(map_repo_error)?;
    let skills = profiles.skills(user_id).await.map_err(map_repo_error)?;
    Ok(Json(json!({
        "profile": {
            "user_id": user_id.to_string(),
            "bio": profile.bio,
            "location": profile.location,
            "visibility": profile.visibility,
        },
        "education": education.iter().map(|e| json!({
            "id": e.id.to_string(),
            "school": e.school,
            "degree": e.degree,
            "field": e.field,
            "start_year": e.start_year,
            "end_year": e.end_year,
            "description": e.description,
        })).collect::<Vec<_>>(),
        "experience": experience.iter().map(|x| json!({
            "id": x.id.to_string(),
            "title": x.title,
            "company": x.company,
            "organization_id": x.organization_id.map(|o| o.to_string()),
            "start_date": x.start_date,
            "end_date": x.end_date,
            "current": x.current,
            "description": x.description,
        })).collect::<Vec<_>>(),
        "skills": skills.iter().map(|s| json!({
            "skill": s.skill,
            "level": s.level,
        })).collect::<Vec<_>>(),
    })))
}

#[derive(Deserialize)]
pub struct EducationRequest {
    pub school: String,
    pub degree: Option<String>,
    pub field: Option<String>,
    pub start_year: i32,
    pub end_year: Option<i32>,
    pub description: Option<String>,
}

#[derive(Deserialize)]
pub struct ExperienceRequest {
    pub title: String,
    pub company: Option<String>,
    pub organization_id: Option<Uuid>,
    pub start_date: chrono::NaiveDate,
    pub end_date: Option<chrono::NaiveDate>,
    pub current: bool,
    pub description: Option<String>,
}

#[derive(Deserialize)]
pub struct SkillRequest {
    pub skill: String,
    pub level: String,
}

pub async fn add_education(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(req): Json<EducationRequest>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    validate_text(&req.school, "school", NAME_MAX)?;
    let profiles = Profiles::new(state.pool.clone());
    let row = profiles
        .add_education(
            auth_user.user_id,
            NewEducation {
                school: &req.school,
                degree: req.degree.as_deref(),
                field: req.field.as_deref(),
                start_year: req.start_year,
                end_year: req.end_year,
                description: req.description.as_deref(),
            },
        )
        .await
        .map_err(map_repo_error)?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "id": row.id.to_string() })),
    ))
}

pub async fn remove_education(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let profiles = Profiles::new(state.pool.clone());
    let removed = profiles
        .remove_education(auth_user.user_id, id)
        .await
        .map_err(map_repo_error)?;
    if !removed {
        return Err(ApiError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn add_experience(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(req): Json<ExperienceRequest>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    validate_text(&req.title, "title", NAME_MAX)?;
    let profiles = Profiles::new(state.pool.clone());
    let row = profiles
        .add_experience(
            auth_user.user_id,
            NewExperience {
                organization_id: req.organization_id,
                title: &req.title,
                company: req.company.as_deref(),
                start_date: req.start_date,
                end_date: req.end_date,
                current: req.current,
                description: req.description.as_deref(),
            },
        )
        .await
        .map_err(map_repo_error)?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "id": row.id.to_string() })),
    ))
}

pub async fn remove_experience(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let profiles = Profiles::new(state.pool.clone());
    let removed = profiles
        .remove_experience(auth_user.user_id, id)
        .await
        .map_err(map_repo_error)?;
    if !removed {
        return Err(ApiError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn add_skill(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(req): Json<SkillRequest>,
) -> ApiResult<StatusCode> {
    validate_text(&req.skill, "skill", SKILL_MAX)?;
    let profiles = Profiles::new(state.pool.clone());
    profiles
        .add_skill(auth_user.user_id, &req.skill, &req.level)
        .await
        .map_err(map_repo_error)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn remove_skill(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(skill): Path<String>,
) -> ApiResult<StatusCode> {
    let profiles = Profiles::new(state.pool.clone());
    let removed = profiles
        .remove_skill(auth_user.user_id, &skill)
        .await
        .map_err(map_repo_error)?;
    if !removed {
        return Err(ApiError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}
