//! Session management + RBAC integration tests: list / revoke-one /
//! revoke-all with ownership enforcement, and the admin-only route.

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
    let pool = keystone_db::test_util::test_pool().await?;
    keystone_db::test_util::setup(&pool)
        .await
        .expect("db setup");
    Some(router(AppState {
        pool,
        started_at: Instant::now(),
        auth: test_auth(),
        rate_limit: std::sync::Arc::new(keystone_api::middleware::RateLimiter::new()),
    }))
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

fn delete(uri: &str, token: &str) -> Request<Body> {
    Request::builder()
        .method("DELETE")
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .expect("request must build")
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

fn refresh_cookie_value(response: &axum::response::Response) -> Option<String> {
    for value in response.headers().get_all(header::SET_COOKIE) {
        let raw = value.to_str().ok()?;
        if let Some(rest) = raw.strip_prefix(&format!("{REFRESH_COOKIE}=")) {
            return rest.split(';').next().map(str::to_owned);
        }
    }
    None
}

/// Register + verify + login, returning (access token, refresh cookie).
async fn register_and_login(app: &axum::Router, email: &str) -> (String, String) {
    let reg = app
        .clone()
        .oneshot(post_json(
            "/api/v1/auth/register",
            serde_json::json!({ "email": email, "password": "correct-horse-battery-staple" }),
        ))
        .await
        .unwrap();
    let reg_body = json_body(reg).await;
    let token = reg_body["verification_token"].as_str().unwrap();
    app.clone()
        .oneshot(post_json(
            "/api/v1/auth/verify-email",
            serde_json::json!({ "token": token }),
        ))
        .await
        .unwrap();

    let login = app
        .clone()
        .oneshot(post_json(
            "/api/v1/auth/login",
            serde_json::json!({ "email": email, "password": "correct-horse-battery-staple" }),
        ))
        .await
        .unwrap();
    let cookie = refresh_cookie_value(&login).expect("refresh cookie");
    let access = json_body(login).await["access_token"]
        .as_str()
        .unwrap()
        .to_owned();
    (access, cookie)
}

#[tokio::test]
async fn list_revoke_one_and_revoke_all_with_ownership() {
    let Some(app) = test_app().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };

    let (alice_token, alice_cookie) = register_and_login(&app, "alice@example.com").await;
    // Second device for Alice = a second login (register is one-per-email).
    let login2 = app
        .clone()
        .oneshot(post_json(
            "/api/v1/auth/login",
            serde_json::json!({ "email": "alice@example.com", "password": "correct-horse-battery-staple" }),
        ))
        .await
        .unwrap();
    let alice_cookie2 = refresh_cookie_value(&login2).expect("second alice cookie");
    let (bob_token, _bob_cookie) = register_and_login(&app, "bob@example.com").await;

    // Alice lists two live sessions; the one matching her current cookie is
    // flagged `current`.
    let list = app
        .clone()
        .oneshot(get("/api/v1/auth/sessions", &alice_token).with_cookie(&alice_cookie))
        .await
        .unwrap();
    assert_eq!(list.status(), StatusCode::OK);
    let sessions = json_body(list).await;
    let list = sessions["sessions"].as_array().unwrap();
    assert_eq!(list.len(), 2, "alice has two live sessions");
    assert_eq!(list.iter().filter(|s| s["current"] == true).count(), 1);

    // Alice revokes her OTHER session by id.
    let other_id = list.iter().find(|s| s["current"] == false).unwrap()["id"]
        .as_str()
        .unwrap();
    let revoke = app
        .clone()
        .oneshot(delete(
            &format!("/api/v1/auth/sessions/{other_id}"),
            &alice_token,
        ))
        .await
        .unwrap();
    assert_eq!(revoke.status(), StatusCode::NO_CONTENT);

    let after = json_body(
        app.clone()
            .oneshot(get("/api/v1/auth/sessions", &alice_token).with_cookie(&alice_cookie))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(after["sessions"].as_array().unwrap().len(), 1);

    // Ownership: Bob cannot revoke Alice's session (404, no existence leak).
    let bob_revoke = app
        .clone()
        .oneshot(delete(
            &format!("/api/v1/auth/sessions/{other_id}"),
            &bob_token,
        ))
        .await
        .unwrap();
    assert_eq!(bob_revoke.status(), StatusCode::NOT_FOUND);

    // Revoke-all for Bob.
    let revoke_all = app
        .clone()
        .oneshot(delete("/api/v1/auth/sessions", &bob_token))
        .await
        .unwrap();
    assert_eq!(revoke_all.status(), StatusCode::NO_CONTENT);
    let bob_list = json_body(
        app.clone()
            .oneshot(get("/api/v1/auth/sessions", &bob_token))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(bob_list["sessions"].as_array().unwrap().len(), 0);
    let _ = alice_cookie2;
}

#[tokio::test]
async fn admin_status_requires_admin_role() {
    let Some(app) = test_app().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };

    // A regular user is forbidden.
    let (user_token, _) = register_and_login(&app, "carol@example.com").await;
    let forbidden = app
        .clone()
        .oneshot(get("/api/v1/admin/status", &user_token))
        .await
        .unwrap();
    assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);
    let body = json_body(forbidden).await;
    assert_eq!(body["code"], "forbidden");

    // Unauthenticated → 401.
    let unauth = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/admin/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauth.status(), StatusCode::UNAUTHORIZED);

    // An admin token opens the endpoint. The role comes from the JWT claims,
    // so issue one directly with the test signing service.
    let admin_token = keystone_auth::jwt::AccessTokenService::new(
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
    )
    .issue("00000000-0000-0000-0000-000000000001", "admin", None)
    .expect("admin token must issue");

    let ok = app
        .clone()
        .oneshot(get("/api/v1/admin/status", &admin_token))
        .await
        .unwrap();
    assert_eq!(ok.status(), StatusCode::OK);
    let stats = json_body(ok).await;
    assert_eq!(stats["status"], "ok");
    assert!(stats["users"].as_i64().unwrap() >= 1);
}

trait WithCookie {
    fn with_cookie(self, cookie: &str) -> Request<Body>;
}

impl WithCookie for Request<Body> {
    fn with_cookie(mut self, cookie: &str) -> Request<Body> {
        self.headers_mut().insert(
            header::COOKIE,
            HeaderValue::from_str(&format!("{REFRESH_COOKIE}={cookie}")).unwrap(),
        );
        self
    }
}
