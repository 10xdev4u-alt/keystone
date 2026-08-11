//! Learning API — Month 6: courses/progress/certificates, assessments,
//! credits, mentorship, events.
//!
//! Authorization model:
//!   - course author owns course content + assessments (moderators/admins
//!     can publish/archive as platform staff)
//!   - enroll + lesson completion require an authenticated, enrolled user;
//!     completion re-derives progress server-side and issues the certificate
//!   - certificates verify by hash token — never forge-able
//!   - assessment attempts: enrolled users; the grading key is server-side
//!     and never exposed by question reads
//!   - credits: balance reads are public to the owner; earn/redemption are
//!     append-only and double-spend-safe at the repo
//!   - mentorship: mentee requests, mentor accepts/declines/schedules;
//!     feedback one per participant
//!   - events: anyone registers; capacity + waitlist enforced at the repo

use crate::auth::{audit, map_repo_error, AuthUser};
use crate::content::{slugify, MaybeUser};
use crate::error::{ApiError, ApiResult};
use crate::social::PageQuery;
use crate::AppState;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use keystone_db::repositories::assessments::{AnswerInput, Assessments};
use keystone_db::repositories::credits::Credits;
use keystone_db::repositories::events::{Events, NewEvent};
use keystone_db::repositories::learning::{Learning, NewCourse};
use keystone_db::repositories::mentorship::Mentorship;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::Digest;
use uuid::Uuid;

const TITLE_MAX: usize = 200;

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

/// Platform staff can publish/manage any course content.
fn is_staff(role: &str) -> bool {
    matches!(role, "moderator" | "admin" | "super_admin")
}

// ── Courses ─────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct CreateCourseRequest {
    pub title: String,
    pub description: Option<String>,
}

#[derive(Deserialize)]
pub struct ModuleRequest {
    pub position: i32,
    pub title: String,
}

#[derive(Deserialize)]
pub struct LessonRequest {
    pub position: i32,
    pub title: String,
    pub content: String,
    pub duration_seconds: Option<i32>,
}

#[derive(Deserialize)]
pub struct AnswerRequest {
    pub question_id: Uuid,
    pub response: String,
}

pub async fn create_course(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(req): Json<CreateCourseRequest>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    validate_text(&req.title, "title", TITLE_MAX)?;
    let slug = slugify(&req.title);
    let learning = Learning::new(state.pool.clone());
    let course = learning
        .create_course(NewCourse {
            author_id: auth_user.user_id,
            title: &req.title,
            slug: &slug,
            description: req.description.as_deref(),
        })
        .await
        .map_err(map_repo_error)?;
    audit(
        &state.pool,
        auth_user.user_id,
        "course_created",
        "course",
        &course.id.to_string(),
        None,
    )
    .await;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "course": course_json(&course) })),
    ))
}

pub async fn list_courses(
    State(state): State<AppState>,
    Query(query): Query<PageQuery>,
) -> ApiResult<Json<Value>> {
    let learning = Learning::new(state.pool.clone());
    let rows = learning
        .published_courses(query.limit.clamp(1, 50), query.offset.max(0))
        .await
        .map_err(map_repo_error)?;
    let items: Vec<Value> = rows.iter().map(course_json).collect();
    Ok(Json(json!({ "courses": items })))
}

