//! Q&A API — answers, per-answer voting, acceptance, and the bounty
//! lifecycle.
//!
//! Permissions:
//!   - answering a question: any authenticated user
//!   - accepting an answer: the question's author or platform staff
//!   - voting: any authenticated user (one vote per answer, upsert/switching)
//!   - opening a bounty: the question's author
//!   - awarding a bounty: the question's author (repos enforce the
//!     open + unexpired + same-question invariants transactionally)

use crate::auth::{audit, map_repo_error, AuthUser};
use crate::error::{ApiError, ApiResult};
use crate::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use keystone_db::repositories::posts::Posts;
use keystone_db::repositories::qa::{NewBounty, Qa};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

const ANSWER_BODY_MAX: usize = 20_000;
const BOUNTY_AMOUNT_MAX: i32 = 1_000_000;

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

/// Load the question and require the actor to be its author (or staff).
async fn require_question_author(
    pool: &sqlx::PgPool,
    question_id: Uuid,
    actor: &AuthUser,
) -> Result<(), ApiError> {
    let posts = Posts::new(pool.clone());
    let post = posts
        .get_by_id(question_id)
        .await
        .map_err(map_repo_error)?
        .ok_or(ApiError::NotFound)?;
    if post.author_id != actor.user_id && !is_staff(&actor.role) {
        return Err(ApiError::Forbidden);
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
pub struct CreateAnswerRequest {
    pub body: String,
}

#[derive(Debug, Deserialize)]
pub struct VoteAnswerRequest {
    /// -1, 0 (remove), or 1.
    pub vote: i16,
}

#[derive(Debug, Deserialize)]
pub struct CreateBountyRequest {
    pub amount: i32,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Deserialize)]
pub struct AwardBountyRequest {
    pub answer_id: Uuid,
}

// ── Answers ────────────────────────────────────────────────────────────────

pub async fn create_answer(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(question_id): Path<Uuid>,
    Json(req): Json<CreateAnswerRequest>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    validate_text(&req.body, "answer body", ANSWER_BODY_MAX)?;
    let qa = Qa::new(state.pool.clone());
    let answer = qa
        .create_answer(question_id, auth_user.user_id, &req.body)
        .await
        .map_err(|e| match e {
            keystone_db::repositories::RepoError::InvalidInput(msg) => ApiError::BadRequest(msg),
            other => map_repo_error(other),
        })?;
    audit(
        &state.pool,
        auth_user.user_id,
        "answer_created",
        "answer",
        &answer.id.to_string(),
        None,
    )
    .await;
    tracing::info!(answer_id = %answer.id, question_id = %question_id, "answer created");
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "answer": {
                "id": answer.id.to_string(),
                "question_id": answer.question_id.to_string(),
                "body": answer.body,
                "created_at": answer.created_at,
            }
        })),
    ))
}

