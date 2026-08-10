//! Content API integration tests against a real Postgres: posts lifecycle
//! with the Month-3 authorization matrix (owner / moderator / admin),
//! comments nesting, reactions, bookmarks, visibility rules, reports +
//! moderation queue, and reviews.
//!
//! Self-skips when TEST_DATABASE_URL is unset.

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

type App = (axum::Router, sqlx::PgPool);

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

/// Full register → verify → login, returning the access token.
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
            Some(json!({ "email": email, "password": "correct-horse-battery-staple" })),
        ))
        .await
        .expect("login must not panic");
    assert_eq!(login.status(), StatusCode::OK, "login {email}");
    json_body(login).await["access_token"]
        .as_str()
        .expect("access token")
        .to_owned()
}

/// Create a user directly (any role) and mint an access token for them.
async fn make_user_with_role(pool: &sqlx::PgPool, email: &str, role: &str) -> String {
    let users = keystone_db::repositories::users::Users::new(pool.clone());
    let user = users
        .create(keystone_db::repositories::users::NewUser {
            email,
            password_hash: "not-a-real-hash",
            first_name: None,
            last_name: None,
            username: Some(email.split('@').next().unwrap()),
        })
        .await
        .expect("user must be created");
    sqlx::query("UPDATE users SET role = $1, status = 'active', is_verified = true WHERE id = $2")
        .bind(role)
        .bind(user.id)
        .execute(pool)
        .await
        .expect("role update must work");
    test_auth()
        .jwt
        .issue(&user.id.to_string(), role, None)
        .expect("token must mint")
}

// ── Posts lifecycle + authorization matrix ─────────────────────────────────

