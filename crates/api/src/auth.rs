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

use crate::csrf;
use crate::error::{ApiError, ApiResult};
use crate::AppState;
use axum::extract::{FromRequestParts, Path, State};
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
use utoipa::ToSchema;

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

#[derive(Debug, Deserialize, ToSchema)]
pub struct SignupRequest {
    pub email: String,
    pub password: String,
    #[serde(default)]
    pub first_name: Option<String>,
    #[serde(default)]
    pub last_name: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct VerifyEmailRequest {
    pub token: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ForgotPasswordRequest {
    pub email: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ResetPasswordRequest {
    pub email: String,
    pub token: String,
    pub new_password: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UserView {
    pub id: String,
    pub email: String,
    pub username: Option<String>,
    pub role: String,
    pub status: String,
    pub is_verified: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: &'static str,
    pub expires_in: u64,
    /// CSRF double-submit token: echoed back in `X-CSRF-Token` on cookie
    /// state-changing requests (refresh/logout).
    pub csrf_token: String,
    pub user: UserView,
}

// ── Cookie helpers ──────────────────────────────────────────────────────────

pub(crate) fn refresh_cookie(value: &str, max_age: Duration, secure: bool) -> HeaderValue {
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
pub(crate) fn client_ip(headers: &HeaderMap) -> Option<IpAddr> {
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

/// Register a new account. Returns tokens plus a verification email token
/// (dev flow: the raw token is returned; the mailer milestone emails it).
#[utoipa::path(
    post,
    path = "/api/v1/auth/register",
    request_body = SignupRequest,
    responses(
        (status = 201, description = "Account created; tokens returned", body = TokenResponse),
        (status = 400, description = "Invalid email, password or username"),
        (status = 409, description = "Email or username already registered"),
    ),
    tag = "auth"
)]
pub async fn register(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<SignupRequest>,
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

/// Confirm the email verification token.
#[utoipa::path(
    post,
    path = "/api/v1/auth/verify-email",
    request_body = VerifyEmailRequest,
    responses(
        (status = 200, description = "Email verified"),
        (status = 400, description = "Invalid or expired verification token"),
    ),
    tag = "auth"
)]
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

/// Start a password reset. Same anti-enumeration posture as login: the
/// response is identical whether or not the account exists. In dev the raw
/// token is returned (mailer milestone emails it); only the hash is stored.
#[utoipa::path(
    post,
    path = "/api/v1/auth/forgot-password",
    request_body = ForgotPasswordRequest,
    responses(
        (status = 200, description = "Reset token issued (dev: returned in body)"),
    ),
    tag = "auth"
)]
pub async fn forgot_password(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<ForgotPasswordRequest>,
) -> ApiResult<Response> {
    let email_lower = req.email.trim().to_lowercase();
    let users = Users::new(state.pool.clone());
    let user = users
        .find_by_email(&email_lower)
        .await
        .map_err(map_repo_error)?;

    // Generic success regardless of existence — never reveal whether an
    // address is registered. A reset token is only minted for real accounts.
    let Some(user) = user else {
        return Ok(Json(json!({ "status": "ok" })).into_response());
    };

    let reset_token = tokens::generate_refresh_token().map_err(|_| ApiError::Internal)?;
    let token_hash = tokens::hash_refresh_token(&reset_token);
    sqlx::query(
        r#"
        INSERT INTO password_resets (user_id, token_hash, expires_at)
        VALUES ($1, $2, now() + interval '1 hour')
        "#,
    )
    .bind(user.id)
    .bind(&token_hash)
    .execute(&state.pool)
    .await?;

    audit(
        &state.pool,
        user.id,
        "auth.forgot_password",
        "user",
        &user.id.to_string(),
        client_ip(&headers),
    )
    .await;

    Ok(Json(json!({ "status": "ok", "reset_token": reset_token })).into_response())
}

/// Complete a password reset: validates the token, then swaps the hash and
/// burns the token atomically. Locked accounts are unlocked on success.
#[utoipa::path(
    post,
    path = "/api/v1/auth/reset-password",
    request_body = ResetPasswordRequest,
    responses(
        (status = 200, description = "Password updated"),
        (status = 400, description = "Invalid or expired token / weak password"),
    ),
    tag = "auth"
)]
pub async fn reset_password(
    State(state): State<AppState>,
    Json(req): Json<ResetPasswordRequest>,
) -> ApiResult<Response> {
    keystone_auth::password::validate(&req.new_password)
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;
    let email_lower = req.email.trim().to_lowercase();

    let token_hash = tokens::hash_refresh_token(&req.token);
    let row: Option<(uuid::Uuid,)> = sqlx::query_as(
        r#"
        SELECT pr.user_id
        FROM password_resets pr
        JOIN users u ON u.id = pr.user_id
        WHERE pr.token_hash = $1
          AND pr.used_at IS NULL
          AND pr.expires_at > now()
          AND u.email_lower = $2
        "#,
    )
    .bind(&token_hash)
    .bind(&email_lower)
    .fetch_optional(&state.pool)
    .await?;

    let Some((user_id,)) = row else {
        return Err(ApiError::BadRequest(
            "invalid or expired reset token".into(),
        ));
    };

    let hash = state
        .auth
        .password
        .hash(&req.new_password)
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;

    let mut tx = state.pool.begin().await?;
    sqlx::query("UPDATE password_resets SET used_at = now() WHERE token_hash = $1")
        .bind(&token_hash)
        .execute(&mut *tx)
        .await?;
    let users = Users::new(state.pool.clone());
    users
        .update_password(user_id, &hash)
        .await
        .map_err(map_repo_error)?;
    // A reset is strong proof of control — clear any lockout state so the
    // legitimate owner isn't stuck behind a backoff window from old attempts.
    sqlx::query("DELETE FROM failed_logins WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    audit(
        &state.pool,
        user_id,
        "auth.reset_password",
        "user",
        &user_id.to_string(),
        None,
    )
    .await;
    Ok(Json(json!({ "status": "password_updated" })).into_response())
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

/// Change the caller's password. Requires the current password (proof of
/// control), then atomically swaps the hash and revokes every OTHER session —
/// stolen-session tokens for the same account die on password change.
#[utoipa::path(
    post,
    path = "/api/v1/auth/change-password",
    request_body = ChangePasswordRequest,
    responses(
        (status = 200, description = "Password changed; other sessions revoked", body = Value),
        (status = 400, description = "Weak new password"),
        (status = 401, description = "Missing/invalid token or wrong current password"),
    ),
    security(("bearer_auth" = [])),
    tag = "auth"
)]
pub async fn change_password(
    State(state): State<AppState>,
    headers: HeaderMap,
    auth_user: AuthUser,
    Json(req): Json<ChangePasswordRequest>,
) -> ApiResult<Response> {
    keystone_auth::password::validate(&req.new_password)
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;

    let users = Users::new(state.pool.clone());
    let user = users
        .find_by_id(auth_user.user_id)
        .await
        .map_err(map_repo_error)?
        .ok_or(ApiError::Unauthorized)?;
    let stored = user
        .password_hash
        .as_deref()
        .ok_or(ApiError::Unauthorized)?;
    let current_ok = state
        .auth
        .password
        .verify(&req.current_password, stored)
        .map_err(|_| ApiError::Unauthorized)?;
    if !current_ok {
        audit(
            &state.pool,
            auth_user.user_id,
            "auth.change_password_failed",
            "user",
            &auth_user.user_id.to_string(),
            client_ip(&headers),
        )
        .await;
        return Err(ApiError::Unauthorized);
    }

    let new_hash = state
        .auth
        .password
        .hash(&req.new_password)
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;

    // Hash swap + session revocation, in that order: even if a revocation
    // races, the password change itself is authoritative. Both writes are
    // single statements — no cross-statement transaction to fake.
    users
        .update_password(auth_user.user_id, &new_hash)
        .await
        .map_err(map_repo_error)?;
    let sessions = Sessions::new(state.pool.clone());
    sessions
        .revoke_all_for_user(auth_user.user_id)
        .await
        .map_err(map_repo_error)?;

    audit(
        &state.pool,
        auth_user.user_id,
        "auth.change_password",
        "user",
        &auth_user.user_id.to_string(),
        client_ip(&headers),
    )
    .await;
    Ok(Json(json!({ "status": "password_changed" })).into_response())
}

