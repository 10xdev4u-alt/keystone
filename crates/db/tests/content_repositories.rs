//! Content-core repository tests against a real Postgres: posts (versioning,
//! soft delete, counters, slug collisions), comments (nesting rules), tags,
//! reactions, and bookmarks.
//!
//! Self-skips when TEST_DATABASE_URL is unset (unit-only environments).

use keystone_db::repositories::bookmarks::Bookmarks;
use keystone_db::repositories::comments::{Comments, NewComment};
use keystone_db::repositories::posts::{NewPost, PostUpdate, Posts};
use keystone_db::repositories::reactions::Reactions;
use keystone_db::repositories::tags::Tags;
use keystone_db::repositories::users::{NewUser, Users};
use sqlx::PgPool;
use uuid::Uuid;

/// Isolated per-test schema — parallel-safe against every other test binary.
async fn test_pool() -> Option<PgPool> {
    keystone_db::test_util::test_pool_isolated().await
}

/// A fresh user to own content.
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

#[tokio::test]
async fn posts_create_read_update_version_and_soft_delete() {
    let Some(pool) = test_pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let author = make_user(&pool, "poster@example.com").await;
    let posts = Posts::new(pool.clone());

    // Create — version 1 snapshot is written atomically.
    let post = posts
        .create(NewPost {
            author_id: author,
            kind: "article",
            title: Some("Hello World"),
            slug: "hello-world",
            body: "First post body.",
            summary: Some("A greeting"),
            visibility: "public",
        })
        .await
        .expect("post must be created");
    assert_eq!(post.status, "published");
    assert_eq!(post.view_count, 0);

    let versions = posts.versions(post.id).await.unwrap();
    assert_eq!(versions.len(), 1);
    assert_eq!(versions[0].body, "First post body.");

    // Read by slug and by id.
    let by_slug = posts
        .get_by_slug("hello-world")
        .await
        .unwrap()
        .expect("found");
    assert_eq!(by_slug.id, post.id);
    let by_id = posts.get_by_id(post.id).await.unwrap().expect("found");
    assert_eq!(by_id.id, post.id);

    // Counters come from the maintained view (zero everywhere at first).
    let listed = posts
        .list(Some("article"), None, 10, None)
        .await
        .unwrap()
        .posts;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].comment_count, 0);
    assert_eq!(listed[0].reaction_count, 0);
    assert_eq!(listed[0].bookmark_count, 0);

    // View increment is transactional and cumulative.
    posts.increment_view(post.id).await.unwrap();
    posts.increment_view(post.id).await.unwrap();
    assert_eq!(
        posts.get_by_id(post.id).await.unwrap().unwrap().view_count,
        2
    );

    // Update appends a second version and changes the content.
    let updated = posts
        .update(
            post.id,
            PostUpdate {
                title: Some("Hello World v2"),
                body: "Updated body.",
                summary: None,
                change_note: Some("fixed typo"),
                editor_id: author,
            },
        )
        .await
        .unwrap()
        .expect("update must touch a live post");
    assert_eq!(updated.body, "Updated body.");
    let versions = posts.versions(post.id).await.unwrap();
    assert_eq!(versions.len(), 2, "history must be append-only");
    assert_eq!(versions[0].body, "Updated body.");
    assert_eq!(versions[1].body, "First post body.");

    // Soft delete hides it from reads.
    posts.soft_delete(post.id).await.unwrap().expect("deleted");
    assert!(posts.get_by_slug("hello-world").await.unwrap().is_none());
    assert!(posts.get_by_id(post.id).await.unwrap().is_none());
    let listed = posts.list(None, None, 10, None).await.unwrap().posts;
    assert!(listed.is_empty());
    // History survives deletion.
    assert_eq!(posts.versions(post.id).await.unwrap().len(), 2);
}

