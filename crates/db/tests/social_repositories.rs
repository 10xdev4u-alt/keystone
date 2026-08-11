//! Month-4 repository tests against a real Postgres: communities with role
//! invariants, community posts + pinning, poll single-vote invariants,
//! Q&A answers/votes/bounty lifecycle, and post locking.
//!
//! Self-skips when TEST_DATABASE_URL is unset.

use chrono::Duration;
use keystone_db::repositories::communities::{Communities, NewCommunity};
use keystone_db::repositories::community_posts::CommunityPosts;
use keystone_db::repositories::posts::{NewPost, Posts};
use keystone_db::repositories::qa::{NewBounty, Qa};
use keystone_db::repositories::users::{NewUser, Users};
use sqlx::PgPool;
use uuid::Uuid;

/// Isolated per-test schema — parallel-safe against every other test binary.
async fn test_pool() -> Option<PgPool> {
    keystone_db::test_util::test_pool_isolated().await
}

async fn make_user(pool: &PgPool, email: &str) -> Uuid {
    let users = Users::new(pool.clone());
    let user = users
        .create(NewUser {
            email,
            password_hash: "not-a-real-hash",
            first_name: None,
            last_name: None,
            username: Some(email.split('@').next().unwrap()),
        })
        .await
        .expect("user must be created");
    user.id
}

async fn make_post(pool: &PgPool, author: Uuid, kind: &str, slug: &str) -> Uuid {
    let posts = Posts::new(pool.clone());
    let post = posts
        .create(NewPost {
            author_id: author,
            kind,
            title: None,
            slug,
            body: "body",
            summary: None,
            cover_image_url: None,
            visibility: "public",
        })
        .await
        .expect("post must be created");
    post.id
}

// ── Communities ────────────────────────────────────────────────────────────

#[tokio::test]
async fn community_roles_single_owner_and_transfer() {
    let Some(pool) = test_pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let owner = make_user(&pool, "cowner@example.com").await;
    let alice = make_user(&pool, "cmember@example.com").await;
    let communities = Communities::new(pool.clone());

    let community = communities
        .create(NewCommunity {
            name: "Rust Guild",
            slug: "rust-guild",
            description: Some("All things Rust"),
            visibility: "public",
            created_by: owner,
        })
        .await
        .unwrap();
    assert_eq!(
        communities
            .role_of(community.id, owner)
            .await
            .unwrap()
            .as_deref(),
        Some("owner")
    );

    // Join + leave.
    communities.join(community.id, alice).await.unwrap();
    assert_eq!(
        communities
            .role_of(community.id, alice)
            .await
            .unwrap()
            .as_deref(),
        Some("member")
    );
    communities.join(community.id, alice).await.unwrap(); // idempotent
    assert!(communities.leave(community.id, alice).await.unwrap());
    assert!(communities
        .role_of(community.id, alice)
        .await
        .unwrap()
        .is_none());

    // The owner cannot leave or be demoted.
    let err = communities.leave(community.id, owner).await.unwrap_err();
    assert!(matches!(
        err,
        keystone_db::repositories::RepoError::InvalidInput(_)
    ));
    let err = communities
        .set_role(community.id, owner, "member")
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        keystone_db::repositories::RepoError::InvalidInput(_)
    ));

    // Promoting transfers ownership: one owner at a time, old owner → admin.
    communities.join(community.id, alice).await.unwrap();
    communities
        .set_role(community.id, alice, "owner")
        .await
        .unwrap();
    assert_eq!(
        communities
            .role_of(community.id, alice)
            .await
            .unwrap()
            .as_deref(),
        Some("owner")
    );
    assert_eq!(
        communities
            .role_of(community.id, owner)
            .await
            .unwrap()
            .as_deref(),
        Some("admin")
    );
    let members = communities.members(community.id).await.unwrap();
    assert_eq!(
        members.iter().filter(|m| m.role == "owner").count(),
        1,
        "exactly one owner"
    );
}

