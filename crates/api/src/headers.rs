//! Security response headers applied to every response.
//!
//! Follows current OWASP Secure Headers guidance:
//!   - `Strict-Transport-Security` — enforce TLS for a year, include subdomains.
//!   - `X-Content-Type-Options: nosniff` — never MIME-sniff responses.
//!   - `X-Frame-Options: DENY` — this API is never meant to be framed.
//!   - `Referrer-Policy: no-referrer` — never leak tokens via referrer.
//!   - `Permissions-Policy` — disable geolocation/camera/mic by default.
//!   - `Cross-Origin-Opener-Policy: same-origin` — isolate the origin.
//!
//! The CSP frame-ancestors directive is intentionally omitted: an API serves no
//! HTML, and `X-Frame-Options` already covers framing. A `frame-ancestors` CSP
//! would need a per-deployment allowlist and adds nothing for a JSON API.

use axum::http::header::{HeaderName, HeaderValue};
use axum::http::Request;
use axum::middleware::Next;
use axum::response::Response;

/// One year in seconds, matching the HSTS max-age guidance (>= 180 days).
/// OWASP: max-age must be >= 180 days (15_552_000 seconds) — enforced at
/// compile time.
const HSTS_MAX_AGE: u64 = 31_536_000;
const _: () = assert!(HSTS_MAX_AGE >= 15_552_000);

/// Middleware that stamps immutable security headers onto every response.
pub async fn security_headers(request: Request<axum::body::Body>, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();

    headers.insert(
        HeaderName::from_static("strict-transport-security"),
        HeaderValue::from_static("max-age=31536000; includeSubDomains"),
    );
    headers.insert(
        HeaderName::from_static("x-content-type-options"),
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        HeaderName::from_static("x-frame-options"),
        HeaderValue::from_static("DENY"),
    );
    headers.insert(
        HeaderName::from_static("referrer-policy"),
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        HeaderName::from_static("permissions-policy"),
        HeaderValue::from_static("geolocation=(), camera=(), microphone=(), payment=(), usb=()"),
    );
    headers.insert(
        HeaderName::from_static("cross-origin-opener-policy"),
        HeaderValue::from_static("same-origin"),
    );

    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::StatusCode;
    use axum::routing::get;
    use axum::Router;

    fn app() -> Router {
        Router::new()
            .route("/", get(|| async { StatusCode::NO_CONTENT }))
            .layer(axum::middleware::from_fn(security_headers))
    }

    #[tokio::test]
    async fn stamps_all_security_headers() {
        use tower::ServiceExt;

        let response = app()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        let headers = response.headers();
        assert_eq!(
            headers
                .get("strict-transport-security")
                .and_then(|v| v.to_str().ok()),
            Some("max-age=31536000; includeSubDomains")
        );
        assert_eq!(
            headers
                .get("x-content-type-options")
                .and_then(|v| v.to_str().ok()),
            Some("nosniff")
        );
        assert_eq!(
            headers.get("x-frame-options").and_then(|v| v.to_str().ok()),
            Some("DENY")
        );
        assert_eq!(
            headers.get("referrer-policy").and_then(|v| v.to_str().ok()),
            Some("no-referrer")
        );
        assert!(headers.contains_key("permissions-policy"));
        assert!(headers.contains_key("cross-origin-opener-policy"));
    }
}