pub async fn get_course(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> ApiResult<Json<Value>> {
    let learning = Learning::new(state.pool.clone());
    let course = learning
        .get_course_by_slug(&slug)
        .await
        .map_err(map_repo_error)?
        .ok_or(ApiError::NotFound)?;
    let modules = learning.modules(course.id).await.map_err(map_repo_error)?;
    let mut module_tree = Vec::new();
    for module in modules {
        let lessons = learning.lessons(module.id).await.map_err(map_repo_error)?;
        module_tree.push(json!({
            "id": module.id.to_string(),
            "position": module.position,
            "title": module.title,
            "lessons": lessons.iter().map(|l| json!({
                "id": l.id.to_string(),
                "position": l.position,
                "title": l.title,
                "duration_seconds": l.duration_seconds,
            })).collect::<Vec<_>>(),
        }));
    }
    Ok(Json(
        json!({ "course": course_json(&course), "modules": module_tree }),
    ))
}

/// Publish — the course author or platform staff.
pub async fn publish_course(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(slug): Path<String>,
) -> ApiResult<StatusCode> {
    let learning = Learning::new(state.pool.clone());
    let course = learning
        .get_course_by_slug(&slug)
        .await
        .map_err(map_repo_error)?
        .ok_or(ApiError::NotFound)?;
    if course.author_id != auth_user.user_id && !is_staff(&auth_user.role) {
        return Err(ApiError::Forbidden);
    }
    learning
        .publish_course(course.id, course.author_id)
        .await
        .map_err(map_repo_error)?;
    audit(
        &state.pool,
        auth_user.user_id,
        "course_published",
        "course",
        &course.id.to_string(),
        None,
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn add_module(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(slug): Path<String>,
    Json(req): Json<ModuleRequest>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let course = course_by_slug_or_404(&state.pool, &slug).await?;
    require_course_editor(&state.pool, course.id, &auth_user).await?;
    let learning = Learning::new(state.pool.clone());
    let module = learning
        .add_module(course.id, req.position, &req.title)
        .await
        .map_err(map_repo_error)?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "id": module.id.to_string() })),
    ))
}

pub async fn add_lesson(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((slug, module_id)): Path<(String, Uuid)>,
    Json(req): Json<LessonRequest>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let course = course_by_slug_or_404(&state.pool, &slug).await?;
    require_course_editor(&state.pool, course.id, &auth_user).await?;
    let learning = Learning::new(state.pool.clone());
    let lesson = learning
        .add_lesson(
            module_id,
            req.position,
            &req.title,
            &req.content,
            req.duration_seconds,
        )
        .await
        .map_err(map_repo_error)?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "id": lesson.id.to_string() })),
    ))
}

/// Resolve a course slug to its live row or 404.
async fn course_by_slug_or_404(
    pool: &sqlx::PgPool,
    slug: &str,
) -> Result<keystone_db::repositories::learning::Course, ApiError> {
    let learning = Learning::new(pool.clone());
    learning
        .get_course_by_slug(slug)
        .await
        .map_err(map_repo_error)?
        .ok_or(ApiError::NotFound)
}

/// The course author (or platform staff) may edit content. The actor's role
/// comes from the token — consistent with every other authorization check.
async fn require_course_editor(
    pool: &sqlx::PgPool,
    course_id: Uuid,
    actor: &AuthUser,
) -> Result<(), ApiError> {
    let learning = Learning::new(pool.clone());
    let course = learning
        .get_course(course_id)
        .await
        .map_err(map_repo_error)?
        .ok_or(ApiError::NotFound)?;
    if course.author_id != actor.user_id && !is_staff(&actor.role) {
        return Err(ApiError::Forbidden);
    }
    Ok(())
}

pub async fn enroll(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(slug): Path<String>,
) -> ApiResult<StatusCode> {
    let course = course_by_slug_or_404(&state.pool, &slug).await?;
    let learning = Learning::new(state.pool.clone());
    learning
        .enroll(course.id, auth_user.user_id)
        .await
        .map_err(map_repo_error)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn complete_lesson(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((slug, lesson_id)): Path<(String, Uuid)>,
) -> ApiResult<Json<Value>> {
    let course = course_by_slug_or_404(&state.pool, &slug).await?;
    let learning = Learning::new(state.pool.clone());
    if !learning
        .is_enrolled(course.id, auth_user.user_id)
        .await
        .map_err(map_repo_error)?
    {
        return Err(ApiError::Forbidden);
    }
    // Certificate token: random bytes, hash stored, raw token returned once.
    let mut token = [0u8; 32];
    getrandom::fill(&mut token).map_err(|_| ApiError::Internal)?;
    let token_b64 =
        base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, token);
    let token_hash = hex::encode(sha2::Sha256::digest(token_b64.as_bytes()));
    let certificate = learning
        .complete_lesson(course.id, lesson_id, auth_user.user_id, &token_hash)
        .await
        .map_err(map_repo_error)?;
    Ok(Json(json!({
        "completed": true,
        "certificate": certificate.map(|c| json!({
            "id": c.id.to_string(),
            "issued_at": c.issued_at,
            "token": token_b64,
        })),
    })))
}

