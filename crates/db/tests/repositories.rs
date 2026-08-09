//! Repository integration tests against a real PostgreSQL.
//!
//! Self-skip when TEST_DATABASE_URL is unset (unit-only environments); run in
//! CI where the Postgres service is always present.

use keystone_db::repositories::sessions::{NewSession, Sessions};
use keystone_db::repositories::users::{NewUser, Users};
use keystone_db::repositories::RepoError;
use keystone_db::test_util;

fn ip(octets: [u8; 4]) -> std::net::IpAddr {
    std::net::IpAddr::V4(std::net::Ipv4Addr::from(octets))
}

#[tokio::test]
async fn users_create_find_and_unique_email() {
    let Some(pool) = test_util::test_pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    test_util::setup(&pool).await.expect("db setup");

    let users = Users::new(pool.clone());
    let created = users
        .create(NewUser {
            email: "Ada@Example.com",
            password_hash: "$argon2id$v=19$m=19456,t=2,p=1$fake$fakehash",
            first_name: Some("Ada"),
            last_name: Some("Lovelace"),
            username: Some("ada"),
        })
        .await
        .expect("create must succeed");

    assert_eq!(created.email_lower, "ada@example.com");
    assert_eq!(created.status, "pending_verification");
    assert_eq!(created.role, "user");
    assert!(!created.is_verified);

    // Case-insensitive lookup.
    let found = users
        .find_by_email("aDa@example.com")
        .await
        .unwrap()
        .expect("found");
    assert_eq!(found.id, created.id);
    assert!(users
        .find_by_email("nobody@example.com")
        .await
        .unwrap()
        .is_none());

    // Duplicate email (any case) is a typed error.
    let dup = users
        .create(NewUser {
            email: "ADA@example.com",
            password_hash: "x",
            first_name: None,
            last_name: None,
            username: Some("ada2"),
        })
        .await;
    assert!(matches!(dup, Err(RepoError::EmailTaken))); // Soft-deleted users (deleted_at set) are invisible to auth lookups.
    sqlx::query("UPDATE users SET deleted_at = now() WHERE id = $1")
        .bind(created.id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(users
        .find_by_email("ada@example.com")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn sessions_rotate_reuse_and_revoke() {
    let Some(pool) = test_util::test_pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    test_util::setup(&pool).await.expect("db setup");

    let users = Users::new(pool.clone());
    let user = users
        .create(NewUser {
            email: "grace@example.com",
            password_hash: "$argon2id$v=19$m=19456,t=2,p=1$fake$fakehash",
            first_name: Some("Grace"),
            last_name: Some("Hopper"),
            username: Some("grace"),
        })
        .await
        .unwrap();

    let sessions = Sessions::new(pool.clone());
    let expiry = chrono::Utc::now() + chrono::Duration::hours(1);

    let s1 = sessions
        .create(NewSession {
            user_id: user.id,
            refresh_token_hash: "hash-token-1",
            expires_at: expiry,
            user_agent: Some("test-agent"),
            ip_address: Some(ip([127, 0, 0, 1])),
        })
        .await
        .unwrap();

    // Live lookup by hash works.
    let live = sessions
        .find_live_by_hash("hash-token-1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(live.id, s1.id);

    // Rotate: token-2 replaces token-1.
    let s2 = sessions
        .create(NewSession {
            user_id: user.id,
            refresh_token_hash: "hash-token-2",
            expires_at: expiry,
            user_agent: None,
            ip_address: None,
        })
        .await
        .unwrap();
    sessions.rotate(s1.id, s2.id).await.unwrap();

    // The old token is no longer live; the new one is.
    assert!(sessions
        .find_live_by_hash("hash-token-1")
        .await
        .unwrap()
        .is_none());
    assert!(sessions
        .find_live_by_hash("hash-token-2")
        .await
        .unwrap()
        .is_some());

    // Reuse detection: the rotated-away token is an ancestor of s2.
    let ancestors = sessions.ancestor_hashes(s2.id).await.unwrap();
    assert_eq!(ancestors, vec!["hash-token-1".to_string()]);

    // Revoking the family kills both.
    sessions.revoke_family(s2.id).await.unwrap();
    assert!(sessions
        .find_live_by_hash("hash-token-2")
        .await
        .unwrap()
        .is_none());

    // Revoke-all on a fresh pair.
    let s3 = sessions
        .create(NewSession {
            user_id: user.id,
            refresh_token_hash: "hash-token-3",
            expires_at: expiry,
            user_agent: None,
            ip_address: None,
        })
        .await
        .unwrap();
    let s4 = sessions
        .create(NewSession {
            user_id: user.id,
            refresh_token_hash: "hash-token-4",
            expires_at: expiry,
            user_agent: None,
            ip_address: None,
        })
        .await
        .unwrap();
    assert_eq!(sessions.live_for_user(user.id).await.unwrap().len(), 2);
    sessions.revoke_all_for_user(user.id).await.unwrap();
    assert!(sessions.live_for_user(user.id).await.unwrap().is_empty());
    assert!(sessions
        .find_live_by_hash("hash-token-3")
        .await
        .unwrap()
        .is_none());
    assert!(sessions
        .find_live_by_hash("hash-token-4")
        .await
        .unwrap()
        .is_none());
    let _ = (s3, s4);
}

#[tokio::test]
async fn failed_logins_feed_lockout_policy() {
    let Some(pool) = test_util::test_pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    test_util::setup(&pool).await.expect("db setup");

    let users = Users::new(pool.clone());
    let user = users
        .create(NewUser {
            email: "linus@example.com",
            password_hash: "phc-hash",
            first_name: None,
            last_name: None,
            username: Some("linus"),
        })
        .await
        .unwrap();

    for _ in 0..5 {
        users
            .record_failed_login(user.id, Some(&ip([10, 0, 0, 7])))
            .await
            .unwrap();
    }

    let window = chrono::Utc::now() - chrono::Duration::minutes(5);
    let count = users.recent_failure_count(user.id, window).await.unwrap();
    assert_eq!(count, 5);

    // Successful login records last_login_at.
    users.record_login(user.id).await.unwrap();
    let after = users.find_by_id(user.id).await.unwrap().unwrap();
    assert!(after.last_login_at.is_some());
}
