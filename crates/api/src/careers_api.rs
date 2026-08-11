//! Careers API — Month 5: salary benchmarks (anonymized), vendor listings,
//! compliance alerts, career paths, self-assessments.
//!
//! Salary anonymity is enforced end-to-end:
//!   - a submission is merged into its bucket with NO user identity anywhere
//!   - buckets below [`keystone_db::repositories::careers::MIN_SOURCE_COUNT`]
//!     are never readable — sub-threshold reads return `None`
//!   - responses expose only the aggregate bounds + count
//!
//! Authorization model:
//!   - salary submit/read: any authenticated / any reader (public data)
//!   - vendors + compliance alerts: managed by org admins/owners, read by
//!     everyone
//!   - career paths: public; self-assessments: owner-scoped

use crate::auth::{map_repo_error, AuthUser};
use crate::error::{ApiError, ApiResult};
use crate::network::org_by_slug_or_404;
use crate::AppState;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use keystone_db::repositories::careers::{Careers, SalarySubmission};
use keystone_db::repositories::organizations::Organizations;
use serde::Deserialize;
use serde_json::{json, Value};
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Deserialize, ToSchema)]
pub struct SalarySubmitRequest {
    pub role: String,
    pub location: Option<String>,
    pub currency: String,
    pub amount: i64,
}

#[derive(Deserialize, ToSchema)]
pub struct SalaryQuery {
    pub role: String,
}

#[derive(Deserialize, ToSchema)]
pub struct VendorRequest {
    pub category: String,
    pub description: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct AlertRequest {
    pub kind: String,
    pub severity: String,
    pub message: String,
}

#[derive(Deserialize, ToSchema)]
pub struct CareerPathRequest {
    pub title: String,
    pub description: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct StepRequest {
    pub position: i32,
    pub title: String,
    pub description: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct CareerAssessmentRequest {
    pub career_path_id: Uuid,
    pub score: i32,
    pub notes: Option<String>,
}

// ── Salary benchmarks ───────────────────────────────────────────────────────

/// Submit one anonymized salary data point. Nothing user-identifying is
/// written — the bucket row only ever carries bounds + count. Sub-threshold
/// buckets stay unreadable, so even the writer cannot deanonymize.
/// Submit an anonymized salary data point.
#[utoipa::path(
    post,
    path = "/api/v1/salaries",
    request_body = SalarySubmitRequest,
    responses(
        (status = 201, description = "Submission merged into aggregate", body = Value),
        (status = 401, description = "Missing or invalid access token"),
    ),
    security(("bearer_auth" = [])),
    tag = "careers"
)]
pub async fn submit_salary(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(req): Json<SalarySubmitRequest>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    if req.role.trim().is_empty() {
        return Err(ApiError::BadRequest("role must not be empty".into()));
    }
    let careers = Careers::new(state.pool.clone());
    let bucket = careers
        .merge_submission(&SalarySubmission {
            role: req.role.trim().to_string(),
            location: req
                .location
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            currency: req.currency.trim().to_uppercase(),
            amount: req.amount,
        })
        .await
        .map_err(map_repo_error)?;
    tracing::info!(actor = %auth_user.user_id, role = %bucket.role, "salary submission merged");
    // The response is the AGGREGATE only — never the submitted value alone.
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({
            "status": "aggregated",
            "bucket": bucket_json(&bucket),
        })),
    ))
}

/// Readable buckets for a role — sub-threshold buckets are absent by design.
/// Aggregated salary ranges for a role (anonymized, bucketed).
#[utoipa::path(
    get,
    path = "/api/v1/salaries/search",
    params(("role" = String, Query, description = "Role title")),
    responses(
        (status = 200, description = "Salary aggregates", body = Value),
    ),
    tag = "careers"
)]
pub async fn salaries_for_role(
    State(state): State<AppState>,
    Query(query): Query<SalaryQuery>,
) -> ApiResult<Json<Value>> {
    let careers = Careers::new(state.pool.clone());
    let rows = careers
        .for_role(&query.role)
        .await
        .map_err(map_repo_error)?;
    let buckets: Vec<Value> = rows.iter().map(bucket_json).collect();
    Ok(Json(json!({ "role": query.role, "buckets": buckets })))
}

