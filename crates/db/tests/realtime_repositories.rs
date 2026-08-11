//! Month-7 repository tests against a real Postgres: the id-sequenced
//! notification feed with cursor read-state (gap recovery, unread
//! consistency under concurrency), digest batching idempotency, chat
//! conversations with membership-gated reads + direct-pair uniqueness,
//! presence privacy, and the LISTEN/NOTIFY event bus end-to-end.
//!
//! Self-skips when TEST_DATABASE_URL is unset.

use keystone_db::event_bus::{EventBus, PgNotifyBus};
use keystone_db::repositories::chat::Chat;
use keystone_db::repositories::notifications::{Notifications, PreferenceUpdate};
use keystone_db::repositories::users::{NewUser, Users};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

async fn test_pool() -> Option<PgPool> {
    keystone_db::test_util::test_pool_isolated().await
}

async fn make_user(pool: &PgPool, email: &str) -> Uuid {
    let users = Users::new(pool.clone());
    let user = users
        .create(NewUser {
            email,
            password_hash: "not-a-real-hash",
            first_name: Some("Test"),
            last_name: Some("User"),
            username: Some(email.split('@').next().unwrap()),
        })
        .await
        .expect("user must be created");
    user.id
}

// ── Notifications: feed, cursors, gap recovery ─────────────────────────────

#[tokio::test]
async fn notification_feed_cursor_and_gap_recovery() {
    let Some(pool) = test_pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let user = make_user(&pool, "feed@test.dev").await;
    let feed_actor = make_user(&pool, "actor@test.dev").await;
    let repo = Notifications::new(pool.clone());

    // Seed 5 notifications in order.
    let mut ids = Vec::new();
    for i in 0..5 {
        let n = repo
            .create(&keystone_db::repositories::notifications::NewNotification {
                user_id: user,
                kind: "follow",
                actor_id: Some(feed_actor),
                entity_type: "user",
                entity_id: Some(feed_actor),
                payload: json!({ "n": i }),
            })
            .await
            .unwrap();
        ids.push(n.id);
    }
    // Ids are strictly increasing.
    for w in ids.windows(2) {
        assert!(w[0] < w[1], "ids must increase");
    }

    // Unread count reflects everything above the cursor (0).
    assert_eq!(repo.unread_count(user).await.unwrap(), 5);

    // Gap recovery: everything after the 2nd notification, ascending.
    let after = repo.list_after(user, ids[1]).await.unwrap();
    assert_eq!(after.len(), 3);
    assert_eq!(after[0].id, ids[2]);
    assert_eq!(after[2].id, ids[4]);
    assert_eq!(after[0].payload["n"], json!(2));

    // Mark read up to the 4th → 1 unread.
    repo.mark_read(user, ids[3]).await.unwrap();
    assert_eq!(repo.unread_count(user).await.unwrap(), 1);

    // Mark-read is idempotent + monotonic: marking a lower cursor does nothing.
    repo.mark_read(user, ids[1]).await.unwrap();
    assert_eq!(repo.unread_count(user).await.unwrap(), 1);

    // Mark all → zero unread.
    repo.mark_all_read(user).await.unwrap();
    assert_eq!(repo.unread_count(user).await.unwrap(), 0);

    // Cursor paging, newest first.
    let page = repo.list(user, None, 2).await.unwrap();
    assert_eq!(page.len(), 2);
    assert_eq!(page[0].id, ids[4]);
    // Second page: everything older than the first page's tail (ids 3, 2, 1).
    let page2 = repo.list(user, Some(page[1].id), 10).await.unwrap();
    let page2_ids: Vec<i64> = page2.iter().map(|n| n.id).collect();
    assert_eq!(page2_ids, vec![ids[2], ids[1], ids[0]]);
    // The pages are disjoint and exhaustive — no gaps, no overlaps.
    let all: Vec<i64> = page.iter().chain(page2.iter()).map(|n| n.id).collect();
    assert_eq!(all, ids.iter().rev().copied().collect::<Vec<_>>());
}

#[tokio::test]
async fn cursor_paging_disjoint_and_exhaustive() {
    let Some(pool) = test_pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let user = make_user(&pool, "paging@test.dev").await;
    let actor = make_user(&pool, "paging-actor@test.dev").await;
    let repo = Notifications::new(pool.clone());
    let mut ids = Vec::new();
    for i in 0..25 {
        let n = repo
            .create(&keystone_db::repositories::notifications::NewNotification {
                user_id: user,
                kind: "follow",
                actor_id: Some(actor),
                entity_type: "user",
                entity_id: None,
                payload: json!({ "n": i }),
            })
            .await
            .unwrap();
        ids.push(n.id);
    }
    // Walk pages of 10 until exhaustion; expect 3 pages: 10, 10, 5.
    let mut got = Vec::new();
    let mut before: Option<i64> = None;
    loop {
        let page = repo.list(user, before, 10).await.unwrap();
        if page.is_empty() {
            break;
        }
        before = Some(page.last().unwrap().id);
        got.extend(page.into_iter().map(|n| n.id));
    }
    assert_eq!(got.len(), 25, "walking pages must exhaust the feed exactly");
    assert_eq!(
        got,
        ids.iter().rev().copied().collect::<Vec<_>>(),
        "no dupes, no gaps"
    );
}

