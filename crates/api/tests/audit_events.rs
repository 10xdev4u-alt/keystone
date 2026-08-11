//! Audit-trail integration tests: every auth-sensitive action must leave an
//! append-only `audit_logs` row (register, verify, login, failed login,
//! lockout, refresh rotation, logout).
//!
//! Runs against a real Postgres (CI service). Self-skips when
//! TEST_DATABASE_URL is unset.

use axum::body::Body;
use axum::http::header::{self, HeaderValue};
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use keystone_api::auth::AuthServices;
use keystone_api::{router, AppState};
use serde_json::Value;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tower::ServiceExt;

const REFRESH_COOKIE: &str = "keystone_refresh";

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

/// Pool + router, with the pool kept for audit queries and the rate limiter
/// kept so the test can reset windows between phases.
async fn test_app() -> Option<(
    axum::Router,
    PgPool,
    std::sync::Arc<keystone_api::middleware::RateLimiter>,
)> {
    let pool = keystone_db::test_util::test_pool().await?;
    keystone_db::test_util::setup(&pool)
        .await
        .expect("db setup");
    let rate_limit = std::sync::Arc::new(keystone_api::middleware::RateLimiter::new());
    let app = router(AppState {
        pool: pool.clone(),
        started_at: Instant::now(),
        auth: test_auth(),
        rate_limit: rate_limit.clone(),
        realtime: std::sync::Arc::new(keystone_api::realtime::RealtimeHub::new()),
        storage: std::sync::Arc::new(keystone_db::storage::MemoryStorage::new()),
        oauth: None,
    });
    Some((app, pool, rate_limit))
}

async fn json_body(response: axum::response::Response) -> Value {
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body must read")
        .to_bytes();
    serde_json::from_slice(&bytes).expect("body must be JSON")
}

fn post_json(uri: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .expect("request must build")
}

fn refresh_cookie_value(response: &axum::response::Response) -> Option<String> {
    for value in response.headers().get_all(header::SET_COOKIE) {
        let raw = value.to_str().ok()?;
        if let Some(rest) = raw.strip_prefix(&format!("{REFRESH_COOKIE}=")) {
            return rest.split(';').next().map(str::to_owned);
        }
    }
    None
}

fn cookies_header(refresh: &str, csrf: &str) -> HeaderValue {
    HeaderValue::from_str(&format!("{REFRESH_COOKIE}={refresh}; keystone_csrf={csrf}"))
        .expect("cookie must be valid")
}

async fn actions_for(pool: &PgPool, user_id: uuid::Uuid) -> Vec<String> {
    sqlx::query_scalar("SELECT action FROM audit_logs WHERE actor_user_id = $1 ORDER BY created_at")
        .bind(user_id)
        .fetch_all(pool)
        .await
        .expect("audit query must work")
}

async fn register(app: &axum::Router, email: &str) -> (uuid::Uuid, String) {
    let response = app
        .clone()
        .oneshot(post_json(
            "/api/v1/auth/register",
            serde_json::json!({
                "email": email,
                "password": "correct-horse-battery-staple",
            }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = json_body(response).await;
    let id = body["user_id"]
        .as_str()
        .expect("user id")
        .parse()
        .expect("valid uuid");
    let token = body["verification_token"]
        .as_str()
        .expect("dev verification token")
        .to_owned();
    (id, token)
}

#[tokio::test]
async fn auth_flow_leaves_complete_audit_trail() {
    let Some((app, pool, rate_limit)) = test_app().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };

    // --- Alice: happy path (register → verify → login → refresh → logout) ---
    let (alice_id, alice_token) = register(&app, "alice@example.com").await;

    let verify = app
        .clone()
        .oneshot(post_json(
            "/api/v1/auth/verify-email",
            serde_json::json!({ "token": alice_token }),
        ))
        .await
        .unwrap();
    assert_eq!(verify.status(), StatusCode::OK);

    let login = app
        .clone()
        .oneshot(post_json(
            "/api/v1/auth/login",
            serde_json::json!({
                "email": "alice@example.com",
                "password": "correct-horse-battery-staple",
            }),
        ))
        .await
        .unwrap();
    assert_eq!(login.status(), StatusCode::OK);
    let cookie1 = refresh_cookie_value(&login).expect("refresh cookie");
    let login_body = json_body(login).await;
    let csrf1 = login_body["csrf_token"].as_str().expect("csrf token");

    let refresh = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/refresh")
                .header(header::COOKIE, cookies_header(&cookie1, csrf1))
                .header("x-csrf-token", HeaderValue::from_str(csrf1).unwrap())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(refresh.status(), StatusCode::OK);
    let cookie2 = refresh_cookie_value(&refresh).expect("rotated cookie");
    let refresh_body = json_body(refresh).await;
    let csrf2 = refresh_body["csrf_token"].as_str().expect("csrf token");

    let logout = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/logout")
                .header(header::COOKIE, cookies_header(&cookie2, csrf2))
                .header("x-csrf-token", HeaderValue::from_str(csrf2).unwrap())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(logout.status(), StatusCode::NO_CONTENT);

    assert_eq!(
        actions_for(&pool, alice_id).await,
        vec![
            "auth.register",
            "auth.verify_email",
            "auth.login",
            "auth.session_rotated",
            "auth.logout",
        ]
    );

    // --- Bob: failure path (five failed logins, then lockout) ---
    // Reset the auth-tier window first: the happy path already consumed most
    // of the 10 req/min budget, and this phase is about lockout, not limits.
    rate_limit.clear();
    let (bob_id, _) = register(&app, "bob@example.com").await;
    for _ in 0..5 {
        let response = app
            .clone()
            .oneshot(post_json(
                "/api/v1/auth/login",
                serde_json::json!({
                    "email": "bob@example.com",
                    "password": "wrong-password",
                }),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
    // Sixth attempt crosses the threshold → lockout.
    let locked = app
        .clone()
        .oneshot(post_json(
            "/api/v1/auth/login",
            serde_json::json!({
                "email": "bob@example.com",
                "password": "correct-horse-battery-staple",
            }),
        ))
        .await
        .unwrap();
    assert_eq!(locked.status(), StatusCode::TOO_MANY_REQUESTS);

    assert_eq!(
        actions_for(&pool, bob_id).await,
        vec![
            "auth.register",
            "auth.login_failed",
            "auth.login_failed",
            "auth.login_failed",
            "auth.login_failed",
            "auth.login_failed",
            "auth.login_locked",
        ]
    );
}