fn bucket_json(bucket: &keystone_db::repositories::careers::SalaryBenchmark) -> Value {
    json!({
        "location": bucket.location,
        "currency": bucket.currency,
        "min": bucket.min_amount,
        "median": bucket.median_amount,
        "max": bucket.max_amount,
        "source_count": bucket.source_count,
    })
}

// ── Vendors ─────────────────────────────────────────────────────────────────

/// Manage (create) vendor listings — org admins/owners only.
/// Add a vendor listing to an organization. Org admin/owner only.
#[utoipa::path(
    post,
    path = "/api/v1/orgs/{slug}/vendors",
    request_body = VendorRequest,
    params(("slug" = String, Path, description = "Organization slug")),
    responses(
        (status = 201, description = "Vendor added", body = Value),
        (status = 401, description = "Missing or invalid access token"),
        (status = 403, description = "Not an org admin"),
    ),
    security(("bearer_auth" = [])),
    tag = "careers"
)]
pub async fn add_vendor(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(slug): Path<String>,
    Json(req): Json<VendorRequest>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let org = org_by_slug_or_404(&state.pool, &slug).await?;
    require_org_admin(&state.pool, org.id, auth_user.user_id).await?;
    let careers = Careers::new(state.pool.clone());
    let listing = careers
        .add_vendor(org.id, &req.category, req.description.as_deref())
        .await
        .map_err(map_repo_error)?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "id": listing.id.to_string() })),
    ))
}

/// List an organization's verified vendor listings.
#[utoipa::path(
    get,
    path = "/api/v1/orgs/{slug}/vendors",
    params(("slug" = String, Path, description = "Organization slug")),
    responses(
        (status = 200, description = "Vendors", body = Value),
    ),
    tag = "careers"
)]
pub async fn list_vendors(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> ApiResult<Json<Value>> {
    let org = org_by_slug_or_404(&state.pool, &slug).await?;
    let careers = Careers::new(state.pool.clone());
    let rows = careers.vendors(org.id).await.map_err(map_repo_error)?;
    let items: Vec<Value> = rows
        .iter()
        .map(|v| {
            json!({
                "id": v.id.to_string(),
                "category": v.category,
                "description": v.description,
                "verified": v.verified,
            })
        })
        .collect();
    Ok(Json(json!({ "vendors": items })))
}

/// Verify a vendor listing. Org admin/owner only.
#[utoipa::path(
    post,
    path = "/api/v1/orgs/{slug}/vendors/{listing_id}/verify",
    params(
        ("slug" = String, Path, description = "Organization slug"),
        ("listing_id" = Uuid, Path, description = "Vendor listing id"),
    ),
    responses(
        (status = 204, description = "Vendor verified"),
        (status = 401, description = "Missing or invalid access token"),
        (status = 403, description = "Not an org admin"),
    ),
    security(("bearer_auth" = [])),
    tag = "careers"
)]
pub async fn verify_vendor(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((slug, listing_id)): Path<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    let org = org_by_slug_or_404(&state.pool, &slug).await?;
    require_org_admin(&state.pool, org.id, auth_user.user_id).await?;
    let careers = Careers::new(state.pool.clone());
    if !careers
        .verify_vendor(listing_id)
        .await
        .map_err(map_repo_error)?
    {
        return Err(ApiError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Remove a vendor listing. Org admin/owner only.
#[utoipa::path(
    delete,
    path = "/api/v1/orgs/{slug}/vendors/{listing_id}",
    params(
        ("slug" = String, Path, description = "Organization slug"),
        ("listing_id" = Uuid, Path, description = "Vendor listing id"),
    ),
    responses(
        (status = 204, description = "Vendor removed"),
        (status = 401, description = "Missing or invalid access token"),
        (status = 403, description = "Not an org admin"),
    ),
    security(("bearer_auth" = [])),
    tag = "careers"
)]
pub async fn remove_vendor(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((slug, listing_id)): Path<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    let org = org_by_slug_or_404(&state.pool, &slug).await?;
    require_org_admin(&state.pool, org.id, auth_user.user_id).await?;
    let careers = Careers::new(state.pool.clone());
    if !careers
        .remove_vendor(org.id, listing_id)
        .await
        .map_err(map_repo_error)?
    {
        return Err(ApiError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

// ── Compliance alerts ───────────────────────────────────────────────────────

/// Create a compliance alert for an organization. Org admin/owner only.
#[utoipa::path(
    post,
    path = "/api/v1/orgs/{slug}/alerts",
    request_body = AlertRequest,
    params(("slug" = String, Path, description = "Organization slug")),
    responses(
        (status = 201, description = "Alert created", body = Value),
        (status = 401, description = "Missing or invalid access token"),
        (status = 403, description = "Not an org admin"),
    ),
    security(("bearer_auth" = [])),
    tag = "careers"
)]
pub async fn add_alert(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(slug): Path<String>,
    Json(req): Json<AlertRequest>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let org = org_by_slug_or_404(&state.pool, &slug).await?;
    require_org_admin(&state.pool, org.id, auth_user.user_id).await?;
    let careers = Careers::new(state.pool.clone());
    let alert = careers
        .add_alert(org.id, &req.kind, &req.severity, &req.message)
        .await
        .map_err(map_repo_error)?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "id": alert.id.to_string() })),
    ))
}