#[tokio::test]
async fn post_slug_collision_surfaces_unique_violation() {
    let Some(pool) = test_pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let author = make_user(&pool, "slugger@example.com").await;
    let posts = Posts::new(pool.clone());
    posts
        .create(NewPost {
            author_id: author,
            kind: "post",
            title: None,
            slug: "same-slug",
            body: "first",
            summary: None,
            visibility: "public",
        })
        .await
        .unwrap();

    let err = posts
        .create(NewPost {
            author_id: author,
            kind: "post",
            title: None,
            slug: "same-slug",
            body: "second",
            summary: None,
            visibility: "public",
        })
        .await
        .unwrap_err();
    match err {
        keystone_db::repositories::RepoError::UniqueViolation(constraint) => {
            assert_eq!(constraint, "posts_slug_key")
        }
        other => panic!("expected slug unique violation, got {other:?}"),
    }
}

#[tokio::test]
async fn comments_nest_within_a_post_only() {
    let Some(pool) = test_pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let author = make_user(&pool, "commenter@example.com").await;
    let posts = Posts::new(pool.clone());
    let comments = Comments::new(pool.clone());

    let post_a = posts
        .create(NewPost {
            author_id: author,
            kind: "post",
            title: None,
            slug: "post-a",
            body: "a",
            summary: None,
            visibility: "public",
        })
        .await
        .unwrap();
    let post_b = posts
        .create(NewPost {
            author_id: author,
            kind: "post",
            title: None,
            slug: "post-b",
            body: "b",
            summary: None,
            visibility: "public",
        })
        .await
        .unwrap();

    // Root comment + a reply on the same post.
    let root = comments
        .create(NewComment {
            post_id: post_a.id,
            author_id: author,
            parent_id: None,
            body: "root",
        })
        .await
        .unwrap();
    comments
        .create(NewComment {
            post_id: post_a.id,
            author_id: author,
            parent_id: Some(root.id),
            body: "reply",
        })
        .await
        .unwrap();

    // Cross-post parent is rejected.
    let err = comments
        .create(NewComment {
            post_id: post_b.id,
            author_id: author,
            parent_id: Some(root.id),
            body: "sneaky",
        })
        .await
        .unwrap_err();
    assert!(
        matches!(err, keystone_db::repositories::RepoError::InvalidInput(_)),
        "cross-post nesting must be rejected"
    );

    // Soft-deleted parent is not a valid target either.
    comments.soft_delete(root.id).await.unwrap();
    let err = comments
        .create(NewComment {
            post_id: post_a.id,
            author_id: author,
            parent_id: Some(root.id),
            body: "reply to ghost",
        })
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        keystone_db::repositories::RepoError::InvalidInput(_)
    ));

    // Listing shows only the visible comment (root was soft-deleted).
    let listed = comments.list_by_post(post_a.id).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].body, "reply");
}

#[tokio::test]
async fn reactions_upsert_and_remove() {
    let Some(pool) = test_pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let author = make_user(&pool, "reactor@example.com").await;
    let posts = Posts::new(pool.clone());
    let reactions = Reactions::new(pool.clone());

    let post = posts
        .create(NewPost {
            author_id: author,
            kind: "post",
            title: None,
            slug: "react-me",
            body: "hi",
            summary: None,
            visibility: "public",
        })
        .await
        .unwrap();

    // Set, then change kind (one reaction per user per post).
    reactions.set(post.id, author, "like").await.unwrap();
    let reaction = reactions.set(post.id, author, "love").await.unwrap();
    assert_eq!(reaction.kind, "love");
    assert_eq!(
        reactions.get(post.id, author).await.unwrap().unwrap().kind,
        "love"
    );

    // Derived counter reflects exactly one reaction.
    let listed = posts.list(None, None, 10, None).await.unwrap().posts;
    assert_eq!(listed[0].reaction_count, 1);

    reactions.remove(post.id, author).await.unwrap();
    assert!(reactions.get(post.id, author).await.unwrap().is_none());
    let listed = posts.list(None, None, 10, None).await.unwrap().posts;
    assert_eq!(listed[0].reaction_count, 0);
}