pub async fn list_answers(
    State(state): State<AppState>,
    Path(question_id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    let qa = Qa::new(state.pool.clone());
    let rows = qa.list_answers(question_id).await.map_err(map_repo_error)?;
    let items: Vec<Value> = rows
        .iter()
        .map(|a| {
            json!({
                "id": a.id.to_string(),
                "author_id": a.author_id.to_string(),
                "body": a.body,
                "score": a.score,
                "accepted": a.accepted_at.is_some(),
                "accepted_at": a.accepted_at,
                "created_at": a.created_at,
            })
        })
        .collect();
    Ok(Json(json!({ "answers": items })))
}

pub async fn vote_answer(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(answer_id): Path<Uuid>,
    Json(req): Json<VoteAnswerRequest>,
) -> ApiResult<StatusCode> {
    let qa = Qa::new(state.pool.clone());
    qa.vote_answer(answer_id, auth_user.user_id, req.vote)
        .await
        .map_err(|e| match e {
            keystone_db::repositories::RepoError::InvalidInput(msg) => ApiError::BadRequest(msg),
            other => map_repo_error(other),
        })?;
    tracing::info!(answer_id = %answer_id, actor = %auth_user.user_id, vote = req.vote, "answer vote");
    Ok(StatusCode::NO_CONTENT)
}

pub async fn accept_answer(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((question_id, answer_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    require_question_author(&state.pool, question_id, &auth_user).await?;
    let qa = Qa::new(state.pool.clone());
    qa.accept_answer(question_id, answer_id)
        .await
        .map_err(|e| match e {
            keystone_db::repositories::RepoError::InvalidInput(msg) => ApiError::BadRequest(msg),
            other => map_repo_error(other),
        })?;
    audit(
        &state.pool,
        auth_user.user_id,
        "answer_accepted",
        "answer",
        &answer_id.to_string(),
        None,
    )
    .await;
    tracing::info!(answer_id = %answer_id, question_id = %question_id, "answer accepted");
    Ok(StatusCode::NO_CONTENT)
}

// ── Bounties ───────────────────────────────────────────────────────────────

pub async fn create_bounty(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(question_id): Path<Uuid>,
    Json(req): Json<CreateBountyRequest>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    if req.amount <= 0 || req.amount > BOUNTY_AMOUNT_MAX {
        return Err(ApiError::BadRequest(format!(
            "amount must be between 1 and {BOUNTY_AMOUNT_MAX}"
        )));
    }
    if req.expires_at <= chrono::Utc::now() {
        return Err(ApiError::BadRequest(
            "expires_at must be in the future".into(),
        ));
    }
    require_question_author(&state.pool, question_id, &auth_user).await?;

    let qa = Qa::new(state.pool.clone());
    let bounty = qa
        .create_bounty(NewBounty {
            question_id,
            amount: req.amount,
            expires_at: req.expires_at,
        })
        .await
        .map_err(|e| match e {
            keystone_db::repositories::RepoError::UniqueViolation(msg) => ApiError::Conflict(msg),
            other => map_repo_error(other),
        })?;
    audit(
        &state.pool,
        auth_user.user_id,
        "bounty_created",
        "bounty",
        &bounty.id.to_string(),
        None,
    )
    .await;
    tracing::info!(bounty_id = %bounty.id, question_id = %question_id, amount = bounty.amount, "bounty opened");
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "bounty": {
                "id": bounty.id.to_string(),
                "question_id": bounty.question_id.to_string(),
                "amount": bounty.amount,
                "status": bounty.status,
                "expires_at": bounty.expires_at,
            }
        })),
    ))
}

pub async fn get_bounty(
    State(state): State<AppState>,
    Path(question_id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    let qa = Qa::new(state.pool.clone());
    let bounty = qa
        .bounty_for_question(question_id)
        .await
        .map_err(map_repo_error)?;
    match bounty {
        Some(b) => Ok(Json(json!({
            "bounty": {
                "id": b.id.to_string(),
                "amount": b.amount,
                "status": b.status,
                "expires_at": b.expires_at,
                "awarded_answer_id": b.awarded_answer_id.map(|id| id.to_string()),
                "awarded_at": b.awarded_at,
            }
        }))),
        None => Ok(Json(json!({ "bounty": Value::Null }))),
    }
}

pub async fn award_bounty(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(bounty_id): Path<Uuid>,
    Json(req): Json<AwardBountyRequest>,
) -> ApiResult<Json<Value>> {
    // The awarder must own the bounty's question.
    let qa = Qa::new(state.pool.clone());
    let bounty = qa
        .bounty_for_question_by_id(bounty_id)
        .await
        .map_err(map_repo_error)?
        .ok_or(ApiError::NotFound)?;
    require_question_author(&state.pool, bounty.question_id, &auth_user).await?;

    let awarded = qa
        .award_bounty(bounty_id, req.answer_id)
        .await
        .map_err(|e| match e {
            keystone_db::repositories::RepoError::InvalidInput(msg) => ApiError::BadRequest(msg),
            other => map_repo_error(other),
        })?;
    match awarded {
        Some(b) => {
            audit(
                &state.pool,
                auth_user.user_id,
                "bounty_awarded",
                "bounty",
                &b.id.to_string(),
                None,
            )
            .await;
            tracing::info!(bounty_id = %b.id, answer_id = %req.answer_id, "bounty awarded");
            Ok(Json(json!({
                "bounty": { "id": b.id.to_string(), "status": "awarded" }
            })))
        }
        None => Err(ApiError::BadRequest(
            "bounty is not open or has expired".into(),
        )),
    }
}
