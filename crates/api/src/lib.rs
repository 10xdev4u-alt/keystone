//! HTTP API crate for Keystone (Axum).
//!
//! Health endpoints follow the split convention:
//!   GET /healthz      — process liveness (no dependencies)
//!   GET /readyz       — readiness (database reachable, migrations applied)
//!   GET /api/v1/health — application health JSON
//!
//! Errors are RFC 7807 problem+json (see `error` module).
#![forbid(unsafe_code)]

pub mod auth;
pub mod content;
pub mod csrf;
pub mod error;
pub mod headers;
pub mod middleware;
pub mod moderation;
pub mod oauth;
pub mod rbac;

use axum::extract::State;
use axum::http::header::{self, HeaderName, HeaderValue};
use axum::http::{Method, StatusCode};
use axum::middleware as axum_mw;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use error::ApiError;
use serde_json::json;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Instant;
use tower_http::cors::CorsLayer;

/// Shared application state.
#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub started_at: Instant,
    pub auth: auth::AuthServices,
    pub rate_limit: Arc<middleware::RateLimiter>,
    /// OAuth login; `None` when no provider is configured (routes absent).
    pub oauth: Option<oauth::OAuthService>,
}

/// Build the API router with the given state.
///
/// Rate tiers: state-changing auth routes are strict; reads get a generous
/// default. The fallback (404) is intentionally not rate-limited.
pub fn router(state: AppState) -> Router {
    let router = Router::new()
        .route("/api/v1/auth/register", post(auth::register))
        .route("/api/v1/auth/verify-email", post(auth::verify_email))
        .route("/api/v1/auth/login", post(auth::login))
        .route_layer(axum_mw::from_fn_with_state(
            state.clone(),
            middleware::rate_limit_auth,
        ))
        // Cookie-authenticated routes get the double-submit CSRF guard in
        // their own sub-router so it never wraps credential-based routes
        // (register/login have no CSRF pair yet). The guard is strict: any
        // state-changing request must present a matching cookie + header.
        .merge(
            Router::new()
                .route("/api/v1/auth/refresh", post(auth::refresh))
                .route("/api/v1/auth/logout", post(auth::logout))
                .route_layer(axum_mw::from_fn_with_state(
                    state.clone(),
                    middleware::rate_limit_auth,
                ))
                .route_layer(axum_mw::from_fn(csrf::csrf_guard)),
        )
        .route("/api/v1/auth/me", get(auth::me))
        .route(
            "/api/v1/auth/sessions",
            get(auth::list_sessions).delete(auth::revoke_all_sessions),
        )
        .route(
            "/api/v1/auth/sessions/{id}",
            axum::routing::delete(auth::revoke_session),
        )
        // ── Content spine ────────────────────────────────────────────────
        .route(
            "/api/v1/posts",
            get(content::list_posts).post(content::create_post),
        )
        .route(
            "/api/v1/posts/{id}",
            get(content::get_post)
                .patch(content::update_post)
                .delete(content::delete_post),
        )
        .route("/api/v1/posts/{id}/versions", get(content::post_versions))
        .route("/api/v1/posts/{id}/view", post(content::record_view))
        .route(
            "/api/v1/posts/{id}/comments",
            get(content::list_comments).post(content::create_comment),
        )
        .route(
            "/api/v1/comments/{id}",
            axum::routing::delete(content::delete_comment),
        )
        .route(
            "/api/v1/posts/{id}/reaction",
            axum::routing::put(content::set_reaction).delete(content::remove_reaction),
        )
        .route("/api/v1/posts/{id}/reactions", get(content::get_reactions))
        .route(
            "/api/v1/posts/{id}/bookmark",
            axum::routing::put(content::add_bookmark).delete(content::remove_bookmark),
        )
        .route("/api/v1/me/bookmarks", get(content::my_bookmarks))
        .route("/api/v1/reports", post(moderation::file_report))
        .route(
            "/api/v1/reviews",
            get(moderation::list_reviews).put(moderation::upsert_review),
        )
        // ── Moderation (staff-only) ───────────────────────────────────────
        .merge(
            Router::new()
                .route("/api/v1/moderation/reports", get(moderation::report_queue))
                .route(
                    "/api/v1/moderation/reports/{id}/resolve",
                    post(moderation::resolve_report),
                )
                .route_layer(axum_mw::from_fn_with_state(
                    state.clone(),
                    rbac::require_moderator,
                )),
        )
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/api/v1/health", get(api_health))
        .route_layer(axum_mw::from_fn_with_state(
            state.clone(),
            middleware::rate_limit_default,
        ))
        // Admin routes in their own sub-router so the RBAC layer only wraps
        // them — route_layer applies to everything added before it.
        .merge(
            Router::new()
                .route("/api/v1/admin/status", get(admin_status))
                .route_layer(axum_mw::from_fn_with_state(
                    state.clone(),
                    rbac::require_admin,
                )),
        )
        .fallback(not_found);

    // OAuth routes exist only when a provider is configured — no surface to
    // probe when login-by-Google is off.
    let mut router = router;
    if state.oauth.is_some() {
        router = router.merge(
            Router::<AppState>::new()
                .route("/api/v1/auth/oauth/google/start", get(oauth::start))
                .route("/api/v1/auth/oauth/google/callback", get(oauth::callback)),
        );
    }

    router
        .layer(axum_mw::from_fn(headers::security_headers))
        .with_state(state)
}

