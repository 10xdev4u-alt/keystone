//! Q&A API integration tests: answers, voting, acceptance, and the bounty
//! lifecycle over HTTP. Real Postgres; self-skips without TEST_DATABASE_URL.

use axum::body::Body;
use axum::http::header;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use keystone_api::auth::AuthServices;
use keystone_api::{router, AppState};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tower::ServiceExt;

type App = (axum::Router, sqlx::PgPool);

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

async fn test_app() -> Option<App> {
    let pool = keystone_db::test_util::test_pool_isolated().await?;
    let app = router(AppState {
        pool: pool.clone(),
        started_at: Instant::now(),
        auth: test_auth(),
        rate_limit: std::sync::Arc::new(keystone_api::middleware::RateLimiter::new()),
        oauth: None,
    });
    Some((app, pool))
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

fn request(method: &str, uri: &str, token: Option<&str>, body: Option<Value>) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(token) = token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    match body {
        Some(body) => builder
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .expect("request must build"),
        None => builder.body(Body::empty()).expect("request must build"),
    }
}

async fn register(app: &axum::Router, email: &str) -> String {
    let response = app
        .clone()
        .oneshot(request(
            "POST",
            "/api/v1/auth/register",
            None,
            Some(json!({
                "email": email,
                "password": "correct-horse-battery-staple",
                "username": email.split('@').next().unwrap(),
            })),
        ))
        .await
        .expect("register must not panic");
    assert_eq!(response.status(), StatusCode::CREATED, "register {email}");
    let body = json_body(response).await;
    let token = body["verification_token"].as_str().expect("dev token");
    app.clone()
        .oneshot(request(
            "POST",
            "/api/v1/auth/verify-email",
            None,
            Some(json!({ "token": token })),
        ))
        .await
        .unwrap();
    let login = app
        .clone()
        .oneshot(request(
            "POST",
            "/api/v1/auth/login",
            None,
            Some(json!({ "email": email, "password": "correct-horse-battery-staple" })),
        ))
        .await
        .unwrap();
    json_body(login).await["access_token"]
        .as_str()
        .unwrap()
        .to_owned()
}

async fn create_question(app: &axum::Router, token: &str) -> String {
    let response = app
        .clone()
        .oneshot(request(
            "POST",
            "/api/v1/posts",
            Some(token),
            Some(json!({"kind": "question", "body": "Why is Rust fast?", "visibility": "public"})),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    json_body(response).await["post"]["id"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[tokio::test]
async fn answer_vote_and_accept_flow() {
    let Some((app, _pool)) = test_app().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let asker = register(&app, "qasker@example.com").await;
    let answerer = register(&app, "qanswerer@example.com").await;
    let voter = register(&app, "qvoter@example.com").await;
    let question = create_question(&app, &asker).await;

    // Only question-kind posts accept answers.
    let essay = {
        let response = app
            .clone()
            .oneshot(request(
                "POST",
                "/api/v1/posts",
                Some(&asker),
                Some(json!({"kind": "article", "body": "not a question", "visibility": "public"})),
            ))
            .await
            .unwrap();
        json_body(response).await["post"]["id"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let rejected = app
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/posts/{essay}/answers"),
            Some(&answerer),
            Some(json!({"body": "nope"})),
        ))
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);

    // Answer, then answer again.
    let a1 = app
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/posts/{question}/answers"),
            Some(&answerer),
            Some(json!({"body": "Memory safety."})),
        ))
        .await
        .unwrap();
    assert_eq!(a1.status(), StatusCode::CREATED);
    let a1_id = json_body(a1).await["answer"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let a2 = app
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/posts/{question}/answers"),
            Some(&answerer),
            Some(json!({"body": "Zero-cost abstractions."})),
        ))
        .await
        .unwrap();
    let a2_id = json_body(a2).await["answer"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    // Anonymous cannot answer or vote.
    let anon = app
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/posts/{question}/answers"),
            None,
            Some(json!({"body": "anon"})),
        ))
        .await
        .unwrap();
    assert_eq!(anon.status(), StatusCode::UNAUTHORIZED);

    // Vote up, switch to down, remove via 0 — score follows.
    app.clone()
        .oneshot(request(
            "PUT",
            &format!("/api/v1/answers/{a1_id}/vote"),
            Some(&voter),
            Some(json!({"vote": 1})),
        ))
        .await
        .unwrap();
    app.clone()
        .oneshot(request(
            "PUT",
            &format!("/api/v1/answers/{a1_id}/vote"),
            Some(&voter),
            Some(json!({"vote": -1})),
        ))
        .await
        .unwrap();
    let list = app
        .clone()
        .oneshot(request(
            "GET",
            &format!("/api/v1/posts/{question}/answers"),
            None,
            None,
        ))
        .await
        .unwrap();
    let rows = json_body(list).await;
    let a1_row = rows["answers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == a1_id)
        .unwrap();
    assert_eq!(a1_row["score"], -1);
    app.clone()
        .oneshot(request(
            "PUT",
            &format!("/api/v1/answers/{a1_id}/vote"),
            Some(&voter),
            Some(json!({"vote": 0})),
        ))
        .await
        .unwrap();

    // Only the asker (or staff) accepts.
    let not_asker = app
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/posts/{question}/answers/{a2_id}/accept"),
            Some(&answerer),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(not_asker.status(), StatusCode::FORBIDDEN);
    let accept = app
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/posts/{question}/answers/{a2_id}/accept"),
            Some(&asker),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(accept.status(), StatusCode::NO_CONTENT);
    let list = app
        .clone()
        .oneshot(request(
            "GET",
            &format!("/api/v1/posts/{question}/answers"),
            None,
            None,
        ))
        .await
        .unwrap();
    let rows = json_body(list).await;
    let accepted: Vec<_> = rows["answers"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|a| a["accepted"] == true)
        .collect();
    assert_eq!(accepted.len(), 1);
    assert_eq!(accepted[0]["id"], a2_id);
}

