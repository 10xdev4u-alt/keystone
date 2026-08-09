//! Auth HTTP surface — register, verify, login, refresh, logout, me.
//!
//! Security posture (each item maps to a threat-model rule):
//! - Access token: returned in the body only; the SPA keeps it in memory.
//! - Refresh token: opaque, random, delivered ONLY in an httpOnly SameSite=Strict
//!   cookie scoped to `/api/v1/auth`; stored hashed (SHA-256), never in clear.
//! - Rotation on every refresh; presenting a rotated-away token revokes the
//!   whole session family (`token_reuse_detected`).
//! - Login errors are deliberately generic (no user enumeration).
//! - Account lockout with exponential backoff before password verification.
//! - `email_lower`/`status`/`role` live in the DB; the JWT only mirrors them.

use crate::error::{ApiError, ApiResult};
use crate::AppState;
use axum::extract::{FromRequestParts, State};
use axum::http::header::{self, HeaderMap, HeaderValue};
use axum::http::{request::Parts, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use keystone_auth::password::PasswordHasher;
use keystone_auth::service::LockoutPolicy;
use keystone_auth::tokens;
use keystone_db::repositories::sessions::{NewSession, Sessions};
use keystone_db::repositories::users::{NewUser, Users};
use keystone_db::repositories::RepoError;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::PgPool;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

/// Single source of truth for the refresh cookie name (the HrX `accessToken`
/// vs `access_token` drift exists precisely because this was duplicated).
pub const REFRESH_COOKIE: &str = "keystone_refresh";
const REFRESH_COOKIE_PATH: &str = "/api/v1/auth";

/// Auth dependencies bundled into app state.
#[derive(Clone)]
pub struct AuthServices {
    pub password: Arc<PasswordHasher>,
    pub jwt: Arc<keystone_auth::jwt::AccessTokenService>,
    pub lockout: LockoutPolicy,
    pub access_ttl: Duration,
    pub refresh_ttl: Duration,
    pub secure_cookies: bool,
}

// ── Request / response types ────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    pub email: String,
    pub password: String,
    #[serde(default)]
    pub first_name: Option<String>,
    #[serde(default)]
    pub last_name: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct VerifyEmailRequest {
    pub token: String,
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct UserView {
    pub id: String,
    pub email: String,
    pub username: Option<String>,
    pub role: String,
    pub status: String,
    pub is_verified: bool,
}

#[derive(Debug, Serialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: &'static str,
    pub expires_in: u64,
    pub user: UserView,
}

// ── Cookie helpers ──────────────────────────────────────────────────────────

fn refresh_cookie(value: &str, max_age: Duration, secure: bool) -> HeaderValue {
    let mut cookie = format!(
        "{REFRESH_COOKIE}={value}; Max-Age={}; Path={REFRESH_COOKIE_PATH}; HttpOnly; SameSite=Strict",
        max_age.as_secs()
    );
    if secure {
        cookie.push_str("; Secure");
    }
    HeaderValue::from_str(&cookie).expect("cookie header must be valid")
}

fn read_refresh_cookie(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    raw.split(';').find_map(|part| {
        let part = part.trim();
        part.strip_prefix(&format!("{REFRESH_COOKIE}="))
            .map(str::to_owned)
    })
}

/// Best-effort client IP: first `X-Forwarded-For` entry when behind a proxy,
/// else none. Trust boundary documented in the threat model.
fn client_ip(headers: &HeaderMap) -> Option<IpAddr> {
    let value = headers.get("x-forwarded-for")?.to_str().ok()?;
    value.split(',').next()?.trim().parse().ok()
}

// ── JWT extractor ───────────────────────────────────────────────────────────

/// Authenticated identity extracted from the `Authorization: Bearer` header.
pub struct AuthUser {
    pub user_id: uuid::Uuid,
    pub role: String,
    pub impersonator_id: Option<String>,
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let header = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .ok_or(ApiError::Unauthorized)?;
        let token = header
            .strip_prefix("Bearer ")
            .ok_or(ApiError::Unauthorized)?;
        let parsed = state
            .auth
            .jwt
            .verify(token)
            .map_err(|_| ApiError::Unauthorized)?;
        let user_id = parsed.user_id.parse().map_err(|_| ApiError::Unauthorized)?;
        Ok(AuthUser {
            user_id,
            role: parsed.role,
            impersonator_id: parsed.impersonator_id,
        })
    }
}

