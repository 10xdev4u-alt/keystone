//! Month-5 network API integration tests: organizations + role rules,
//! the social graph (follow/connect/block), the profile visibility matrix,
//! and anonymized salary benchmarks. Real Postgres; self-skips without
//! TEST_DATABASE_URL.

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
        realtime: std::sync::Arc::new(keystone_api::realtime::RealtimeHub::new()),
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

#[tokio::test]
async fn orgs_roles_and_ownership_transfer() {
    let Some((app, _pool)) = test_app().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let owner = register_and_login(&app, "org-owner@example.com").await;
    let member = register_and_login(&app, "org-member@example.com").await;
    let outsider = register_and_login(&app, "org-outsider@example.com").await;

    // Create → creator is sole owner.
    let created = app
        .clone()
        .oneshot(request(
            "POST",
            "/api/v1/orgs",
            Some(&owner),
            Some(json!({"name": "Acme Corp", "industry": "software"})),
        ))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let slug = json_body(created).await["organization"]["slug"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(slug, "acme-corp");

    // Member joins the org, then the owner transfers ownership to them.
    let join = app
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/orgs/{slug}/join"),
            Some(&member),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(join.status(), StatusCode::NO_CONTENT);
    let member_id = member_id_with_role(&app, &slug, "member").await;

    // Non-member cannot change roles.
    let forbidden = app
        .clone()
        .oneshot(request(
            "PATCH",
            &format!("/api/v1/orgs/{slug}/members/{member_id}"),
            Some(&outsider),
            Some(json!({"role": "admin"})),
        ))
        .await
        .unwrap();
    assert_eq!(
        forbidden.status(),
        StatusCode::FORBIDDEN,
        "outsider cannot set roles"
    );

    let transfer = app
        .clone()
        .oneshot(request(
            "PATCH",
            &format!("/api/v1/orgs/{slug}/members/{member_id}"),
            Some(&owner),
            Some(json!({"role": "owner"})),
        ))
        .await
        .unwrap();
    assert_eq!(
        transfer.status(),
        StatusCode::NO_CONTENT,
        "owner can transfer"
    );

    // The former owner is now just admin — no more role changes.
    let demoted = app
        .clone()
        .oneshot(request(
            "PATCH",
            &format!("/api/v1/orgs/{slug}/members/{member_id}"),
            Some(&owner),
            Some(json!({"role": "admin"})),
        ))
        .await
        .unwrap();
    assert_eq!(
        demoted.status(),
        StatusCode::FORBIDDEN,
        "former owner lost the gate"
    );
}

/// The user id of a member holding the given role, if any.
async fn member_id_with_role(app: &axum::Router, slug: &str, role: &str) -> String {
    let members = app
        .clone()
        .oneshot(request(
            "GET",
            &format!("/api/v1/orgs/{slug}/members"),
            None,
            None,
        ))
        .await
        .unwrap();
    let body = json_body(members).await;
    body["members"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["role"] == role)
        .expect("member with role")["user_id"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[tokio::test]
async fn org_claim_round_trip() {
    let Some((app, _pool)) = test_app().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let owner = register_and_login(&app, "claim-owner@example.com").await;
    let member = register_and_login(&app, "claim-member@example.com").await;
    let outsider = register_and_login(&app, "claim-outsider@example.com").await;

    let created = app
        .clone()
        .oneshot(request(
            "POST",
            "/api/v1/orgs",
            Some(&owner),
            Some(json!({"name": "Domain Ltd"})),
        ))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let slug = json_body(created).await["organization"]["slug"]
        .as_str()
        .unwrap()
        .to_owned();

    // A non-member cannot file a claim (drive-by takeover guard).
    let drive_by = app
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/orgs/{slug}/claims"),
            Some(&outsider),
            Some(json!({"domain": "example.com"})),
        ))
        .await
        .unwrap();
    assert_eq!(drive_by.status(), StatusCode::FORBIDDEN);

    // A member files; the raw token comes back once.
    app.clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/orgs/{slug}/join"),
            Some(&member),
            None,
        ))
        .await
        .unwrap();
    let filed = app
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/orgs/{slug}/claims"),
            Some(&member),
            Some(json!({"domain": "example.com"})),
        ))
        .await
        .unwrap();
    assert_eq!(filed.status(), StatusCode::CREATED);
    let claim_body = json_body(filed).await;
    let claim_id = claim_body["claim_id"].as_str().unwrap().to_owned();
    let token = claim_body["token"].as_str().unwrap().to_owned();

    // Wrong token → 400; right token → approved; reuse → 400.
    let wrong = app
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/orgs/{slug}/claims/{claim_id}/verify"),
            Some(&member),
            Some(json!({"token": "wrong"})),
        ))
        .await
        .unwrap();
    assert_eq!(wrong.status(), StatusCode::BAD_REQUEST);

    let verify = app
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/orgs/{slug}/claims/{claim_id}/verify"),
            Some(&member),
            Some(json!({"token": token})),
        ))
        .await
        .unwrap();
    assert_eq!(verify.status(), StatusCode::OK);
    assert_eq!(json_body(verify).await["status"], "approved");

    let reuse = app
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/orgs/{slug}/claims/{claim_id}/verify"),
            Some(&member),
            Some(json!({"token": token})),
        ))
        .await
        .unwrap();
    assert_eq!(
        reuse.status(),
        StatusCode::BAD_REQUEST,
        "claim tokens are single-use"
    );
}

