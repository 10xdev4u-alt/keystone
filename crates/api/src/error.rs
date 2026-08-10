//! API error model — RFC 7807 (`application/problem+json`).
//!
//! One canonical error shape for the whole API:
//! ```json
//! {
//!   "type": "about:blank",
//!   "title": "Resource not found",
//!   "status": 404,
//!   "code": "not_found",
//!   "detail": "resource not found"
//! }
//! ```
//! No stack traces, no raw database errors, no secrets — ever.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

/// Canonical API error.
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("resource not found")]
    NotFound,
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("unauthorized")]
    Unauthorized,
    #[error("refresh token was reused; session family revoked")]
    ReuseDetected,
    #[error("forbidden")]
    Forbidden,
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("too many requests")]
    TooManyRequests,
    #[error("bad gateway: {0}")]
    BadGateway(String),
    #[error("internal error")]
    Internal,
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

impl ApiError {
    fn status(&self) -> StatusCode {
        match self {
            ApiError::NotFound => StatusCode::NOT_FOUND,
            ApiError::BadRequest(_) => StatusCode::BAD_REQUEST,
            ApiError::Unauthorized | ApiError::ReuseDetected => StatusCode::UNAUTHORIZED,
            ApiError::Forbidden => StatusCode::FORBIDDEN,
            ApiError::Conflict(_) => StatusCode::CONFLICT,
            ApiError::TooManyRequests => StatusCode::TOO_MANY_REQUESTS,
            ApiError::BadGateway(_) => StatusCode::BAD_GATEWAY,
            ApiError::Internal | ApiError::Database(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn code(&self) -> &'static str {
        match self {
            ApiError::NotFound => "not_found",
            ApiError::BadRequest(_) => "bad_request",
            ApiError::Unauthorized => "unauthorized",
            ApiError::ReuseDetected => "token_reuse_detected",
            ApiError::Forbidden => "forbidden",
            ApiError::Conflict(_) => "conflict",
            ApiError::TooManyRequests => "too_many_requests",
            ApiError::BadGateway(_) => "bad_gateway",
            ApiError::Internal | ApiError::Database(_) => "internal",
        }
    }

    fn title(&self) -> &'static str {
        match self {
            ApiError::NotFound => "Resource not found",
            ApiError::BadRequest(_) => "Bad request",
            ApiError::Unauthorized => "Unauthorized",
            ApiError::ReuseDetected => "Token reuse detected",
            ApiError::Forbidden => "Forbidden",
            ApiError::Conflict(_) => "Conflict",
            ApiError::TooManyRequests => "Too many requests",
            ApiError::BadGateway(_) => "Bad gateway",
            ApiError::Internal | ApiError::Database(_) => "Internal error",
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.status();
        // The detail for internal errors must never leak internals.
        let detail = match &self {
            ApiError::Internal => "an unexpected error occurred".to_owned(),
            ApiError::Database(err) => {
                tracing::error!(error = %err, "database error mapped to 500");
                "an unexpected error occurred".to_owned()
            }
            other => other.to_string(),
        };

        (
            status,
            Json(json!({
                "type": "about:blank",
                "title": self.title(),
                "status": status.as_u16(),
                "code": self.code(),
                "detail": detail,
            })),
        )
            .into_response()
    }
}

/// Result alias used by handlers.
pub type ApiResult<T> = Result<T, ApiError>;

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt;

    async fn body_json(response: Response) -> serde_json::Value {
        let body = response
            .into_body()
            .collect()
            .await
            .expect("body must read")
            .to_bytes();
        serde_json::from_slice(&body).expect("body must be JSON")
    }

    #[tokio::test]
    async fn not_found_is_problem_json() {
        let response = ApiError::NotFound.into_response();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let value = body_json(response).await;
        assert_eq!(value["status"], 404);
        assert_eq!(value["code"], "not_found");
        assert_eq!(value["type"], "about:blank");
    }

    #[tokio::test]
    async fn database_error_never_leaks_internals() {
        let err = ApiError::Database(sqlx::Error::Configuration(
            "secret connection string".into(),
        ));
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let value = body_json(response).await;
        assert_eq!(value["detail"], "an unexpected error occurred");
        assert!(!value.to_string().contains("secret"));
    }
}
