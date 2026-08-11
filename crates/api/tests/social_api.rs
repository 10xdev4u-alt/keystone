//! Month-4 social API integration tests: community lifecycle + role rules,
//! community posts + pinning, poll voting invariants, and discussion
//! locking. Real Postgres; self-skips without TEST_DATABASE_URL.

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

async fn create_post(app: &axum::Router, token: &str, kind: &str) -> String {
    let response = app
        .clone()
        .oneshot(request(
            "POST",
            "/api/v1/posts",
            Some(token),
            Some(json!({"kind": kind, "body": "content", "visibility": "public"})),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED, "post {kind}");
    json_body(response).await["post"]["id"]
        .as_str()
        .unwrap()
        .to_owned()
}

// ── Communities ────────────────────────────────────────────────────────────

#[tokio::test]
async fn community_lifecycle_roles_and_join_leave() {
    let Some((app, pool)) = test_app().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let owner = register(&app, "sowner@example.com").await;
    let member = register(&app, "smember@example.com").await;
    let outsider = register(&app, "soutsider@example.com").await;

    // Anonymous create is rejected.
    let anon = app
        .clone()
        .oneshot(request(
            "POST",
            "/api/v1/communities",
            None,
            Some(json!({"name": "Nope", "visibility": "public"})),
        ))
        .await
        .unwrap();
    assert_eq!(anon.status(), StatusCode::UNAUTHORIZED);

    // Create — creator is owner; slug from name.
    let created = app
        .clone()
        .oneshot(request(
            "POST",
            "/api/v1/communities",
            Some(&owner),
            Some(json!({"name": "Rust Guild", "visibility": "public", "description": "All things Rust"})),
        ))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let body = json_body(created).await;
    assert_eq!(body["community"]["slug"], "rust-guild");

    // Duplicate slug conflicts.
    let dup = app
        .clone()
        .oneshot(request(
            "POST",
            "/api/v1/communities",
            Some(&owner),
            Some(json!({"name": "Rust Guild", "visibility": "public"})),
        ))
        .await
        .unwrap();
    assert_eq!(dup.status(), StatusCode::CONFLICT);

    // Public reads.
    let get = app
        .clone()
        .oneshot(request("GET", "/api/v1/communities/rust-guild", None, None))
        .await
        .unwrap();
    assert_eq!(get.status(), StatusCode::OK);
    let list = app
        .clone()
        .oneshot(request("GET", "/api/v1/communities", None, None))
        .await
        .unwrap();
    assert_eq!(
        json_body(list).await["communities"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    // Join + leave.
    let join = app
        .clone()
        .oneshot(request(
            "POST",
            "/api/v1/communities/rust-guild/join",
            Some(&member),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(join.status(), StatusCode::NO_CONTENT);
    let members = app
        .clone()
        .oneshot(request(
            "GET",
            "/api/v1/communities/rust-guild/members",
            None,
            None,
        ))
        .await
        .unwrap();
    let list = json_body(members).await;
    assert_eq!(list["members"].as_array().unwrap().len(), 2);
    let owner_row = list["members"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["role"] == "owner")
        .expect("owner present");
    assert_eq!(owner_row["role"], "owner");

    // Leave works for the member; the owner is refused.
    let leave = app
        .clone()
        .oneshot(request(
            "DELETE",
            "/api/v1/communities/rust-guild/leave",
            Some(&member),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(leave.status(), StatusCode::NO_CONTENT);
    let owner_leave = app
        .clone()
        .oneshot(request(
            "DELETE",
            "/api/v1/communities/rust-guild/leave",
            Some(&owner),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(owner_leave.status(), StatusCode::BAD_REQUEST);

    // Only the owner can change roles.
    let outsider_join = app
        .clone()
        .oneshot(request(
            "POST",
            "/api/v1/communities/rust-guild/join",
            Some(&outsider),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(outsider_join.status(), StatusCode::NO_CONTENT);
    let outsider_id = jwt_subject(&outsider);

    // A plain member changing roles is refused.
    let forbidden = app
        .clone()
        .oneshot(request(
            "PATCH",
            &format!("/api/v1/communities/rust-guild/members/{outsider_id}"),
            Some(&member),
            Some(json!({"role": "moderator"})),
        ))
        .await
        .unwrap();
    assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);

    // The owner promotes the outsider to moderator.
    let promote = app
        .clone()
        .oneshot(request(
            "PATCH",
            &format!("/api/v1/communities/rust-guild/members/{outsider_id}"),
            Some(&owner),
            Some(json!({"role": "moderator"})),
        ))
        .await
        .unwrap();
    assert_eq!(promote.status(), StatusCode::OK);

    // The owner transfers ownership to the outsider; old owner → admin.
    let transfer = app
        .clone()
        .oneshot(request(
            "PATCH",
            &format!("/api/v1/communities/rust-guild/members/{outsider_id}"),
            Some(&owner),
            Some(json!({"role": "owner"})),
        ))
        .await
        .unwrap();
    assert_eq!(transfer.status(), StatusCode::OK);
    let members_after = app
        .clone()
        .oneshot(request(
            "GET",
            "/api/v1/communities/rust-guild/members",
            None,
            None,
        ))
        .await
        .unwrap();
    let rows = json_body(members_after).await;
    let owners: Vec<_> = rows["members"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["role"] == "owner")
        .collect();
    assert_eq!(owners.len(), 1, "exactly one owner after transfer");
    assert_eq!(owners[0]["user_id"], outsider_id);
    let _ = pool;
}

/// The `sub` claim from a test JWT — identifies the user without a DB round
/// trip (payload is base64url-encoded JSON).
fn jwt_subject(token: &str) -> String {
    let payload = token.split('.').nth(1).expect("jwt has three parts");
    use base64::Engine;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .expect("payload must be base64url");
    let value: Value = serde_json::from_slice(&bytes).expect("payload must be JSON");
    value["sub"].as_str().expect("sub claim").to_owned()
}

#[tokio::test]
async fn community_posts_pin_and_permissions() {
    let Some((app, _pool)) = test_app().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let owner = register(&app, "powner@example.com").await;
    let member = register(&app, "pmember@example.com").await;
    let outsider = register(&app, "poutsider@example.com").await;

    app.clone()
        .oneshot(request(
            "POST",
            "/api/v1/communities",
            Some(&owner),
            Some(json!({"name": "Pin Guild", "visibility": "public"})),
        ))
        .await
        .unwrap();
    app.clone()
        .oneshot(request(
            "POST",
            "/api/v1/communities/pin-guild/join",
            Some(&member),
            None,
        ))
        .await
        .unwrap();

    let post = create_post(&app, &owner, "discussion").await;

    // Non-members cannot add posts.
    let forbidden = app
        .clone()
        .oneshot(request(
            "POST",
            "/api/v1/communities/pin-guild/posts",
            Some(&outsider),
            Some(json!({"post_id": post})),
        ))
        .await
        .unwrap();
    assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);

    // Member adds; owner pins; member cannot pin.
    let added = app
        .clone()
        .oneshot(request(
            "POST",
            "/api/v1/communities/pin-guild/posts",
            Some(&member),
            Some(json!({"post_id": post})),
        ))
        .await
        .unwrap();
    assert_eq!(added.status(), StatusCode::NO_CONTENT);
    let member_pin = app
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/communities/pin-guild/posts/{post}/pin"),
            Some(&member),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(member_pin.status(), StatusCode::FORBIDDEN);
    let pin = app
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/communities/pin-guild/posts/{post}/pin"),
            Some(&owner),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(pin.status(), StatusCode::NO_CONTENT);

    // Feed shows the pinned post.
    let feed = app
        .clone()
        .oneshot(request(
            "GET",
            "/api/v1/communities/pin-guild/posts",
            None,
            None,
        ))
        .await
        .unwrap();
    let feed_body = json_body(feed).await;
    assert_eq!(feed_body["posts"].as_array().unwrap().len(), 1);
    assert_eq!(feed_body["posts"][0]["pinned"], true);

    // Unpin and remove by the owner.
    app.clone()
        .oneshot(request(
            "DELETE",
            &format!("/api/v1/communities/pin-guild/posts/{post}/pin"),
            Some(&owner),
            None,
        ))
        .await
        .unwrap();
    let removed = app
        .clone()
        .oneshot(request(
            "DELETE",
            &format!("/api/v1/communities/pin-guild/posts/{post}"),
            Some(&owner),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(removed.status(), StatusCode::NO_CONTENT);
}

// ── Polls ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn poll_voting_via_api() {
    let Some((app, _pool)) = test_app().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let author = register(&app, "polster@example.com").await;
    let voter = register(&app, "poller@example.com").await;
    let stranger = register(&app, "pstranger@example.com").await;
    let poll = create_post(&app, &author, "poll").await;

    // Only the owner adds options.
    let stranger_option = app
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/posts/{poll}/poll/options"),
            Some(&stranger),
            Some(json!({"text": "hijack"})),
        ))
        .await
        .unwrap();
    assert_eq!(stranger_option.status(), StatusCode::FORBIDDEN);
    let rust = app
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/posts/{poll}/poll/options"),
            Some(&author),
            Some(json!({"text": "Rust"})),
        ))
        .await
        .unwrap();
    assert_eq!(rust.status(), StatusCode::CREATED);
    let rust_id = json_body(rust).await["option"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let other = app
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/posts/{poll}/poll/options"),
            Some(&author),
            Some(json!({"text": "Other"})),
        ))
        .await
        .unwrap();
    let other_id = json_body(other).await["option"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    // Vote, then switch — totals stay at 1.
    app.clone()
        .oneshot(request(
            "PUT",
            &format!("/api/v1/posts/{poll}/poll/votes"),
            Some(&voter),
            Some(json!({"option_id": rust_id})),
        ))
        .await
        .unwrap();
    app.clone()
        .oneshot(request(
            "PUT",
            &format!("/api/v1/posts/{poll}/poll/votes"),
            Some(&voter),
            Some(json!({"option_id": other_id})),
        ))
        .await
        .unwrap();
    let poll_view = app
        .clone()
        .oneshot(request(
            "GET",
            &format!("/api/v1/posts/{poll}/poll"),
            Some(&voter),
            None,
        ))
        .await
        .unwrap();
    let body = json_body(poll_view).await;
    assert_eq!(body["total_votes"], 1);
    assert_eq!(body["my_vote"], other_id);
    let rust_row = body["options"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["id"] == rust_id)
        .unwrap();
    assert_eq!(rust_row["votes"], 0);

    // Anonymous read has no my_vote.
    let anon = app
        .clone()
        .oneshot(request(
            "GET",
            &format!("/api/v1/posts/{poll}/poll"),
            None,
            None,
        ))
        .await
        .unwrap();
    assert!(json_body(anon).await["my_vote"].is_null());

    // Remove vote.
    app.clone()
        .oneshot(request(
            "DELETE",
            &format!("/api/v1/posts/{poll}/poll/votes"),
            Some(&voter),
            None,
        ))
        .await
        .unwrap();
    let poll_view = app
        .clone()
        .oneshot(request(
            "GET",
            &format!("/api/v1/posts/{poll}/poll"),
            Some(&voter),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(json_body(poll_view).await["total_votes"], 0);
}

// ── Locking ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn discussion_lock_blocks_comments_until_unlocked() {
    let Some((app, _pool)) = test_app().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let owner = register(&app, "locker@example.com").await;
    let other = register(&app, "lother@example.com").await;
    let discussion = create_post(&app, &owner, "discussion").await;

    // Only owner/staff lock.
    let other_lock = app
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/posts/{discussion}/lock"),
            Some(&other),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(other_lock.status(), StatusCode::FORBIDDEN);
    let lock = app
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/posts/{discussion}/lock"),
            Some(&owner),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(lock.status(), StatusCode::NO_CONTENT);

    // Comments are refused with 423 while locked.
    let blocked = app
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/posts/{discussion}/comments"),
            Some(&other),
            Some(json!({"body": "still open?"})),
        ))
        .await
        .unwrap();
    assert_eq!(blocked.status(), StatusCode::LOCKED);

    // Unlock restores commenting.
    app.clone()
        .oneshot(request(
            "DELETE",
            &format!("/api/v1/posts/{discussion}/lock"),
            Some(&owner),
            None,
        ))
        .await
        .unwrap();
    let comment = app
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/posts/{discussion}/comments"),
            Some(&other),
            Some(json!({"body": "open again"})),
        ))
        .await
        .unwrap();
    assert_eq!(comment.status(), StatusCode::CREATED);
}
