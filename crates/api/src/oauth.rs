//! OAuth 2.0 authorization-code login (Google first; provider-agnostic).
//!
//! Browser flow:
//!   1. `GET /api/v1/auth/oauth/google/start` → 302 to the provider's
//!      authorization URL. A random `state` is stored in a short-lived
//!      httpOnly SameSite=Lax cookie (Lax so it survives the cross-site
//!      redirect back).
//!   2. The provider redirects the browser to the callback with `code` and
//!      the echoed `state`.
//!   3. The callback validates `state` (constant-time compare), exchanges the
//!      code for an access token, fetches the userinfo, finds-or-creates the
//!      user, and issues a normal refresh session — the same refresh + CSRF
//!      cookies as password login. The browser lands on the SPA.
//!
//! No token ever appears in a URL or a log line: the access token from the
//! provider is used once for userinfo and discarded; the SPA obtains its own
//! access token via the normal refresh flow.

use crate::auth;
use crate::error::ApiError;
use axum::extract::{Query, State};
use axum::http::header::{self, HeaderValue};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Redirect, Response};
use base64::Engine;
use keystone_db::repositories::users::{User, Users};
use keystone_db::repositories::RepoError;
use serde::Deserialize;
use std::time::Duration;
use url::form_urlencoded;

const OAUTH_STATE_COOKIE: &str = "keystone_oauth_state";
const OAUTH_STATE_BYTES: usize = 32;
const OAUTH_STATE_TTL: std::time::Duration = std::time::Duration::from_secs(600);

/// Bounded upstream I/O: a hung provider must time out into a 502, never
/// hold a request open indefinitely (reqwest has no total timeout by
/// default).
const OAUTH_HTTP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Perform one provider call with a hard timeout. The future's error is
/// reqwest's — anything else is a 502 with the call labelled.
async fn provider_call<T, F>(label: &str, timeout: Duration, future: F) -> Result<T, ApiError>
where
    F: std::future::Future<Output = reqwest::Result<T>>,
{
    tokio::time::timeout(timeout, future)
        .await
        .map_err(|_| {
            tracing::error!(upstream = label, timeout_secs = ?timeout, "OAuth provider call timed out");
            ApiError::BadGateway(format!("OAuth {label} timed out"))
        })?
        .map_err(|e| {
            tracing::error!(upstream = label, error = %e, "OAuth provider call failed");
            ApiError::BadGateway(format!("OAuth {label} failed"))
        })
}

/// OAuth HTTP client + provider configuration, held in [`crate::AppState`].
#[derive(Clone)]
pub struct OAuthService {
    pub provider: keystone_config::OAuthProviderConfig,
    pub post_login_redirect: String,
    pub http: reqwest::Client,
}

impl OAuthService {
    pub fn new(
        provider: keystone_config::OAuthProviderConfig,
        post_login_redirect: String,
    ) -> Result<Self, ApiError> {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|e| {
                tracing::error!(error = %e, "failed to build OAuth HTTP client");
                ApiError::Internal
            })?;
        Ok(Self {
            provider,
            post_login_redirect,
            http,
        })
    }
}

#[derive(Debug, Deserialize)]
pub struct CallbackQuery {
    pub code: String,
    pub state: String,
}

/// Shape of the provider's token-endpoint response (success case).
#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
}

/// Shape of the provider's userinfo response (Google OpenID Connect).
#[derive(Debug, Deserialize)]
struct UserInfo {
    email: Option<String>,
    email_verified: Option<bool>,
    name: Option<String>,
}

/// 1. Redirect the browser to the provider's authorization endpoint.
pub async fn start(State(state): State<crate::AppState>) -> Result<Response, ApiError> {
    let oauth = state.oauth.as_ref().ok_or(ApiError::NotFound)?;

    let mut bytes = [0u8; OAUTH_STATE_BYTES];
    getrandom::fill(&mut bytes).map_err(|e| {
        tracing::error!(error = %e, "failed to generate OAuth state");
        ApiError::Internal
    })?;
    let state_token = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);

    let query = form_urlencoded::Serializer::new(String::new())
        .append_pair("client_id", &oauth.provider.client_id)
        .append_pair("redirect_uri", &oauth.provider.redirect_uri)
        .append_pair("response_type", "code")
        .append_pair("scope", &oauth.provider.scopes.join(" "))
        .append_pair("state", &state_token)
        .finish();
    let auth_url = format!("{}?{}", oauth.provider.auth_url, query);

    // 303 See Other: the browser should GET the authorization URL.
    let mut response = Redirect::to(&auth_url).into_response();
    response.headers_mut().append(
        header::SET_COOKIE,
        state_cookie(&state_token, state.auth.secure_cookies),
    );
    Ok(response)
}

