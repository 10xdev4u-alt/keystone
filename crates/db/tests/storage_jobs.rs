//! Month-8 storage & jobs tests against a real Postgres:
//!   - quota enforcement: register() rejects when used + new > limit, atomic
//!     under concurrent uploads (racing registers cannot overshoot)
//!   - advisory-lock jobs: N concurrent runners → exactly one executes;
//!     different jobs run concurrently
//!   - upload → thumbnail → download round-trip through the memory backend
//!
//! Self-skips when TEST_DATABASE_URL is unset.

use keystone_db::jobs::run_exclusive;
use keystone_db::repositories::files::{Files, NewFileRecord};
use keystone_db::repositories::users::{NewUser, Users};
use keystone_db::storage::{make_thumbnail, MemoryStorage, StorageBackend};
use sqlx::PgPool;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use uuid::Uuid;

async fn test_pool() -> Option<PgPool> {
    keystone_db::test_util::test_pool_isolated().await
}

/// A pool with one connection per racer — race tests must not serialize
/// through a tiny pool or the advisory-lock semantics are never exercised.
async fn test_pool_racy() -> Option<PgPool> {
    keystone_db::test_util::test_pool_isolated_with(12).await
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

fn record<'a>(owner: Uuid, key: &'a str, size: i64) -> NewFileRecord<'a> {
    NewFileRecord {
        owner_id: owner,
        bucket: "keystone",
        object_key: key,
        original_name: key,
        content_type: "text/plain",
        size_bytes: size,
        sha256: "abc123",
        width: None,
        height: None,
        parent_id: None,
        is_public: false,
    }
}

// ── Quota enforcement ───────────────────────────────────────────────────────

#[tokio::test]
async fn quota_enforcement_rejects_overshoot() {
    let Some(pool) = test_pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let owner = make_user(&pool, "quota@test.dev").await;
    let files = Files::new(pool.clone());

    // Default quota is 1 GiB; tighten it to 1 KiB.
    files.set_quota(owner, 1024).await.unwrap();
    assert_eq!(files.quota(owner).await.unwrap(), 1024);

    let key_a = format!("users/{owner}/a.txt");
    let key_b = format!("users/{owner}/b.txt");
    let key_c = format!("users/{owner}/c.txt");
    files.register(&record(owner, &key_a, 700)).await.unwrap();
    files.register(&record(owner, &key_b, 300)).await.unwrap();
    assert_eq!(files.used_bytes(owner).await.unwrap(), 1000);

    // 25 more bytes would exceed the 1024 cap.
    let err = files.register(&record(owner, &key_c, 25)).await;
    assert!(
        matches!(
            err,
            Err(keystone_db::repositories::RepoError::InvalidInput(_))
        ),
        "overshoot must be rejected as InvalidInput"
    );
    assert_eq!(
        files.used_bytes(owner).await.unwrap(),
        1000,
        "no partial write"
    );
}

#[tokio::test]
async fn concurrent_uploads_cannot_overshoot_quota() {
    let Some(pool) = test_pool_racy().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let owner = make_user(&pool, "race-quota@test.dev").await;
    let files = Files::new(pool.clone());
    files.set_quota(owner, 1000).await.unwrap();

    // 20 racers each trying to register 100 bytes; only 10 can fit.
    let mut handles = Vec::new();
    for i in 0..20 {
        let files = Files::new(pool.clone());
        let key = format!("users/{owner}/f{i}.txt");
        handles.push(tokio::spawn(async move {
            files.register(&record(owner, &key, 100)).await.is_ok()
        }));
    }
    let mut results = Vec::new();
    for h in handles {
        results.push(h.await.unwrap());
    }
    let successes = results.into_iter().filter(|ok| *ok).count();
    assert_eq!(successes, 10, "exactly the quota's worth must succeed");
    assert_eq!(files.used_bytes(owner).await.unwrap(), 1000);
}

#[tokio::test]
async fn file_records_round_trip_and_ownership_delete() {
    let Some(pool) = test_pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let owner = make_user(&pool, "files@test.dev").await;
    let files = Files::new(pool.clone());

    let row = files
        .register(&record(owner, "notes.txt", 42))
        .await
        .unwrap();
    let fetched = files.get(row.id).await.unwrap().expect("row exists");
    assert_eq!(fetched.original_name, "notes.txt");
    assert_eq!(fetched.size_bytes, 42);

    let listed = files.list_for_owner(owner, None, 10).await.unwrap();
    assert_eq!(listed.len(), 1);

    // Delete is owner-scoped: another user cannot delete it.
    let intruder = make_user(&pool, "intruder@test.dev").await;
    files.delete(row.id, intruder).await.unwrap(); // no-op (no row matched)
    assert!(files.get(row.id).await.unwrap().is_some(), "owner only");
    files.delete(row.id, owner).await.unwrap();
    assert!(files.get(row.id).await.unwrap().is_none());
}

