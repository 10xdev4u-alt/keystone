//! End-to-end OAuth authorization-code flow against a LOCAL mock provider —
//! the same wire protocol as Google, without network access. Verifies:
//! state cookie + constant-time check, code exchange, userinfo, find-or-create,
//! session issuance, and the no-token-in-URL rule.
//!
//! Runs against a real Postgres (CI service). Self-skips when
//! TEST_DATABASE_URL is unset.

use axum::body::Body;
use axum::extract::Query;
use axum::http::header::{self, HeaderValue};
use axum::http::{Request, StatusCode};
use axum::response::{IntoResponse, Redirect};
use http_body_util::BodyExt;
use keystone_api::auth::AuthServices;
use keystone_api::oauth::OAuthService;
use keystone_api::{router, AppState};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tower::ServiceExt;

const REFRESH_COOKIE: &str = "keystone_refresh";
const CSRF_COOKIE: &str = "keystone_csrf";
const OAUTH_STATE_COOKIE: &str = "keystone_oauth_state";

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

/// OAuth provider config pointing at the local mock.
fn mock_provider(base: &str) -> keystone_config::OAuthProviderConfig {
    keystone_config::OAuthProviderConfig {
        client_id: "test-client".into(),
        client_secret: "test-secret".into(),
        auth_url: format!("{base}/auth"),
        token_url: format!("{base}/token"),
        userinfo_url: format!("{base}/userinfo"),
        redirect_uri: "http://app.local/callback".into(),
        scopes: vec!["openid".into(), "email".into()],
    }
}

async fn test_app() -> Option<(axum::Router, PgPool)> {
    let pool = keystone_db::test_util::test_pool().await?;
    keystone_db::test_util::setup(&pool)
        .await
        .expect("db setup");
    let provider = {
        let base = spawn_mock_provider().await;
        mock_provider(&base)
    };
    let oauth = OAuthService::new(provider, "http://app.local/welcome".into())
        .expect("oauth service builds");
    let app = router(AppState {
        pool: pool.clone(),
        started_at: Instant::now(),
        auth: test_auth(),
        rate_limit: Arc::new(keystone_api::middleware::RateLimiter::new()),
        realtime: Arc::new(keystone_api::realtime::RealtimeHub::new()),
        oauth: Some(oauth),
    });
    Some((app, pool))
}

// ── Mock provider ───────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct MockAuthQuery {
    state: String,
    redirect_uri: String,
    #[allow(dead_code)]
    client_id: String,
    #[allow(dead_code)]
    response_type: String,
}

async fn mock_auth(Query(query): Query<MockAuthQuery>) -> impl IntoResponse {
    // Mimic the provider: bounce the browser to the callback with a code and
    // the SAME state it was given.
    Redirect::temporary(&format!(
        "{}?code=mock-code&state={}",
        query.redirect_uri, query.state
    ))
}

async fn mock_token() -> impl IntoResponse {
    axum::Json(json!({
        "access_token": "mock-access-token",
        "token_type": "Bearer",
        "expires_in": 3600,
    }))
}

async fn mock_userinfo() -> impl IntoResponse {
    axum::Json(json!({
        "sub": "mock-subject",
        "email": "oauth@example.com",
        "email_verified": true,
        "name": "OAuth User",
    }))
}

async fn spawn_mock_provider() -> String {
    let app = axum::Router::new()
        .route("/auth", axum::routing::get(mock_auth))
        .route("/token", axum::routing::post(mock_token))
        .route("/userinfo", axum::routing::get(mock_userinfo));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock provider");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("mock provider runs");
    });
    format!("http://{addr}")
}

// ── Helpers ─────────────────────────────────────────────────────────────────

async fn json_body(response: axum::response::Response) -> Value {
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body must read")
        .to_bytes();
    serde_json::from_slice(&bytes).expect("body must be JSON")
}

/// Extract one cookie value by name from a response's Set-Cookie headers.
fn cookie_value(response: &axum::response::Response, name: &str) -> Option<String> {
    for value in response.headers().get_all(header::SET_COOKIE) {
        let raw = value.to_str().ok()?;
        if let Some(rest) = raw.strip_prefix(&format!("{name}=")) {
            return rest.split(';').next().map(str::to_owned);
        }
    }
    None
}