/// 3. Provider callback: validate state, exchange code, provision the user.
pub async fn callback(
    State(state): State<crate::AppState>,
    headers: HeaderMap,
    Query(query): Query<CallbackQuery>,
) -> Result<Response, ApiError> {
    let oauth = state.oauth.as_ref().ok_or(ApiError::NotFound)?;

    // The state cookie must match the echoed state — constant-time compare.
    let cookie_state = read_state_cookie(&headers)
        .ok_or_else(|| ApiError::BadRequest("missing or expired OAuth state cookie".into()))?;
    if !ct_eq(&cookie_state, &query.state) {
        return Err(ApiError::BadRequest("OAuth state mismatch".into()));
    }

    // Exchange the authorization code for an access token — bounded by
    // OAUTH_HTTP_TIMEOUT; a hung provider yields 502, not a hung request.
    let exchange = oauth
        .http
        .post(&oauth.provider.token_url)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", &query.code),
            ("redirect_uri", &oauth.provider.redirect_uri),
            ("client_id", &oauth.provider.client_id),
            ("client_secret", &oauth.provider.client_secret),
        ])
        .send();
    let token_response: TokenResponse =
        provider_call::<reqwest::Response, _>("token exchange", OAUTH_HTTP_TIMEOUT, exchange)
            .await?
            .error_for_status()
            .map_err(|e| {
                tracing::error!(error = %e, "OAuth provider rejected the code");
                ApiError::BadGateway("OAuth provider rejected the authorization code".into())
            })?
            .json()
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "malformed OAuth token response");
                ApiError::BadGateway("malformed OAuth token response".into())
            })?;

    // Fetch the profile with the freshly exchanged token — same hard bound.
    let profile = oauth
        .http
        .get(&oauth.provider.userinfo_url)
        .bearer_auth(&token_response.access_token)
        .send();
    let userinfo: UserInfo =
        provider_call::<reqwest::Response, _>("userinfo", OAUTH_HTTP_TIMEOUT, profile)
            .await?
            .error_for_status()
            .map_err(|e| {
                tracing::error!(error = %e, "OAuth userinfo rejected");
                ApiError::BadGateway("OAuth userinfo rejected".into())
            })?
            .json()
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "malformed OAuth userinfo");
                ApiError::BadGateway("malformed OAuth userinfo".into())
            })?;

    let email = userinfo
        .email
        .as_deref()
        .ok_or_else(|| ApiError::BadRequest("OAuth provider returned no email".into()))?
        .to_owned();
    if !userinfo.email_verified.unwrap_or(false) {
        return Err(ApiError::BadRequest(
            "OAuth email is not verified by the provider".into(),
        ));
    }

    // Find-or-create by email (case-insensitive via the email_lower column).
    let users = Users::new(state.pool.clone());
    let (user, created) = match users
        .find_by_email(&email)
        .await
        .map_err(auth::map_repo_error)?
    {
        Some(user) => (user, false),
        None => {
            let user = create_oauth_user(&users, &email, &userinfo).await?;
            (user, true)
        }
    };
    if user.status != "active" {
        return Err(ApiError::Forbidden);
    }

    auth::audit(
        &state.pool,
        user.id,
        if created {
            "auth.oauth_signup"
        } else {
            "auth.oauth_login"
        },
        "user",
        &user.id.to_string(),
        auth::client_ip(&headers),
    )
    .await;

    // Same session shape as password login; the SPA then refreshes to get an
    // access token. No token is put in the redirect URL.
    let (refresh_token, csrf_token, _) =
        auth::issue_session_cookies(&state, user.id, &headers).await?;
    let mut response = Redirect::to(&oauth.post_login_redirect).into_response();
    response.headers_mut().append(
        header::SET_COOKIE,
        auth::refresh_cookie(
            &refresh_token,
            state.auth.refresh_ttl,
            state.auth.secure_cookies,
        ),
    );
    response.headers_mut().append(
        header::SET_COOKIE,
        crate::csrf::csrf_cookie(
            &csrf_token,
            state.auth.refresh_ttl,
            state.auth.secure_cookies,
        ),
    );
    Ok(response)
}

