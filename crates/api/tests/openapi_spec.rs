//! OpenAPI document contract tests.
//!
//! No database required: the spec is generated at compile time from the
//! `#[utoipa::path]` annotations. These tests keep the generated frontend
//! contract honest — if a route or schema disappears, this suite fails.

use keystone_api::openapi::ApiDoc;
use utoipa::OpenApi;

fn doc() -> serde_json::Value {
    let json = ApiDoc::openapi().to_json().expect("spec must serialize");
    serde_json::from_str(&json).expect("spec must be valid JSON")
}

#[test]
fn spec_exposes_auth_paths() {
    let doc = doc();
    let paths = doc["paths"].as_object().expect("paths object");
    for route in [
        "/api/v1/auth/register",
        "/api/v1/auth/verify-email",
        "/api/v1/auth/login",
        "/api/v1/auth/refresh",
        "/api/v1/auth/logout",
        "/api/v1/auth/me",
        "/api/v1/auth/sessions",
        "/api/v1/auth/sessions/{id}",
    ] {
        assert!(paths.contains_key(route), "missing path {route}");
    }
}

#[test]
fn spec_exposes_content_social_qa_paths() {
    let doc = doc();
    let paths = doc["paths"].as_object().expect("paths object");
    for route in [
        "/api/v1/posts",
        "/api/v1/posts/{id}",
        "/api/v1/posts/{id}/versions",
        "/api/v1/posts/{id}/view",
        "/api/v1/posts/{id}/comments",
        "/api/v1/comments/{id}",
        "/api/v1/posts/{id}/reaction",
        "/api/v1/posts/{id}/reactions",
        "/api/v1/posts/{id}/bookmark",
        "/api/v1/me/bookmarks",
        "/api/v1/communities",
        "/api/v1/communities/{slug}",
        "/api/v1/communities/{slug}/join",
        "/api/v1/communities/{slug}/leave",
        "/api/v1/communities/{slug}/members",
        "/api/v1/communities/{slug}/members/{member_id}",
        "/api/v1/communities/{slug}/posts",
        "/api/v1/communities/{slug}/posts/{post_id}",
        "/api/v1/communities/{slug}/posts/{post_id}/pin",
        "/api/v1/posts/{id}/poll",
        "/api/v1/posts/{id}/poll/options",
        "/api/v1/posts/{id}/poll/votes",
        "/api/v1/posts/{id}/lock",
        "/api/v1/posts/{id}/answers",
        "/api/v1/answers/{id}/vote",
        "/api/v1/posts/{id}/answers/{answer_id}/accept",
        "/api/v1/posts/{id}/bounty",
        "/api/v1/bounties/{id}/award",
        "/api/v1/orgs",
        "/api/v1/orgs/{slug}",
        "/api/v1/orgs/{slug}/join",
        "/api/v1/orgs/{slug}/leave",
        "/api/v1/orgs/{slug}/members",
        "/api/v1/orgs/{slug}/members/{member_id}",
        "/api/v1/orgs/{slug}/claims",
        "/api/v1/orgs/{slug}/claims/{claim_id}/verify",
        "/api/v1/users/{user_id}/follow",
        "/api/v1/users/{user_id}/connect",
        "/api/v1/users/{user_id}/connections/accept",
        "/api/v1/users/{user_id}/connections/reject",
        "/api/v1/users/{user_id}/block",
        "/api/v1/me/following",
        "/api/v1/me/connections",
        "/api/v1/users/{user_id}/profile",
        "/api/v1/me/profile",
        "/api/v1/me/education",
        "/api/v1/me/education/{id}",
        "/api/v1/me/experience",
        "/api/v1/me/experience/{id}",
        "/api/v1/me/skills",
        "/api/v1/me/skills/{skill}",
        "/api/v1/salaries",
        "/api/v1/salaries/search",
        "/api/v1/orgs/{slug}/vendors",
        "/api/v1/orgs/{slug}/vendors/{listing_id}",
        "/api/v1/orgs/{slug}/vendors/{listing_id}/verify",
        "/api/v1/orgs/{slug}/alerts",
        "/api/v1/orgs/{slug}/alerts/{alert_id}/resolve",
        "/api/v1/career-paths",
        "/api/v1/career-paths/{path_id}",
        "/api/v1/me/assessments",
    ] {
        assert!(paths.contains_key(route), "missing path {route}");
    }
}