fn cookies_header(refresh: &str, csrf: &str) -> HeaderValue {
    HeaderValue::from_str(&format!("{REFRESH_COOKIE}={refresh}; {CSRF_COOKIE}={csrf}"))
        .expect("cookie must be valid")
}

/// Extract + decode a `key=value` query param from a URL.
fn query_param(url_str: &str, key: &str) -> Option<String> {
    let (_, query) = url_str.split_once('?')?;
    url::form_urlencoded::parse(query.as_bytes())
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.into_owned())
}

#[tokio::test]
async fn oauth_google_flow_end_to_end() {
    let Some((app, pool)) = test_app().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };

    // 1. Start → redirect to the (mock) provider with state + client params.
    let start = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/auth/oauth/google/start")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(start.status(), StatusCode::SEE_OTHER);
    let location = start
        .headers()
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .expect("redirect location")
        .to_owned();
    assert!(
        location.starts_with("http://127.0.0.1:"),
        "must point at the mock"
    );
    let state = query_param(&location, "state").expect("state in auth URL");
    assert_eq!(
        query_param(&location, "client_id").as_deref(),
        Some("test-client")
    );
    assert_eq!(
        query_param(&location, "redirect_uri").as_deref(),
        Some("http://app.local/callback")
    );
    assert!(query_param(&location, "scope")
        .as_deref()
        .unwrap_or("")
        .contains("openid"));
    let state_cookie = cookie_value(&start, OAUTH_STATE_COOKIE).expect("state cookie set");
    assert_eq!(state_cookie, state, "cookie must match the URL state");

    // 2. Callback with the echoed state → session issued, browser redirected.
    let callback = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/v1/auth/oauth/google/callback?code=mock-code&state={state}"
                ))
                .header(
                    header::COOKIE,
                    HeaderValue::from_str(&format!("{OAUTH_STATE_COOKIE}={state}")).unwrap(),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(callback.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        callback
            .headers()
            .get(header::LOCATION)
            .and_then(|v| v.to_str().ok()),
        Some("http://app.local/welcome"),
        "no token may appear in the redirect URL"
    );
    let refresh1 = cookie_value(&callback, REFRESH_COOKIE).expect("refresh cookie");
    let csrf1 = cookie_value(&callback, CSRF_COOKIE).expect("csrf cookie");
    assert_ne!(refresh1, "", "refresh token must be non-empty");

    // The OAuth user exists, active and verified.
    let created: i64 = sqlx::query_scalar("SELECT count(*) FROM users WHERE email = $1")
        .bind("oauth@example.com")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(created, 1, "exactly one OAuth user");

    // 3. The SPA-style refresh: cookies + CSRF header → access token → /me.
    let refresh = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/refresh")
                .header(header::COOKIE, cookies_header(&refresh1, &csrf1))
                .header("x-csrf-token", HeaderValue::from_str(&csrf1).unwrap())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(refresh.status(), StatusCode::OK);
    let refresh_body = json_body(refresh).await;
    let access_token = refresh_body["access_token"].as_str().expect("access token");

    let me = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/auth/me")
                .header(header::AUTHORIZATION, format!("Bearer {access_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(me.status(), StatusCode::OK);
    let me_body = json_body(me).await;
    assert_eq!(me_body["user"]["email"], "oauth@example.com");
    assert_eq!(me_body["user"]["status"], "active");
    assert_eq!(me_body["user"]["is_verified"], true);

    // 4. State mismatch is rejected before any exchange.
    let tampered = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/auth/oauth/google/callback?code=mock-code&state=forged-state")
                .header(
                    header::COOKIE,
                    HeaderValue::from_str(&format!("{OAUTH_STATE_COOKIE}={state}")).unwrap(),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(tampered.status(), StatusCode::BAD_REQUEST);

    // 5. Signing in again finds the existing user (no duplicate).
    let callback2 = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/v1/auth/oauth/google/callback?code=mock-code&state={state}"
                ))
                .header(
                    header::COOKIE,
                    HeaderValue::from_str(&format!("{OAUTH_STATE_COOKIE}={state}")).unwrap(),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(callback2.status(), StatusCode::SEE_OTHER);
    let created_after: i64 = sqlx::query_scalar("SELECT count(*) FROM users WHERE email = $1")
        .bind("oauth@example.com")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(created_after, 1, "second login must not duplicate the user");
}