/// CORS layer restricted to the configured origins.
///
/// When `origins` is empty the layer is not applied at all — same-origin /
/// reverse-proxy only, which is the secure default for local development.
pub fn cors_layer(origins: &[String]) -> CorsLayer {
    let allowed: Vec<HeaderValue> = origins.iter().filter_map(|o| o.parse().ok()).collect();
    CorsLayer::new()
        .allow_origin(allowed)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([
            header::CONTENT_TYPE,
            header::AUTHORIZATION,
            header::ACCEPT,
            HeaderName::from_static("x-request-id"),
            HeaderName::from_static("x-csrf-token"),
        ])
        .allow_credentials(true)
}

async fn healthz() -> &'static str {
    "ok"
}

async fn readyz(State(state): State<AppState>) -> Response {
    match keystone_db::ping(&state.pool).await {
        Ok(()) => StatusCode::OK.into_response(),
        Err(_) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

async fn api_health(State(state): State<AppState>) -> Json<serde_json::Value> {
    let db_ok = keystone_db::ping(&state.pool).await.is_ok();
    Json(json!({
        "status": if db_ok { "ok" } else { "degraded" },
        "db": db_ok,
        "uptime_secs": state.started_at.elapsed().as_secs(),
    }))
}

async fn not_found() -> ApiError {
    ApiError::NotFound
}

/// Operator visibility: instance stats. RBAC-guarded (admin/super_admin).
async fn admin_status(State(state): State<AppState>) -> Json<serde_json::Value> {
    let users = sqlx::query_scalar::<_, i64>("SELECT count(*) FROM users WHERE deleted_at IS NULL")
        .fetch_one(&state.pool)
        .await;
    let live_sessions = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM sessions WHERE revoked_at IS NULL AND expires_at > now()",
    )
    .fetch_one(&state.pool)
    .await;

    match (users, live_sessions) {
        (Ok(users), Ok(live_sessions)) => Json(json!({
            "status": "ok",
            "uptime_secs": state.started_at.elapsed().as_secs(),
            "users": users,
            "live_sessions": live_sessions,
        })),
        _ => Json(json!({
            "status": "degraded",
            "uptime_secs": state.started_at.elapsed().as_secs(),
            "users": null,
            "live_sessions": null,
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use sqlx::postgres::PgPoolOptions;
    use tower::ServiceExt;

    fn lazy_app() -> Router {
        // connect_lazy never opens a socket, so this works with no DB running.
        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://keystone:keystone@localhost:5432/keystone")
            .expect("lazy pool must not require a live database");
        router(AppState {
            pool,
            started_at: Instant::now(),
            auth: crate::auth::AuthServices {
                password: std::sync::Arc::new(
                    keystone_auth::password::PasswordHasher::from_config(
                        &keystone_config::Argon2Config {
                            memory_kib: 19_456,
                            iterations: 2,
                            parallelism: 1,
                        },
                    )
                    .expect("params must be valid"),
                ),
                jwt: std::sync::Arc::new(keystone_auth::jwt::AccessTokenService::new(
                    &keystone_config::JwtConfig {
                        issuer: "keystone-test".into(),
                        audience: "keystone-api".into(),
                        access_expiration_secs: 900,
                        refresh_expiration_secs: 604_800,
                        private_key_b64: Some(
                            "c2VjcmV0LXNlY3JldC1zZWNyZXQtc2VjcmV0LXNlY3JldC0xMjM0NTY3ODkw".into(),
                        ),
                        private_key_path: None,
                    },
                    keystone_auth::jwt::JwtKeys::from_secret(b"01234567890123456789012345678901")
                        .expect("key must be valid"),
                )),
                lockout: keystone_auth::service::LockoutPolicy::new(
                    5,
                    std::time::Duration::from_secs(300),
                    std::time::Duration::from_secs(60),
                ),
                access_ttl: std::time::Duration::from_secs(900),
                refresh_ttl: std::time::Duration::from_secs(604_800),
                secure_cookies: false,
            },
            rate_limit: std::sync::Arc::new(crate::middleware::RateLimiter::new()),
            oauth: None,
        })
    }

    #[tokio::test]
    async fn healthz_answers_without_a_database() {
        let response = lazy_app()
            .oneshot(
                Request::builder()
                    .uri("/healthz")
                    .body(Body::empty())
                    .expect("request must build"),
            )
            .await
            .expect("handler must not panic");

        assert_eq!(response.status(), StatusCode::OK);
        let body = response
            .into_body()
            .collect()
            .await
            .expect("body must read")
            .to_bytes();
        assert_eq!(&body[..], b"ok");
    }

    #[tokio::test]
    async fn unknown_route_returns_problem_json() {
        let response = lazy_app()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/does-not-exist")
                    .body(Body::empty())
                    .expect("request must build"),
            )
            .await
            .expect("handler must not panic");

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            response
                .headers()
                .get(header::CONTENT_TYPE)
                .map(|v| v.as_bytes()),
            Some(b"application/json".as_slice())
        );
        let body = response
            .into_body()
            .collect()
            .await
            .expect("body must read")
            .to_bytes();
        let value: serde_json::Value = serde_json::from_slice(&body).expect("body must be JSON");
        assert_eq!(value["code"], "not_found");
        assert_eq!(value["status"], 404);
    }
}