#[test]
fn spec_exposes_learning_realtime_files_moderation_paths() {
    let doc = doc();
    let paths = doc["paths"].as_object().expect("paths object");
    for route in [
        "/api/v1/courses",
        "/api/v1/courses/{slug}",
        "/api/v1/courses/{slug}/publish",
        "/api/v1/courses/{slug}/enroll",
        "/api/v1/courses/{slug}/modules",
        "/api/v1/courses/{slug}/modules/{module_id}/lessons",
        "/api/v1/courses/{slug}/lessons/{lesson_id}/complete",
        "/api/v1/courses/{slug}/progress",
        "/api/v1/me/certificates",
        "/api/v1/courses/{slug}/assessments",
        "/api/v1/courses/{slug}/assessments/{assessment_id}/questions",
        "/api/v1/assessments/{id}",
        "/api/v1/assessments/{id}/attempts",
        "/api/v1/attempts/{id}/submit",
        "/api/v1/me/credits",
        "/api/v1/me/credits/redeem",
        "/api/v1/me/credits/ledger",
        "/api/v1/me/mentor-profile",
        "/api/v1/mentors",
        "/api/v1/users/{mentor_id}/mentorship",
        "/api/v1/mentorship/{request_id}/accept",
        "/api/v1/mentorship/{request_id}/decline",
        "/api/v1/mentorship/{request_id}/sessions",
        "/api/v1/sessions/{session_id}/feedback",
        "/api/v1/events",
        "/api/v1/events/{slug}",
        "/api/v1/events/{slug}/register",
        "/api/v1/events/{slug}/registration",
        "/api/v1/events/{slug}/speakers/{speaker_id}",
        "/api/v1/notifications",
        "/api/v1/notifications/feed",
        "/api/v1/notifications/unread-count",
        "/api/v1/notifications/read",
        "/api/v1/notifications/preferences",
        "/api/v1/conversations",
        "/api/v1/conversations/{id}/messages",
        "/api/v1/conversations/{id}/read",
        "/api/v1/conversations/{id}/presence",
        "/api/v1/ws/chat/{id}",
        "/api/v1/files/presign",
        "/api/v1/files",
        "/api/v1/files/{id}",
        "/api/v1/reports",
        "/api/v1/reviews",
        "/api/v1/moderation/reports",
        "/api/v1/moderation/reports/{id}/resolve",
    ] {
        assert!(paths.contains_key(route), "missing path {route}");
    }
}

#[test]
fn spec_tags_each_operation() {
    let doc = doc();
    let paths = doc["paths"].as_object().unwrap();
    let mut tagged = 0;
    for op in paths.values() {
        for (method, operation) in op.as_object().unwrap() {
            if method == "parameters" {
                continue;
            }
            let tags = operation["tags"]
                .as_array()
                .expect("operation must be tagged");
            assert!(!tags.is_empty(), "operation {method} untagged");
            tagged += 1;
        }
    }
    assert!(
        tagged >= 105,
        "expected >= 105 tagged operations, got {tagged}"
    );
}

#[test]
fn spec_declares_bearer_security_scheme() {
    let doc = doc();
    let schemes = doc["components"]["securitySchemes"]
        .as_object()
        .expect("securitySchemes object");
    let scheme = schemes
        .get("bearer_auth")
        .expect("bearer_auth scheme registered");
    assert_eq!(scheme["type"], "http");
    assert_eq!(scheme["scheme"], "bearer");
    assert_eq!(scheme["bearerFormat"], "JWT");
}

#[test]
fn spec_documents_request_and_response_schemas() {
    let doc = doc();
    let schemas = doc["components"]["schemas"].as_object().expect("schemas");
    for schema in [
        "SignupRequest",
        "LoginRequest",
        "VerifyEmailRequest",
        "UserView",
        "TokenResponse",
    ] {
        assert!(schemas.contains_key(schema), "missing schema {schema}");
    }
}

#[test]
fn spec_marks_authenticated_routes_with_security() {
    let doc = doc();
    let paths = doc["paths"].as_object().unwrap();
    for route in ["/api/v1/auth/me", "/api/v1/auth/sessions"] {
        let operations = paths[route].as_object().unwrap();
        for (method, op) in operations {
            if method == "parameters" {
                continue;
            }
            let security = op["security"]
                .as_array()
                .expect("authenticated route must declare security");
            assert!(
                security
                    .iter()
                    .any(|req| req.as_object().unwrap().contains_key("bearer_auth")),
                "{route} missing bearer_auth security requirement"
            );
        }
    }
}