#[tokio::test]
async fn social_graph_and_profile_visibility_matrix() {
    let Some((app, _pool)) = test_app().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let alice = register_and_login(&app, "alice-graph@example.com").await;
    let bob = register_and_login(&app, "bob-graph@example.com").await;
    let carol = register_and_login(&app, "carol-graph@example.com").await;
    let alice_id = user_id(&app, &alice).await;
    let bob_id = user_id(&app, &bob).await;
    let carol_id = user_id(&app, &carol).await;

    // Carol has a public profile — it is the subject of the block checks.
    let carol_public = app
        .clone()
        .oneshot(request(
            "PUT",
            "/api/v1/me/profile",
            Some(&carol),
            Some(json!({"bio": "public carol", "visibility": "public"})),
        ))
        .await
        .unwrap();
    assert_eq!(carol_public.status(), StatusCode::OK);

    // Alice sets a connections-only profile.
    let set = app
        .clone()
        .oneshot(request(
            "PUT",
            "/api/v1/me/profile",
            Some(&alice),
            Some(json!({"bio": "private-ish", "visibility": "connections"})),
        ))
        .await
        .unwrap();
    assert_eq!(set.status(), StatusCode::OK);

    // Carol (no connection) gets 404 — existence hidden.
    let hidden = app
        .clone()
        .oneshot(request(
            "GET",
            &format!("/api/v1/users/{alice_id}/profile"),
            Some(&carol),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(hidden.status(), StatusCode::NOT_FOUND);

    // Anonymous cannot see it either.
    let anon = app
        .clone()
        .oneshot(request(
            "GET",
            &format!("/api/v1/users/{alice_id}/profile"),
            None,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(anon.status(), StatusCode::NOT_FOUND);

    // Bob connects to Alice; once accepted, Bob can read.
    app.clone()
        .oneshot(request(
            "PUT",
            &format!("/api/v1/users/{alice_id}/connect"),
            Some(&bob),
            None,
        ))
        .await
        .unwrap();
    app.clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/users/{bob_id}/connections/accept"),
            Some(&alice),
            None,
        ))
        .await
        .unwrap();
    let visible = app
        .clone()
        .oneshot(request(
            "GET",
            &format!("/api/v1/users/{alice_id}/profile"),
            Some(&bob),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(
        visible.status(),
        StatusCode::OK,
        "accepted connection can read"
    );

    // Alice blocks Carol → the reverse direction is hidden too.
    app.clone()
        .oneshot(request(
            "PUT",
            &format!("/api/v1/users/{carol_id}/block"),
            Some(&alice),
            None,
        ))
        .await
        .unwrap();
    let blocked = app
        .clone()
        .oneshot(request(
            "GET",
            &format!("/api/v1/users/{carol_id}/profile"),
            Some(&alice),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(blocked.status(), StatusCode::NOT_FOUND, "block is mutual");

    // Unblock restores.
    app.clone()
        .oneshot(request(
            "DELETE",
            &format!("/api/v1/users/{carol_id}/block"),
            Some(&alice),
            None,
        ))
        .await
        .unwrap();
    let restored = app
        .clone()
        .oneshot(request(
            "GET",
            &format!("/api/v1/users/{carol_id}/profile"),
            Some(&alice),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(restored.status(), StatusCode::OK);
}

async fn user_id(app: &axum::Router, token: &str) -> String {
    let me = app
        .clone()
        .oneshot(request("GET", "/api/v1/auth/me", Some(token), None))
        .await
        .unwrap();
    json_body(me).await["user"]["id"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[tokio::test]
async fn salary_benchmarks_stay_anonymous_until_aggregated() {
    let Some((app, _pool)) = test_app().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let token = register_and_login(&app, "salary@example.com").await;

    // Sub-threshold: search shows no bucket even after 4 submissions.
    for _ in 0..4 {
        let submit = app
            .clone()
            .oneshot(request(
                "POST",
                "/api/v1/salaries",
                Some(&token),
                Some(json!({
                    "role": "Engineer",
                    "location": "Berlin",
                    "currency": "EUR",
                    "amount": 100_000,
                })),
            ))
            .await
            .unwrap();
        assert_eq!(submit.status(), StatusCode::ACCEPTED);
        let search = app
            .clone()
            .oneshot(request(
                "GET",
                "/api/v1/salaries/search?role=Engineer",
                None,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(
            json_body(search).await["buckets"].as_array().unwrap().len(),
            0,
            "sub-threshold bucket must not be readable"
        );
    }

    // The 5th submission makes the bucket visible — bounds only.
    let submit = app
        .clone()
        .oneshot(request(
            "POST",
            "/api/v1/salaries",
            Some(&token),
            Some(json!({
                "role": "Engineer",
                "location": "Berlin",
                "currency": "EUR",
                "amount": 95_000,
            })),
        ))
        .await
        .unwrap();
    assert_eq!(submit.status(), StatusCode::ACCEPTED);
    let search = app
        .clone()
        .oneshot(request(
            "GET",
            "/api/v1/salaries/search?role=Engineer",
            None,
            None,
        ))
        .await
        .unwrap();
    let body = json_body(search).await;
    let buckets = body["buckets"].as_array().unwrap();
    assert_eq!(buckets.len(), 1);
    let bucket = &buckets[0];
    assert_eq!(bucket["source_count"], 5);
    assert!(bucket["min"].as_i64().unwrap() <= 95_000);
    assert!(bucket["max"].as_i64().unwrap() >= 100_000);
    // No identity field ever appears in the response.
    let serialized = bucket.to_string();
    assert!(
        !serialized.contains("user") && !serialized.contains("submitted_by"),
        "salary responses must carry no identity"
    );
}

#[tokio::test]
async fn vendors_and_alerts_require_org_staff() {
    let Some((app, _pool)) = test_app().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let owner = register_and_login(&app, "vendor-owner@example.com").await;
    let member = register_and_login(&app, "vendor-member@example.com").await;

    let created = app
        .clone()
        .oneshot(request(
            "POST",
            "/api/v1/orgs",
            Some(&owner),
            Some(json!({"name": "Vendor HQ"})),
        ))
        .await
        .unwrap();
    let slug = json_body(created).await["organization"]["slug"]
        .as_str()
        .unwrap()
        .to_owned();

    // Plain member cannot manage vendors or alerts.
    let member_add = app
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/orgs/{slug}/vendors"),
            Some(&member),
            Some(json!({"category": "security"})),
        ))
        .await
        .unwrap();
    assert_eq!(member_add.status(), StatusCode::FORBIDDEN);

    // Owner can create + verify + remove; alerts resolve.
    let add = app
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/orgs/{slug}/vendors"),
            Some(&owner),
            Some(json!({"category": "security", "description": "pentesting"})),
        ))
        .await
        .unwrap();
    assert_eq!(add.status(), StatusCode::CREATED);
    let listing_id = json_body(add).await["id"].as_str().unwrap().to_owned();

    let verify = app
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/orgs/{slug}/vendors/{listing_id}/verify"),
            Some(&owner),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(verify.status(), StatusCode::NO_CONTENT);

    let remove = app
        .clone()
        .oneshot(request(
            "DELETE",
            &format!("/api/v1/orgs/{slug}/vendors/{listing_id}"),
            Some(&owner),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(remove.status(), StatusCode::NO_CONTENT);

    let alert = app
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/orgs/{slug}/alerts"),
            Some(&owner),
            Some(json!({"kind": "gdpr", "severity": "critical", "message": "breach"})),
        ))
        .await
        .unwrap();
    assert_eq!(alert.status(), StatusCode::CREATED);
    let alert_id = json_body(alert).await["id"].as_str().unwrap().to_owned();
    let resolve = app
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/orgs/{slug}/alerts/{alert_id}/resolve"),
            Some(&owner),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resolve.status(), StatusCode::NO_CONTENT);
}