// ── Jobs: exactly-one-runner ────────────────────────────────────────────────

#[tokio::test]
async fn advisory_lock_guarantees_single_runner() {
    let Some(pool) = test_pool_racy().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let counter = Arc::new(AtomicUsize::new(0));
    let ran: Vec<bool> = {
        let mut handles = Vec::new();
        for _ in 0..8 {
            let pool = pool.clone();
            let counter = counter.clone();
            handles.push(tokio::spawn(async move {
                run_exclusive(&pool, "stats-aggregate", async move {
                    // Slow enough that racers pile up behind the lock.
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                    counter.fetch_add(1, Ordering::SeqCst);
                })
                .await
                .unwrap()
            }));
        }
        let mut results = Vec::new();
        for h in handles {
            results.push(h.await.unwrap());
        }
        results
    };
    assert_eq!(ran.iter().filter(|r| **r).count(), 1, "exactly one runner");
    assert_eq!(
        counter.load(Ordering::SeqCst),
        1,
        "the work ran exactly once"
    );
}

#[tokio::test]
async fn different_jobs_run_concurrently() {
    let Some(pool) = test_pool_racy().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let a = Arc::new(AtomicUsize::new(0));
    let b = Arc::new(AtomicUsize::new(0));
    let mut handles = Vec::new();
    for _ in 0..4 {
        let pool_a = pool.clone();
        let a = a.clone();
        handles.push(tokio::spawn(async move {
            run_exclusive(&pool_a, "job-a", async move {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                a.fetch_add(1, Ordering::SeqCst);
            })
            .await
            .unwrap()
        }));
        let pool_b = pool.clone();
        let b = b.clone();
        handles.push(tokio::spawn(async move {
            run_exclusive(&pool_b, "job-b", async move {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                b.fetch_add(1, Ordering::SeqCst);
            })
            .await
            .unwrap()
        }));
    }
    for h in handles {
        h.await.unwrap();
    }
    assert_eq!(a.load(Ordering::SeqCst), 1, "job-a ran once");
    assert_eq!(b.load(Ordering::SeqCst), 1, "job-b ran once");
}

// ── Upload → thumbnail → download ───────────────────────────────────────────

#[tokio::test]
async fn upload_thumbnail_download_round_trip() {
    let Some(pool) = test_pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let owner = make_user(&pool, "media@test.dev").await;
    let storage = MemoryStorage::new();
    let files = Files::new(pool.clone());

    // A real 1x1 PNG (as the browser would upload).
    let png: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90,
        0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x08, 0xD7, 0x63, 0xF8,
        0xFF, 0xFF, 0x3F, 0x00, 0x05, 0xFE, 0x02, 0xFE, 0xDC, 0xCC, 0x59, 0xE7, 0x00, 0x00, 0x00,
        0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];

    // The presigned flow: client gets a PUT url, bytes land in the bucket,
    // metadata is registered (this test simulates the PUT with put_bytes).
    let key = format!("users/{owner}/pic.png");
    let put_url = storage.presign_put(&key, "image/png", 300).await.unwrap();
    assert!(put_url.starts_with("memory://"));
    storage.put_bytes(&key, png, "image/png").await.unwrap();

    // Thumbnail is generated server-side and stored under thumbs/.
    let (thumb, w, h) = make_thumbnail(png, 256).unwrap();
    assert!(w >= 1 && h >= 1);
    let thumb_key = format!("thumbs/{key}");
    storage
        .put_bytes(&thumb_key, &thumb, "image/jpeg")
        .await
        .unwrap();

    // Download path: presigned GET resolves to the stored bytes.
    let get_url = storage.presign_get(&key, 300).await.unwrap();
    assert!(get_url.starts_with("memory://"));
    let downloaded = storage.get_bytes(&key).await.unwrap();
    assert_eq!(downloaded, png, "download round-trips the original");
    let downloaded_thumb = storage.get_bytes(&thumb_key).await.unwrap();
    assert_eq!(downloaded_thumb, thumb);

    // Metadata registered with quota accounting (image size only).
    files
        .register(&NewFileRecord {
            owner_id: owner,
            bucket: "keystone",
            object_key: &key,
            original_name: "pic.png",
            content_type: "image/png",
            size_bytes: png.len() as i64,
            sha256: "sha",
            width: Some(w as i32),
            height: Some(h as i32),
            parent_id: None,
            is_public: false,
        })
        .await
        .unwrap();
    assert_eq!(files.used_bytes(owner).await.unwrap(), png.len() as i64);
}
