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
        "RegisterRequest",
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