#[tokio::test]
async fn community_posts_feed_pins_and_removes() {
    let Some(pool) = test_pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let owner = make_user(&pool, "cfeed@example.com").await;
    let communities = Communities::new(pool.clone());
    let community_posts = CommunityPosts::new(pool.clone());
    let community = communities
        .create(NewCommunity {
            name: "Feed Guild",
            slug: "feed-guild",
            description: None,
            visibility: "public",
            created_by: owner,
        })
        .await
        .unwrap();

    let p1 = make_post(&pool, owner, "discussion", "feed-1").await;
    let p2 = make_post(&pool, owner, "discussion", "feed-2").await;
    community_posts.add(community.id, p1, owner).await.unwrap();
    community_posts.add(community.id, p2, owner).await.unwrap();
    community_posts.add(community.id, p1, owner).await.unwrap(); // idempotent

    // Newest first; pinning p1 lifts it above p2.
    let feed = community_posts.list(community.id, 10, 0).await.unwrap();
    assert_eq!(feed.len(), 2);
    assert_eq!(feed[0].post_id, p2);
    community_posts
        .set_pinned(community.id, p1, true)
        .await
        .unwrap();
    let feed = community_posts.list(community.id, 10, 0).await.unwrap();
    assert_eq!(feed[0].post_id, p1);
    assert!(feed[0].pinned);

    // Soft-deleted posts vanish from the feed.
    let posts = Posts::new(pool.clone());
    posts.soft_delete(p2).await.unwrap();
    let feed = community_posts.list(community.id, 10, 0).await.unwrap();
    assert_eq!(feed.len(), 1);

    assert!(community_posts.remove(community.id, p1).await.unwrap());
    assert!(community_posts
        .list(community.id, 10, 0)
        .await
        .unwrap()
        .is_empty());
}

// ── Polls ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn polls_one_vote_per_user_and_derived_counts() {
    let Some(pool) = test_pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let author = make_user(&pool, "polster@example.com").await;
    let voter = make_user(&pool, "pollvoter@example.com").await;
    let polls = keystone_db::repositories::polls::Polls::new(pool.clone());
    let poll = make_post(&pool, author, "poll", "best-lang").await;

    let rust = polls.add_option(poll, "Rust").await.unwrap();
    let other = polls.add_option(poll, "Other").await.unwrap();
    assert_eq!(polls.options(poll).await.unwrap().len(), 2);

    polls.vote(poll, voter, rust.id).await.unwrap();
    polls.vote(poll, voter, rust.id).await.unwrap(); // same option: no-op
    assert_eq!(polls.total_votes(poll).await.unwrap(), 1);

    // Switching the vote moves it — still exactly one vote.
    polls.vote(poll, voter, other.id).await.unwrap();
    assert_eq!(polls.total_votes(poll).await.unwrap(), 1);
    assert_eq!(
        polls.voted_option(poll, voter).await.unwrap(),
        Some(other.id)
    );
    let results = polls.results(poll).await.unwrap();
    assert_eq!(results[0].votes, 0);
    assert_eq!(results[1].votes, 1);

    // An option from another poll is refused.
    let other_poll = make_post(&pool, author, "poll", "other-poll").await;
    let stray = polls.add_option(other_poll, "Stray").await.unwrap();
    let err = polls.vote(poll, voter, stray.id).await.unwrap_err();
    assert!(matches!(
        err,
        keystone_db::repositories::RepoError::InvalidInput(_)
    ));

    // Withdrawing removes the vote entirely.
    assert!(polls.remove_vote(poll, voter).await.unwrap());
    assert_eq!(polls.total_votes(poll).await.unwrap(), 0);
}