#[tokio::test]
async fn unread_count_consistent_under_concurrency() {
    let Some(pool) = test_pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let user = make_user(&pool, "race@test.dev").await;
    let race_actor = make_user(&pool, "actor2@test.dev").await;
    let repo = Notifications::new(pool.clone());

    // 40 concurrent creates.
    let mut handles = Vec::new();
    for _ in 0..40 {
        let pool = pool.clone();
        handles.push(tokio::spawn(async move {
            let repo = Notifications::new(pool);
            repo.create(&keystone_db::repositories::notifications::NewNotification {
                user_id: user,
                kind: "follow",
                actor_id: Some(race_actor),
                entity_type: "user",
                entity_id: None,
                payload: json!({}),
            })
            .await
            .unwrap()
            .id
        }));
    }
    let mut ids: Vec<i64> = Vec::new();
    for h in handles {
        ids.push(h.await.unwrap());
    }
    ids.sort_unstable();

    // Concurrent mark-reads with racing cursors.
    let mut readers = Vec::new();
    for _ in 0..10 {
        let repo = Notifications::new(pool.clone());
        readers.push(tokio::spawn(async move {
            let latest = repo.mark_all_read(user).await.unwrap();
            repo.mark_read(user, latest.saturating_sub(5))
                .await
                .unwrap();
        }));
    }
    for r in readers {
        r.await.unwrap();
    }

    // Invariant: unread == count of ids strictly above the stored cursor,
    // recomputed independently — never negative, never stale.
    let unread = repo.unread_count(user).await.unwrap();
    let cursor: i64 =
        sqlx::query_scalar("SELECT read_cursor FROM notification_states WHERE user_id = $1")
            .bind(user)
            .fetch_one(&pool)
            .await
            .unwrap();
    let expected = ids.iter().filter(|id| **id > cursor).count() as i64;
    assert_eq!(unread, expected);
    assert!(unread >= 0);
}

