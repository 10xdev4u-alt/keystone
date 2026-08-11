//! OpenAPI (Swagger) document for the Keystone HTTP API.
//!
//! The spec is the single source of truth for the frontend: the TanStack Query
//! client is generated from `/openapi.json` (see `web/`), keeping zero
//! hand-written API types. Every handler carries `#[utoipa::path]` so the
//! document stays in sync with the router by construction.

use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::{Modify, OpenApi};

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Keystone API",
        version = "1.0.0",
        description = "Community platform API. All errors are RFC 7807 problem+json. \
                       Authenticated endpoints take `Authorization: Bearer <access-token>`; \
                       refresh/auth cookies are httpOnly SameSite=Strict and never read by JS.",
    ),
    servers(
        (url = "/", description = "Same-origin deployment")
    ),
    paths(
        crate::auth::register,
        crate::auth::verify_email,
        crate::auth::login,
        crate::auth::refresh,
        crate::auth::logout,
        crate::auth::me,
        crate::auth::list_sessions,
        crate::auth::revoke_session,
        crate::auth::revoke_all_sessions,
    ),
    components(schemas(
        crate::auth::RegisterRequest,
        crate::auth::VerifyEmailRequest,
        crate::auth::LoginRequest,
        crate::auth::UserView,
        crate::auth::TokenResponse,
    )),
    modifiers(&ApiDoc),
    tags(
        (name = "auth", description = "Authentication, sessions and account"),
    )
)]
pub struct ApiDoc;

impl Modify for ApiDoc {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        if let Some(components) = openapi.components.as_mut() {
            components.add_security_scheme(
                "bearer_auth",
                SecurityScheme::Http(
                    HttpBuilder::new()
                        .scheme(HttpAuthScheme::Bearer)
                        .bearer_format("JWT")
                        .build(),
                ),
            );
        }
    }
}
