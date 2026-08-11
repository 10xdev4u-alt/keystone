//! End-to-end auth flow tests: register → verify → login → me → refresh →
//! rotation → reuse detection → logout → lockout.
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

async fn test_app() -> Option<axum::Router> {
    let pool = keystone_db::test_util::test_pool_isolated().await?;
    keystone_db::test_util::setup(&pool)
        .await
        .expect("db setup");
    Some(router(AppState {
        pool,
        started_at: Instant::now(),
        auth: test_auth(),
        rate_limit: std::sync::Arc::new(keystone_api::middleware::RateLimiter::new()),
        realtime: std::sync::Arc::new(keystone_api::realtime::RealtimeHub::new()),
        storage: std::sync::Arc::new(keystone_db::storage::MemoryStorage::new()),
        oauth: None,
    }))
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

fn get(uri: &str, token: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .expect("request must build")
}

/// Extract the refresh cookie value from a response's Set-Cookie headers.
fn refresh_cookie_value(response: &axum::response::Response) -> Option<String> {
    for value in response.headers().get_all(header::SET_COOKIE) {
        let raw = value.to_str().ok()?;
        if let Some(rest) = raw.strip_prefix(&format!("{REFRESH_COOKIE}=")) {
            return rest.split(';').next().map(str::to_owned);
        }
    }
    None
}

fn cookie_header(value: &str) -> HeaderValue {
    HeaderValue::from_str(&format!("{REFRESH_COOKIE}={value}")).expect("cookie must be valid")
}

/// Cookie header carrying the refresh cookie AND the matching CSRF cookie,
/// as a legitimate SPA would send after login.
fn cookies_header(refresh: &str, csrf: &str) -> HeaderValue {
    HeaderValue::from_str(&format!("{REFRESH_COOKIE}={refresh}; keystone_csrf={csrf}"))
        .expect("cookie must be valid")
}

#[tokio::test]
async fn full_auth_flow_with_rotation_and_reuse_detection() {
    let Some(app) = test_app().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };

    // Register.
    let response = app
        .clone()
        .oneshot(post_json(
            "/api/v1/auth/register",
            serde_json::json!({
                "email": "barbara@example.com",
                "password": "correct-horse-battery-staple",
                "first_name": "Barbara",
                "last_name": "Liskov",
                "username": "bliskov",
            }),
        ))
        .await
        .expect("handler must not panic");
    assert_eq!(response.status(), StatusCode::CREATED);
    let register_body = json_body(response).await;
    let verification_token = register_body["verification_token"]
        .as_str()
        .expect("dev verification token present");

    // Duplicate registration conflicts.
    let dup = app
        .clone()
        .oneshot(post_json(
            "/api/v1/auth/register",
            serde_json::json!({
                "email": "BARBARA@example.com",
                "password": "correct-horse-battery-staple",
            }),
        ))
        .await
        .unwrap();
    assert_eq!(dup.status(), StatusCode::CONFLICT);

    // Verify email.
    let verify = app
        .clone()
        .oneshot(post_json(
            "/api/v1/auth/verify-email",
            serde_json::json!({ "token": verification_token }),
        ))
        .await
        .unwrap();
    assert_eq!(verify.status(), StatusCode::OK);

    // Login.
    let login = app
        .clone()
        .oneshot(post_json(
            "/api/v1/auth/login",
            serde_json::json!({
                "email": "barbara@example.com",
                "password": "correct-horse-battery-staple",
            }),
        ))
        .await
        .unwrap();
    assert_eq!(login.status(), StatusCode::OK);
    let cookie1 = refresh_cookie_value(&login).expect("refresh cookie set");
    let login_body = json_body(login).await;
    let access_token = login_body["access_token"].as_str().expect("access token");
    let csrf1 = login_body["csrf_token"].as_str().expect("csrf token");

    // /me with the Bearer token.
    let me = app
        .clone()
        .oneshot(get("/api/v1/auth/me", access_token))
        .await
        .unwrap();
    assert_eq!(me.status(), StatusCode::OK);
    let me_body = json_body(me).await;
    assert_eq!(me_body["user"]["email"], "barbara@example.com");
    assert_eq!(me_body["user"]["status"], "active");

    // Refresh WITHOUT the CSRF header → 403 (double-submit enforced).
    let no_csrf = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/refresh")
                .header(header::COOKIE, cookie_header(&cookie1))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(no_csrf.status(), StatusCode::FORBIDDEN);

    // Refresh with cookie + CSRF pair → rotation, new cookie.
    let refresh1 = app
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
    assert_eq!(refresh1.status(), StatusCode::OK);
    let cookie2 = refresh_cookie_value(&refresh1).expect("rotated cookie");
    assert_ne!(cookie1, cookie2, "refresh must rotate the token");
    let refresh_body = json_body(refresh1).await;
    let csrf2 = refresh_body["csrf_token"]
        .as_str()
        .expect("rotated csrf token");

    // Reusing the ROTATED-AWAY cookie → 401 token_reuse_detected + family
    // revoked. (The CSRF pair must be the CURRENT one to pass the guard.)
    let reuse = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/refresh")
                .header(header::COOKIE, cookies_header(&cookie1, csrf2))
                .header("x-csrf-token", HeaderValue::from_str(csrf2).unwrap())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(reuse.status(), StatusCode::UNAUTHORIZED);
    let reuse_body = json_body(reuse).await;
    assert_eq!(reuse_body["code"], "token_reuse_detected");

    // The current cookie is also dead now (family revoked).
    let dead = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/refresh")
                .header(header::COOKIE, cookies_header(&cookie2, csrf2))
                .header("x-csrf-token", HeaderValue::from_str(csrf2).unwrap())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(dead.status(), StatusCode::UNAUTHORIZED);

    // /me without a token → 401.
    let no_token = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/auth/me")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(no_token.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn wrong_password_locks_account_after_five_failures() {
    let Some(app) = test_app().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };

    let register = app
        .clone()
        .oneshot(post_json(
            "/api/v1/auth/register",
            serde_json::json!({
                "email": "alan@example.com",
                "password": "correct-horse-battery-staple",
            }),
        ))
        .await
        .unwrap();
    assert_eq!(register.status(), StatusCode::CREATED);
    let reg_body = json_body(register).await;
    let token = reg_body["verification_token"].as_str().unwrap();
    app.clone()
        .oneshot(post_json(
            "/api/v1/auth/verify-email",
            serde_json::json!({ "token": token }),
        ))
        .await
        .unwrap();

    // Five bad passwords are allowed (each a generic 401, recording a
    // failure); the SIXTH attempt is refused by the lockout policy.
    for _ in 0..5 {
        let response = app
            .clone()
            .oneshot(post_json(
                "/api/v1/auth/login",
                serde_json::json!({
                    "email": "alan@example.com",
                    "password": "wrong-password",
                }),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    // Sixth attempt crosses the threshold → 429 lockout.
    let locked = app
        .clone()
        .oneshot(post_json(
            "/api/v1/auth/login",
            serde_json::json!({
                "email": "alan@example.com",
                "password": "wrong-password",
            }),
        ))
        .await
        .unwrap();
    assert_eq!(locked.status(), StatusCode::TOO_MANY_REQUESTS);

    // Even the CORRECT password is refused while locked out.
    let still_locked = app
        .clone()
        .oneshot(post_json(
            "/api/v1/auth/login",
            serde_json::json!({
                "email": "alan@example.com",
                "password": "correct-horse-battery-staple",
            }),
        ))
        .await
        .unwrap();
    assert_eq!(still_locked.status(), StatusCode::TOO_MANY_REQUESTS);

    // Unknown emails are indistinguishable from wrong passwords.
    let unknown = app
        .clone()
        .oneshot(post_json(
            "/api/v1/auth/login",
            serde_json::json!({
                "email": "nobody@example.com",
                "password": "whatever",
            }),
        ))
        .await
        .unwrap();
    assert_eq!(unknown.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn register_validates_inputs() {
    let Some(app) = test_app().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };

    for (email, password) in [
        ("not-an-email", "correct-horse-battery-staple"),
        ("", "correct-horse-battery-staple"),
        ("a@example.com", "short"),
    ] {
        let response = app
            .clone()
            .oneshot(post_json(
                "/api/v1/auth/register",
                serde_json::json!({ "email": email, "password": password }),
            ))
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "expected 400 for email={email:?} password={password:?}"
        );
    }
}