#[tokio::test]
async fn digest_batch_is_idempotent_and_groups_by_user() {
    let Some(pool) = test_pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let alice = make_user(&pool, "digest-a@test.dev").await;
    let bob = make_user(&pool, "digest-b@test.dev").await;
    let actor = make_user(&pool, "digest-actor@test.dev").await;
    let repo = Notifications::new(pool.clone());

    // Only Alice opts into digests.
    repo.upsert_preferences(
        alice,
        &PreferenceUpdate {
            digest: Some(true),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    for (user, kind) in [(alice, "follow"), (alice, "comment"), (bob, "follow")] {
        repo.create(&keystone_db::repositories::notifications::NewNotification {
            user_id: user,
            kind,
            actor_id: Some(actor),
            entity_type: "post",
            entity_id: None,
            payload: json!({}),
        })
        .await
        .unwrap();
    }

    let before = chrono::Utc::now() + chrono::Duration::seconds(5);
    let batch = repo.digest_batch(before, 100).await.unwrap();

    // Only Alice appears (digest disabled for Bob), with her 2 items.
    assert_eq!(batch.len(), 1);
    assert_eq!(batch[0].user_id, alice);
    assert_eq!(batch[0].notifications.len(), 2);
    assert_eq!(batch[0].notifications[0].kind, "follow");
    assert_eq!(batch[0].notifications[1].kind, "comment");

    // Idempotent: nothing left to batch.
    let again = repo.digest_batch(before, 100).await.unwrap();
    assert!(again.is_empty(), "digest batching must be idempotent");
}

#[tokio::test]
async fn preferences_mute_kinds_and_partial_updates() {
    let Some(pool) = test_pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let user = make_user(&pool, "prefs@test.dev").await;
    let repo = Notifications::new(pool.clone());

    let defaults = repo.get_preferences(user).await.unwrap();
    assert!(defaults.in_app);
    assert!(!defaults.digest);

    repo.upsert_preferences(
        user,
        &PreferenceUpdate {
            digest: Some(true),
            muted_kinds: Some(vec!["promo".into()]),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let updated = repo.get_preferences(user).await.unwrap();
    assert!(updated.digest);
    assert!(updated.in_app, "untouched field must be preserved");
    assert!(repo.is_muted(user, "promo").await.unwrap());
    assert!(!repo.is_muted(user, "follow").await.unwrap());
}

// ── Chat: conversations, membership, presence ───────────────────────────────

#[tokio::test]
async fn direct_conversation_is_unique_and_membership_gated() {
    let Some(pool) = test_pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let alice = make_user(&pool, "alice@test.dev").await;
    let bob = make_user(&pool, "bob@test.dev").await;
    let eve = make_user(&pool, "eve@test.dev").await;
    let chat = Chat::new(pool.clone());

    // Same pair twice → the same conversation.
    let c1 = chat.find_or_create_direct(alice, bob).await.unwrap();
    let c2 = chat.find_or_create_direct(bob, alice).await.unwrap();
    assert_eq!(
        c1.id, c2.id,
        "direct pair must be unique regardless of order"
    );
    assert!(chat.is_member(c1.id, alice).await.unwrap());
    assert!(chat.is_member(c1.id, bob).await.unwrap());

    // A third user is NOT a member and cannot read/send — and the rejection
    // must be a clean InvalidInput (400-class), never a Database error (500).
    assert!(!chat.is_member(c1.id, eve).await.unwrap());
    let send = chat.send_message(c1.id, eve, "hi").await;
    assert!(
        matches!(
            send,
            Err(keystone_db::repositories::RepoError::InvalidInput(_))
        ),
        "non-member send must be InvalidInput, got: {send:?}"
    );
    let list = chat.list_messages(c1.id, eve, None, 10).await;
    assert!(
        matches!(
            list,
            Err(keystone_db::repositories::RepoError::InvalidInput(_))
        ),
        "non-member read must be InvalidInput, got: {list:?}"
    );

    // Member sends + reads work.
    chat.send_message(c1.id, alice, "hello bob").await.unwrap();
    chat.send_message(c1.id, bob, "hey alice").await.unwrap();
    let msgs = chat.list_messages(c1.id, alice, None, 10).await.unwrap();
    assert_eq!(msgs.len(), 2);
    // Newest first — chat history pages back in time via the `before` cursor.
    assert_eq!(msgs[0].body, "hey alice");
    assert_eq!(msgs[1].body, "hello bob");
}

#[tokio::test]
async fn group_conversation_and_unread_counts() {
    let Some(pool) = test_pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let alice = make_user(&pool, "grp-a@test.dev").await;
    let bob = make_user(&pool, "grp-b@test.dev").await;
    let carol = make_user(&pool, "grp-c@test.dev").await;
    let chat = Chat::new(pool.clone());

    let group = chat
        .create_group(alice, "Rust crew", &[bob, carol])
        .await
        .unwrap();
    assert_eq!(group.kind, "group");

    // Alice sends; Bob and Carol both have 1 unread; Alice has 0.
    chat.send_message(group.id, alice, "standup at 10")
        .await
        .unwrap();
    let alice_list = chat.list_for_user(alice).await.unwrap();
    let bob_list = chat.list_for_user(bob).await.unwrap();
    assert_eq!(alice_list[0].unread, 0);
    assert_eq!(bob_list[0].unread, 1);
    assert_eq!(bob_list[0].last_message.as_deref(), Some("standup at 10"));

    // Bob marks read → his unread drops, delivery ack stamps delivered_at.
    chat.mark_read(group.id, bob).await.unwrap();
    let bob_list = chat.list_for_user(bob).await.unwrap();
    assert_eq!(bob_list[0].unread, 0);

    // Add a member (actor must already be a member); outsiders cannot.
    let outsider = make_user(&pool, "grp-d@test.dev").await;
    assert!(chat.add_member(group.id, outsider, outsider).await.is_err());
    chat.add_member(group.id, alice, outsider).await.unwrap();
    assert!(chat.is_member(group.id, outsider).await.unwrap());
}

#[tokio::test]
async fn presence_is_member_visible_only() {
    let Some(pool) = test_pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let alice = make_user(&pool, "pres-a@test.dev").await;
    let bob = make_user(&pool, "pres-b@test.dev").await;
    let chat = Chat::new(pool.clone());

    let direct = chat.find_or_create_direct(alice, bob).await.unwrap();
    chat.set_presence(alice, "online").await.unwrap();
    chat.set_presence(bob, "away").await.unwrap();

    let visible = chat.presence_for(direct.id).await.unwrap();
    assert_eq!(visible.len(), 2);
    let statuses: Vec<&str> = visible.iter().map(|p| p.status.as_str()).collect();
    assert!(statuses.contains(&"online"));
    assert!(statuses.contains(&"away"));
    assert!(visible.iter().all(|p| p.last_seen_at <= chrono::Utc::now()));
}

// ── Event bus: LISTEN/NOTIFY end-to-end ─────────────────────────────────────

#[tokio::test]
async fn event_bus_relays_published_events() {
    let Some(pool) = test_pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    // Unique channel per run so parallel bus tests never cross-pollinate.
    let channel = format!("bus-{}", Uuid::new_v4().simple());
    let bus = PgNotifyBus::new(pool.clone());
    let mut rx = bus.receiver(&channel);

    bus.publish(&channel, r#"{"hello":"world"}"#).await.unwrap();

    let event = tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv())
        .await
        .expect("listener must relay the notification within 10s")
        .expect("channel must not close");
    assert_eq!(event.channel, channel);
    assert_eq!(event.payload, r#"{"hello":"world"}"#);

    // Second publish also arrives (the listener keeps listening).
    bus.publish(&channel, "second").await.unwrap();
    let event = tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv())
        .await
        .expect("second event must arrive")
        .expect("channel must not close");
    assert_eq!(event.payload, "second");
}