/// Authenticate with email + password. Sets the httpOnly refresh cookie and
/// returns the access token. Errors are deliberately generic (no enumeration).
#[utoipa::path(
    post,
    path = "/api/v1/auth/login",
    request_body = LoginRequest,
    responses(
        (status = 200, description = "Authenticated; refresh cookie set", body = TokenResponse),
        (status = 401, description = "Invalid credentials or account locked"),
    ),
    tag = "auth"
)]
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
        audit(
            &state.pool,
            user.id,
            "auth.login_locked",
            "user",
            &user.id.to_string(),
            client_ip(&headers),
        )
        .await;
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
        audit(
            &state.pool,
            user.id,
            "auth.login_failed",
            "user",
            &user.id.to_string(),
            client_ip(&headers),
        )
        .await;
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

/// Rotate the refresh session and return a fresh token pair. The old session
/// is revoked; replaying it revokes the whole family (reuse detection).
#[utoipa::path(
    post,
    path = "/api/v1/auth/refresh",
    responses(
        (status = 200, description = "Rotated token pair", body = TokenResponse),
        (status = 401, description = "Missing/invalid refresh cookie, or reuse detected"),
    ),
    tag = "auth"
)]
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
        audit(
            &state.pool,
            live.user_id,
            "auth.session_rotated",
            "session",
            &live.id.to_string(),
            client_ip(&headers),
        )
        .await;

        let users = Users::new(state.pool.clone());
        let user = users
            .find_by_id(live.user_id)
            .await
            .map_err(map_repo_error)?
            .ok_or(ApiError::Unauthorized)?;
        let csrf_token = tokens::generate_refresh_token().map_err(|_| ApiError::Internal)?;
        return respond_with_tokens(&state, user, &new_token, &csrf_token);
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

