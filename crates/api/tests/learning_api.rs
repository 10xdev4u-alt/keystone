//! Month-6 API integration tests: course lifecycle + atomic certificates,
//! assessments with server-side grading, credits redemption, idempotent
//! event registration with waitlist promotion, mentorship state machine.
//! Real Postgres; self-skips without TEST_DATABASE_URL.

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
    // Each request is a distinct test client — the functional suites must
    // not trip the auth rate limiter (rate_limit.rs owns that behavior).
    let n = CLIENT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let ip = format!("10.{}.{}.{}", (n >> 16) & 0xff, (n >> 8) & 0xff, n & 0xff);
    builder = builder.header("x-forwarded-for", ip);
    match body {
        Some(body) => builder
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .expect("request must build"),
        None => builder.body(Body::empty()).expect("request must build"),
    }
}

static CLIENT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

async fn register_and_login(app: &axum::Router, email: &str) -> String {
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
        .expect("verify must not panic");
    let login = app
        .clone()
        .oneshot(request(
            "POST",
            "/api/v1/auth/login",
            None,
            Some(json!({
                "email": email,
                "password": "correct-horse-battery-staple",
            })),
        ))
        .await
        .expect("login must not panic");
    assert_eq!(login.status(), StatusCode::OK, "login {email}");
    json_body(login).await["access_token"]
        .as_str()
        .unwrap()
        .to_owned()
}

/// A published 2-lesson course; returns (slug, lesson ids) via the API.
async fn make_course(app: &axum::Router, token: &str) -> (String, String, String) {
    let created = app
        .clone()
        .oneshot(request(
            "POST",
            "/api/v1/courses",
            Some(token),
            Some(json!({"title": "Rust from Zero", "description": "a course"})),
        ))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let body = json_body(created).await;
    let slug = body["course"]["slug"].as_str().unwrap().to_owned();

    app.clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/courses/{slug}/publish"),
            Some(token),
            None,
        ))
        .await
        .unwrap();
    let module = app
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/courses/{slug}/modules"),
            Some(token),
            Some(json!({"position": 0, "title": "Basics"})),
        ))
        .await
        .unwrap();
    let module_id = json_body(module).await["id"].as_str().unwrap().to_owned();
    let mut lesson_ids = Vec::new();
    for (position, title) in [(0, "Hello"), (1, "Ownership")] {
        let lesson = app
            .clone()
            .oneshot(request(
                "POST",
                &format!("/api/v1/courses/{slug}/modules/{module_id}/lessons"),
                Some(token),
                Some(json!({"position": position, "title": title, "content": "body"})),
            ))
            .await
            .unwrap();
        assert_eq!(lesson.status(), StatusCode::CREATED);
        lesson_ids.push(json_body(lesson).await["id"].as_str().unwrap().to_owned());
    }
    (slug, lesson_ids[0].clone(), lesson_ids[1].clone())
}

