//! Month-8 uploads API tests against a real Postgres + in-memory storage:
//!   - full presign → PUT → register → GET → delete round-trip
//!   - quota rejection surfaces as 400 through the API
//!   - forged keys (another user's prefix) are rejected
//!   - non-owners get 404 on GET/DELETE (existence never confirmed)
//!   - content-type allowlist and auth enforcement
//!
//! Self-skips when TEST_DATABASE_URL is unset or unreachable.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use keystone_api::{router, AppState};
use keystone_auth::jwt::AccessTokenService;
use keystone_db::storage::StorageBackend;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Instant;
use tower::util::ServiceExt;

async fn test_app() -> Option<(
    axum::Router,
    Arc<keystone_db::storage::MemoryStorage>,
    sqlx::PgPool,
)> {
    let pool = keystone_db::test_util::test_pool_isolated().await?;
    keystone_db::test_util::setup(&pool)
        .await
        .expect("db setup");
    let storage = Arc::new(keystone_db::storage::MemoryStorage::new());
    let app = router(AppState {
        pool: pool.clone(),
        started_at: Instant::now(),
        auth: test_auth(),
        rate_limit: Arc::new(keystone_api::middleware::RateLimiter::new()),
        realtime: Arc::new(keystone_api::realtime::RealtimeHub::new()),
        storage: storage.clone(),
        oauth: None,
    });
    Some((app, storage, pool))
}