#[tokio::test]
async fn bookmarks_add_list_remove() {
    let Some(pool) = test_pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let author = make_user(&pool, "bookmarker@example.com").await;
    let posts = Posts::new(pool.clone());
    let bookmarks = Bookmarks::new(pool.clone());

    let post = posts
        .create(NewPost {
            author_id: author,
            kind: "post",
            title: None,
            slug: "save-me",
            body: "hi",
            summary: None,
            visibility: "public",
        })
        .await
        .unwrap();

    bookmarks.add(author, post.id).await.unwrap();
    bookmarks.add(author, post.id).await.unwrap(); // idempotent
    assert!(bookmarks.is_bookmarked(author, post.id).await.unwrap());
    assert_eq!(
        bookmarks.post_ids_for_user(author).await.unwrap(),
        vec![post.id]
    );

    // Derived counter counts the bookmark once.
    let listed = posts.list(None, None, 10, None).await.unwrap().posts;
    assert_eq!(listed[0].bookmark_count, 1);

    assert!(bookmarks.remove(author, post.id).await.unwrap());
    assert!(!bookmarks.is_bookmarked(author, post.id).await.unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn counters_never_drift_under_concurrent_writes() {
    // Month-3 "Done when" criterion: derived counters must never drift, even
    // when many writers race the same post. The counts come from the
    // maintained `post_counts` view, so the only way to fail is a broken
    // view or a lost write.
    let Some(pool) = test_pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let author = make_user(&pool, "stress@example.com").await;
    let posts = Posts::new(pool.clone());

    let post = posts
        .create(NewPost {
            author_id: author,
            kind: "post",
            title: None,
            slug: "stress-me",
            body: "hi",
            summary: None,
            visibility: "public",
        })
        .await
        .unwrap();

    const N: usize = 50;
    let mut handles = Vec::new();
    for i in 0..N {
        let pool = pool.clone();
        handles.push(tokio::spawn(async move {
            let comments = Comments::new(pool.clone());
            let reactions = Reactions::new(pool.clone());
            let bookmarks = Bookmarks::new(pool.clone());
            let body = format!("comment {i}");
            let _ = comments
                .create(NewComment {
                    post_id: post.id,
                    author_id: author,
                    parent_id: None,
                    body: &body,
                })
                .await;
            let _ = reactions
                .set(post.id, author, if i % 2 == 0 { "like" } else { "love" })
                .await;
            let _ = bookmarks.add(author, post.id).await;
        }));
    }
    for handle in handles {
        handle.await.expect("writer task must not panic");
    }

    let listed = posts.list(None, None, 10, None).await.unwrap().posts;
    assert_eq!(listed[0].comment_count, N as i64, "every comment counted");
    assert_eq!(
        listed[0].reaction_count, 1,
        "one reaction per user per post"
    );
    assert_eq!(listed[0].bookmark_count, 1, "bookmark counted once");
}

#[tokio::test]
async fn tags_ensure_is_case_insensitive_and_attachments_work() {
    let Some(pool) = test_pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let author = make_user(&pool, "tagger@example.com").await;
    let posts = Posts::new(pool.clone());
    let tags = Tags::new(pool.clone());

    let post = posts
        .create(NewPost {
            author_id: author,
            kind: "article",
            title: Some("Tagged"),
            slug: "tagged",
            body: "hi",
            summary: None,
            visibility: "public",
        })
        .await
        .unwrap();

    let rust = tags.ensure("Rust", "rust").await.unwrap();
    let rust_again = tags.ensure("rust", "rust").await.unwrap();
    assert_eq!(rust.id, rust_again.id, "case-insensitive find-or-create");

    tags.attach(post.id, rust.id).await.unwrap();
    tags.attach(post.id, rust.id).await.unwrap(); // idempotent

    let for_post = tags.for_post(post.id).await.unwrap();
    assert_eq!(for_post.len(), 1);
    assert_eq!(for_post[0].name, "Rust");

    assert!(tags.remove(post.id, rust.id).await.unwrap());
    assert!(tags.for_post(post.id).await.unwrap().is_empty());
}

#[tokio::test]
async fn series_append_ordered_posts_and_soft_delete() {
    let Some(pool) = test_pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let author = make_user(&pool, "seriator@example.com").await;
    let posts = Posts::new(pool.clone());
    let series = keystone_db::repositories::series::SeriesRepo::new(pool.clone());

    let created = series
        .create(keystone_db::repositories::series::NewSeries {
            author_id: author,
            title: "My Series",
            slug: "my-series",
            description: Some("A test series"),
        })
        .await
        .unwrap();
    assert_eq!(created.slug, "my-series");
    assert_eq!(
        series.get_by_slug("my-series").await.unwrap().unwrap().id,
        created.id
    );

    // Appends land in order (max + 1 each time).
    let mut post_ids = Vec::new();
    for n in 0..3 {
        let post = posts
            .create(NewPost {
                author_id: author,
                kind: "article",
                title: Some(&format!("Part {n}")),
                slug: &format!("my-series-part-{n}"),
                body: "body",
                summary: None,
                visibility: "public",
            })
            .await
            .unwrap();
        post_ids.push(post.id);
        series.add_post(created.id, post.id).await.unwrap();
    }
    series.add_post(created.id, post_ids[1]).await.unwrap(); // idempotent

    let listed = series.list_posts(created.id).await.unwrap();
    assert_eq!(listed.len(), 3);
    assert_eq!(listed[0].post_id, post_ids[0]);
    assert_eq!(listed[1].position, 1);
    assert_eq!(listed[2].position, 2);

    // Removing a post shifts nothing; soft-deleting a post hides it.
    assert!(series.remove_post(created.id, post_ids[1]).await.unwrap());
    assert_eq!(series.list_posts(created.id).await.unwrap().len(), 2);
    posts.soft_delete(post_ids[0]).await.unwrap();
    let listed = series.list_posts(created.id).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].post_id, post_ids[2]);

    // Slug uniqueness is scoped to live rows.
    let err = series
        .create(keystone_db::repositories::series::NewSeries {
            author_id: author,
            title: "Dup",
            slug: "my-series",
            description: None,
        })
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        keystone_db::repositories::RepoError::UniqueViolation(_)
    ));

    // Soft delete hides the series entirely.
    series
        .soft_delete(created.id)
        .await
        .unwrap()
        .expect("deleted");
    assert!(series.get_by_slug("my-series").await.unwrap().is_none());
}

