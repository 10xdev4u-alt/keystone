//! Month-5 repository tests against a real Postgres: organizations +
//! membership roles + claims, the user_links social graph, profiles
//! (education/experience/skills), and careers (salary anonymization,
//! vendors, compliance, career paths).
//!
//! Self-skips when TEST_DATABASE_URL is unset.

use keystone_db::repositories::careers::{Careers, SalarySubmission};
use keystone_db::repositories::links::UserLinks;
use keystone_db::repositories::organizations::{NewOrganization, Organizations};
use keystone_db::repositories::profiles::{NewEducation, NewExperience, Profiles};
use keystone_db::repositories::users::{NewUser, Users};
use sha2::Digest;
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

#[tokio::test]
async fn organizations_membership_and_owner_transfer() {
    let Some(pool) = test_pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let owner = make_user(&pool, "owner@example.com").await;
    let member = make_user(&pool, "member@example.com").await;
    let orgs = Organizations::new(pool.clone());

    let org = orgs
        .create(NewOrganization {
            name: "Acme Corp",
            slug: "acme",
            description: None,
            website: Some("https://acme.example"),
            industry: Some("software"),
            created_by: owner,
        })
        .await
        .unwrap();
    assert_eq!(org.slug, "acme");
    assert_eq!(
        orgs.member_role(org.id, owner).await.unwrap().as_deref(),
        Some("owner")
    );

    // Join as a plain member; duplicate join is a no-op.
    orgs.join(org.id, member).await.unwrap();
    orgs.join(org.id, member).await.unwrap();
    assert_eq!(
        orgs.member_role(org.id, member).await.unwrap().as_deref(),
        Some("member")
    );

    // The sole owner cannot leave.
    let err = orgs.leave(org.id, owner).await.unwrap_err();
    assert!(err.to_string().contains("no owner"));

    // Transferring ownership demotes the current owner atomically.
    orgs.set_role(org.id, member, "owner").await.unwrap();
    assert_eq!(
        orgs.member_role(org.id, member).await.unwrap().as_deref(),
        Some("owner")
    );
    assert_eq!(
        orgs.member_role(org.id, owner).await.unwrap().as_deref(),
        Some("admin")
    );

    // Now the (former) owner can leave; the new owner still cannot.
    assert!(orgs.leave(org.id, owner).await.unwrap());
    let err = orgs.leave(org.id, member).await.unwrap_err();
    assert!(err.to_string().contains("no owner"));

    let members = orgs.members(org.id).await.unwrap();
    assert_eq!(members.len(), 1);
    assert_eq!(members[0].user_id, member);
    assert_eq!(members[0].role, "owner");
}

#[tokio::test]
async fn organization_claims_verify_hash_and_expiry() {
    let Some(pool) = test_pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let owner = make_user(&pool, "claim-owner@example.com").await;
    let claimant = make_user(&pool, "claimant@example.com").await;
    let orgs = Organizations::new(pool.clone());

    let org = orgs
        .create(NewOrganization {
            name: "Domain Ltd",
            slug: "domain-ltd",
            description: None,
            website: None,
            industry: None,
            created_by: owner,
        })
        .await
        .unwrap();

    // Only the HASH is stored — never the raw token.
    let raw_token = "s3cret-claim-token";
    let token_hash = hex::encode(sha2::Sha256::digest(raw_token.as_bytes()));
    let claim = orgs
        .create_claim(
            org.id,
            claimant,
            "example.com",
            &token_hash,
            chrono::Utc::now() + chrono::Duration::hours(1),
        )
        .await
        .unwrap();
    assert_eq!(claim.status, "pending");
    assert_eq!(claim.token_hash, token_hash);
    assert_ne!(claim.token_hash, raw_token);

    // Wrong hash → false; right hash → approved; already decided → false.
    let wrong_hash = hex::encode(sha2::Sha256::digest(b"wrong-token"));
    assert!(!orgs.verify_claim(claim.id, &wrong_hash).await.unwrap());
    assert!(orgs.verify_claim(claim.id, &token_hash).await.unwrap());
    assert!(
        !orgs.verify_claim(claim.id, &token_hash).await.unwrap(),
        "already decided"
    );

    // Expired claims are refused.
    let expired = orgs
        .create_claim(
            org.id,
            claimant,
            "expired.example",
            &token_hash,
            chrono::Utc::now() - chrono::Duration::minutes(1),
        )
        .await
        .unwrap();
    assert!(!orgs.verify_claim(expired.id, raw_token).await.unwrap());
}

