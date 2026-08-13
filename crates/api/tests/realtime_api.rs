//! Month-7 API tests against a real Postgres:
//!   - notification center: follow trigger → feed + unread counts, mark-read,
//!     preferences
//!   - SSE feed with Last-Event-ID gap recovery (real HTTP, real stream)
//!   - chat REST + WebSocket end-to-end (membership, presence, fan-out)
//!   - unauthorized WS join rejected BEFORE the upgrade (403)
//!   - message rate cap (sliding window)
//!   - hub fan-out scale (N subscribers × M notifications, all delivered)
//!
//! Self-skips when TEST_DATABASE_URL is unset.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::Router;
use futures::{SinkExt, StreamExt};
use http_body_util::BodyExt;
use keystone_api::auth::AuthServices;
use keystone_api::{router, AppState};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tower::ServiceExt;
use uuid::Uuid;

type App = (Router, PgPool);

fn test_auth() -> keystone_api::auth::AuthServices {
    use keystone_auth::jwt::{AccessTokenService, JwtKeys};
    use keystone_auth::password::PasswordHasher;
    use keystone_auth::service::LockoutPolicy;
    AuthServices {
        password: Arc::new(
            PasswordHasher::from_config(&keystone_config::Argon2Config {
                memory_kib: 19_456,
                iterations: 2,
                parallelism: 1,
            })
            .expect("params valid"),
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
            JwtKeys::from_secret(b"01234567890123456789012345678901").expect("key valid"),
        )),
        lockout: LockoutPolicy::new(5, Duration::from_secs(300), Duration::from_secs(60)),
        access_ttl: Duration::from_secs(900),
        refresh_ttl: Duration::from_secs(604_800),
        secure_cookies: false,
    }
}

static CLIENT: AtomicU64 = AtomicU64::new(1);