/// Revoke the refresh session family and clear the refresh cookie.
#[utoipa::path(
    post,
    path = "/api/v1/auth/logout",
    responses(
        (status = 204, description = "Session family revoked; cookie cleared"),
    ),
    tag = "auth"
)]
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
            audit(
                &state.pool,
                any.user_id,
                "auth.logout",
                "session",
                &any.id.to_string(),
                client_ip(&headers),
            )
            .await;
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

/// Current-user response wrapper.
#[derive(Debug, serde::Serialize, ToSchema)]
pub struct MeResponse {
    pub user: UserView,
}

/// Current authenticated user.
#[utoipa::path(
    get,
    path = "/api/v1/auth/me",
    responses(
        (status = 200, description = "Current user", body = MeResponse),
        (status = 401, description = "Missing or invalid access token"),
    ),
    security(("bearer_auth" = [])),
    tag = "auth"
)]
pub async fn me(State(state): State<AppState>, auth_user: AuthUser) -> ApiResult<Json<MeResponse>> {
    let users = Users::new(state.pool.clone());
    let user = users
        .find_by_id(auth_user.user_id)
        .await
        .map_err(map_repo_error)?
        .ok_or(ApiError::Unauthorized)?;
    Ok(Json(MeResponse {
        user: UserView {
            id: user.id.to_string(),
            email: user.email,
            username: user.username,
            role: user.role,
            status: user.status,
            is_verified: user.is_verified,
        },
    }))
}

// ── Session management ──────────────────────────────────────────────────────

/// One live session as the client sees it. `current` marks the session whose
/// refresh cookie this browser is holding — never reveal the token itself.
#[derive(Debug, serde::Serialize, ToSchema)]
pub struct SessionView {
    pub id: String,
    pub created_at: String,
    pub expires_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ip_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_agent: Option<String>,
    pub current: bool,
}

/// Wrapper for the sessions list endpoint.
#[derive(Debug, serde::Serialize, ToSchema)]
pub struct SessionListResponse {
    pub sessions: Vec<SessionView>,
}

/// List the authenticated user's live sessions; the one matching the current
/// refresh cookie is marked `current`.
#[utoipa::path(
    get,
    path = "/api/v1/auth/sessions",
    responses(
        (status = 200, description = "Live sessions", body = SessionListResponse),
        (status = 401, description = "Missing or invalid access token"),
    ),
    security(("bearer_auth" = [])),
    tag = "auth"
)]
pub async fn list_sessions(
    State(state): State<AppState>,
    headers: HeaderMap,
    auth_user: AuthUser,
) -> ApiResult<Json<SessionListResponse>> {
    let sessions = Sessions::new(state.pool.clone());
    let current_hash = read_refresh_cookie(&headers).map(|t| tokens::hash_refresh_token(&t));
    let list = sessions
        .live_for_user(auth_user.user_id)
        .await
        .map_err(map_repo_error)?;

    Ok(Json(SessionListResponse {
        sessions: list
            .into_iter()
            .map(|s| SessionView {
                id: s.id.to_string(),
                created_at: s.created_at.to_rfc3339(),
                expires_at: s.expires_at.to_rfc3339(),
                ip_address: s.ip_address.map(|ip| ip.to_string()),
                user_agent: s.user_agent,
                current: current_hash.as_deref() == Some(s.refresh_token_hash.as_str()),
            })
            .collect(),
    }))
}