#[tokio::test]
async fn posts_lifecycle_and_authorization_matrix() {
    let Some((app, pool)) = test_app().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let alice = register_and_login(&app, "alice@example.com").await;
    let bob = register_and_login(&app, "bob@example.com").await;

    // Unauthenticated create is rejected.
    let anon = app
        .clone()
        .oneshot(request(
            "POST",
            "/api/v1/posts",
            None,
            Some(json!({"kind": "article", "title": "Nope", "body": "x"})),
        ))
        .await
        .unwrap();
    assert_eq!(anon.status(), StatusCode::UNAUTHORIZED);

    // Alice creates a post; the slug is generated from the title.
    let created = app
        .clone()
        .oneshot(request(
            "POST",
            "/api/v1/posts",
            Some(&alice),
            Some(json!({
                "kind": "article",
                "title": "Hello, Rust!",
                "body": "First post.",
                "tags": ["rust", "axum"],
            })),
        ))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let created_body = json_body(created).await;
    let post_id = created_body["post"]["id"].as_str().unwrap().to_owned();
    let slug = created_body["post"]["slug"].as_str().unwrap().to_owned();
    assert_eq!(slug, "hello-rust");
    assert_eq!(created_body["post"]["status"], "published");

    // Public read by slug.
    let read = app
        .clone()
        .oneshot(request("GET", &format!("/api/v1/posts/{slug}"), None, None))
        .await
        .unwrap();
    assert_eq!(read.status(), StatusCode::OK);
    assert_eq!(
        json_body(read).await["post"]["id"],
        created_body["post"]["id"]
    );

    // List includes it with derived counters.
    let list = app
        .clone()
        .oneshot(request("GET", "/api/v1/posts", None, None))
        .await
        .unwrap();
    assert_eq!(list.status(), StatusCode::OK);
    let list_body = json_body(list).await;
    assert_eq!(list_body["posts"].as_array().unwrap().len(), 1);
    assert_eq!(list_body["posts"][0]["comment_count"], 0);

    // Slug collision is auto-suffixed (-2, -3…).
    let dup = app
        .clone()
        .oneshot(request(
            "POST",
            "/api/v1/posts",
            Some(&alice),
            Some(json!({"kind": "article", "title": "Hello, Rust!", "body": "Second."})),
        ))
        .await
        .unwrap();
    assert_eq!(dup.status(), StatusCode::CREATED);
    let dup_slug = json_body(dup).await["post"]["slug"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(dup_slug, "hello-rust-1");

    // Alice edits her post.
    let patch = app
        .clone()
        .oneshot(request(
            "PATCH",
            &format!("/api/v1/posts/{post_id}"),
            Some(&alice),
            Some(json!({"body": "Updated body.", "change_note": "typo"})),
        ))
        .await
        .unwrap();
    assert_eq!(patch.status(), StatusCode::OK);
    assert_eq!(json_body(patch).await["post"]["body"], "Updated body.");

    // Version history shows both snapshots — owner only.
    let versions = app
        .clone()
        .oneshot(request(
            "GET",
            &format!("/api/v1/posts/{post_id}/versions"),
            Some(&alice),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(versions.status(), StatusCode::OK);
    let versions_body = json_body(versions).await;
    assert_eq!(versions_body["versions"].as_array().unwrap().len(), 2);

    // Bob cannot edit or read the history.
    let foreign_patch = app
        .clone()
        .oneshot(request(
            "PATCH",
            &format!("/api/v1/posts/{post_id}"),
            Some(&bob),
            Some(json!({"body": "hijack"})),
        ))
        .await
        .unwrap();
    assert_eq!(foreign_patch.status(), StatusCode::FORBIDDEN);
    let foreign_versions = app
        .clone()
        .oneshot(request(
            "GET",
            &format!("/api/v1/posts/{post_id}/versions"),
            Some(&bob),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(foreign_versions.status(), StatusCode::FORBIDDEN);

    // Bob (plain user) cannot delete; a moderator can.
    let bob_delete = app
        .clone()
        .oneshot(request(
            "DELETE",
            &format!("/api/v1/posts/{post_id}"),
            Some(&bob),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(bob_delete.status(), StatusCode::FORBIDDEN);

    let mod_token = make_user_with_role(&pool, "mod@example.com", "moderator").await;
    let mod_delete = app
        .clone()
        .oneshot(request(
            "DELETE",
            &format!("/api/v1/posts/{post_id}"),
            Some(&mod_token),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(mod_delete.status(), StatusCode::NO_CONTENT);

    // Soft-deleted: gone from reads and listings, history survives.
    let gone = app
        .clone()
        .oneshot(request("GET", &format!("/api/v1/posts/{slug}"), None, None))
        .await
        .unwrap();
    assert_eq!(gone.status(), StatusCode::NOT_FOUND);
    let versions_after = app
        .clone()
        .oneshot(request(
            "GET",
            &format!("/api/v1/posts/{post_id}/versions"),
            Some(&alice),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(
        json_body(versions_after).await["versions"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

// ── Comments, reactions, bookmarks, views ──────────────────────────────────

#[tokio::test]
async fn comments_reactions_bookmarks_and_views() {
    let Some((app, _pool)) = test_app().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let alice = register_and_login(&app, "alice2@example.com").await;
    let bob = register_and_login(&app, "bob2@example.com").await;

    let created = app
        .clone()
        .oneshot(request(
            "POST",
            "/api/v1/posts",
            Some(&alice),
            Some(json!({"kind": "post", "body": "Discuss.", "visibility": "public"})),
        ))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let post_id = json_body(created).await["post"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    // Bob comments; alice replies to it.
    let comment = app
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/posts/{post_id}/comments"),
            Some(&bob),
            Some(json!({"body": "First!"})),
        ))
        .await
        .unwrap();
    assert_eq!(comment.status(), StatusCode::CREATED);
    let parent_id = json_body(comment).await["comment"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let reply = app
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/posts/{post_id}/comments"),
            Some(&alice),
            Some(json!({"body": "Reply!", "parent_id": parent_id})),
        ))
        .await
        .unwrap();
    assert_eq!(reply.status(), StatusCode::CREATED);

    // Bob cannot delete alice's reply; bob deletes his own comment.
    let foreign = app
        .clone()
        .oneshot(request(
            "DELETE",
            &format!(
                "/api/v1/comments/{}",
                json_body(reply).await["comment"]["id"].as_str().unwrap()
            ),
            Some(&bob),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(foreign.status(), StatusCode::FORBIDDEN);
    let own = app
        .clone()
        .oneshot(request(
            "DELETE",
            &format!("/api/v1/comments/{parent_id}"),
            Some(&bob),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(own.status(), StatusCode::NO_CONTENT);

    // Comment list shows only the live reply.
    let list = app
        .clone()
        .oneshot(request(
            "GET",
            &format!("/api/v1/posts/{post_id}/comments"),
            None,
            None,
        ))
        .await
        .unwrap();
    let list_body = json_body(list).await;
    let comments = list_body["comments"].as_array().unwrap();
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0]["body"], "Reply!");

    // Reactions: set → change kind → counts reflect one, mine follows token.
    app.clone()
        .oneshot(request(
            "PUT",
            &format!("/api/v1/posts/{post_id}/reaction"),
            Some(&bob),
            Some(json!({"kind": "like"})),
        ))
        .await
        .unwrap();
    app.clone()
        .oneshot(request(
            "PUT",
            &format!("/api/v1/posts/{post_id}/reaction"),
            Some(&bob),
            Some(json!({"kind": "love"})),
        ))
        .await
        .unwrap();
    let reactions_anon = app
        .clone()
        .oneshot(request(
            "GET",
            &format!("/api/v1/posts/{post_id}/reactions"),
            None,
            None,
        ))
        .await
        .unwrap();
    let anon_body = json_body(reactions_anon).await;
    assert_eq!(anon_body["total"], 1);
    assert!(anon_body["mine"].is_null());
    let reactions_me = app
        .clone()
        .oneshot(request(
            "GET",
            &format!("/api/v1/posts/{post_id}/reactions"),
            Some(&bob),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(json_body(reactions_me).await["mine"], "love");
    app.clone()
        .oneshot(request(
            "DELETE",
            &format!("/api/v1/posts/{post_id}/reaction"),
            Some(&bob),
            None,
        ))
        .await
        .unwrap();

    // Bookmarks + my list.
    app.clone()
        .oneshot(request(
            "PUT",
            &format!("/api/v1/posts/{post_id}/bookmark"),
            Some(&bob),
            None,
        ))
        .await
        .unwrap();
    let mine = app
        .clone()
        .oneshot(request("GET", "/api/v1/me/bookmarks", Some(&bob), None))
        .await
        .unwrap();
    let mine_body = json_body(mine).await;
    let bookmarks = mine_body["post_ids"].as_array().unwrap();
    assert!(bookmarks.iter().any(|b| b == &post_id));
    app.clone()
        .oneshot(request(
            "DELETE",
            &format!("/api/v1/posts/{post_id}/bookmark"),
            Some(&bob),
            None,
        ))
        .await
        .unwrap();

    // View counter is transactional and cumulative.
    app.clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/posts/{post_id}/view"),
            None,
            None,
        ))
        .await
        .unwrap();
    app.clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/posts/{post_id}/view"),
            None,
            None,
        ))
        .await
        .unwrap();
    let post = app
        .clone()
        .oneshot(request(
            "GET",
            &format!("/api/v1/posts/{post_id}"),
            None,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(json_body(post).await["post"]["view_count"], 2);
}

// ── Visibility ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn private_and_unlisted_posts_respect_visibility() {
    let Some((app, pool)) = test_app().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let alice = register_and_login(&app, "alice3@example.com").await;
    let bob = register_and_login(&app, "bob3@example.com").await;
    let mod_token = make_user_with_role(&pool, "mod3@example.com", "moderator").await;

    let private_slug;
    {
        let created = app
            .clone()
            .oneshot(request(
                "POST",
                "/api/v1/posts",
                Some(&alice),
                Some(json!({"kind": "post", "title": "Secret", "body": "sssh", "visibility": "private"})),
            ))
            .await
            .unwrap();
        private_slug = json_body(created).await["post"]["slug"]
            .as_str()
            .unwrap()
            .to_owned();
    }

    // Anonymous and other users cannot see it — existence is hidden (404).
    let anon = app
        .clone()
        .oneshot(request(
            "GET",
            &format!("/api/v1/posts/{private_slug}"),
            None,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(anon.status(), StatusCode::NOT_FOUND);
    let bob_read = app
        .clone()
        .oneshot(request(
            "GET",
            &format!("/api/v1/posts/{private_slug}"),
            Some(&bob),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(bob_read.status(), StatusCode::NOT_FOUND);

    // Owner and staff can.
    let owner = app
        .clone()
        .oneshot(request(
            "GET",
            &format!("/api/v1/posts/{private_slug}"),
            Some(&alice),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(owner.status(), StatusCode::OK);
    let staff = app
        .clone()
        .oneshot(request(
            "GET",
            &format!("/api/v1/posts/{private_slug}"),
            Some(&mod_token),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(staff.status(), StatusCode::OK);
}

// ── Reports, moderation queue, reviews ─────────────────────────────────────

#[tokio::test]
async fn reports_moderation_queue_and_reviews() {
    let Some((app, pool)) = test_app().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let alice = register_and_login(&app, "alice4@example.com").await;
    let mod_token = make_user_with_role(&pool, "mod4@example.com", "moderator").await;
    let target = "11111111-1111-4111-8111-111111111111";

    // A user files a report; the queue is staff-only.
    let filed = app
        .clone()
        .oneshot(request(
            "POST",
            "/api/v1/reports",
            Some(&alice),
            Some(json!({
                "entity_type": "post",
                "entity_id": target,
                "reason": "spam",
                "detail": "Repeated ads",
            })),
        ))
        .await
        .unwrap();
    assert_eq!(filed.status(), StatusCode::CREATED);

    let blocked = app
        .clone()
        .oneshot(request(
            "GET",
            "/api/v1/moderation/reports",
            Some(&alice),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(blocked.status(), StatusCode::FORBIDDEN);

    let queue = app
        .clone()
        .oneshot(request(
            "GET",
            "/api/v1/moderation/reports",
            Some(&mod_token),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(queue.status(), StatusCode::OK);
    let queue_body = json_body(queue).await;
    assert_eq!(queue_body["reports"].as_array().unwrap().len(), 1);
    let report_id = queue_body["reports"][0]["id"].as_str().unwrap().to_owned();

    // Resolving records the decision and empties the queue.
    let resolved = app
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/v1/moderation/reports/{report_id}/resolve"),
            Some(&mod_token),
            Some(json!({"resolution_note": "hidden"})),
        ))
        .await
        .unwrap();
    assert_eq!(resolved.status(), StatusCode::OK);
    assert_eq!(json_body(resolved).await["report"]["status"], "resolved");
    let queue_after = app
        .clone()
        .oneshot(request(
            "GET",
            "/api/v1/moderation/reports",
            Some(&mod_token),
            None,
        ))
        .await
        .unwrap();
    assert!(json_body(queue_after).await["reports"]
        .as_array()
        .unwrap()
        .is_empty());

    // Reviews: upsert (same row on re-review), list by entity, auth required.
    let entity = "22222222-2222-4222-8222-222222222222";
    let first = app
        .clone()
        .oneshot(request(
            "PUT",
            "/api/v1/reviews",
            Some(&alice),
            Some(json!({
                "entity_type": "employer",
                "entity_id": entity,
                "rating": 4,
                "title": "Good",
                "body": "Would recommend.",
            })),
        ))
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::CREATED);
    let first_id = json_body(first).await["review"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let second = app
        .clone()
        .oneshot(request(
            "PUT",
            "/api/v1/reviews",
            Some(&alice),
            Some(json!({
                "entity_type": "employer",
                "entity_id": entity,
                "rating": 5,
                "body": null,
            })),
        ))
        .await
        .unwrap();
    assert_eq!(second.status(), StatusCode::CREATED);
    assert_eq!(
        json_body(second).await["review"]["id"],
        first_id,
        "same row upserted"
    );

    let reviews = app
        .clone()
        .oneshot(request(
            "GET",
            &format!("/api/v1/reviews?entity_type=employer&entity_id={entity}"),
            None,
            None,
        ))
        .await
        .unwrap();
    let reviews_body = json_body(reviews).await;
    assert_eq!(reviews_body["reviews"].as_array().unwrap().len(), 1);
    assert_eq!(reviews_body["reviews"][0]["rating"], 5);

    // Anonymous upsert is rejected; bad rating is rejected.
    let anon_review = app
        .clone()
        .oneshot(request(
            "PUT",
            "/api/v1/reviews",
            None,
            Some(json!({
                "entity_type": "employer",
                "entity_id": entity,
                "rating": 3,
            })),
        ))
        .await
        .unwrap();
    assert_eq!(anon_review.status(), StatusCode::UNAUTHORIZED);
    let bad_rating = app
        .clone()
        .oneshot(request(
            "PUT",
            "/api/v1/reviews",
            Some(&alice),
            Some(json!({
                "entity_type": "employer",
                "entity_id": entity,
                "rating": 9,
            })),
        ))
        .await
        .unwrap();
    assert_eq!(bad_rating.status(), StatusCode::BAD_REQUEST);
}
