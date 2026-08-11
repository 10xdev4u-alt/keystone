//! Month-6 repository tests against a real Postgres: courses/progress/
//! certificates (progress cannot lie), assessment attempts with anti-cheat,
//! the immutable credits ledger with double-spend defense, mentorship
//! request state machine, and idempotent event registration with waitlists.
//!
//! Self-skips when TEST_DATABASE_URL is unset.

use keystone_db::repositories::assessments::{AnswerInput, Assessments};
use keystone_db::repositories::credits::Credits;
use keystone_db::repositories::events::{Events, NewEvent};
use keystone_db::repositories::learning::{Learning, NewCourse};
use keystone_db::repositories::mentorship::Mentorship;
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
            first_name: Some("Test"),
            last_name: Some("User"),
            username: Some(email.split('@').next().unwrap()),
        })
        .await
        .expect("user must be created");
    user.id
}

/// A published course with one module of two lessons; returns course id.
async fn make_course(pool: &PgPool, author: Uuid, slug: &str) -> (Uuid, Uuid, Uuid) {
    let learning = Learning::new(pool.clone());
    let course = learning
        .create_course(NewCourse {
            author_id: author,
            title: "Rust 101",
            slug,
            description: None,
        })
        .await
        .unwrap();
    learning.publish_course(course.id, author).await.unwrap();
    let module = learning.add_module(course.id, 0, "Basics").await.unwrap();
    let l1 = learning
        .add_lesson(module.id, 0, "Hello", "x", None)
        .await
        .unwrap();
    let l2 = learning
        .add_lesson(module.id, 1, "Ownership", "y", None)
        .await
        .unwrap();
    (course.id, l1.id, l2.id)
}

#[tokio::test]
async fn course_completion_issues_certificate_atomically() {
    let Some(pool) = test_pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let author = make_user(&pool, "author@example.com").await;
    let student = make_user(&pool, "student@example.com").await;
    let learning = Learning::new(pool.clone());
    let (course_id, l1, l2) = make_course(&pool, author, "rust-101").await;

    // Enroll (idempotent) then complete the first lesson — no certificate.
    learning.enroll(course_id, student).await.unwrap();
    learning.enroll(course_id, student).await.unwrap();
    let cert = learning
        .complete_lesson(course_id, l1, student, "hash-1")
        .await
        .unwrap();
    assert!(cert.is_none(), "half-complete course issues nothing");
    assert_eq!(
        learning
            .progress_for(student, course_id)
            .await
            .unwrap()
            .len(),
        1
    );

    // Second lesson completes the course → certificate issued atomically.
    let cert = learning
        .complete_lesson(course_id, l2, student, "hash-1")
        .await
        .unwrap()
        .expect("certificate on completion");
    assert_eq!(cert.user_id, student);
    assert_eq!(cert.course_id, course_id);

    // Re-marking issues nothing new (unique user+course) — no double-issue.
    let again = learning
        .complete_lesson(course_id, l1, student, "hash-2")
        .await
        .unwrap();
    assert!(again.is_none(), "certificate must not double-issue");
    assert_eq!(
        learning.certificates_for_user(student).await.unwrap().len(),
        1
    );

    // Verification is hash-based and exact.
    assert!(learning
        .verify_certificate(student, course_id, "hash-1")
        .await
        .unwrap());
    assert!(!learning
        .verify_certificate(student, course_id, "hash-2")
        .await
        .unwrap());
    assert!(!learning
        .verify_certificate(student, course_id, "forged")
        .await
        .unwrap());

    // A lesson from a different course cannot be credited here.
    let (other_id, _, _) = make_course(&pool, author, "rust-102").await;
    let err = learning
        .complete_lesson(other_id, l1, student, "x")
        .await
        .unwrap_err();
    assert!(err.to_string().contains("does not belong"));
}

#[tokio::test]
async fn credit_redemption_cannot_double_spend() {
    let Some(pool) = test_pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let user = make_user(&pool, "spender@example.com").await;
    let credits = Credits::new(pool.clone());

    // Earn 10.
    credits
        .append(user, 10, "signup bonus", None, None)
        .await
        .unwrap();
    assert_eq!(credits.balance(user).await.unwrap(), 10);

    // 10 concurrent 3-credit redemptions — at most 3 can succeed.
    const REDEEMERS: usize = 10;
    const AMOUNT: i32 = 3;
    let mut handles = Vec::new();
    for _ in 0..REDEEMERS {
        let credits = Credits::new(pool.clone());
        handles.push(tokio::spawn(async move {
            credits
                .redeem(user, AMOUNT, "reward", None, None)
                .await
                .is_ok()
        }));
    }
    let mut ok = 0;
    for handle in handles {
        if handle.await.unwrap() {
            ok += 1;
        }
    }
    assert!(
        ok <= 3,
        "at most 3 redemptions of 3 from a balance of 10 (got {ok})"
    );
    let balance = credits.balance(user).await.unwrap();
    assert!(
        balance >= 0,
        "balance must never go negative (got {balance})"
    );
    assert_eq!(balance, 10 - ok as i64 * AMOUNT as i64, "ledger is exact");

    // The ledger is append-only: every entry is visible, nothing updated.
    let ledger = credits.ledger(user).await.unwrap();
    assert_eq!(ledger.len(), 1 + ok);
    assert!(ledger.iter().all(|e| e.delta != 0));

    // Overdraft is refused outright.
    assert!(credits
        .redeem(user, balance as i32 + 1, "too much", None, None)
        .await
        .is_err());
}