#[tokio::test]
async fn course_completion_issues_certificate_via_api() {
    let Some((app, _pool)) = test_app().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let author = register_and_login(&app, "course-author@example.com").await;
    let student = register_and_login(&app, "course-student@example.com").await;
    let (slug, l1, l2) = make_course(&app, &author).await;

    // A non-enrolled student cannot complete lessons.
    let forbidden = app
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/courses/{slug}/lessons/{l1}/complete"),
            Some(&student),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);

    // Enroll, complete lesson 1 → no certificate yet.
    let enroll = app
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/courses/{slug}/enroll"),
            Some(&student),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(enroll.status(), StatusCode::NO_CONTENT);
    let first = app
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/courses/{slug}/lessons/{l1}/complete"),
            Some(&student),
            None,
        ))
        .await
        .unwrap();
    let first_body = json_body(first).await;
    assert!(
        first_body["certificate"].is_null(),
        "half course → no certificate"
    );

    // Complete lesson 2 → certificate + one-time token.
    let second = app
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/courses/{slug}/lessons/{l2}/complete"),
            Some(&student),
            None,
        ))
        .await
        .unwrap();
    let second_body = json_body(second).await;
    let token = second_body["certificate"]["token"]
        .as_str()
        .expect("token returned once");
    assert!(!token.is_empty());

    // Re-completing issues nothing new.
    let again = app
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/courses/{slug}/lessons/{l1}/complete"),
            Some(&student),
            None,
        ))
        .await
        .unwrap();
    assert!(json_body(again).await["certificate"].is_null());

    // Progress reports the derived percent.
    let progress = app
        .clone()
        .oneshot(request(
            "GET",
            &format!("/api/v1/courses/{slug}/progress"),
            Some(&student),
            None,
        ))
        .await
        .unwrap();
    let progress_body = json_body(progress).await;
    assert_eq!(progress_body["percent"], 100);
    assert_eq!(progress_body["completed_lessons"], 2);

    // My certificates lists exactly one.
    let certs = app
        .clone()
        .oneshot(request(
            "GET",
            "/api/v1/me/certificates",
            Some(&student),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(
        json_body(certs).await["certificates"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn assessment_api_grades_server_side() {
    let Some((app, _pool)) = test_app().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let author = register_and_login(&app, "assess-api-author@example.com").await;
    let student = register_and_login(&app, "assess-api-student@example.com").await;
    let (slug, _, _) = make_course(&app, &author).await;

    let assessment = app
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/courses/{slug}/assessments"),
            Some(&author),
            Some(json!({"title": "Basics quiz", "pass_threshold": 50})),
        ))
        .await
        .unwrap();
    let assessment_id = json_body(assessment).await["id"]
        .as_str()
        .unwrap()
        .to_owned();

    // Author adds questions WITH the grading key; a non-author cannot.
    let outsider = register_and_login(&app, "assess-api-outsider@example.com").await;
    let forbidden = app
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/courses/{slug}/assessments/{assessment_id}/questions"),
            Some(&outsider),
            Some(json!({"position": 0, "prompt": "1+1?", "correct_response": "2"})),
        ))
        .await
        .unwrap();
    assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);
    let q1 = app
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/courses/{slug}/assessments/{assessment_id}/questions"),
            Some(&author),
            Some(json!({"position": 0, "prompt": "1+1?", "correct_response": "2"})),
        ))
        .await
        .unwrap();
    let q1_id = json_body(q1).await["id"].as_str().unwrap().to_owned();

    // Public question read NEVER leaks the key.
    let read = app
        .clone()
        .oneshot(request(
            "GET",
            &format!("/api/v1/assessments/{assessment_id}"),
            None,
            None,
        ))
        .await
        .unwrap();
    let read_body = json_body(read).await;
    assert!(
        !read_body.to_string().contains("correct_response"),
        "grading key must never be exposed"
    );

    // Enroll → start → submit → scored server-side.
    app.clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/courses/{slug}/enroll"),
            Some(&student),
            None,
        ))
        .await
        .unwrap();
    let started = app
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/assessments/{assessment_id}/attempts"),
            Some(&student),
            None,
        ))
        .await
        .unwrap();
    let attempt_id = json_body(started).await["attempt_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let submitted = app
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/attempts/{attempt_id}/submit"),
            Some(&student),
            Some(json!({
                "answers": [{"question_id": q1_id, "response": "2"}],
            })),
        ))
        .await
        .unwrap();
    let graded = json_body(submitted).await;
    assert_eq!(graded["score"], 100);
    assert_eq!(graded["passed"], true);
}