fn test_auth() -> keystone_api::auth::AuthServices {
    keystone_api::auth::AuthServices {
        password: Arc::new(
            keystone_auth::password::PasswordHasher::from_config(&keystone_config::Argon2Config {
                memory_kib: 19_456,
                iterations: 2,
                parallelism: 1,
            })
            .expect("valid params"),
        ),
        jwt: Arc::new(AccessTokenService::new(
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
            std::time::Duration::from_secs(300),
            std::time::Duration::from_secs(60),
        ),
        access_ttl: std::time::Duration::from_secs(900),
        refresh_ttl: std::time::Duration::from_secs(604_800),
        secure_cookies: false,
    }
}

async fn json_body(response: axum::response::Response) -> Value {
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body must read")
        .to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

fn request(method: &str, uri: &str, token: Option<&str>, body: Option<Value>) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(token) = token {
        builder = builder.header("Authorization", format!("Bearer {token}"));
    }
    let body = body.map(|v| v.to_string()).unwrap_or_default();
    builder
        .header("content-type", "application/json")
        .body(Body::from(body))
        .unwrap()
}

async fn register_user(app: &Router, email: &str) -> (String, String) {
    // Register → (dev) verify-email → login. Mirrors auth_flow.rs.
    let res = app
        .clone()
        .oneshot(request(
            "POST",
            "/api/v1/auth/register",
            None,
            Some(json!({
                "email": email,
                "password": "Hunter2!Hunter2!",
                "first_name": "Test",
                "last_name": "User",
                "username": email.split('@').next().unwrap()
            })),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::CREATED, "register must succeed");
    let body = json_body(res).await;
    let verification_token = body["verification_token"]
        .as_str()
        .expect("dev verification token present");

    let verify = app
        .clone()
        .oneshot(request(
            "POST",
            "/api/v1/auth/verify-email",
            None,
            Some(json!({ "token": verification_token })),
        ))
        .await
        .unwrap();
    assert_eq!(verify.status(), StatusCode::OK, "email verification");

    let login = app
        .clone()
        .oneshot(request(
            "POST",
            "/api/v1/auth/login",
            None,
            Some(json!({
                "email": email,
                "password": "Hunter2!Hunter2!"
            })),
        ))
        .await
        .unwrap();
    assert_eq!(login.status(), StatusCode::OK, "login after verification");
    let body = json_body(login).await;
    let token = body["access_token"].as_str().unwrap().to_string();
    let id = body["user"]["id"].as_str().unwrap().to_string();
    (token, id)
}

// ── Round trip ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn presign_upload_register_download_delete_round_trip() {
    let Some((app, storage, _pool)) = test_app().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let (alice, _id) = register_user(&app, "files-a@test.dev").await;

    // 1. Presign.
    let res = app
        .clone()
        .oneshot(request(
            "POST",
            "/api/v1/files/presign",
            Some(&alice),
            Some(json!({
                "original_name": "hello world.png",
                "content_type": "image/png",
                "size_bytes": 68
            })),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = json_body(res).await;
    let key = body["key"].as_str().unwrap().to_string();
    assert!(body["put_url"].as_str().unwrap().starts_with("memory://"));
    assert!(
        key.starts_with("users/"),
        "key is server-minted under the user"
    );

    // 2. Client PUTs bytes straight to storage.
    let png: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90,
        0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x08, 0xD7, 0x63, 0xF8,
        0xFF, 0xFF, 0x3F, 0x00, 0x05, 0xFE, 0x02, 0xFE, 0xDC, 0xCC, 0x59, 0xE7, 0x00, 0x00, 0x00,
        0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];
    storage.put_bytes(&key, png, "image/png").await.unwrap();

    // 3. Register metadata (quota enforcement point + thumbnail generation).
    let res = app
        .clone()
        .oneshot(request(
            "POST",
            "/api/v1/files",
            Some(&alice),
            Some(json!({
                "key": key,
                "original_name": "hello world.png",
                "content_type": "image/png",
                "size_bytes": png.len(),
                "sha256": "0123456789abcdef0123456789abcdef",
                "width": null,
                "height": null
            })),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = json_body(res).await;
    let file = &body["file"];
    let id = file["id"].as_str().unwrap().to_string();
    // Thumbnail is generated server-side; dimensions are capped at 512.
    let w = file["width"].as_i64().expect("width present");
    let h = file["height"].as_i64().expect("height present");
    assert!((1..=512).contains(&w) && (1..=512).contains(&h));
    assert!(file["get_url"].as_str().unwrap().starts_with("memory://"));

    // 4. GET with a fresh presigned download url.
    let res = app
        .clone()
        .oneshot(request(
            "GET",
            &format!("/api/v1/files/{id}"),
            Some(&alice),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = json_body(res).await;
    assert_eq!(body["file"]["original_name"], "hello world.png");

    // 5. List shows the file; delete removes it.
    let res = app
        .clone()
        .oneshot(request("GET", "/api/v1/files", Some(&alice), None))
        .await
        .unwrap();
    let body = json_body(res).await;
    assert_eq!(body["items"].as_array().unwrap().len(), 1);

    let res = app
        .clone()
        .oneshot(request(
            "DELETE",
            &format!("/api/v1/files/{id}"),
            Some(&alice),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let res = app
        .clone()
        .oneshot(request(
            "GET",
            &format!("/api/v1/files/{id}"),
            Some(&alice),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

// ── Quota through the API ───────────────────────────────────────────────────

#[tokio::test]
async fn quota_overshoot_is_rejected_with_400() {
    let Some((app, storage, pool)) = test_app().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let (alice, id) = register_user(&app, "files-quota@test.dev").await;

    let files = keystone_db::repositories::files::Files::new(pool);
    files
        .set_quota(uuid::Uuid::parse_str(&id).unwrap(), 128)
        .await
        .unwrap();

    let res = app
        .clone()
        .oneshot(request(
            "POST",
            "/api/v1/files/presign",
            Some(&alice),
            Some(json!({
                "original_name": "a.txt",
                "content_type": "text/plain",
                "size_bytes": 100
            })),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let key = json_body(res).await["key"].as_str().unwrap().to_string();
    storage
        .put_bytes(&key, &b"x".repeat(100), "text/plain")
        .await
        .unwrap();

    let res = app
        .clone()
        .oneshot(request(
            "POST",
            "/api/v1/files",
            Some(&alice),
            Some(json!({
                "key": key,
                "original_name": "a.txt",
                "content_type": "text/plain",
                "size_bytes": 100,
                "sha256": "0123456789abcdef0123456789abcdef",
                "width": null,
                "height": null
            })),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // Second registration of the same size would exceed 128.
    let res = app
        .clone()
        .oneshot(request(
            "POST",
            "/api/v1/files/presign",
            Some(&alice),
            Some(json!({
                "original_name": "b.txt",
                "content_type": "text/plain",
                "size_bytes": 100
            })),
        ))
        .await
        .unwrap();
    let key2 = json_body(res).await["key"].as_str().unwrap().to_string();
    storage
        .put_bytes(&key2, &b"y".repeat(100), "text/plain")
        .await
        .unwrap();
    let res = app
        .clone()
        .oneshot(request(
            "POST",
            "/api/v1/files",
            Some(&alice),
            Some(json!({
                "key": key2,
                "original_name": "b.txt",
                "content_type": "text/plain",
                "size_bytes": 100,
                "sha256": "0123456789abcdef0123456789abcdef",
                "width": null,
                "height": null
            })),
        ))
        .await
        .unwrap();
    assert_eq!(
        res.status(),
        StatusCode::BAD_REQUEST,
        "quota overshoot must be a 400"
    );
}

// ── Security: forged keys, ownership, auth, allowlist ───────────────────────

#[tokio::test]
async fn forged_key_and_nonowner_access_are_rejected() {
    let Some((app, _storage, _pool)) = test_app().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let (alice, _a) = register_user(&app, "files-sec-a@test.dev").await;
    let (bob, bob_id) = register_user(&app, "files-sec-b@test.dev").await;

    // Alice tries to register a key minted for Bob → 403.
    let res = app
        .clone()
        .oneshot(request(
            "POST",
            "/api/v1/files",
            Some(&alice),
            Some(json!({
                "key": format!("users/{bob_id}/forged.txt"),
                "original_name": "forged.txt",
                "content_type": "text/plain",
                "size_bytes": 10,
                "sha256": "0123456789abcdef0123456789abcdef",
                "width": null,
                "height": null
            })),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN, "forged key rejected");

    // Alice's real upload is invisible to Bob (404, existence never confirmed).
    let res = app
        .clone()
        .oneshot(request(
            "POST",
            "/api/v1/files/presign",
            Some(&alice),
            Some(json!({
                "original_name": "secret.txt",
                "content_type": "text/plain",
                "size_bytes": 10
            })),
        ))
        .await
        .unwrap();
    let key = json_body(res).await["key"].as_str().unwrap().to_string();
    let res = app
        .clone()
        .oneshot(request(
            "POST",
            "/api/v1/files",
            Some(&alice),
            Some(json!({
                "key": key,
                "original_name": "secret.txt",
                "content_type": "text/plain",
                "size_bytes": 10,
                "sha256": "0123456789abcdef0123456789abcdef",
                "width": null,
                "height": null
            })),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let id = json_body(res).await["file"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    let res = app
        .clone()
        .oneshot(request(
            "GET",
            &format!("/api/v1/files/{id}"),
            Some(&bob),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND, "non-owner read is 404");

    let res = app
        .clone()
        .oneshot(request(
            "DELETE",
            &format!("/api/v1/files/{id}"),
            Some(&bob),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(
        res.status(),
        StatusCode::NOT_FOUND,
        "non-owner delete is 404"
    );

    // No token → 401.
    let res = app
        .clone()
        .oneshot(request("GET", "/api/v1/files", None, None))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    // Disallowed content type → 400.
    let res = app
        .clone()
        .oneshot(request(
            "POST",
            "/api/v1/files/presign",
            Some(&alice),
            Some(json!({
                "original_name": "evil.html",
                "content_type": "text/html",
                "size_bytes": 10
            })),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}