pub async fn course_progress(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(slug): Path<String>,
) -> ApiResult<Json<Value>> {
    let course = course_by_slug_or_404(&state.pool, &slug).await?;
    let learning = Learning::new(state.pool.clone());
    let rows = learning
        .progress_for(auth_user.user_id, course.id)
        .await
        .map_err(map_repo_error)?;
    let completed = rows.iter().filter(|r| r.completed).count();
    let total = learning
        .course_lesson_ids(course.id)
        .await
        .map_err(map_repo_error)?
        .len();
    let percent = (completed * 100).checked_div(total).unwrap_or(0);
    Ok(Json(json!({
        "course_id": course.id.to_string(),
        "completed_lessons": completed,
        "total_lessons": total,
        "percent": percent,
        "lessons": rows.iter().map(|r| json!({
            "lesson_id": r.lesson_id.to_string(),
            "completed": r.completed,
            "progress_percent": r.progress_percent,
            "completed_at": r.completed_at,
        })).collect::<Vec<_>>(),
    })))
}

pub async fn my_certificates(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> ApiResult<Json<Value>> {
    let learning = Learning::new(state.pool.clone());
    let rows = learning
        .certificates_for_user(auth_user.user_id)
        .await
        .map_err(map_repo_error)?;
    let items: Vec<Value> = rows
        .iter()
        .map(|c| {
            json!({
                "id": c.id.to_string(),
                "course_id": c.course_id.to_string(),
                "issued_at": c.issued_at,
            })
        })
        .collect();
    Ok(Json(json!({ "certificates": items })))
}

// ── Assessments ─────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct AssessmentRequest {
    pub title: String,
    pub pass_threshold: i32,
    pub time_limit_seconds: Option<i32>,
}

#[derive(Deserialize)]
pub struct QuestionRequest {
    pub position: i32,
    pub prompt: String,
    pub correct_response: Option<String>,
}

#[derive(Deserialize)]
pub struct SubmitRequest {
    pub answers: Vec<AnswerRequest>,
}

pub async fn create_assessment(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(slug): Path<String>,
    Json(req): Json<AssessmentRequest>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let course = course_by_slug_or_404(&state.pool, &slug).await?;
    require_course_editor(&state.pool, course.id, &auth_user).await?;
    let assessments = Assessments::new(state.pool.clone());
    let assessment = assessments
        .create_assessment(
            course.id,
            &req.title,
            req.pass_threshold,
            req.time_limit_seconds,
        )
        .await
        .map_err(map_repo_error)?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "id": assessment.id.to_string() })),
    ))
}

pub async fn add_question(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((slug, assessment_id)): Path<(String, Uuid)>,
    Json(req): Json<QuestionRequest>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let course = course_by_slug_or_404(&state.pool, &slug).await?;
    require_course_editor(&state.pool, course.id, &auth_user).await?;
    let assessments = Assessments::new(state.pool.clone());
    let question = assessments
        .add_question(
            assessment_id,
            req.position,
            &req.prompt,
            req.correct_response.as_deref(),
        )
        .await
        .map_err(map_repo_error)?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "id": question.id.to_string() })),
    ))
}

