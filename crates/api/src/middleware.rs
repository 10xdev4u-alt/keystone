//! Middleware: rate limiting.
//!
//! In-memory fixed-window counters per (tier, client key). Single-instance
//! correct; a shared store (Redis/Postgres) is a documented follow-up when the
//! API scales past one process. Auth routes get a strict tier; the rest get a
//! generous default. Every 429 carries a `Retry-After` header.

use crate::error::ApiError;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Route rate tiers. Tuning lives here until it earns env config.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RateTier {
    /// Auth routes: strict, brute-force protection on top of lockout.
    Auth,
    /// Everything else.
    Default,
}

#[derive(Debug, Clone, Copy)]
struct Limit {
    max: u32,
    window: Duration,
}

const fn limit(max: u32, secs: u64) -> Limit {
    Limit {
        max,
        window: Duration::from_secs(secs),
    }
}

fn limits(tier: RateTier) -> Limit {
    match tier {
        RateTier::Auth => limit(10, 60),
        RateTier::Default => limit(120, 60),
    }
}

#[derive(Debug)]
struct Window {
    started: Instant,
    count: u32,
}

/// Thread-safe fixed-window limiter. Keyed by client address (or a constant
/// when the address is unknown — tests, non-proxied setups).
#[derive(Debug, Default)]
pub struct RateLimiter {
    inner: Mutex<HashMap<(RateTier, String), Window>>,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one request for `key` at `tier`. Returns the seconds until the
    /// window resets when the limit is exceeded.
    pub fn check(&self, tier: RateTier, key: &str, now: Instant) -> Result<(), u64> {
        let limit = limits(tier);
        let mut map = self.inner.lock().expect("limiter mutex poisoned");
        let entry = map.entry((tier, key.to_owned())).or_insert(Window {
            started: now,
            count: 0,
        });
        if now.duration_since(entry.started) >= limit.window {
            *entry = Window {
                started: now,
                count: 1,
            };
            return Ok(());
        }
        if entry.count >= limit.max {
            let retry_after = limit
                .window
                .saturating_sub(now.duration_since(entry.started));
            return Err(retry_after.as_secs().max(1));
        }
        entry.count += 1;
        Ok(())
    }

    /// Remove all windows — used by tests for isolation.
    pub fn clear(&self) {
        self.inner.lock().expect("limiter mutex poisoned").clear();
    }
}

/// Client key: the first `X-Forwarded-For` entry when behind a proxy, else a
/// shared bucket. Trust boundary: the proxy must overwrite XFF.
pub fn client_key(headers: &HeaderMap) -> String {
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(str::trim)
        .and_then(|v| v.parse::<IpAddr>().ok())
        .map(|ip| ip.to_string())
        .unwrap_or_else(|| "unknown".to_owned())
}

async fn enforce(
    state: &crate::AppState,
    tier: RateTier,
    headers: &HeaderMap,
    request: Request<Body>,
    next: Next,
) -> Response {
    let key = client_key(headers);
    if let Err(retry_after) = state.rate_limit.check(tier, &key, Instant::now()) {
        let mut response = ApiError::TooManyRequests.into_response();
        if let Ok(value) = axum::http::HeaderValue::from_str(&retry_after.to_string()) {
            response.headers_mut().insert("retry-after", value);
        }
        return response;
    }
    next.run(request).await
}

/// Strict tier — auth routes.
pub async fn rate_limit_auth(
    State(state): State<crate::AppState>,
    headers: HeaderMap,
    request: Request<Body>,
    next: Next,
) -> Response {
    enforce(&state, RateTier::Auth, &headers, request, next).await
}

/// Generous tier — the rest of the API.
pub async fn rate_limit_default(
    State(state): State<crate::AppState>,
    headers: HeaderMap,
    request: Request<Body>,
    next: Next,
) -> Response {
    enforce(&state, RateTier::Default, &headers, request, next).await
}

/// Alias so callers don't need the StatusCode import.
pub const TOO_MANY: StatusCode = StatusCode::TOO_MANY_REQUESTS;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_tier_allows_then_rejects() {
        let limiter = RateLimiter::new();
        let now = Instant::now();
        // 10 allowed within the window.
        for i in 0..10 {
            assert_eq!(
                limiter.check(RateTier::Auth, "1.2.3.4", now + Duration::from_millis(i)),
                Ok(())
            );
        }
        let err = limiter.check(RateTier::Auth, "1.2.3.4", now + Duration::from_millis(11));
        assert!(err.is_err(), "11th request in window must be limited");
        assert!(err.unwrap_err() >= 1);

        // Different key is unaffected.
        assert_eq!(limiter.check(RateTier::Auth, "5.6.7.8", now), Ok(()));

        // Window expiry resets the counter.
        let later = now + Duration::from_secs(61);
        assert_eq!(limiter.check(RateTier::Auth, "1.2.3.4", later), Ok(()));
    }

    #[test]
    fn tiers_are_independent() {
        let limiter = RateLimiter::new();
        let now = Instant::now();
        // Exhaust the Default tier; Auth tier still has budget.
        for i in 0..120 {
            assert_eq!(
                limiter.check(RateTier::Default, "9.9.9.9", now + Duration::from_millis(i)),
                Ok(())
            );
        }
        assert!(limiter.check(RateTier::Default, "9.9.9.9", now).is_err());
        assert_eq!(limiter.check(RateTier::Auth, "9.9.9.9", now), Ok(()));
    }
}