#[tokio::test]
async fn credits_balance_and_redemption_api() {
    let Some((app, pool)) = test_app().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let token = register_and_login(&app, "credit-user@example.com").await;
    let user_id = {
        let me = app
            .clone()
            .oneshot(request("GET", "/api/v1/auth/me", Some(&token), None))
            .await
            .unwrap();
        json_body(me).await["user"]["id"]
            .as_str()
            .unwrap()
            .to_owned()
    };

    // Earn via the system path (append-only), then redeem through the API.
    let credits = keystone_db::repositories::credits::Credits::new(pool.clone());
    credits
        .append(
            uuid::Uuid::parse_str(&user_id).unwrap(),
            10,
            "signup",
            None,
            None,
        )
        .await
        .unwrap();
    let balance = app
        .clone()
        .oneshot(request("GET", "/api/v1/me/credits", Some(&token), None))
        .await
        .unwrap();
    assert_eq!(json_body(balance).await["balance"], 10);

    let redeemed = app
        .clone()
        .oneshot(request(
            "POST",
            "/api/v1/me/credits/redeem",
            Some(&token),
            Some(json!({"amount": 4, "reason": "course"})),
        ))
        .await
        .unwrap();
    assert_eq!(redeemed.status(), StatusCode::CREATED);
    assert_eq!(json_body(redeemed).await["balance"], 6);

    // Overdraft → 400, balance untouched.
    let overdraft = app
        .clone()
        .oneshot(request(
            "POST",
            "/api/v1/me/credits/redeem",
            Some(&token),
            Some(json!({"amount": 99, "reason": "too much"})),
        ))
        .await
        .unwrap();
    assert_eq!(overdraft.status(), StatusCode::BAD_REQUEST);
    let balance = app
        .clone()
        .oneshot(request("GET", "/api/v1/me/credits", Some(&token), None))
        .await
        .unwrap();
    assert_eq!(json_body(balance).await["balance"], 6);

    // The ledger is immutable and complete.
    let ledger = app
        .clone()
        .oneshot(request(
            "GET",
            "/api/v1/me/credits/ledger",
            Some(&token),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(
        json_body(ledger).await["ledger"].as_array().unwrap().len(),
        2
    );
}

#[tokio::test]
async fn events_api_waitlist_and_promotion() {
    let Some((app, _pool)) = test_app().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let organizer = register_and_login(&app, "event-org@example.com").await;
    let a = register_and_login(&app, "event-a@example.com").await;
    let b = register_and_login(&app, "event-b@example.com").await;
    let c = register_and_login(&app, "event-c@example.com").await;

    let created = app
        .clone()
        .oneshot(request(
            "POST",
            "/api/v1/events",
            Some(&organizer),
            Some(json!({
                "title": "Rust Meetup",
                "starts_at": "2027-01-01T10:00:00Z",
                "ends_at": "2027-01-01T12:00:00Z",
                "capacity": 2,
            })),
        ))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let slug = json_body(created).await["event"]["slug"]
        .as_str()
        .unwrap()
        .to_owned();

    let reg_a = app
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/events/{slug}/register"),
            Some(&a),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(json_body(reg_a).await["status"], "registered");
    let reg_b = app
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/events/{slug}/register"),
            Some(&b),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(json_body(reg_b).await["status"], "registered");
    let reg_c = app
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/events/{slug}/register"),
            Some(&c),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(json_body(reg_c).await["status"], "waitlisted");

    // A cancels → C (the waitlist) is promoted atomically.
    app.clone()
        .oneshot(request(
            "DELETE",
            &format!("/api/v1/events/{slug}/registration"),
            Some(&a),
            None,
        ))
        .await
        .unwrap();
    let promoted = app
        .clone()
        .oneshot(request(
            "GET",
            &format!("/api/v1/events/{slug}"),
            Some(&c),
            None,
        ))
        .await
        .unwrap();
    let promoted_body = json_body(promoted).await;
    assert_eq!(
        promoted_body["my_registration"], "registered",
        "waitlist promoted"
    );

    // Organizer adds a speaker; a non-organizer cannot.
    let outsider = register_and_login(&app, "event-outsider@example.com").await;
    let c_id = {
        let me = app
            .clone()
            .oneshot(request("GET", "/api/v1/auth/me", Some(&c), None))
            .await
            .unwrap();
        json_body(me).await["user"]["id"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let forbidden = app
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/events/{slug}/speakers/{c_id}"),
            Some(&outsider),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);
    let add = app
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/events/{slug}/speakers/{c_id}"),
            Some(&organizer),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(add.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn mentorship_api_flow() {
    let Some((app, _pool)) = test_app().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let mentor = register_and_login(&app, "mentor-api@example.com").await;
    let mentee = register_and_login(&app, "mentee-api@example.com").await;
    let mentor_id = {
        let me = app
            .clone()
            .oneshot(request("GET", "/api/v1/auth/me", Some(&mentor), None))
            .await
            .unwrap();
        json_body(me).await["user"]["id"]
            .as_str()
            .unwrap()
            .to_owned()
    };

    // Mentor profile → visible in the directory.
    app.clone()
        .oneshot(request(
            "PUT",
            "/api/v1/me/mentor-profile",
            Some(&mentor),
            Some(json!({"bio": "rust mentor", "areas": "rust", "available": true})),
        ))
        .await
        .unwrap();
    let mentors = app
        .clone()
        .oneshot(request("GET", "/api/v1/mentors", None, None))
        .await
        .unwrap();
    assert_eq!(
        json_body(mentors).await["mentors"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    // Mentee requests; mentor accepts; session scheduled; feedback given.
    let requested = app
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/users/{mentor_id}/mentorship"),
            Some(&mentee),
            Some(json!({"message": "help me learn"})),
        ))
        .await
        .unwrap();
    assert_eq!(requested.status(), StatusCode::CREATED);
    let request_id = json_body(requested).await["request_id"]
        .as_str()
        .unwrap()
        .to_owned();

    let accepted = app
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/mentorship/{request_id}/accept"),
            Some(&mentor),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(accepted.status(), StatusCode::NO_CONTENT);

    let session = app
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/mentorship/{request_id}/sessions"),
            Some(&mentor),
            Some(json!({"scheduled_at": "2027-02-01T10:00:00Z", "duration_minutes": 30})),
        ))
        .await
        .unwrap();
    let session_id = json_body(session).await["session_id"]
        .as_str()
        .unwrap()
        .to_owned();

    let feedback = app
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/sessions/{session_id}/feedback"),
            Some(&mentee),
            Some(json!({"rating": 5, "comment": "great"})),
        ))
        .await
        .unwrap();
    assert_eq!(feedback.status(), StatusCode::CREATED);
    assert_eq!(json_body(feedback).await["rating"], 5);
}