// ── Handlers ────────────────────────────────────────────────────────────────

pub async fn register(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<RegisterRequest>,
) -> ApiResult<Response> {
    keystone_auth::email::validate(&req.email).map_err(|e| ApiError::BadRequest(e.into()))?;
    keystone_auth::password::validate(&req.password)
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;

    let hash = state
        .auth
        .password
        .hash(&req.password)
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;

    let users = Users::new(state.pool.clone());
    let user = users
        .create(NewUser {
            email: &req.email,
            password_hash: &hash,
            first_name: req.first_name.as_deref(),
            last_name: req.last_name.as_deref(),
            username: req.username.as_deref(),
        })
        .await
        .map_err(map_repo_error)?;

    // Email verification: raw token to the caller (dev flow only — the mailer
    // milestone replaces this with an actual email). Only the hash is stored.
    let verification_token = tokens::generate_refresh_token().map_err(|_| ApiError::Internal)?;
    let token_hash = tokens::hash_refresh_token(&verification_token);
    sqlx::query(
        r#"
        INSERT INTO email_verifications (user_id, token_hash, expires_at)
        VALUES ($1, $2, now() + interval '24 hours')
        "#,
    )
    .bind(user.id)
    .bind(&token_hash)
    .execute(&state.pool)
    .await?;

    audit(
        &state.pool,
        user.id,
        "auth.register",
        "user",
        &user.id.to_string(),
        client_ip(&headers),
    )
    .await;

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "user_id": user.id,
            "verification_token": verification_token, // DEV ONLY — mailer milestone replaces this
        })),
    )
        .into_response())
}

pub async fn verify_email(
    State(state): State<AppState>,
    Json(req): Json<VerifyEmailRequest>,
) -> ApiResult<Response> {
    let token_hash = tokens::hash_refresh_token(&req.token);
    let row: Option<(uuid::Uuid,)> = sqlx::query_as(
        r#"
        SELECT user_id FROM email_verifications
        WHERE token_hash = $1 AND used_at IS NULL AND expires_at > now()
        "#,
    )
    .bind(&token_hash)
    .fetch_optional(&state.pool)
    .await?;
    let Some((user_id,)) = row else {
        return Err(ApiError::BadRequest(
            "invalid or expired verification token".into(),
        ));
    };

    let mut tx = state.pool.begin().await?;
    sqlx::query("UPDATE email_verifications SET used_at = now() WHERE token_hash = $1")
        .bind(&token_hash)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE users SET status = 'active', is_verified = true WHERE id = $1")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    audit(
        &state.pool,
        user_id,
        "auth.verify_email",
        "user",
        &user_id.to_string(),
        None,
    )
    .await;
    Ok(Json(json!({ "status": "verified" })).into_response())
}