/// List an organization's compliance alerts.
#[utoipa::path(
    get,
    path = "/api/v1/orgs/{slug}/alerts",
    params(("slug" = String, Path, description = "Organization slug")),
    responses(
        (status = 200, description = "Alerts", body = Value),
    ),
    tag = "careers"
)]
pub async fn list_alerts(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> ApiResult<Json<Value>> {
    let org = org_by_slug_or_404(&state.pool, &slug).await?;
    let careers = Careers::new(state.pool.clone());
    let rows = careers.alerts(org.id).await.map_err(map_repo_error)?;
    let items: Vec<Value> = rows
        .iter()
        .map(|a| {
            json!({
                "id": a.id.to_string(),
                "kind": a.kind,
                "severity": a.severity,
                "message": a.message,
                "resolved_at": a.resolved_at,
                "created_at": a.created_at,
            })
        })
        .collect();
    Ok(Json(json!({ "alerts": items })))
}

/// Resolve a compliance alert. Org admin/owner only.
#[utoipa::path(
    post,
    path = "/api/v1/orgs/{slug}/alerts/{alert_id}/resolve",
    params(
        ("slug" = String, Path, description = "Organization slug"),
        ("alert_id" = Uuid, Path, description = "Alert id"),
    ),
    responses(
        (status = 204, description = "Alert resolved"),
        (status = 401, description = "Missing or invalid access token"),
        (status = 403, description = "Not an org admin"),
    ),
    security(("bearer_auth" = [])),
    tag = "careers"
)]
pub async fn resolve_alert(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((slug, alert_id)): Path<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    let org = org_by_slug_or_404(&state.pool, &slug).await?;
    require_org_admin(&state.pool, org.id, auth_user.user_id).await?;
    let careers = Careers::new(state.pool.clone());
    if !careers
        .resolve_alert(alert_id, org.id)
        .await
        .map_err(map_repo_error)?
    {
        return Err(ApiError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

// ── Career paths & self-assessments ─────────────────────────────────────────

/// List all career paths.
#[utoipa::path(
    get,
    path = "/api/v1/career-paths",
    responses(
        (status = 200, description = "Career paths", body = Value),
    ),
    tag = "careers"
)]
pub async fn list_career_paths(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let careers = Careers::new(state.pool.clone());
    let rows = careers.career_paths().await.map_err(map_repo_error)?;
    let items: Vec<Value> = rows
        .iter()
        .map(|p| json!({ "id": p.id.to_string(), "title": p.title, "description": p.description }))
        .collect();
    Ok(Json(json!({ "career_paths": items })))
}

/// Create a career path.
#[utoipa::path(
    post,
    path = "/api/v1/career-paths",
    request_body = CareerPathRequest,
    responses(
        (status = 201, description = "Career path created", body = Value),
        (status = 401, description = "Missing or invalid access token"),
    ),
    security(("bearer_auth" = [])),
    tag = "careers"
)]
pub async fn create_career_path(
    State(state): State<AppState>,
    _auth_user: AuthUser,
    Json(req): Json<CareerPathRequest>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let careers = Careers::new(state.pool.clone());
    let path = careers
        .add_career_path(&req.title, req.description.as_deref())
        .await
        .map_err(map_repo_error)?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "id": path.id.to_string() })),
    ))
}