/// Public question read — the grading key is never selected.
pub async fn get_assessment(
    State(state): State<AppState>,
    Path(assessment_id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    let assessments = Assessments::new(state.pool.clone());
    let assessment = assessments
        .assessment(assessment_id)
        .await
        .map_err(map_repo_error)?
        .ok_or(ApiError::NotFound)?;
    let questions = assessments
        .questions(assessment_id)
        .await
        .map_err(map_repo_error)?;
    Ok(Json(json!({
        "id": assessment.id.to_string(),
        "course_id": assessment.course_id.to_string(),
        "title": assessment.title,
        "pass_threshold": assessment.pass_threshold,
        "time_limit_seconds": assessment.time_limit_seconds,
        "questions": questions.iter().map(|q| json!({
            "id": q.id.to_string(),
            "position": q.position,
            "prompt": q.prompt,
        })).collect::<Vec<_>>(),
    })))
}

pub async fn start_attempt(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(assessment_id): Path<Uuid>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let assessments = Assessments::new(state.pool.clone());
    let attempt = assessments
        .start_attempt(assessment_id, auth_user.user_id)
        .await
        .map_err(map_repo_error)?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "attempt_id": attempt.id.to_string(), "started_at": attempt.started_at })),
    ))
}

pub async fn submit_attempt(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(attempt_id): Path<Uuid>,
    Json(req): Json<SubmitRequest>,
) -> ApiResult<Json<Value>> {
    let assessments = Assessments::new(state.pool.clone());
    let answers: Vec<AnswerInput> = req
        .answers
        .into_iter()
        .map(|a| AnswerInput {
            question_id: a.question_id,
            response: a.response,
        })
        .collect();
    let attempt = assessments
        .submit_attempt(attempt_id, auth_user.user_id, &answers)
        .await
        .map_err(map_repo_error)?;
    Ok(Json(json!({
        "score": attempt.score,
        "passed": attempt.passed,
        "submitted_at": attempt.submitted_at,
    })))
}

pub async fn my_attempts(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(assessment_id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    let assessments = Assessments::new(state.pool.clone());
    let rows = assessments
        .attempts_for(auth_user.user_id, assessment_id)
        .await
        .map_err(map_repo_error)?;
    let items: Vec<Value> = rows
        .iter()
        .map(|a| {
            json!({
                "id": a.id.to_string(),
                "started_at": a.started_at,
                "submitted_at": a.submitted_at,
                "score": a.score,
                "passed": a.passed,
            })
        })
        .collect();
    Ok(Json(json!({ "attempts": items })))
}

// ── Credits ─────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct CreditRequest {
    pub amount: i32,
    pub reason: String,
}

pub async fn my_balance(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> ApiResult<Json<Value>> {
    let credits = Credits::new(state.pool.clone());
    let balance = credits
        .balance(auth_user.user_id)
        .await
        .map_err(map_repo_error)?;
    Ok(Json(json!({ "balance": balance })))
}

/// Redeem credits — double-spend-safe at the repo (SERIALIZABLE).
pub async fn redeem_credits(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(req): Json<CreditRequest>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let credits = Credits::new(state.pool.clone());
    let entry = credits
        .redeem(auth_user.user_id, req.amount, &req.reason, None, None)
        .await
        .map_err(map_repo_error)?;
    let balance = credits
        .balance(auth_user.user_id)
        .await
        .map_err(map_repo_error)?;
    audit(
        &state.pool,
        auth_user.user_id,
        "credits_redeemed",
        "credit_ledger",
        &entry.id.to_string(),
        None,
    )
    .await;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "delta": entry.delta, "balance": balance })),
    ))
}

pub async fn my_ledger(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> ApiResult<Json<Value>> {
    let credits = Credits::new(state.pool.clone());
    let rows = credits
        .ledger(auth_user.user_id)
        .await
        .map_err(map_repo_error)?;
    let items: Vec<Value> = rows
        .iter()
        .map(|e| {
            json!({
                "id": e.id.to_string(),
                "delta": e.delta,
                "reason": e.reason,
                "created_at": e.created_at,
            })
        })
        .collect();
    Ok(Json(json!({ "ledger": items })))
}

// ── Mentorship ──────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct MentorProfileRequest {
    pub bio: Option<String>,
    pub areas: Option<String>,
    pub available: bool,
}

#[derive(Deserialize)]
pub struct MentorshipRequest {
    pub message: Option<String>,
}

#[derive(Deserialize)]
pub struct SessionRequest {
    pub scheduled_at: chrono::DateTime<chrono::Utc>,
    pub duration_minutes: i32,
}