#[tokio::test]
async fn reviews_upsert_list_and_resurrect_after_delete() {
    let Some(pool) = test_pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let author = make_user(&pool, "reviewer@example.com").await;
    let reviews = keystone_db::repositories::reviews::Reviews::new(pool.clone());
    let entity = Uuid::new_v4();

    let first = reviews
        .upsert(keystone_db::repositories::reviews::NewReview {
            author_id: author,
            entity_type: "employer",
            entity_id: entity,
            rating: 4,
            title: Some("Solid"),
            body: Some("Would recommend."),
        })
        .await
        .unwrap();
    assert_eq!(first.rating, 4);

    // Editing replaces (one review per author per entity).
    let second = reviews
        .upsert(keystone_db::repositories::reviews::NewReview {
            author_id: author,
            entity_type: "employer",
            entity_id: entity,
            rating: 5,
            title: Some("Solid v2"),
            body: None,
        })
        .await
        .unwrap();
    assert_eq!(second.id, first.id);
    assert_eq!(second.rating, 5);

    let listed = reviews.list_by_entity("employer", entity).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].rating, 5);

    // Different entity type is a different review.
    let other = Uuid::new_v4();
    reviews
        .upsert(keystone_db::repositories::reviews::NewReview {
            author_id: author,
            entity_type: "vendor",
            entity_id: other,
            rating: 1,
            title: None,
            body: Some("Meh"),
        })
        .await
        .unwrap();
    assert_eq!(
        reviews
            .list_by_entity("employer", entity)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        reviews.list_by_entity("vendor", other).await.unwrap().len(),
        1
    );

    // Soft delete hides; a later upsert resurrects the same row.
    reviews
        .soft_delete(second.id)
        .await
        .unwrap()
        .expect("deleted");
    assert!(reviews
        .list_by_entity("employer", entity)
        .await
        .unwrap()
        .is_empty());
    let revived = reviews
        .upsert(keystone_db::repositories::reviews::NewReview {
            author_id: author,
            entity_type: "employer",
            entity_id: entity,
            rating: 3,
            title: None,
            body: None,
        })
        .await
        .unwrap();
    assert_eq!(revived.id, second.id, "same row resurrected");
    assert_eq!(revived.status, "published");
    assert!(reviews
        .get(author, "employer", entity)
        .await
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn reports_flow_lifecycle_and_moderation_trail() {
    let Some(pool) = test_pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let reporter = make_user(&pool, "whistle@example.com").await;
    let moderator = make_user(&pool, "mod@example.com").await;
    let reports = keystone_db::repositories::reports::Reports::new(pool.clone());
    let moderation = keystone_db::repositories::moderation::Moderation::new(pool.clone());
    let target = Uuid::new_v4();

    let report = reports
        .create(keystone_db::repositories::reports::NewReport {
            reporter_id: reporter,
            entity_type: "post",
            entity_id: target,
            reason: "spam",
            detail: Some("Repeated advertising"),
        })
        .await
        .unwrap();
    assert_eq!(report.status, "open");

    let open = reports.list_open(10, 0).await.unwrap();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].entity_id, target);

    // The moderator resolves it and the append-only trail records the call.
    let resolved = reports
        .update_status(report.id, "resolved", moderator, Some("hidden the post"))
        .await
        .unwrap()
        .expect("report exists");
    assert_eq!(resolved.status, "resolved");
    assert_eq!(resolved.resolved_by, Some(moderator));
    assert!(resolved.resolved_at.is_some());
    assert!(reports.list_open(10, 0).await.unwrap().is_empty());

    moderation
        .record(keystone_db::repositories::moderation::NewModerationAction {
            moderator_id: moderator,
            action: "delete_post",
            target_type: "post",
            target_id: target,
            reason: Some("spam per report"),
        })
        .await
        .unwrap();

    let trail = moderation.list_by_target("post", target).await.unwrap();
    assert_eq!(trail.len(), 1);
    assert_eq!(trail[0].action, "delete_post");
    assert_eq!(trail[0].moderator_id, moderator);
}