#[tokio::test]
async fn user_links_follow_connect_and_block_semantics() {
    let Some(pool) = test_pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let alice = make_user(&pool, "alice@example.com").await;
    let bob = make_user(&pool, "bob@example.com").await;
    let links = UserLinks::new(pool.clone());

    // Follow is immediate and idempotent.
    links.follow(alice, bob).await.unwrap();
    links.follow(alice, bob).await.unwrap();
    assert_eq!(links.following(alice).await.unwrap(), vec![bob]);
    assert!(!links.are_blocked(alice, bob).await.unwrap());

    // Connect is a state machine: pending → accepted.
    links.connect(alice, bob).await.unwrap();
    assert_eq!(
        links.between(alice, bob).await.unwrap().unwrap().status,
        "pending"
    );
    assert!(links.connections(alice).await.unwrap().is_empty());
    assert!(links.accept(bob, alice).await.unwrap());
    assert_eq!(
        links.between(alice, bob).await.unwrap().unwrap().status,
        "accepted"
    );
    assert_eq!(links.connections(alice).await.unwrap(), vec![bob]);
    assert_eq!(links.connections(bob).await.unwrap(), vec![alice]);

    // Reject removes a pending link entirely.
    links.connect(bob, alice).await.unwrap();
    assert!(links.reject(alice, bob).await.unwrap());
    assert!(links.between(bob, alice).await.unwrap().is_none());

    // Block is mutual: blocking in EITHER direction separates both.
    links.block(bob, alice).await.unwrap();
    assert!(
        links.are_blocked(alice, bob).await.unwrap(),
        "blocked by target"
    );
    assert!(
        links.are_blocked(bob, alice).await.unwrap(),
        "blocker side too"
    );

    // Lifting the block restores visibility.
    assert!(links.remove(bob, alice, "block").await.unwrap());
    assert!(!links.are_blocked(alice, bob).await.unwrap());

    // Self-links are refused.
    assert!(links.follow(alice, alice).await.is_err());
    assert!(links.block(alice, alice).await.is_err());
}

#[tokio::test]
async fn profiles_education_experience_skills() {
    let Some(pool) = test_pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let user = make_user(&pool, "profiler@example.com").await;
    let profiles = Profiles::new(pool.clone());

    // Visibility vocabulary is enforced.
    assert!(profiles.set(user, Some("hi"), None, "nope").await.is_err());
    let profile = profiles
        .set(user, Some("hi"), Some("Berlin"), "connections")
        .await
        .unwrap();
    assert_eq!(profile.visibility, "connections");
    assert_eq!(
        profiles.get(user).await.unwrap().unwrap().bio.as_deref(),
        Some("hi")
    );

    // Education: ordering and removal.
    profiles
        .add_education(
            user,
            NewEducation {
                school: "TU Berlin",
                degree: Some("BSc"),
                field: Some("CS"),
                start_year: 2010,
                end_year: Some(2014),
                description: None,
            },
        )
        .await
        .unwrap();
    profiles
        .add_education(
            user,
            NewEducation {
                school: "MIT",
                degree: None,
                field: None,
                start_year: 2014,
                end_year: Some(2016),
                description: None,
            },
        )
        .await
        .unwrap();
    let education = profiles.education(user).await.unwrap();
    assert_eq!(education.len(), 2);
    assert_eq!(education[0].school, "MIT", "newest start year first");
    assert!(profiles
        .remove_education(user, education[1].id)
        .await
        .unwrap());
    assert_eq!(profiles.education(user).await.unwrap().len(), 1);

    // Experience: current entry has no end date.
    profiles
        .add_experience(
            user,
            NewExperience {
                organization_id: None,
                title: "Engineer",
                company: Some("Acme"),
                start_date: chrono::NaiveDate::from_ymd_opt(2020, 1, 1).unwrap(),
                end_date: None,
                current: true,
                description: None,
            },
        )
        .await
        .unwrap();
    let experience = profiles.experience(user).await.unwrap();
    assert_eq!(experience.len(), 1);
    assert!(experience[0].current);

    // Skills upsert by level.
    profiles.add_skill(user, "rust", "advanced").await.unwrap();
    profiles.add_skill(user, "rust", "expert").await.unwrap();
    profiles
        .add_skill(user, "sql", "intermediate")
        .await
        .unwrap();
    assert!(profiles.add_skill(user, "go", "guru").await.is_err());
    let skills = profiles.skills(user).await.unwrap();
    assert_eq!(skills.len(), 2, "rust upserted, not duplicated");
    let rust = skills.iter().find(|s| s.skill == "rust").unwrap();
    assert_eq!(rust.level, "expert");
    assert!(profiles.remove_skill(user, "sql").await.unwrap());
}