async fn test_app() -> Option<App> {
    let pool = keystone_db::test_util::test_pool_isolated().await?;
    let app = router(AppState {
        pool: pool.clone(),
        started_at: Instant::now(),
        auth: test_auth(),
        rate_limit: Arc::new(keystone_api::middleware::RateLimiter::new()),
        realtime: Arc::new(keystone_api::realtime::RealtimeHub::new()),
        storage: std::sync::Arc::new(keystone_db::storage::MemoryStorage::new()),
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
    // Distinct client IPs so the auth rate tier never trips in this suite.
    let n = CLIENT.fetch_add(1, Ordering::Relaxed);
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

async fn register_and_login(app: &Router, email: &str) -> String {
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
    let body = json_body(login).await;
    body["access_token"]
        .as_str()
        .expect("access token")
        .to_owned()
}

/// Register + return (token, user_id) — user id from /auth/me.
async fn register_user(app: &Router, email: &str) -> (String, Uuid) {
    let token = register_and_login(app, email).await;
    let me = app
        .clone()
        .oneshot(request("GET", "/api/v1/auth/me", Some(&token), None))
        .await
        .expect("me must not panic");
    let body = json_body(me).await;
    let id: Uuid = body["user"]["id"].as_str().unwrap().parse().unwrap();
    (token, id)
}

// ── Hub fan-out scale ───────────────────────────────────────────────────────

#[tokio::test]
async fn hub_fan_out_scale_all_subscribers_receive_all() {
    let hub = keystone_api::realtime::RealtimeHub::new();
    let subscribers = 20;
    let notifications = 100;
    let feed_user = Uuid::from_u128(7);

    // One feed user, N subscribers on that user's channel.
    let mut receivers = Vec::new();
    for _ in 0..subscribers {
        receivers.push(hub.subscribe_feed(feed_user));
    }
    let mut receivers = Vec::new();
    for _ in 0..subscribers {
        receivers.push(hub.subscribe_feed(feed_user));
    }
    for id in 1..=notifications {
        hub.publish_feed(
            feed_user,
            keystone_api::realtime::FeedEvent {
                id,
                kind: "test".into(),
                payload: json!({ "n": id }),
            },
        );
    }
    let mut handles = Vec::new();
    for mut rx in receivers {
        handles.push(tokio::spawn(async move {
            let mut seen = Vec::new();
            for _ in 0..notifications {
                match tokio::time::timeout(Duration::from_secs(5), rx.recv()).await {
                    Ok(Ok(event)) => seen.push(event.id),
                    other => panic!("subscriber missed an event: {other:?}"),
                }
            }
            seen
        }));
    }
    for h in handles {
        let seen = h.await.unwrap();
        assert_eq!(
            seen.len(),
            notifications as usize,
            "fan-out must not lose events"
        );
        // Strictly increasing — order preserved per subscriber.
        for w in seen.windows(2) {
            assert!(w[0] < w[1], "per-subscriber order must hold");
        }
    }
}

// ── Notification center via API ─────────────────────────────────────────────

#[tokio::test]
async fn follow_triggers_notification_and_mark_read_flow() {
    let Some((app, _pool)) = test_app().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let (alice, _alice_id) = register_user(&app, "n-alice@test.dev").await;
    let (bob, bob_id) = register_user(&app, "n-bob@test.dev").await;

    // Alice follows Bob → Bob gets a `follow` notification.
    let follow = app
        .clone()
        .oneshot(request(
            "PUT",
            &format!("/api/v1/users/{bob_id}/follow"),
            Some(&alice),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(follow.status(), StatusCode::NO_CONTENT);

    // Bob's feed: one notification, unread.
    let feed = app
        .clone()
        .oneshot(request("GET", "/api/v1/notifications", Some(&bob), None))
        .await
        .unwrap();
    let body = json_body(feed).await;
    assert_eq!(body["unread"], 1);
    let items = body["notifications"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["kind"], "follow");
    assert_eq!(items[0]["is_read"], false);

    // Unread-count endpoint agrees.
    let count = app
        .clone()
        .oneshot(request(
            "GET",
            "/api/v1/notifications/unread-count",
            Some(&bob),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(json_body(count).await["unread"], 1);

    // Mark all read → unread 0, items now read.
    let read = app
        .clone()
        .oneshot(request(
            "POST",
            "/api/v1/notifications/read",
            Some(&bob),
            Some(json!({})),
        ))
        .await
        .unwrap();
    let body = json_body(read).await;
    assert_eq!(body["unread"], 0);
    let feed = app
        .clone()
        .oneshot(request("GET", "/api/v1/notifications", Some(&bob), None))
        .await
        .unwrap();
    assert_eq!(json_body(feed).await["notifications"][0]["is_read"], true);

    // Preferences: opt into digests, reflect back.
    let prefs = app
        .clone()
        .oneshot(request(
            "PUT",
            "/api/v1/notifications/preferences",
            Some(&bob),
            Some(json!({ "digest": true, "muted_kinds": ["promo"] })),
        ))
        .await
        .unwrap();
    let body = json_body(prefs).await;
    assert_eq!(body["preferences"]["digest"], true);
    assert_eq!(
        body["preferences"]["in_app"], true,
        "untouched field preserved"
    );
    assert_eq!(body["preferences"]["muted_kinds"], json!(["promo"]));
}

// ── SSE gap recovery (real HTTP) ────────────────────────────────────────────

#[tokio::test]
async fn sse_gap_recovery_replays_after_last_event_id() {
    let Some((app, pool)) = test_app().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let (alice, alice_id) = register_user(&app, "sse-alice@test.dev").await;
    let (_bob, bob_id) = register_user(&app, "sse-bob@test.dev").await;

    // Seed two notifications directly (ids 1 and 2) — the cursor boundary for
    // the replay is id 2, so the feed must deliver exactly ids 3, 4, 5 next.
    let repo = keystone_db::repositories::notifications::Notifications::new(pool.clone());
    for n in 0..2 {
        repo.create(&keystone_db::repositories::notifications::NewNotification {
            user_id: alice_id,
            kind: "follow",
            actor_id: Some(bob_id),
            entity_type: "user",
            entity_id: Some(bob_id),
            payload: json!({ "n": n }),
        })
        .await
        .expect("seed notification must insert");
    }
    // Three more notifications: ids 3, 4, 5 (the gap to recover).
    for n in 2..5 {
        repo.create(&keystone_db::repositories::notifications::NewNotification {
            user_id: alice_id,
            kind: "follow",
            actor_id: Some(bob_id),
            entity_type: "user",
            entity_id: Some(bob_id),
            payload: json!({ "n": n }),
        })
        .await
        .expect("seed notification must insert");
    }

    // Start a real server for a genuine SSE stream.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let client = reqwest::Client::new();
    let mut resp = client
        .get(format!("http://127.0.0.1:{port}/api/v1/notifications/feed"))
        .header("Authorization", format!("Bearer {alice}"))
        .header("Last-Event-ID", "2")
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success());

    // Accumulate the stream until the replay's last id arrives (then drop).
    let mut buffer = String::new();
    let deadline = Instant::now() + Duration::from_secs(15);
    while buffer.lines().filter(|l| l.starts_with("id:")).count() < 3 && Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_secs(3), resp.chunk()).await {
            Ok(Ok(Some(chunk))) => buffer.push_str(&String::from_utf8_lossy(&chunk)),
            _ => break,
        }
    }
    // Replayed exactly ids 3,4,5 — nothing before the cursor, nothing after.
    for id in [3i64, 4, 5] {
        assert!(
            buffer.contains(&format!("id: {id}")),
            "replay must include id {id}; got: {buffer}"
        );
    }
    for id in [1i64, 2] {
        assert!(
            !buffer.contains(&format!("id: {id}")),
            "replay must NOT include id {id}"
        );
    }
}

// ── Chat REST + WebSocket e2e ───────────────────────────────────────────────

async fn direct_conversation(app: &Router, alice: &str, bob_id: Uuid) -> Uuid {
    let created = app
        .clone()
        .oneshot(request(
            "POST",
            "/api/v1/conversations",
            Some(alice),
            Some(json!({ "type": "direct", "user_id": bob_id.to_string() })),
        ))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::OK, "direct find-or-create");
    let body = json_body(created).await;
    body["conversation"]["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap()
}

#[tokio::test]
async fn unauthorized_ws_join_and_presence_rejected() {
    let Some((app, _pool)) = test_app().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let (alice, _alice_id) = register_user(&app, "ws-a@test.dev").await;
    let (bob, bob_id) = register_user(&app, "ws-b@test.dev").await;
    let (eve, _eve_id) = register_user(&app, "ws-eve@test.dev").await;
    let conv = direct_conversation(&app, &alice, bob_id).await;

    // Real server so the handshake actually happens; the membership check must
    // reject the non-member BEFORE any socket is opened (403, not an upgrade).
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let connect = |token: &str| {
        // Build from the URI so tungstenite fills in the RFC 6455 headers
        // itself; only the bearer token is added.
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;
        let mut request = format!("ws://127.0.0.1:{port}/api/v1/ws/chat/{conv}")
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert("Authorization", format!("Bearer {token}").parse().unwrap());
        request
    };

    // Non-member: the handshake itself is rejected with 403.
    let err = tokio_tungstenite::connect_async(connect(&eve)).await;
    match err {
        Err(tokio_tungstenite::tungstenite::Error::Http(response)) => {
            eprintln!("NONMEMBER_RESPONSE_BODY={:?}", response.body());
            assert_eq!(
                response.status(),
                axum::http::StatusCode::FORBIDDEN,
                "non-member handshake rejected with 403"
            );
        }
        other => panic!("expected an HTTP 403 rejection, got: {other:?}"),
    }

    // Member: the same gate admits them (proves the gate is membership).
    let (mut ws, _) = tokio_tungstenite::connect_async(connect(&bob))
        .await
        .expect("member joins");
    let _ = ws.close(None).await;

    // Non-member presence read → 404 (existence never confirmed).
    let rest = reqwest::Client::new();
    let presence = rest
        .get(format!(
            "http://127.0.0.1:{port}/api/v1/conversations/{conv}/presence"
        ))
        .header("Authorization", format!("Bearer {eve}"))
        .send()
        .await
        .unwrap();
    assert_eq!(presence.status(), axum::http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn ws_browser_subprotocol_auth_and_typing() {
    let Some((app, _pool)) = test_app().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let (alice, alice_id) = register_user(&app, "ws-sub-a@test.dev").await;
    let (bob, bob_id) = register_user(&app, "ws-sub-b@test.dev").await;
    let conv = direct_conversation(&app, &alice, bob_id).await;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    // Browsers cannot set the Authorization header on a WebSocket upgrade, so
    // the SPA authenticates via the `bearer.<jwt>` subprotocol. The handshake
    // must succeed and the server must echo the subprotocol back.
    let connect_browser = |token: &str| {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;
        let mut request = format!("ws://127.0.0.1:{port}/api/v1/ws/chat/{conv}")
            .into_client_request()
            .unwrap();
        request.headers_mut().insert(
            axum::http::header::SEC_WEBSOCKET_PROTOCOL,
            format!("bearer.{token}").parse().unwrap(),
        );
        request
    };

    let (mut alice_ws, alice_resp) = tokio_tungstenite::connect_async(connect_browser(&alice))
        .await
        .expect("subprotocol auth");
    let echoed = alice_resp
        .headers()
        .get(axum::http::header::SEC_WEBSOCKET_PROTOCOL)
        .and_then(|v| v.to_str().ok());
    assert_eq!(echoed, Some(format!("bearer.{}", alice).as_str()));
    let (mut bob_ws, _) = tokio_tungstenite::connect_async(connect_browser(&bob))
        .await
        .expect("second member joins via subprotocol");

    // Ping-pong handshake: the socket handlers subscribe to the conversation
    // channel only after the upgrade response, so prove both directions are
    // live before asserting the payload. Bob's typing must reach Alice…
    bob_ws
        .send(tokio_tungstenite::tungstenite::Message::Text(
            json!({ "type": "typing" }).to_string().into(),
        ))
        .await
        .unwrap();
    let mut alice_seen = false;
    let deadline = Instant::now() + Duration::from_secs(10);
    while !alice_seen && Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_secs(3), alice_ws.next()).await {
            Ok(Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text)))) => {
                let value: Value = serde_json::from_str(&text).unwrap();
                if value["type"] == "typing" {
                    alice_seen = true;
                }
            }
            _ => break,
        }
    }
    assert!(alice_seen, "alice must receive bob's typing frame");

    // …and Alice's typing must reach Bob (filtered by sender: Bob's rx also
    // holds the echo of his own first frame).
    alice_ws
        .send(tokio_tungstenite::tungstenite::Message::Text(
            json!({ "type": "typing" }).to_string().into(),
        ))
        .await
        .unwrap();
    let mut bob_seen = false;
    let deadline = Instant::now() + Duration::from_secs(10);
    while !bob_seen && Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_secs(3), bob_ws.next()).await {
            Ok(Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text)))) => {
                let value: Value = serde_json::from_str(&text).unwrap();
                if value["type"] == "typing" && value["payload"]["user_id"] == alice_id.to_string()
                {
                    bob_seen = true;
                }
            }
            _ => break,
        }
    }
    assert!(bob_seen, "bob must receive alice's typing frame");
    let _ = alice_ws.close(None).await;
    let _ = bob_ws.close(None).await;
}