#[tokio::test]
async fn feed_keyset_pagination_is_total_and_stable() {
    // Keyset requirement: pages must not overlap, must not skip, and must
    // cover every post exactly once even when created_at collides (the id
    // tiebreak makes the ordering total).
    let Some(pool) = test_pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let author = make_user(&pool, "pager@example.com").await;
    let posts = Posts::new(pool.clone());

    for i in 0..7 {
        posts
            .create(NewPost {
                author_id: author,
                kind: "post",
                title: None,
                slug: &format!("page-{i}"),
                body: "hi",
                summary: None,
                visibility: "public",
            })
            .await
            .unwrap();
    }

    // Page through 7 posts two at a time. Same-second inserts exercise the
    // (created_at, id) tiebreak — created_at alone would be ambiguous.
    // Contract: a page with `next_cursor: None` is the last page; a client
    // must stop there, never re-query from the top.
    let mut seen: Vec<Uuid> = Vec::new();
    let mut before: Option<(chrono::DateTime<chrono::Utc>, Uuid)> = None;
    loop {
        let page = posts.list(None, None, 2, before).await.unwrap();
        let exhausted = page.next_cursor.is_none();
        for row in &page.posts {
            assert!(
                !seen.contains(&row.post.id),
                "page {} duplicated a post",
                seen.len()
            );
            seen.push(row.post.id);
        }
        // Newest-first ordering must hold across page boundaries.
        for pair in page.posts.windows(2) {
            assert!(
                pair[0].post.created_at >= pair[1].post.created_at,
                "feed must be newest-first"
            );
        }
        if exhausted {
            break;
        }
        before = page.next_cursor;
    }
    assert_eq!(seen.len(), 7, "every post visited exactly once");
}