pub async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<LoginRequest>,
) -> ApiResult<Response> {
    let users = Users::new(state.pool.clone());
    // Generic failure for both unknown email and wrong password (no enumeration).
    let Some(user) = users
        .find_by_email(&req.email)
        .await
        .map_err(map_repo_error)?
    else {
        return Err(ApiError::Unauthorized);
    };

    // Lockout before doing any password work.
    let now = std::time::SystemTime::now();
    let last_failure = users
        .last_failure_at(user.id)
        .await
        .map_err(map_repo_error)?
        .map(|t| t.into())
        .unwrap_or(now);
    let window_ago = now.checked_sub(state.auth.lockout.window).unwrap_or(now);
    let failures = users
        .recent_failure_count(user.id, window_ago.into())
        .await
        .map_err(map_repo_error)?;
    if state
        .auth
        .lockout
        .evaluate(failures, last_failure, now)
        .is_err()
    {
        return Err(ApiError::TooManyRequests);
    }

    // Password verification.
    let stored = user
        .password_hash
        .as_deref()
        .ok_or(ApiError::Unauthorized)?;
    let password_ok = state
        .auth
        .password
        .verify(&req.password, stored)
        .map_err(|_| ApiError::Unauthorized)?;
    if !password_ok {
        users
            .record_failed_login(user.id, client_ip(&headers).as_ref())
            .await
            .map_err(map_repo_error)?;
        return Err(ApiError::Unauthorized);
    }
    users.record_login(user.id).await.map_err(map_repo_error)?;

    audit(
        &state.pool,
        user.id,
        "auth.login",
        "user",
        &user.id.to_string(),
        client_ip(&headers),
    )
    .await;
    issue_session(&state, user.id, &user.role, &headers).await
}