#[tokio::test]
async fn event_registration_is_idempotent_with_waitlist() {
    let Some(pool) = test_pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let organizer = make_user(&pool, "organizer@example.com").await;
    let events = Events::new(pool.clone());
    let event = events
        .create(NewEvent {
            organizer_id: organizer,
            title: "Rust Meetup",
            slug: "rust-meetup",
            description: None,
            starts_at: chrono::Utc::now() + chrono::Duration::days(7),
            ends_at: chrono::Utc::now() + chrono::Duration::days(7) + chrono::Duration::hours(2),
            capacity: Some(2),
            location: Some("Berlin"),
        })
        .await
        .unwrap();

    // Two seats: first two are registered, third goes to the waitlist.
    let a = make_user(&pool, "attendee-a@example.com").await;
    let b = make_user(&pool, "attendee-b@example.com").await;
    let c = make_user(&pool, "attendee-c@example.com").await;
    assert_eq!(events.register(event.id, a).await.unwrap(), "registered");
    assert_eq!(events.register(event.id, b).await.unwrap(), "registered");
    assert_eq!(events.register(event.id, c).await.unwrap(), "waitlisted");

    // Idempotency under concurrency: 20 duplicate registrations collapse.
    let mut handles = Vec::new();
    for _ in 0..20 {
        let events = Events::new(pool.clone());
        handles.push(tokio::spawn(async move {
            events.register(event.id, a).await.ok()
        }));
    }
    for handle in handles {
        handle.await.unwrap();
    }
    let rows = events.registrations(event.id).await.unwrap();
    assert_eq!(
        rows.len(),
        3,
        "duplicates must collapse to one row per user"
    );
    let a_row = rows.iter().find(|r| r.user_id == a).unwrap();
    assert_eq!(a_row.status, "registered");

    // Cancel a seat → the waitlist is promoted atomically.
    assert!(events.cancel_registration(event.id, a).await.unwrap());
    assert_eq!(
        events
            .registration_status(event.id, c)
            .await
            .unwrap()
            .unwrap(),
        "registered",
        "waitlisted attendee promoted on cancel"
    );
    // Cancelling a non-registered seat is a no-op.
    assert!(!events.cancel_registration(event.id, a).await.unwrap());

    // Speakers are additive and idempotent.
    events.add_speaker(event.id, a).await.unwrap();
    events.add_speaker(event.id, a).await.unwrap();
    assert_eq!(events.speakers(event.id).await.unwrap().len(), 1);
}

#[tokio::test]
async fn assessments_cap_attempts_and_grade_fairly() {
    let Some(pool) = test_pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let author = make_user(&pool, "assess-author@example.com").await;
    let student = make_user(&pool, "assess-student@example.com").await;
    let (course_id, _, _) = make_course(&pool, author, "assessed-course").await;
    let assessments = Assessments::new(pool.clone());    let assessment = assessments
        .create_assessment(course_id, "Rust basics", 50, Some(300))
        .await
        .unwrap();
    // The grading key is stored SERVER-SIDE — students never see or supply it.
    let q1 = assessments
        .add_question(assessment.id, 0, "1+1?", Some("2"))
        .await
        .unwrap();
    let q2 = assessments
        .add_question(assessment.id, 1, "capital of France?", Some("paris"))
        .await
        .unwrap();
    assert_eq!(assessments.questions(assessment.id).await.unwrap().len(), 2);

    // Half-correct = 50 → meets the 50 threshold.
    let attempt = assessments
        .start_attempt(assessment.id, student)
        .await
        .unwrap();
    let graded = assessments
        .submit_attempt(
            attempt.id,
            student,
            &[AnswerInput {
                question_id: q1.id,
                response: "2".into(),
            }],
        )
        .await
        .unwrap();
    assert_eq!(graded.score, Some(50));
    assert!(graded.passed.unwrap(), "50% meets the 50 threshold");

    // Re-submitting the same attempt is refused.
    assert!(assessments
        .submit_attempt(attempt.id, student, &[])
        .await
        .is_err());

    // Attempt cap: MAX_ATTEMPTS total, then the next start is refused.
    let a2 = assessments
        .start_attempt(assessment.id, student)
        .await
        .unwrap();
    assessments
        .submit_attempt(
            a2.id,
            student,
            &[
                AnswerInput {
                    question_id: q1.id,
                    response: "2".into(),
                },
                AnswerInput {
                    question_id: q2.id,
                    response: "paris".into(),
                },
            ],
        )
        .await
        .unwrap();
    let a3 = assessments
        .start_attempt(assessment.id, student)
        .await
        .unwrap();
    assessments
        .submit_attempt(
            a3.id,
            student,
            &[AnswerInput {
                question_id: q1.id,
                response: "1".into(),
            }],
        )
        .await
        .unwrap();
    assert!(assessments.start_attempt(assessment.id, student).await.is_err());
    assert_eq!(assessments.attempts_for(student, assessment.id).await.unwrap().len(), 3);

    // A wrong-only answer grades 0 → fails.
    let fresh = make_user(&pool, "assess-student2@example.com").await;
    let a = assessments
        .start_attempt(assessment.id, fresh)
        .await
        .unwrap();
    let graded = assessments
        .submit_attempt(
            a.id,
            fresh,
            &[AnswerInput {
                question_id: q1.id,
                response: "99".into(),
            }],
        )
        .await
        .unwrap();
    assert_eq!(graded.score, Some(0));
    assert!(!graded.passed.unwrap());
}