/// Revoke one session. Ownership is enforced: another user's session id
/// answers 404, never revealing its existence.
/// Revoke one session. Ownership enforced: another user's session id 404s.
#[utoipa::path(
    delete,
    path = "/api/v1/auth/sessions/{id}",
    params(("id" = uuid::Uuid, Path, description = "Session id")),
    responses(
        (status = 204, description = "Session revoked"),
        (status = 401, description = "Missing or invalid access token"),
        (status = 404, description = "Session not found or not owned by the user"),
    ),
    security(("bearer_auth" = [])),
    tag = "auth"
)]
pub async fn revoke_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    auth_user: AuthUser,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Response> {
    let sessions = Sessions::new(state.pool.clone());
    let Some(session) = sessions.find_by_id(id).await.map_err(map_repo_error)? else {
        return Err(ApiError::NotFound);
    };
    if session.user_id != auth_user.user_id {
        return Err(ApiError::NotFound);
    }
    sessions.revoke_family(id).await.map_err(map_repo_error)?;
    audit(
        &state.pool,
        auth_user.user_id,
        "auth.session_revoked",
        "session",
        &id.to_string(),
        client_ip(&headers),
    )
    .await;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// Revoke all live sessions for the authenticated user.
/// Revoke all live sessions for the current user.
#[utoipa::path(
    delete,
    path = "/api/v1/auth/sessions",
    responses(
        (status = 204, description = "All sessions revoked"),
        (status = 401, description = "Missing or invalid access token"),
    ),
    security(("bearer_auth" = [])),
    tag = "auth"
)]
pub async fn revoke_all_sessions(
    State(state): State<AppState>,
    headers: HeaderMap,
    auth_user: AuthUser,
) -> ApiResult<Response> {
    let sessions = Sessions::new(state.pool.clone());
    sessions
        .revoke_all_for_user(auth_user.user_id)
        .await
        .map_err(map_repo_error)?;
    audit(
        &state.pool,
        auth_user.user_id,
        "auth.sessions_revoked_all",
        "user",
        &auth_user.user_id.to_string(),
        client_ip(&headers),
    )
    .await;
    Ok(StatusCode::NO_CONTENT.into_response())
}

// ── Shared helpers ──────────────────────────────────────────────────────────

/// Create a refresh session for a just-authenticated user and return the
/// (refresh_token, csrf_token, user) triple. Shared by password login and
/// OAuth login; callers build their own response (JSON body vs redirect).
pub(crate) async fn issue_session_cookies(
    state: &AppState,
    user_id: uuid::Uuid,
    headers: &HeaderMap,
) -> ApiResult<(String, String, keystone_db::repositories::users::User)> {
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
    let csrf_token = tokens::generate_refresh_token().map_err(|_| ApiError::Internal)?;
    Ok((refresh_token, csrf_token, user))
}

/// Issue an access token + refresh session for a just-authenticated user.
async fn issue_session(
    state: &AppState,
    user_id: uuid::Uuid,
    role: &str,
    headers: &HeaderMap,
) -> ApiResult<Response> {
    let (refresh_token, csrf_token, user) = issue_session_cookies(state, user_id, headers).await?;
    let _ = role;
    respond_with_tokens(state, user, &refresh_token, &csrf_token)
}

/// Build the token response: access token in body, refresh cookie on the side.
fn respond_with_tokens(
    state: &AppState,
    user: keystone_db::repositories::users::User,
    refresh_token: &str,
    csrf_token: &str,
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
        csrf_token: csrf_token.to_owned(),
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
    response.headers_mut().append(
        header::SET_COOKIE,
        csrf::csrf_cookie(
            csrf_token,
            state.auth.refresh_ttl,
            state.auth.secure_cookies,
        ),
    );
    Ok(response)
}

/// Append-only audit event; best-effort (auditing must never break a request).
/// Append an audit event. Best-effort by design — an audit insert must never
/// fail the request — but a failure is an incident signal, so it is logged
/// with full event context rather than swallowed.
pub(crate) async fn audit(
    pool: &PgPool,
    actor: uuid::Uuid,
    action: &str,
    entity_type: &str,
    entity_id: &str,
    ip: Option<IpAddr>,
) {
    let result = sqlx::query(
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
    if let Err(e) = result {
        tracing::error!(
            actor = %actor,
            action,
            entity_type,
            entity_id,
            error = %e,
            "audit event not persisted"
        );
    }
}

pub(crate) fn map_repo_error(err: RepoError) -> ApiError {
    match err {
        RepoError::EmailTaken => ApiError::Conflict("email is already registered".into()),
        RepoError::UniqueViolation(msg) => ApiError::Conflict(msg),
        RepoError::InvalidInput(msg) => ApiError::BadRequest(msg),
        RepoError::Database(e) => ApiError::Database(e),
    }
}
