//! CSRF protection — double-submit tokens.
//!
//! The only cookie-authenticated routes are refresh/logout (the refresh token
//! rides an httpOnly SameSite=Strict cookie). SameSite already blocks the
//! classic CSRF vector; the double-submit token is defense in depth and the
//! plan's explicit requirement.
//!
//! Flow: login/refresh set a NON-httpOnly `keystone_csrf` cookie (readable by
//! the SPA) and return the token in the body. State-changing cookie requests
//! must echo it back in the `X-CSRF-Token` header.
//!
//! The guard is STRICT and only ever wraps refresh/logout (see `router`): a
//! request that lacks the CSRF cookie, the header, or carries a mismatched
//! pair is rejected with 403. A legitimate client always has both — they are
//! set and rotated together at login/refresh and share the same lifetime.

use axum::body::Body;
use axum::http::header::{self, HeaderValue};
use axum::http::{HeaderMap, Method, Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

/// Single constant for the CSRF cookie (same discipline as REFRESH_COOKIE).
pub const CSRF_COOKIE: &str = "keystone_csrf";
pub const CSRF_HEADER: &str = "x-csrf-token";
const CSRF_COOKIE_PATH: &str = "/api/v1/auth";

/// Build the CSRF cookie value header. NOT httpOnly — the SPA must read it to
/// echo it back; it is not a credential (the refresh token is).
pub fn csrf_cookie(value: &str, max_age: std::time::Duration, secure: bool) -> HeaderValue {
    let mut cookie = format!(
        "{CSRF_COOKIE}={value}; Max-Age={}; Path={CSRF_COOKIE_PATH}; SameSite=Strict",
        max_age.as_secs()
    );
    if secure {
        cookie.push_str("; Secure");
    }
    HeaderValue::from_str(&cookie).expect("csrf cookie must be valid")
}

/// Read the CSRF cookie from a request.
pub fn read_csrf_cookie(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    raw.split(';')
        .find_map(|part| part.trim().strip_prefix(&format!("{CSRF_COOKIE}=")))
        .map(str::to_owned)
}

/// Reject state-changing requests on cookie-authenticated routes unless the
/// `X-CSRF-Token` header matches the `keystone_csrf` cookie. Strict: a
/// missing cookie OR header is rejected — a legitimate client always has
/// both (set together at login, rotated together at refresh).
pub async fn csrf_guard(request: Request<Body>, next: Next) -> Response {
    if request.method() == Method::GET
        || request.method() == Method::HEAD
        || request.method() == Method::OPTIONS
    {
        return next.run(request).await;
    }

    let cookie = read_csrf_cookie(request.headers());
    let header = request
        .headers()
        .get(CSRF_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);

    match (cookie, header) {
        (Some(c), Some(h)) if c == h => next.run(request).await,
        _ => StatusCode::FORBIDDEN.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookie_parses_from_full_cookie_header() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("other=1; keystone_csrf=abc123; keystone_refresh=xyz"),
        );
        assert_eq!(read_csrf_cookie(&headers).as_deref(), Some("abc123"));
    }

    #[test]
    fn cookie_header_is_well_formed() {
        let value = csrf_cookie("tok", std::time::Duration::from_secs(900), false);
        let raw = value.to_str().unwrap();
        assert!(raw.starts_with("keystone_csrf=tok;"));
        assert!(raw.contains("SameSite=Strict"));
        assert!(!raw.contains("HttpOnly"), "SPA must be able to read it");
    }
}