#[tokio::test]
async fn mentorship_state_machine_and_sessions() {
    let Some(pool) = test_pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let mentor = make_user(&pool, "mentor@example.com").await;
    let mentee = make_user(&pool, "mentee@example.com").await;
    let mentorship = Mentorship::new(pool.clone());

    mentorship
        .set_profile(mentor, Some("rust mentor"), Some("rust"), true)
        .await
        .unwrap();
    assert_eq!(mentorship.available_mentors().await.unwrap().len(), 1);

    let request = mentorship
        .request(mentor, mentee, Some("help!"))
        .await
        .unwrap();
    assert_eq!(request.status, "pending");

    // A session cannot be scheduled on a pending request.
    assert!(mentorship
        .schedule_session(request.id, chrono::Utc::now(), 30)
        .await
        .is_err());

    // Mentor accepts → session → complete → feedback.
    assert!(mentorship.accept(request.id, mentor).await.unwrap());
    assert!(
        !mentorship.accept(request.id, mentor).await.unwrap(),
        "one transition only"
    );
    let session = mentorship
        .schedule_session(request.id, chrono::Utc::now(), 30)
        .await
        .unwrap();
    assert_eq!(session.status, "scheduled");
    assert!(mentorship
        .complete_session(session.id, mentor)
        .await
        .unwrap());
    let feedback = mentorship
        .add_feedback(session.id, mentee, 5, Some("great"))
        .await
        .unwrap();
    assert_eq!(feedback.rating, 5);
    assert!(
        mentorship
            .add_feedback(session.id, mentee, 4, None)
            .await
            .is_err(),
        "one feedback per author"
    );

    // Goals attach to the request and flip to complete.
    let goal = mentorship
        .add_goal(request.id, "learn ownership")
        .await
        .unwrap();
    assert!(mentorship.complete_goal(goal.id, request.id).await.unwrap());
    assert!(mentorship.goals(request.id).await.unwrap()[0].completed);

    // A second request can be declined by the mentor or cancelled by the mentee.
    let declined = mentorship
        .request(mentor, mentee, Some("no thanks"))
        .await
        .unwrap();
    assert!(mentorship.decline(declined.id, mentor).await.unwrap());
    let cancelled = mentorship
        .request(mentor, mentee, Some("changed mind"))
        .await
        .unwrap();
    assert!(mentorship.cancel(cancelled.id, mentee).await.unwrap());
    assert_eq!(
        mentorship.requests_for_mentor(mentor).await.unwrap().len(),
        3
    );
}

#[tokio::test]
async fn learning_paths_track_course_progress() {
    let Some(pool) = test_pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let author = make_user(&pool, "path-author@example.com").await;
    let student = make_user(&pool, "path-student@example.com").await;
    let learning = Learning::new(pool.clone());
    let (c1, l1, l2) = make_course(&pool, author, "path-course-1").await;
    let (c2, _, _) = make_course(&pool, author, "path-course-2").await;

    let path = learning.create_path("Rust Track", None).await.unwrap();
    learning.add_path_course(path, c1, 0).await.unwrap();
    learning.add_path_course(path, c2, 1).await.unwrap();

    // Nothing complete yet.
    let courses = learning.path_courses(path, student).await.unwrap();
    assert_eq!(courses.len(), 2);
    assert_eq!(courses[0].0, c1);
    assert_eq!(courses[1].0, c2);

    // Complete one lesson of course 1 → its completed count goes to 1.
    learning
        .complete_lesson(c1, l1, student, "h")
        .await
        .unwrap();
    let courses = learning.path_courses(path, student).await.unwrap();
    assert_eq!(courses[0].2, 1, "course 1 has 1 completed lesson");

    // Complete course 1 fully.
    learning
        .complete_lesson(c1, l2, student, "h")
        .await
        .unwrap();
    let courses = learning.path_courses(path, student).await.unwrap();
    assert_eq!(courses[0].2, 2, "course 1 fully complete");
    assert_eq!(courses[1].2, 0, "course 2 untouched");
}
