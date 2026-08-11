//! Moderation & reviews API.
//!
//! Reports are generic `(entity_type, entity_id)`; the queue is staff-only and
//! every resolution is mirrored into the append-only `moderation_actions`
//! trail. Reviews are the consolidated table — one per (author, entity, type),
//! upsert semantics, rating constrained 1..=5 by the schema.

use crate::auth::{audit, AuthUser};
use crate::content::{CreateReportRequest, ResolveReportRequest, UpsertReviewRequest};
use crate::error::{ApiError, ApiResult};
use crate::AppState;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use keystone_db::repositories::moderation::{Moderation, NewModerationAction};
use keystone_db::repositories::reports::{NewReport, Reports};
use keystone_db::repositories::reviews::{NewReview, Reviews};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use utoipa::ToSchema;
use uuid::Uuid;

const REASON_MAX: usize = 500;
const DETAIL_MAX: usize = 5_000;
const REVIEW_TITLE_MAX: usize = 200;
const REVIEW_BODY_MAX: usize = 10_000;

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

#[derive(Debug, Deserialize, ToSchema)]
pub struct ReportQueueQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
}

fn default_limit() -> i64 {
    20
}

// ── Reports ────────────────────────────────────────────────────────────────

#[tracing::instrument(skip(state, auth_user), fields(actor = %auth_user.user_id, entity = %req.entity_type))]
/// File a moderation report against content or a user.
#[utoipa::path(
    post,
    path = "/api/v1/reports",
    request_body = CreateReportRequest,
    responses(
        (status = 201, description = "Report filed", body = Value),
        (status = 401, description = "Missing or invalid access token"),
    ),
    security(("bearer_auth" = [])),
    tag = "moderation"
)]
pub async fn file_report(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(req): Json<CreateReportRequest>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    if !matches!(
        req.entity_type.as_str(),
        "post" | "comment" | "user" | "review"
    ) {
        return Err(ApiError::BadRequest("unknown report entity type".into()));
    }
    validate_text(&req.reason, "reason", REASON_MAX)?;
    if let Some(detail) = &req.detail {
        validate_text(detail, "detail", DETAIL_MAX)?;
    }

    let reports = Reports::new(state.pool.clone());
    let report = reports
        .create(NewReport {
            reporter_id: auth_user.user_id,
            entity_type: &req.entity_type,
            entity_id: req.entity_id,
            reason: &req.reason,
            detail: req.detail.as_deref(),
        })
        .await
        .map_err(crate::auth::map_repo_error)?;

    audit(
        &state.pool,
        auth_user.user_id,
        "report_filed",
        &req.entity_type,
        &req.entity_id.to_string(),
        None,
    )
    .await;

    tracing::info!(report_id = %report.id, entity = %report.entity_type, "report filed");
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "report": {
                "id": report.id.to_string(),
                "entity_type": report.entity_type,
                "entity_id": report.entity_id.to_string(),
                "status": report.status,
                "created_at": report.created_at,
            }
        })),
    ))
}

/// One open report in the staff queue.
#[derive(Debug, Serialize, ToSchema)]
pub struct ReportView {
    pub id: String,
    pub reporter_id: String,
    pub entity_type: String,
    pub entity_id: String,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub status: String,
    pub created_at: String,
}

/// Staff queue page.
#[derive(Debug, Serialize, ToSchema)]
pub struct ReportQueueResponse {
    pub reports: Vec<ReportView>,
    pub limit: i64,
    pub offset: i64,
}

/// Staff queue of open reports.
#[utoipa::path(
    get,
    path = "/api/v1/moderation/reports",
    params(
        ("limit" = Option<i64>, Query, description = "Page size (1..=50)"),
        ("offset" = Option<i64>, Query, description = "Page offset"),
    ),
    responses(
        (status = 200, description = "Open reports", body = ReportQueueResponse),
        (status = 401, description = "Missing or invalid access token"),
        (status = 403, description = "Not a moderator"),
    ),
    security(("bearer_auth" = [])),
    tag = "moderation"
)]
pub async fn report_queue(
    State(state): State<AppState>,
    _auth_user: AuthUser, // guarded by require_moderator at the router
    Query(query): Query<ReportQueueQuery>,
) -> ApiResult<Json<ReportQueueResponse>> {
    let limit = query.limit.clamp(1, 50);
    let offset = query.offset.max(0);
    let reports = Reports::new(state.pool.clone());
    let rows = reports
        .list_open(limit, offset)
        .await
        .map_err(|e| match e {
            keystone_db::repositories::RepoError::Database(e) => ApiError::Database(e),
            other => ApiError::BadRequest(other.to_string()),
        })?;
    let items = rows
        .into_iter()
        .map(|r| ReportView {
            id: r.id.to_string(),
            reporter_id: r.reporter_id.to_string(),
            entity_type: r.entity_type,
            entity_id: r.entity_id.to_string(),
            reason: r.reason,
            detail: r.detail,
            status: r.status,
            created_at: r.created_at.to_rfc3339(),
        })
        .collect();
    Ok(Json(ReportQueueResponse {
        reports: items,
        limit,
        offset,
    }))
}