/// Provision a new user from OAuth userinfo. Username derives from the email
/// local-part; on collision a short random suffix is appended (bounded retry).
async fn create_oauth_user(
    users: &Users,
    email: &str,
    userinfo: &UserInfo,
) -> Result<User, ApiError> {
    let local = email.split('@').next().unwrap_or("user");
    let base: String = local
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .map(|c| c.to_ascii_lowercase())
        .take(30)
        .collect();
    let base = if base.is_empty() {
        "user".to_owned()
    } else {
        base
    };

    let (first_name, last_name) = split_name(userinfo.name.as_deref());

    for attempt in 0..3 {
        let username = if attempt == 0 {
            base.clone()
        } else {
            let mut suffix = [0u8; 3];
            getrandom::fill(&mut suffix).map_err(|_| ApiError::Internal)?;
            let suffix = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(suffix);
            format!("{base}-{suffix}")
        };
        match users
            .create_oauth(
                email,
                first_name.as_deref(),
                last_name.as_deref(),
                Some(&username),
                true,
            )
            .await
        {
            Ok(user) => return Ok(user),
            Err(RepoError::UniqueViolation(constraint)) if constraint == "users_username_key" => {}
            Err(err) => return Err(auth::map_repo_error(err)),
        }
    }
    Err(ApiError::Conflict(
        "could not allocate a unique username for OAuth signup".into(),
    ))
}

/// "Ada Lovelace" → (Some("Ada"), Some("Lovelace")); None → (None, None).
fn split_name(name: Option<&str>) -> (Option<String>, Option<String>) {
    match name {
        None => (None, None),
        Some(name) => {
            let mut parts = name.split_whitespace();
            let first = parts.next().map(str::to_owned);
            let last = parts.next().map(|rest| {
                // Join remaining words into the last name.
                std::iter::once(rest)
                    .chain(parts)
                    .collect::<Vec<_>>()
                    .join(" ")
            });
            (first, last)
        }
    }
}

fn state_cookie(value: &str, secure: bool) -> HeaderValue {
    let mut cookie = format!(
        "{OAUTH_STATE_COOKIE}={value}; Max-Age={}; Path=/api/v1/auth/oauth; HttpOnly; SameSite=Lax",
        OAUTH_STATE_TTL.as_secs()
    );
    if secure {
        cookie.push_str("; Secure");
    }
    HeaderValue::from_str(&cookie).expect("OAuth state cookie must be valid")
}

fn read_state_cookie(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    raw.split(';')
        .find_map(|part| part.trim().strip_prefix(&format!("{OAUTH_STATE_COOKIE}=")))
        .map(str::to_owned)
}

/// Constant-time string equality — OAuth state is high-entropy but timing-safe
/// comparison is the right habit for anything compared against a credential.
fn ct_eq(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_cookie_is_http_only_and_lax() {
        let value = state_cookie("abc", false);
        let raw = value.to_str().unwrap();
        assert!(raw.starts_with("keystone_oauth_state=abc;"));
        assert!(raw.contains("HttpOnly"));
        assert!(raw.contains("SameSite=Lax"));
        assert!(
            !raw.contains("Secure"),
            "insecure tests must not set Secure"
        );
        assert!(raw.contains("Max-Age=600"));
    }

    #[test]
    fn state_cookie_respects_secure_flag() {
        let value = state_cookie("abc", true);
        assert!(value.to_str().unwrap().contains("; Secure"));
    }

    #[test]
    fn state_cookie_round_trips() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("other=1; keystone_oauth_state=xyz; a=b"),
        );
        assert_eq!(read_state_cookie(&headers).as_deref(), Some("xyz"));
    }

    #[test]
    fn missing_state_cookie_reads_none() {
        let headers = HeaderMap::new();
        assert_eq!(read_state_cookie(&headers), None);
    }

    #[test]
    fn ct_eq_matches_and_rejects() {
        assert!(ct_eq("same-value", "same-value"));
        assert!(!ct_eq("same-value", "same-valuE"));
        assert!(!ct_eq("short", "longer-than-short"));
    }

    #[tokio::test]
    async fn provider_call_times_out_a_hung_upstream() {
        // A provider that never answers must yield 502, not hang the request.
        let err = provider_call("slow provider", Duration::from_millis(20), async {
            tokio::time::sleep(Duration::from_secs(5)).await;
            Ok::<(), _>(())
        })
        .await
        .unwrap_err();
        assert!(matches!(err, ApiError::BadGateway(_)));
    }

    #[tokio::test]
    async fn provider_call_propagates_upstream_success() {
        // A fast, successful call passes through untouched.
        let value = provider_call("fast provider", Duration::from_secs(1), async {
            Ok::<_, reqwest::Error>("all good")
        })
        .await
        .expect("fast provider must not fail");
        assert_eq!(value, "all good");
    }

    #[test]
    fn name_splits_first_and_last() {
        assert_eq!(
            split_name(Some("Ada Lovelace")),
            (Some("Ada".to_owned()), Some("Lovelace".to_owned()))
        );
        assert_eq!(
            split_name(Some("Grace Hopper")),
            (Some("Grace".to_owned()), Some("Hopper".to_owned()))
        );
        assert_eq!(split_name(Some("Linus")), (Some("Linus".to_owned()), None));
        assert_eq!(split_name(None), (None, None));
    }
}