#[derive(Deserialize)]
pub struct FeedbackRequest {
    pub rating: i32,
    pub comment: Option<String>,
}

pub async fn set_mentor_profile(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(req): Json<MentorProfileRequest>,
) -> ApiResult<Json<Value>> {
    let mentorship = Mentorship::new(state.pool.clone());
    let profile = mentorship
        .set_profile(
            auth_user.user_id,
            req.bio.as_deref(),
            req.areas.as_deref(),
            req.available,
        )
        .await
        .map_err(map_repo_error)?;
    Ok(Json(json!({
        "available": profile.available,
        "areas": profile.areas,
    })))
}

pub async fn available_mentors(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let mentorship = Mentorship::new(state.pool.clone());
    let rows = mentorship
        .available_mentors()
        .await
        .map_err(map_repo_error)?;
    let items: Vec<Value> = rows
        .iter()
        .map(|p| {
            json!({
                "user_id": p.user_id.to_string(),
                "bio": p.bio,
                "areas": p.areas,
            })
        })
        .collect();
    Ok(Json(json!({ "mentors": items })))
}

pub async fn request_mentorship(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(mentor_id): Path<Uuid>,
    Json(req): Json<MentorshipRequest>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let mentorship = Mentorship::new(state.pool.clone());
    let request = mentorship
        .request(mentor_id, auth_user.user_id, req.message.as_deref())
        .await
        .map_err(map_repo_error)?;
    audit(
        &state.pool,
        auth_user.user_id,
        "mentorship_requested",
        "mentorship_request",
        &request.id.to_string(),
        None,
    )
    .await;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "request_id": request.id.to_string(), "status": request.status })),
    ))
}

pub async fn accept_mentorship(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(request_id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let mentorship = Mentorship::new(state.pool.clone());
    let accepted = mentorship
        .accept(request_id, auth_user.user_id)
        .await
        .map_err(map_repo_error)?;
    if !accepted {
        return Err(ApiError::BadRequest("no pending request to accept".into()));
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn decline_mentorship(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(request_id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let mentorship = Mentorship::new(state.pool.clone());
    let declined = mentorship
        .decline(request_id, auth_user.user_id)
        .await
        .map_err(map_repo_error)?;
    if !declined {
        return Err(ApiError::BadRequest("no pending request to decline".into()));
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn schedule_session(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(request_id): Path<Uuid>,
    Json(req): Json<SessionRequest>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let mentorship = Mentorship::new(state.pool.clone());
    // Only the mentor schedules on their accepted request.
    let request = mentorship
        .request_by_id(request_id)
        .await
        .map_err(map_repo_error)?;
    if !request.is_some_and(|r| r.mentor_id == auth_user.user_id && r.status == "accepted") {
        return Err(ApiError::Forbidden);
    }
    let session = mentorship
        .schedule_session(request_id, req.scheduled_at, req.duration_minutes)
        .await
        .map_err(map_repo_error)?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "session_id": session.id.to_string(), "status": session.status })),
    ))
}

pub async fn add_feedback(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(session_id): Path<Uuid>,
    Json(req): Json<FeedbackRequest>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let mentorship = Mentorship::new(state.pool.clone());
    let feedback = mentorship
        .add_feedback(
            session_id,
            auth_user.user_id,
            req.rating,
            req.comment.as_deref(),
        )
        .await
        .map_err(map_repo_error)?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "id": feedback.id.to_string(), "rating": feedback.rating })),
    ))
}

// ── Events ──────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct CreateEventRequest {
    pub title: String,
    pub description: Option<String>,
    pub starts_at: chrono::DateTime<chrono::Utc>,
    pub ends_at: chrono::DateTime<chrono::Utc>,
    pub capacity: Option<i32>,
    pub location: Option<String>,
}

