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
        crate::content::create_post,
        crate::content::get_post,
        crate::content::list_posts,
        crate::content::update_post,
        crate::content::delete_post,
        crate::content::post_versions,
        crate::content::record_view,
        crate::content::create_comment,
        crate::content::list_comments,
        crate::content::delete_comment,
        crate::content::set_reaction,
        crate::content::remove_reaction,
        crate::content::get_reactions,
        crate::content::add_bookmark,
        crate::content::remove_bookmark,
        crate::content::my_bookmarks,
        crate::social::create_community,
        crate::social::get_community,
        crate::social::list_communities,
        crate::social::join_community,
        crate::social::leave_community,
        crate::social::list_members,
        crate::social::set_member_role,
        crate::social::add_community_post,
        crate::social::list_community_posts,
        crate::social::pin_community_post,
        crate::social::unpin_community_post,
        crate::social::remove_community_post,
        crate::social::add_poll_option,
        crate::social::vote_poll,
        crate::social::remove_poll_vote,
        crate::social::get_poll,
        crate::social::lock_post,
        crate::social::unlock_post,
        crate::qa::create_answer,
        crate::qa::list_answers,
        crate::qa::vote_answer,
        crate::qa::accept_answer,
        crate::qa::create_bounty,
        crate::qa::get_bounty,
        crate::qa::award_bounty,
    ),
    components(schemas(
        crate::auth::RegisterRequest,
        crate::auth::VerifyEmailRequest,
        crate::auth::LoginRequest,
        crate::auth::UserView,
        crate::auth::TokenResponse,
        crate::content::CreatePostRequest,
        crate::content::UpdatePostRequest,
        crate::content::CreateCommentRequest,
        crate::content::SetReactionRequest,
        crate::content::CreateReportRequest,
        crate::content::ResolveReportRequest,
        crate::content::UpsertReviewRequest,
        crate::content::PostQuery,
        crate::content::ReviewQuery,
        crate::social::CreateCommunityRequest,
        crate::social::AddCommunityPostRequest,
        crate::social::SetMemberRoleRequest,
        crate::social::AddPollOptionRequest,
        crate::social::VotePollRequest,
        crate::social::PageQuery,
        crate::qa::CreateAnswerRequest,
        crate::qa::VoteAnswerRequest,
        crate::qa::CreateBountyRequest,
        crate::qa::AwardBountyRequest,
    )),
    modifiers(&ApiDoc),
    tags(
        (name = "auth", description = "Authentication, sessions and account"),
        (name = "content", description = "Posts, comments, reactions, bookmarks"),
        (name = "social", description = "Communities, polls, locking"),
        (name = "qa", description = "Answers, voting, bounties"),
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