pub async fn refresh(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Response> {
    let token = read_refresh_cookie(&headers).ok_or(ApiError::Unauthorized)?;
    let hash = tokens::hash_refresh_token(&token);
    let sessions = Sessions::new(state.pool.clone());

    if let Some(live) = sessions
        .find_live_by_hash(&hash)
        .await
        .map_err(map_repo_error)?
    {
        // Rotation: old session revoked, new one linked as its replacement.
        let new_token = tokens::generate_refresh_token().map_err(|_| ApiError::Internal)?;
        let new_hash = tokens::hash_refresh_token(&new_token);
        let expires_at = chrono::Utc::now()
            + chrono::Duration::from_std(state.auth.refresh_ttl).expect("ttl in range");
        let new_session = sessions
            .create(NewSession {
                user_id: live.user_id,
                refresh_token_hash: &new_hash,
                expires_at,
                user_agent: None,
                ip_address: client_ip(&headers),
            })
            .await
            .map_err(map_repo_error)?;
        sessions
            .rotate(live.id, new_session.id)
            .await
            .map_err(map_repo_error)?;

        let users = Users::new(state.pool.clone());
        let user = users
            .find_by_id(live.user_id)
            .await
            .map_err(map_repo_error)?
            .ok_or(ApiError::Unauthorized)?;
        return respond_with_tokens(&state, user, &new_token);
    }

    // Not live: is this a rotated-away token being replayed? Revoke the family.
    if let Some(any) = sessions
        .find_any_by_hash(&hash)
        .await
        .map_err(map_repo_error)?
    {
        for live in sessions
            .live_for_user(any.user_id)
            .await
            .map_err(map_repo_error)?
        {
            let ancestors = sessions
                .ancestor_hashes(live.id)
                .await
                .map_err(map_repo_error)?;
            if ancestors.iter().any(|a| a == &hash) {
                sessions
                    .revoke_family(live.id)
                    .await
                    .map_err(map_repo_error)?;
                audit(
                    &state.pool,
                    any.user_id,
                    "auth.token_reuse_detected",
                    "session",
                    &live.id.to_string(),
                    client_ip(&headers),
                )
                .await;
                return Err(ApiError::ReuseDetected);
            }
        }
    }
    Err(ApiError::Unauthorized)
}

pub async fn logout(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Response> {
    if let Some(token) = read_refresh_cookie(&headers) {
        let hash = tokens::hash_refresh_token(&token);
        let sessions = Sessions::new(state.pool.clone());
        if let Some(any) = sessions
            .find_any_by_hash(&hash)
            .await
            .map_err(map_repo_error)?
        {
            sessions
                .revoke_family(any.id)
                .await
                .map_err(map_repo_error)?;
        }
    }
    let mut response = StatusCode::NO_CONTENT.into_response();
    // Clearing cookie: Max-Age=0, same attributes.
    response.headers_mut().append(
        header::SET_COOKIE,
        refresh_cookie("", Duration::ZERO, state.auth.secure_cookies),
    );
    Ok(response)
}

pub async fn me(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> ApiResult<Json<serde_json::Value>> {
    let users = Users::new(state.pool.clone());
    let user = users
        .find_by_id(auth_user.user_id)
        .await
        .map_err(map_repo_error)?
        .ok_or(ApiError::Unauthorized)?;
    Ok(Json(json!({ "user": UserView {
        id: user.id.to_string(),
        email: user.email,
        username: user.username,
        role: user.role,
        status: user.status,
        is_verified: user.is_verified,
    } })))
}

// ── Shared helpers ──────────────────────────────────────────────────────────

/// Issue an access token + refresh session for a just-authenticated user.
async fn issue_session(
    state: &AppState,
    user_id: uuid::Uuid,
    role: &str,
    headers: &HeaderMap,
) -> ApiResult<Response> {
    let refresh_token = tokens::generate_refresh_token().map_err(|_| ApiError::Internal)?;
    let refresh_hash = tokens::hash_refresh_token(&refresh_token);
    let expires_at = chrono::Utc::now()
        + chrono::Duration::from_std(state.auth.refresh_ttl).expect("ttl in range");
    let sessions = Sessions::new(state.pool.clone());
    sessions
        .create(NewSession {
            user_id,
            refresh_token_hash: &refresh_hash,
            expires_at,
            user_agent: headers
                .get(header::USER_AGENT)
                .and_then(|v| v.to_str().ok()),
            ip_address: client_ip(headers),
        })
        .await
        .map_err(map_repo_error)?;

    let users = Users::new(state.pool.clone());
    let user = users
        .find_by_id(user_id)
        .await
        .map_err(map_repo_error)?
        .ok_or(ApiError::Unauthorized)?;
    let _ = role;
    respond_with_tokens(state, user, &refresh_token)
}

/// Build the token response: access token in body, refresh cookie on the side.
fn respond_with_tokens(
    state: &AppState,
    user: keystone_db::repositories::users::User,
    refresh_token: &str,
) -> ApiResult<Response> {
    let access_token = state
        .auth
        .jwt
        .issue(&user.id.to_string(), &user.role, None)
        .map_err(|_| ApiError::Internal)?;

    let body = Json(TokenResponse {
        access_token,
        token_type: "Bearer",
        expires_in: state.auth.access_ttl.as_secs(),
        user: UserView {
            id: user.id.to_string(),
            email: user.email,
            username: user.username,
            role: user.role,
            status: user.status,
            is_verified: user.is_verified,
        },
    });

    let mut response = (StatusCode::OK, body).into_response();
    response.headers_mut().append(
        header::SET_COOKIE,
        refresh_cookie(
            refresh_token,
            state.auth.refresh_ttl,
            state.auth.secure_cookies,
        ),
    );
    Ok(response)
}

/// Append-only audit event; best-effort (auditing must never break a request).
async fn audit(
    pool: &PgPool,
    actor: uuid::Uuid,
    action: &str,
    entity_type: &str,
    entity_id: &str,
    ip: Option<IpAddr>,
) {
    let _ = sqlx::query(
        r#"
        INSERT INTO audit_logs (actor_user_id, action, entity_type, entity_id, ip_address)
        VALUES ($1, $2, $3, $4, $5)
        "#,
    )
    .bind(actor)
    .bind(action)
    .bind(entity_type)
    .bind(entity_id)
    .bind(ip)
    .execute(pool)
    .await;
}

fn map_repo_error(err: RepoError) -> ApiError {
    match err {
        RepoError::EmailTaken => ApiError::Conflict("email is already registered".into()),
        RepoError::UniqueViolation(msg) => ApiError::Conflict(msg),
        RepoError::Database(e) => ApiError::Database(e),
    }
}