#[tokio::test]
async fn salary_buckets_grow_bounds_without_identity() {
    let Some(pool) = test_pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let careers = Careers::new(pool.clone());

    // Merge five anonymized submissions into one bucket.
    for amount in [90_000i64, 110_000, 100_000, 95_000, 120_000] {
        careers
            .merge_submission(&SalarySubmission {
                role: "Engineer".into(),
                location: Some("Berlin".into()),
                currency: "EUR".into(),
                amount,
            })
            .await
            .unwrap();
    }
    let bucket = careers
        .bucket("Engineer", Some("Berlin"), "EUR")
        .await
        .unwrap()
        .expect("bucket exists after submissions");
    assert_eq!(bucket.min_amount, 90_000);
    assert_eq!(bucket.max_amount, 120_000);
    assert_eq!(bucket.source_count, 5);
    assert!(bucket.median_amount >= bucket.min_amount && bucket.median_amount <= bucket.max_amount);

    // A different currency is a different bucket.
    assert!(careers
        .bucket("Engineer", Some("Berlin"), "USD")
        .await
        .unwrap()
        .is_none());

    // Invalid input is refused at the boundary.
    assert!(careers
        .merge_submission(&SalarySubmission {
            role: "Engineer".into(),
            location: None,
            currency: "USD".into(),
            amount: -1,
        })
        .await
        .is_err());

    // The row carries NO identity column — structurally un-deanonymizable.
    let columns: Vec<String> = sqlx::query_scalar(
        "SELECT column_name FROM information_schema.columns WHERE table_name = 'salary_benchmarks'",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert!(
        !columns
            .iter()
            .any(|c| c.contains("user") || c.contains("employer")),
        "salary rows must not reference any identity"
    );
}

#[tokio::test]
async fn vendors_compliance_career_paths_and_assessments() {
    let Some(pool) = test_pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set or unreachable");
        return;
    };
    let owner = make_user(&pool, "vendor-owner@example.com").await;
    let user = make_user(&pool, "assessor@example.com").await;
    let orgs = Organizations::new(pool.clone());
    let careers = Careers::new(pool.clone());

    let org = orgs
        .create(NewOrganization {
            name: "Vendor HQ",
            slug: "vendor-hq",
            description: None,
            website: None,
            industry: None,
            created_by: owner,
        })
        .await
        .unwrap();

    // Vendor lifecycle.
    let listing = careers
        .add_vendor(org.id, "security", Some("pentesting"))
        .await
        .unwrap();
    assert!(!listing.verified);
    assert!(careers.verify_vendor(listing.id).await.unwrap());
    assert!(careers.vendors(org.id).await.unwrap()[0].verified);
    assert!(careers.remove_vendor(org.id, listing.id).await.unwrap());
    assert!(careers.vendors(org.id).await.unwrap().is_empty());

    // Compliance alerts: severity vocabulary + resolve.
    assert!(careers
        .add_alert(org.id, "gdpr", "nuclear", "boom")
        .await
        .is_err());
    let alert = careers
        .add_alert(org.id, "gdpr", "critical", "data breach")
        .await
        .unwrap();
    assert_eq!(careers.alerts(org.id).await.unwrap().len(), 1);
    assert!(careers.resolve_alert(alert.id, org.id).await.unwrap());
    assert!(careers.alerts(org.id).await.unwrap()[0]
        .resolved_at
        .is_some());

    // Career paths with ordered, unique steps.
    let path = careers
        .add_career_path("Engineering", Some("the path"))
        .await
        .unwrap();
    careers.add_step(path.id, 0, "Junior", None).await.unwrap();
    careers.add_step(path.id, 1, "Senior", None).await.unwrap();
    assert!(careers
        .add_step(path.id, 1, "Duplicate", None)
        .await
        .is_err());
    let steps = careers.steps(path.id).await.unwrap();
    assert_eq!(steps.len(), 2);
    assert_eq!(steps[0].title, "Junior");

    // Self-assessments: score bounds enforced.
    assert!(careers
        .add_assessment(user, path.id, 9, None)
        .await
        .is_err());
    let assessment = careers
        .add_assessment(user, path.id, 4, Some("solid"))
        .await
        .unwrap();
    assert_eq!(assessment.score, 4);
    assert_eq!(careers.assessments(user).await.unwrap().len(), 1);
}