#[tokio::test]
async fn ws_chat_message_flow_presence_and_notification() {
    let Some((app, pool)) = test_app().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let (alice, alice_id) = register_user(&app, "chat-a@test.dev").await;
    let (bob, bob_id) = register_user(&app, "chat-b@test.dev").await;

    let conv = direct_conversation(&app, &alice, bob_id).await;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let connect = |token: &str| {
        // Build from the URI so tungstenite fills in the RFC 6455 headers
        // itself; only the bearer token is added.
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;
        let mut request = format!("ws://127.0.0.1:{port}/api/v1/ws/chat/{conv}")
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert("Authorization", format!("Bearer {token}").parse().unwrap());
        tokio_tungstenite::connect_async(request)
    };

    let (mut alice_ws, _) = connect(&alice).await.unwrap();
    let (mut bob_ws, _) = connect(&bob).await.unwrap();

    // Alice sends a message over the socket.
    alice_ws
        .send(tokio_tungstenite::tungstenite::Message::Text(
            json!({ "type": "message", "body": "hello bob" })
                .to_string()
                .into(),
        ))
        .await
        .unwrap();

    // Bob receives it (message frame with the body).
    let mut bob_seen = None;
    let deadline = Instant::now() + Duration::from_secs(10);
    while bob_seen.is_none() && Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_secs(3), bob_ws.next()).await {
            Ok(Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text)))) => {
                let value: Value = serde_json::from_str(&text).unwrap();
                if value["type"] == "message" {
                    bob_seen = Some(value);
                }
            }
            _ => break,
        }
    }
    let bob_seen = bob_seen.expect("bob must receive the message frame");
    assert_eq!(bob_seen["payload"]["body"], "hello bob");
    assert_eq!(bob_seen["payload"]["sender_id"], alice_id.to_string());

    // Alice receives her own echo (conversation channel broadcast).
    let mut alice_seen = None;
    let deadline = Instant::now() + Duration::from_secs(10);
    while alice_seen.is_none() && Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_secs(3), alice_ws.next()).await {
            Ok(Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text)))) => {
                let value: Value = serde_json::from_str(&text).unwrap();
                if value["type"] == "message" {
                    alice_seen = Some(value);
                }
            }
            _ => break,
        }
    }
    assert_eq!(alice_seen.expect("echo")["payload"]["body"], "hello bob");

    // Message persisted through the normal write path (REST read).
    let rest = reqwest::Client::new();
    let messages = rest
        .get(format!(
            "http://127.0.0.1:{port}/api/v1/conversations/{conv}/messages"
        ))
        .header("Authorization", format!("Bearer {bob}"))
        .send()
        .await
        .unwrap();
    let body: Value = messages.json().await.unwrap();
    assert_eq!(body["messages"].as_array().unwrap().len(), 1);

    // Bob has 1 unread in the conversation (feed notification too).
    let conversations = rest
        .get(format!("http://127.0.0.1:{port}/api/v1/conversations"))
        .header("Authorization", format!("Bearer {bob}"))
        .send()
        .await
        .unwrap();
    let body: Value = conversations.json().await.unwrap();
    assert_eq!(body["conversations"][0]["unread"], 1);

    let notifs = rest
        .get(format!(
            "http://127.0.0.1:{port}/api/v1/notifications/unread-count"
        ))
        .header("Authorization", format!("Bearer {bob}"))
        .send()
        .await
        .unwrap();
    let body: Value = notifs.json().await.unwrap();
    assert_eq!(body["unread"], 1, "message triggers a feed notification");

    // Presence visible to members with online status.
    let presence = rest
        .get(format!(
            "http://127.0.0.1:{port}/api/v1/conversations/{conv}/presence"
        ))
        .header("Authorization", format!("Bearer {alice}"))
        .send()
        .await
        .unwrap();
    let body: Value = presence.json().await.unwrap();
    assert_eq!(body["presence"].as_array().unwrap().len(), 2);

    let _ = alice_ws.close(None).await;
    let _ = bob_ws.close(None).await;
    server.abort();
    let _ = pool;
}

// ── Rate cap ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn message_gate_enforces_sliding_window() {
    let gate = keystone_api::chat::MessageGate::new(30, Duration::from_secs(10));
    let mut allowed = 0;
    for _ in 0..31 {
        if gate.allow() {
            allowed += 1;
        }
    }
    assert_eq!(
        allowed, 30,
        "burst of 31 within the window: 30 allowed, 1 refused"
    );
    // After the window, capacity is back.
    tokio::time::sleep(Duration::from_millis(11000)).await;
    assert!(gate.allow(), "window slides: capacity returns");
}
