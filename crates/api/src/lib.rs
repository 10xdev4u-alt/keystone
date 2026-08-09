//! HTTP API crate for keystone (Axum).
//!
//! Health endpoints follow the split convention:
//!   GET /healthz      — process liveness (no dependencies)
//!   GET /readyz       — readiness (database reachable, migrations applied)
//!   GET /api/v1/health — application health JSON
#![forbid(unsafe_code)]

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::json;
use sqlx::PgPool;
use std::time::Instant;

/// Shared application state.
#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub started_at: Instant,
}

/// Build the API router with the given state.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/api/v1/health", get(api_health))
        .with_state(state)
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use sqlx::postgres::PgPoolOptions;
    use tower::ServiceExt;

    #[tokio::test]
    async fn healthz_answers_without_a_database() {
        // connect_lazy never opens a socket, so this works with no DB running.
        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://keystone:keystone@localhost:5432/keystone")
            .expect("lazy pool must not require a live database");
        let app = router(AppState {
            pool,
            started_at: Instant::now(),
        });

        let response = app
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
}
