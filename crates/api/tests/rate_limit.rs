//! Rate limiting integration tests: auth tier 429s + Retry-After, per-key
//! isolation, and the generous default tier.

use axum::body::Body;
use axum::http::header::{self, HeaderValue};
use axum::http::{Request, StatusCode};
use keystone_api::auth::AuthServices;
use keystone_api::{router, AppState};
use serde_json::Value;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tower::ServiceExt;

fn test_auth() -> AuthServices {
    AuthServices {
        password: Arc::new(
            keystone_auth::password::PasswordHasher::from_config(&keystone_config::Argon2Config {
                memory_kib: 19_456,
                iterations: 2,
                parallelism: 1,
            })
            .expect("params valid"),
        ),
        jwt: Arc::new(keystone_auth::jwt::AccessTokenService::new(
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
                .expect("key valid"),
        )),
        lockout: keystone_auth::service::LockoutPolicy::new(
            5,
            Duration::from_secs(300),
            Duration::from_secs(60),
        ),
        access_ttl: Duration::from_secs(900),
        refresh_ttl: Duration::from_secs(604_800),
        secure_cookies: false,
    }
}

async fn test_app() -> Option<(axum::Router, Arc<keystone_api::middleware::RateLimiter>)> {
    let pool = keystone_db::test_util::test_pool_isolated().await?;
    keystone_db::test_util::setup(&pool)
        .await
        .expect("db setup");
    let limiter = Arc::new(keystone_api::middleware::RateLimiter::new());
    let app = router(AppState {
        pool,
        started_at: Instant::now(),
        auth: test_auth(),
        rate_limit: limiter.clone(),
        realtime: std::sync::Arc::new(keystone_api::realtime::RealtimeHub::new()),
        storage: std::sync::Arc::new(keystone_db::storage::MemoryStorage::new()),
        oauth: None,
    });
    Some((app, limiter))
}

fn post_login(body: Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .expect("request must build")
}

fn post_logout() -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/api/v1/auth/logout")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::empty())
        .expect("request must build")
}

#[tokio::test]
async fn auth_tier_limits_and_sends_retry_after() {
    let Some((app, _)) = test_app().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };

    let body = serde_json::json!({ "email": "nobody@example.com", "password": "whatever" });
    let key = HeaderValue::from_static("203.0.113.7");

    // Auth tier: 10 allowed, then 429 with Retry-After.
    for i in 0..10 {
        let response = app
            .clone()
            .oneshot(post_login(body.clone()).with_headers(&key))
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "request {i} should reach the handler"
        );
    }

    let limited = app
        .clone()
        .oneshot(post_login(body.clone()).with_headers(&key))
        .await
        .unwrap();
    assert_eq!(limited.status(), StatusCode::TOO_MANY_REQUESTS);
    let retry_after = limited
        .headers()
        .get("retry-after")
        .expect("429 must carry Retry-After")
        .to_str()
        .unwrap()
        .parse::<u64>()
        .expect("Retry-After must be numeric");
    assert!((1..=60).contains(&retry_after));
}

#[tokio::test]
async fn different_clients_are_independent() {
    let Some((app, _)) = test_app().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };

    let body = serde_json::json!({ "email": "nobody@example.com", "password": "whatever" });
    let client_a = HeaderValue::from_static("203.0.113.1");
    let client_b = HeaderValue::from_static("203.0.113.2");

    // Exhaust client A.
    for _ in 0..10 {
        app.clone()
            .oneshot(post_login(body.clone()).with_headers(&client_a))
            .await
            .unwrap();
    }
    let a_limited = app
        .clone()
        .oneshot(post_login(body.clone()).with_headers(&client_a))
        .await
        .unwrap();
    assert_eq!(a_limited.status(), StatusCode::TOO_MANY_REQUESTS);

    // Client B is untouched.
    let b_ok = app
        .clone()
        .oneshot(post_login(body.clone()).with_headers(&client_b))
        .await
        .unwrap();
    assert_eq!(b_ok.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn session_tier_absorbs_spa_navigation_burst() {
    let Some((app, _)) = test_app().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };

    // A SPA session check (refresh/logout) after every page load: 30 rapid
    // calls from one client must all reach the handler (CSRF 401 or not — the
    // limiter never 429s them). The Auth tier would reject at 11.
    let key = HeaderValue::from_static("203.0.113.9");
    for i in 0..30 {
        let response = app
            .clone()
            .oneshot(post_logout().with_headers(&key))
            .await
            .unwrap();
        assert_ne!(
            response.status(),
            StatusCode::TOO_MANY_REQUESTS,
            "session-route call {i} must not be rate-limited"
        );
    }
}

#[tokio::test]
async fn health_reads_use_generous_tier() {
    let Some((app, _)) = test_app().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };

    // /api/v1/health sits on the Default tier (120/min) — 20 rapid reads pass.
    for i in 0..20 {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "health read {i} must not be rate-limited"
        );
    }
}

trait WithHeaders {
    fn with_headers(self, xff: &HeaderValue) -> Request<Body>;
}

impl WithHeaders for Request<Body> {
    fn with_headers(mut self, xff: &HeaderValue) -> Request<Body> {
        self.headers_mut().insert("x-forwarded-for", xff.clone());
        self
    }
}
