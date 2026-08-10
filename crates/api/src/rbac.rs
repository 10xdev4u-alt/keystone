//! RBAC middleware — role checks from the verified access token.
//!
//! Roles mirror the `users.role` CHECK constraint; the JWT only mirrors the
//! DB value. Guarded routes admit the listed roles and return RFC 7807 403
//! for everyone else.

use crate::auth::AuthUser;
use crate::error::ApiError;
use axum::body::Body;
use axum::extract::State;
use axum::http::Request;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

/// Admit only `admin` and `super_admin`.
pub async fn require_admin(
    State(_state): State<crate::AppState>,
    auth_user: AuthUser,
    request: Request<Body>,
    next: Next,
) -> Response {
    if !matches!(auth_user.role.as_str(), "admin" | "super_admin") {
        return ApiError::Forbidden.into_response();
    }
    next.run(request).await
}

/// Admit `moderator`, `admin`, and `super_admin`.
pub async fn require_moderator(
    State(_state): State<crate::AppState>,
    auth_user: AuthUser,
    request: Request<Body>,
    next: Next,
) -> Response {
    if !matches!(
        auth_user.role.as_str(),
        "moderator" | "admin" | "super_admin"
    ) {
        return ApiError::Forbidden.into_response();
    }
    next.run(request).await
}