#[tokio::test]
async fn bounty_open_award_and_refusals() {
    let Some((app, _pool)) = test_app().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let asker = register(&app, "basker@example.com").await;
    let answerer = register(&app, "banswerer@example.com").await;
    let question = create_question(&app, &asker).await;
    let answer = app
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/posts/{question}/answers"),
            Some(&answerer),
            Some(json!({"body": "The answer."})),
        ))
        .await
        .unwrap();
    let answer_id = json_body(answer).await["answer"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let expires = (chrono::Utc::now() + chrono::Duration::days(7))
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

    // Only the asker opens a bounty; anonymous is rejected.
    let anon = app
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/posts/{question}/bounty"),
            None,
            Some(json!({"amount": 100, "expires_at": expires})),
        ))
        .await
        .unwrap();
    assert_eq!(anon.status(), StatusCode::UNAUTHORIZED);
    let not_asker = app
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/posts/{question}/bounty"),
            Some(&answerer),
            Some(json!({"amount": 100, "expires_at": expires})),
        ))
        .await
        .unwrap();
    assert_eq!(not_asker.status(), StatusCode::FORBIDDEN);

    // Past expiry is rejected; valid open succeeds.
    let past = (chrono::Utc::now() - chrono::Duration::hours(1))
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let bad_expiry = app
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/posts/{question}/bounty"),
            Some(&asker),
            Some(json!({"amount": 100, "expires_at": past})),
        ))
        .await
        .unwrap();
    assert_eq!(bad_expiry.status(), StatusCode::BAD_REQUEST);

    let opened = app
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/posts/{question}/bounty"),
            Some(&asker),
            Some(json!({"amount": 100, "expires_at": expires})),
        ))
        .await
        .unwrap();
    assert_eq!(opened.status(), StatusCode::CREATED);
    let bounty_id = json_body(opened).await["bounty"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    // A second bounty on the same question conflicts.
    let dup = app
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/posts/{question}/bounty"),
            Some(&asker),
            Some(json!({"amount": 50, "expires_at": expires})),
        ))
        .await
        .unwrap();
    assert_eq!(dup.status(), StatusCode::CONFLICT);

    // Public read shows the open bounty.
    let read = app
        .clone()
        .oneshot(request(
            "GET",
            &format!("/api/v1/posts/{question}/bounty"),
            None,
            None,
        ))
        .await
        .unwrap();
    let body = json_body(read).await;
    assert_eq!(body["bounty"]["status"], "open");
    assert_eq!(body["bounty"]["amount"], 100);

    // Only the asker can award.
    let not_asker = app
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/bounties/{bounty_id}/award"),
            Some(&answerer),
            Some(json!({"answer_id": answer_id})),
        ))
        .await
        .unwrap();
    assert_eq!(not_asker.status(), StatusCode::FORBIDDEN);

    // Award succeeds once; second award is refused (idempotent guard → 400).
    let award = app
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/bounties/{bounty_id}/award"),
            Some(&asker),
            Some(json!({"answer_id": answer_id})),
        ))
        .await
        .unwrap();
    assert_eq!(award.status(), StatusCode::OK);
    let again = app
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/bounties/{bounty_id}/award"),
            Some(&asker),
            Some(json!({"answer_id": answer_id})),
        ))
        .await
        .unwrap();
    assert_eq!(again.status(), StatusCode::BAD_REQUEST);
}