pub async fn create_event(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(req): Json<CreateEventRequest>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    validate_text(&req.title, "title", TITLE_MAX)?;
    let slug = slugify(&req.title);
    let events = Events::new(state.pool.clone());
    let event = events
        .create(NewEvent {
            organizer_id: auth_user.user_id,
            title: &req.title,
            slug: &slug,
            description: req.description.as_deref(),
            starts_at: req.starts_at,
            ends_at: req.ends_at,
            capacity: req.capacity,
            location: req.location.as_deref(),
        })
        .await
        .map_err(map_repo_error)?;
    audit(
        &state.pool,
        auth_user.user_id,
        "event_created",
        "event",
        &event.id.to_string(),
        None,
    )
    .await;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "event": event_json(&event) })),
    ))
}

pub async fn list_events(
    State(state): State<AppState>,
    Query(query): Query<PageQuery>,
) -> ApiResult<Json<Value>> {
    let events = Events::new(state.pool.clone());
    let rows = events
        .published_events(query.limit.clamp(1, 50), query.offset.max(0))
        .await
        .map_err(map_repo_error)?;
    let items: Vec<Value> = rows.iter().map(event_json).collect();
    Ok(Json(json!({ "events": items })))
}

pub async fn get_event(
    State(state): State<AppState>,
    maybe: MaybeUser,
    Path(slug): Path<String>,
) -> ApiResult<Json<Value>> {
    let events = Events::new(state.pool.clone());
    let event = events
        .get_by_slug(&slug)
        .await
        .map_err(map_repo_error)?
        .ok_or(ApiError::NotFound)?;
    let speakers = events.speakers(event.id).await.map_err(map_repo_error)?;
    let my_status = match maybe.0 {
        Some(user) => events
            .registration_status(event.id, user.user_id)
            .await
            .map_err(map_repo_error)?,
        None => None,
    };
    Ok(Json(json!({
        "event": event_json(&event),
        "speakers": speakers.iter().map(|u| u.to_string()).collect::<Vec<_>>(),
        "my_registration": my_status,
    })))
}

pub async fn register_event(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(slug): Path<String>,
) -> ApiResult<Json<Value>> {
    let events = Events::new(state.pool.clone());
    let event = events
        .get_by_slug(&slug)
        .await
        .map_err(map_repo_error)?
        .ok_or(ApiError::NotFound)?;
    let status = events
        .register(event.id, auth_user.user_id)
        .await
        .map_err(map_repo_error)?;
    Ok(Json(json!({ "status": status })))
}

pub async fn cancel_registration(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(slug): Path<String>,
) -> ApiResult<StatusCode> {
    let events = Events::new(state.pool.clone());
    let event = events
        .get_by_slug(&slug)
        .await
        .map_err(map_repo_error)?
        .ok_or(ApiError::NotFound)?;
    events
        .cancel_registration(event.id, auth_user.user_id)
        .await
        .map_err(map_repo_error)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn add_speaker(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((slug, speaker_id)): Path<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    let events = Events::new(state.pool.clone());
    let event = events
        .get_by_slug(&slug)
        .await
        .map_err(map_repo_error)?
        .ok_or(ApiError::NotFound)?;
    if event.organizer_id != auth_user.user_id && !is_staff(&auth_user.role) {
        return Err(ApiError::Forbidden);
    }
    events
        .add_speaker(event.id, speaker_id)
        .await
        .map_err(map_repo_error)?;
    Ok(StatusCode::NO_CONTENT)
}

fn course_json(course: &keystone_db::repositories::learning::Course) -> Value {
    json!({
        "id": course.id.to_string(),
        "author_id": course.author_id.to_string(),
        "title": course.title,
        "slug": course.slug,
        "description": course.description,
        "status": course.status,
        "created_at": course.created_at,
    })
}

fn event_json(event: &keystone_db::repositories::events::Event) -> Value {
    json!({
        "id": event.id.to_string(),
        "organizer_id": event.organizer_id.to_string(),
        "title": event.title,
        "slug": event.slug,
        "description": event.description,
        "starts_at": event.starts_at,
        "ends_at": event.ends_at,
        "capacity": event.capacity,
        "location": event.location,
        "status": event.status,
    })
}