// ── Q&A ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn qa_answers_votes_and_acceptance() {
    let Some(pool) = test_pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let asker = make_user(&pool, "asker@example.com").await;
    let answerer = make_user(&pool, "answerer@example.com").await;
    let voter = make_user(&pool, "qavoter@example.com").await;
    let qa = Qa::new(pool.clone());
    let question = make_post(&pool, asker, "question", "why-rust").await;

    // Only question-kind posts accept answers.
    let essay = make_post(&pool, asker, "article", "not-a-question").await;
    let err = qa.create_answer(essay, answerer, "nope").await.unwrap_err();
    assert!(matches!(
        err,
        keystone_db::repositories::RepoError::InvalidInput(_)
    ));

    let a1 = qa
        .create_answer(question, answerer, "Memory safety.")
        .await
        .unwrap();
    let a2 = qa
        .create_answer(question, answerer, "Fearless concurrency.")
        .await
        .unwrap();

    // Votes: upsert direction, one per user.
    qa.vote_answer(a1.id, voter, 1).await.unwrap();
    qa.vote_answer(a1.id, voter, 1).await.unwrap(); // idempotent
    qa.vote_answer(a1.id, voter, -1).await.unwrap(); // switch direction
    let answers = qa.list_answers(question).await.unwrap();
    assert_eq!(answers[0].score, -1);
    qa.vote_answer(a1.id, voter, 0).await.unwrap(); // remove
    let answers = qa.list_answers(question).await.unwrap();
    assert_eq!(answers[0].score, 0);

    // Accepting one answer clears the other — exactly one accepted.
    qa.accept_answer(question, a1.id).await.unwrap();
    qa.accept_answer(question, a2.id).await.unwrap();
    let answers = qa.list_answers(question).await.unwrap();
    let accepted: Vec<_> = answers.iter().filter(|a| a.accepted_at.is_some()).collect();
    assert_eq!(accepted.len(), 1);
    assert_eq!(accepted[0].id, a2.id);

    // A foreign answer cannot be accepted on this question.
    let other_q = make_post(&pool, asker, "question", "other-q").await;
    let foreign = qa
        .create_answer(other_q, answerer, "elsewhere")
        .await
        .unwrap();
    let err = qa.accept_answer(question, foreign.id).await.unwrap_err();
    assert!(matches!(
        err,
        keystone_db::repositories::RepoError::InvalidInput(_)
    ));
}

#[tokio::test]
async fn bounty_lifecycle_invariants_hold() {
    let Some(pool) = test_pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let asker = make_user(&pool, "basker@example.com").await;
    let answerer = make_user(&pool, "banswerer@example.com").await;
    let qa = Qa::new(pool.clone());
    let question = make_post(&pool, asker, "question", "bounty-q").await;
    let answer = qa
        .create_answer(question, answerer, "The answer.")
        .await
        .unwrap();

    let bounty = qa
        .create_bounty(NewBounty {
            question_id: question,
            amount: 100,
            expires_at: chrono::Utc::now() + Duration::days(7),
        })
        .await
        .unwrap();
    assert_eq!(bounty.status, "open");

    // A second bounty on the same question is refused.
    let err = qa
        .create_bounty(NewBounty {
            question_id: question,
            amount: 50,
            expires_at: chrono::Utc::now() + Duration::days(1),
        })
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        keystone_db::repositories::RepoError::UniqueViolation(_)
    ));

    // Awarding an answer from a different question is refused.
    let other_q = make_post(&pool, asker, "question", "other-bounty-q").await;
    let foreign = qa.create_answer(other_q, answerer, "other").await.unwrap();
    let err = qa.award_bounty(bounty.id, foreign.id).await.unwrap_err();
    assert!(matches!(
        err,
        keystone_db::repositories::RepoError::InvalidInput(_)
    ));

    // Award: open + unexpired + belongs → awarded.
    let awarded = qa
        .award_bounty(bounty.id, answer.id)
        .await
        .unwrap()
        .expect("awarded");
    assert_eq!(awarded.status, "awarded");
    assert_eq!(awarded.awarded_answer_id, Some(answer.id));

    // Idempotent: a second award call is a no-op.
    assert!(qa
        .award_bounty(bounty.id, answer.id)
        .await
        .unwrap()
        .is_none());

    // Expired bounties cannot be awarded; the sweeper flips them.
    let late = qa
        .create_bounty(NewBounty {
            question_id: other_q,
            amount: 10,
            expires_at: chrono::Utc::now() - Duration::hours(1),
        })
        .await
        .unwrap();
    assert!(qa
        .award_bounty(late.id, foreign.id)
        .await
        .unwrap()
        .is_none());
    let expired = qa.expire_overdue().await.unwrap();
    assert_eq!(expired, 1);
    assert_eq!(
        qa.bounty_for_question(other_q)
            .await
            .unwrap()
            .unwrap()
            .status,
        "expired"
    );
}

// ── Post locking ───────────────────────────────────────────────────────────

#[tokio::test]
async fn posts_lock_and_unlock_discussions() {
    let Some(pool) = test_pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let author = make_user(&pool, "locker@example.com").await;
    let posts = Posts::new(pool.clone());
    let discussion = make_post(&pool, author, "discussion", "lock-me").await;

    assert!(!posts.is_locked(discussion).await.unwrap());
    assert!(posts.lock(discussion).await.unwrap());
    assert!(posts.is_locked(discussion).await.unwrap());
    assert!(posts.unlock(discussion).await.unwrap());
    assert!(!posts.is_locked(discussion).await.unwrap());
}