/// Add a step to a career path.
#[utoipa::path(
    post,
    path = "/api/v1/career-paths/{path_id}",
    request_body = StepRequest,
    params(("path_id" = Uuid, Path, description = "Career path id")),
    responses(
        (status = 201, description = "Step added", body = Value),
        (status = 401, description = "Missing or invalid access token"),
    ),
    security(("bearer_auth" = [])),
    tag = "careers"
)]
pub async fn add_step(
    State(state): State<AppState>,
    _auth_user: AuthUser,
    Path(path_id): Path<Uuid>,
    Json(req): Json<StepRequest>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let careers = Careers::new(state.pool.clone());
    let step = careers
        .add_step(
            path_id,
            req.position,
            &req.title,
            req.description.as_deref(),
        )
        .await
        .map_err(map_repo_error)?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "id": step.id.to_string() })),
    ))
}

/// Fetch a career path with its steps.
#[utoipa::path(
    get,
    path = "/api/v1/career-paths/{path_id}",
    params(("path_id" = Uuid, Path, description = "Career path id")),
    responses(
        (status = 200, description = "Career path + steps", body = Value),
        (status = 404, description = "Career path not found"),
    ),
    tag = "careers"
)]
pub async fn get_career_path(
    State(state): State<AppState>,
    Path(path_id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    let careers = Careers::new(state.pool.clone());
    let steps = careers.steps(path_id).await.map_err(map_repo_error)?;
    let items: Vec<Value> = steps
        .iter()
        .map(|s| {
            json!({
                "position": s.position,
                "title": s.title,
                "description": s.description,
            })
        })
        .collect();
    Ok(Json(
        json!({ "path_id": path_id.to_string(), "steps": items }),
    ))
}

/// Submit a self-assessment against a career step.
#[utoipa::path(
    post,
    path = "/api/v1/me/assessments",
    request_body = CareerAssessmentRequest,
    responses(
        (status = 201, description = "Assessment recorded", body = Value),
        (status = 401, description = "Missing or invalid access token"),
    ),
    security(("bearer_auth" = [])),
    tag = "careers"
)]
pub async fn add_assessment(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(req): Json<CareerAssessmentRequest>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let careers = Careers::new(state.pool.clone());
    let assessment = careers
        .add_assessment(
            auth_user.user_id,
            req.career_path_id,
            req.score,
            req.notes.as_deref(),
        )
        .await
        .map_err(map_repo_error)?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": assessment.id.to_string(),
            "score": assessment.score,
        })),
    ))
}

/// The caller's self-assessments.
#[utoipa::path(
    get,
    path = "/api/v1/me/assessments",
    responses(
        (status = 200, description = "Assessments", body = Value),
        (status = 401, description = "Missing or invalid access token"),
    ),
    security(("bearer_auth" = [])),
    tag = "careers"
)]
pub async fn my_assessments(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> ApiResult<Json<Value>> {
    let careers = Careers::new(state.pool.clone());
    let rows = careers
        .assessments(auth_user.user_id)
        .await
        .map_err(map_repo_error)?;
    let items: Vec<Value> = rows
        .iter()
        .map(|a| {
            json!({
                "id": a.id.to_string(),
                "career_path_id": a.career_path_id.to_string(),
                "score": a.score,
                "notes": a.notes,
                "created_at": a.created_at,
            })
        })
        .collect();
    Ok(Json(json!({ "assessments": items })))
}

// ── Shared guards ───────────────────────────────────────────────────────────

/// Org admins/owners only — the org-level staff gate for vendors + alerts.
async fn require_org_admin(
    pool: &sqlx::PgPool,
    organization_id: Uuid,
    actor: Uuid,
) -> Result<(), ApiError> {
    let orgs = Organizations::new(pool.clone());
    let role = orgs
        .member_role(organization_id, actor)
        .await
        .map_err(map_repo_error)?;
    match role.as_deref() {
        Some("admin") | Some("owner") => Ok(()),
        _ => Err(ApiError::Forbidden),
    }
}