#[tracing::instrument(skip(state, auth_user), fields(actor = %auth_user.user_id, report_id = %id))]
/// Resolve a report with a moderation action. Staff only.
#[utoipa::path(
    post,
    path = "/api/v1/moderation/reports/{id}/resolve",
    request_body = ResolveReportRequest,
    params(("id" = Uuid, Path, description = "Report id")),
    responses(
        (status = 200, description = "Resolution + action recorded", body = Value),
        (status = 401, description = "Missing or invalid access token"),
        (status = 403, description = "Not a moderator"),
    ),
    security(("bearer_auth" = [])),
    tag = "moderation"
)]
pub async fn resolve_report(
    State(state): State<AppState>,
    auth_user: AuthUser, // guarded by require_moderator at the router
    Path(id): Path<Uuid>,
    Json(req): Json<ResolveReportRequest>,
) -> ApiResult<Json<Value>> {
    if let Some(note) = &req.resolution_note {
        validate_text(note, "resolution note", DETAIL_MAX)?;
    }
    let reports = Reports::new(state.pool.clone());
    let report = reports
        .update_status(
            id,
            "resolved",
            auth_user.user_id,
            req.resolution_note.as_deref(),
        )
        .await
        .map_err(crate::auth::map_repo_error)?
        .ok_or(ApiError::NotFound)?;

    // The append-only trail records the decision for accountability.
    let moderation = Moderation::new(state.pool.clone());
    let action = match report.entity_type.as_str() {
        "comment" => "hide_comment",
        "user" => "suspend_user",
        _ => "delete_post",
    };
    moderation
        .record(NewModerationAction {
            moderator_id: auth_user.user_id,
            action,
            target_type: &report.entity_type,
            target_id: report.entity_id,
            reason: report.resolution_note.as_deref(),
        })
        .await
        .map_err(crate::auth::map_repo_error)?;

    audit(
        &state.pool,
        auth_user.user_id,
        "report_resolved",
        &report.entity_type,
        &report.entity_id.to_string(),
        None,
    )
    .await;

    tracing::info!(report_id = %id, "report resolved");
    Ok(Json(json!({
        "report": {
            "id": report.id.to_string(),
            "status": report.status,
            "resolved_by": report.resolved_by.map(|u| u.to_string()),
            "resolved_at": report.resolved_at,
        }
    })))
}

// ── Reviews ────────────────────────────────────────────────────────────────

#[tracing::instrument(skip(state, auth_user), fields(actor = %auth_user.user_id, entity = %req.entity_type))]
/// Create or update a review of an entity (one per user per entity).
#[utoipa::path(
    put,
    path = "/api/v1/reviews",
    request_body = UpsertReviewRequest,
    responses(
        (status = 200, description = "Review upserted", body = Value),
        (status = 401, description = "Missing or invalid access token"),
    ),
    security(("bearer_auth" = [])),
    tag = "moderation"
)]
pub async fn upsert_review(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(req): Json<UpsertReviewRequest>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    if !matches!(
        req.entity_type.as_str(),
        "employer" | "vendor" | "organization" | "course" | "mentor"
    ) {
        return Err(ApiError::BadRequest("unknown review entity type".into()));
    }
    if !(1..=5).contains(&req.rating) {
        return Err(ApiError::BadRequest(
            "rating must be between 1 and 5".into(),
        ));
    }
    if let Some(title) = &req.title {
        validate_text(title, "title", REVIEW_TITLE_MAX)?;
    }
    if let Some(body) = &req.body {
        validate_text(body, "body", REVIEW_BODY_MAX)?;
    }

    let reviews = Reviews::new(state.pool.clone());
    let review = reviews
        .upsert(NewReview {
            author_id: auth_user.user_id,
            entity_type: &req.entity_type,
            entity_id: req.entity_id,
            rating: req.rating,
            title: req.title.as_deref(),
            body: req.body.as_deref(),
        })
        .await
        .map_err(crate::auth::map_repo_error)?;

    audit(
        &state.pool,
        auth_user.user_id,
        "review_upserted",
        &req.entity_type,
        &req.entity_id.to_string(),
        None,
    )
    .await;

    tracing::info!(review_id = %review.id, entity = %req.entity_type, "review upserted");
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "review": {
                "id": review.id.to_string(),
                "entity_type": review.entity_type,
                "entity_id": review.entity_id.to_string(),
                "rating": review.rating,
                "title": review.title,
                "body": review.body,
                "created_at": review.created_at,
                "updated_at": review.updated_at,
            }
        })),
    ))
}

/// Reviews for an entity.
#[utoipa::path(
    get,
    path = "/api/v1/reviews",
    responses(
        (status = 200, description = "Reviews", body = Value),
    ),
    tag = "moderation"
)]
pub async fn list_reviews(
    State(state): State<AppState>,
    Query(query): Query<crate::content::ReviewQuery>,
) -> ApiResult<Json<Value>> {
    let reviews = Reviews::new(state.pool.clone());
    let rows = reviews
        .list_by_entity(&query.entity_type, query.entity_id)
        .await
        .map_err(crate::auth::map_repo_error)?;
    let items: Vec<Value> = rows
        .into_iter()
        .map(|r| {
            json!({
                "id": r.id.to_string(),
                "author_id": r.author_id.to_string(),
                "rating": r.rating,
                "title": r.title,
                "body": r.body,
                "created_at": r.created_at,
            })
        })
        .collect();
    Ok(Json(json!({ "reviews": items })))
}
