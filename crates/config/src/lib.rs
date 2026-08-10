//! Typed environment configuration for Keystone.
//!
//! One source of truth for every runtime knob. Parsed once at boot, fail-fast:
//! a missing required variable or an unparsable value stops startup with a
//! precise message instead of failing half-way through a request.
//!
//! Env policy: no legacy deploy variables are carried over; new secrets/envs
//! are generated on purpose before production use. Only `.env.example`
//! templates live in the repository.

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::env;
use std::time::Duration;

/// Complete runtime configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub app: AppConfig,
    pub log: LogConfig,
    pub auth: AuthConfig,
}

/// Identity & session configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthConfig {
    pub argon2: Argon2Config,
    pub jwt: JwtConfig,
    pub oauth: OAuthConfig,
}

/// Third-party OAuth login. A provider is enabled only when its client
/// credentials are configured — partial configuration fails fast rather than
/// silently shipping a half-wired login.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OAuthConfig {
    pub google: Option<OAuthProviderConfig>,
    /// Where the browser lands after a successful OAuth login (the SPA).
    pub post_login_redirect: String,
}

/// One OAuth 2.0 authorization-code provider.
///
/// Endpoints are fully configurable: production points at the provider's real
/// URLs, tests point at a local mock — the flow is identical.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthProviderConfig {
    pub client_id: String,
    pub client_secret: String,
    pub auth_url: String,
    pub token_url: String,
    pub userinfo_url: String,
    pub redirect_uri: String,
    pub scopes: Vec<String>,
}

/// Argon2id password-hashing parameters (OWASP-aligned defaults).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Argon2Config {
    pub memory_kib: u32,
    pub iterations: u32,
    pub parallelism: u32,
}

/// JWT signing/verification configuration.
///
/// v1 signs with HS256: `private_key_b64`/`private_key_path` carry the shared
/// secret (base64 wins over file path). When an asymmetric upgrade lands, the
/// public key pair is added here without renaming these fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JwtConfig {
    pub issuer: String,
    pub audience: String,
    pub access_expiration_secs: i64,
    pub refresh_expiration_secs: i64,
    pub private_key_b64: Option<String>,
    pub private_key_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatabaseConfig {
    pub url: String,
    pub max_connections: u32,
    pub connect_timeout: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppConfig {
    pub app_url: String,
    pub api_url: String,
    pub cors_origins: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogConfig {
    pub filter: String,
}

/// Errors produced while loading configuration.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("missing required environment variable: {0}")]
    Missing(String),
    #[error("invalid value for {key}: {message}")]
    Invalid { key: String, message: String },
}

impl Config {
    /// Load from the process environment.
    pub fn from_env() -> Result<Config, ConfigError> {
        Self::from_source(&env::vars().collect())
    }

    /// Load from an explicit key/value source (testable without touching env).
    pub fn from_source(source: &HashMap<String, String>) -> Result<Config, ConfigError> {
        let get = |key: &str| source.get(key).cloned();

        let database_url =
            get("DATABASE_URL").ok_or_else(|| ConfigError::Missing("DATABASE_URL".into()))?;

        let max_connections = parse_u32(source, "DATABASE_MAX_CONNECTIONS", 10)?;
        let connect_timeout_ms = parse_u64(source, "PG_CONNECTION_TIMEOUT_MS", 30_000)?;

        let port = parse_u16(source, "PORT", 4000)?;
        let host = get("HOST").unwrap_or_else(|| "0.0.0.0".into());

        let app_url = get("APP_URL").unwrap_or_else(|| "http://localhost:5173".into());
        let api_url = get("API_URL").unwrap_or_else(|| "http://localhost:4000".into());
        let cors_origins = source
            .get("CORS_ORIGINS")
            .map(|v| {
                v.split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();

        let log_filter = get("RUST_LOG").unwrap_or_else(|| "info,keystone=debug".into());

        let oauth = parse_oauth(source, &app_url, &api_url)?;

        let auth = AuthConfig {
            argon2: Argon2Config {
                memory_kib: parse_u32(source, "ARGON2_MEMORY", 19_456)?,
                iterations: parse_u32(source, "ARGON2_ITERATIONS", 2)?,
                parallelism: parse_u32(source, "ARGON2_PARALLELISM", 1)?,
            },
            jwt: JwtConfig {
                issuer: get("JWT_ISSUER").unwrap_or_else(|| "keystone".into()),
                audience: get("JWT_AUDIENCE").unwrap_or_else(|| "keystone-api".into()),
                access_expiration_secs: parse_i64(source, "JWT_ACCESS_EXPIRATION", 900)?,
                refresh_expiration_secs: parse_i64(source, "JWT_REFRESH_EXPIRATION", 604_800)?,
                private_key_b64: get("JWT_PRIVATE_KEY_B64").filter(|v| !v.is_empty()),
                private_key_path: get("JWT_PRIVATE_KEY_PATH").filter(|v| !v.is_empty()),
            },
            oauth,
        };

        Ok(Config {
            server: ServerConfig { host, port },
            database: DatabaseConfig {
                url: database_url,
                max_connections,
                connect_timeout: Duration::from_millis(connect_timeout_ms),
            },
            app: AppConfig {
                app_url,
                api_url,
                cors_origins,
            },
            log: LogConfig { filter: log_filter },
            auth,
        })
    }
}

/// OAuth providers: enabled only when BOTH client id and secret are present;
/// a lone credential is a misconfiguration, not a feature toggle.
fn parse_oauth(
    source: &HashMap<String, String>,
    app_url: &str,
    api_url: &str,
) -> Result<OAuthConfig, ConfigError> {
    let get = |key: &str| source.get(key).cloned().filter(|v| !v.is_empty());

    let client_id = get("OAUTH_GOOGLE_CLIENT_ID");
    let client_secret = get("OAUTH_GOOGLE_CLIENT_SECRET");
    let google = match (client_id, client_secret) {
        (Some(client_id), Some(client_secret)) => {
            let auth_url = get("OAUTH_GOOGLE_AUTH_URL")
                .unwrap_or_else(|| "https://accounts.google.com/o/oauth2/v2/auth".into());
            let token_url = get("OAUTH_GOOGLE_TOKEN_URL")
                .unwrap_or_else(|| "https://oauth2.googleapis.com/token".into());
            let userinfo_url = get("OAUTH_GOOGLE_USERINFO_URL")
                .unwrap_or_else(|| "https://openidconnect.googleapis.com/v1/userinfo".into());
            let redirect_uri = get("OAUTH_GOOGLE_REDIRECT_URI")
                .unwrap_or_else(|| format!("{api_url}/api/v1/auth/oauth/google/callback"));
            let scopes = get("OAUTH_GOOGLE_SCOPES")
                .unwrap_or_else(|| "openid email profile".into())
                .split_whitespace()
                .map(str::to_owned)
                .collect();
            Some(OAuthProviderConfig {
                client_id,
                client_secret,
                auth_url,
                token_url,
                userinfo_url,
                redirect_uri,
                scopes,
            })
        }
        (None, None) => None,
        (Some(_), None) => {
            return Err(ConfigError::Invalid {
                key: "OAUTH_GOOGLE_CLIENT_SECRET".into(),
                message: "client id is set without a client secret; configure both or neither"
                    .into(),
            })
        }
        (None, Some(_)) => {
            return Err(ConfigError::Invalid {
                key: "OAUTH_GOOGLE_CLIENT_ID".into(),
                message: "client secret is set without a client id; configure both or neither"
                    .into(),
            })
        }
    };

    let post_login_redirect =
        get("APP_POST_LOGIN_REDIRECT").unwrap_or_else(|| format!("{app_url}/"));
    Ok(OAuthConfig {
        google,
        post_login_redirect,
    })
}

fn parse_u16(
    source: &HashMap<String, String>,
    key: &str,
    default: u16,
) -> Result<u16, ConfigError> {
    match source.get(key) {
        None => Ok(default),
        Some(raw) => raw.parse::<u16>().map_err(|_| ConfigError::Invalid {
            key: key.into(),
            message: format!("expected an integer in 0..65535, got {raw:?}"),
        }),
    }
}

fn parse_u32(
    source: &HashMap<String, String>,
    key: &str,
    default: u32,
) -> Result<u32, ConfigError> {
    match source.get(key) {
        None => Ok(default),
        Some(raw) => raw
            .parse::<u32>()
            .map(|v| v.max(1))
            .map_err(|_| ConfigError::Invalid {
                key: key.into(),
                message: format!("expected a positive integer, got {raw:?}"),
            }),
    }
}

fn parse_u64(
    source: &HashMap<String, String>,
    key: &str,
    default: u64,
) -> Result<u64, ConfigError> {
    match source.get(key) {
        None => Ok(default),
        Some(raw) => raw.parse::<u64>().map_err(|_| ConfigError::Invalid {
            key: key.into(),
            message: format!("expected an integer, got {raw:?}"),
        }),
    }
}

fn parse_i64(
    source: &HashMap<String, String>,
    key: &str,
    default: i64,
) -> Result<i64, ConfigError> {
    match source.get(key) {
        None => Ok(default),
        Some(raw) => raw.parse::<i64>().map_err(|_| ConfigError::Invalid {
            key: key.into(),
            message: format!("expected an integer, got {raw:?}"),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(entries: &[(&str, &str)]) -> HashMap<String, String> {
        entries
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn missing_database_url_fails_fast() {
        let err = Config::from_source(&env(&[])).unwrap_err();
        assert!(err.to_string().contains("DATABASE_URL"));
    }

    #[test]
    fn defaults_apply_when_unset() {
        let cfg =
            Config::from_source(&env(&[("DATABASE_URL", "postgres://localhost/db")])).unwrap();
        assert_eq!(cfg.server.host, "0.0.0.0");
        assert_eq!(cfg.server.port, 4000);
        assert_eq!(cfg.database.max_connections, 10);
        assert_eq!(cfg.database.connect_timeout, Duration::from_millis(30_000));
        assert!(cfg.app.cors_origins.is_empty());
        assert_eq!(cfg.log.filter, "info,keystone=debug");
    }

    #[test]
    fn explicit_values_are_parsed() {
        let cfg = Config::from_source(&env(&[
            ("DATABASE_URL", "postgres://localhost/db"),
            ("PORT", "8080"),
            ("HOST", "127.0.0.1"),
            ("DATABASE_MAX_CONNECTIONS", "25"),
            ("PG_CONNECTION_TIMEOUT_MS", "5000"),
            ("APP_URL", "https://keystone.example"),
            ("API_URL", "https://api.keystone.example"),
            ("CORS_ORIGINS", "https://a.example, https://b.example"),
            ("RUST_LOG", "debug"),
        ]))
        .unwrap();
        assert_eq!(cfg.server.port, 8080);
        assert_eq!(cfg.server.host, "127.0.0.1");
        assert_eq!(cfg.database.max_connections, 25);
        assert_eq!(cfg.database.connect_timeout, Duration::from_millis(5000));
        assert_eq!(
            cfg.app.cors_origins,
            vec!["https://a.example", "https://b.example"]
        );
        assert_eq!(cfg.log.filter, "debug");
    }

    #[test]
    fn invalid_port_is_rejected() {
        let err = Config::from_source(&env(&[("DATABASE_URL", "x"), ("PORT", "not-a-port")]))
            .unwrap_err();
        assert!(err.to_string().contains("PORT"));
    }

    #[test]
    fn auth_defaults_apply_when_unset() {
        let cfg =
            Config::from_source(&env(&[("DATABASE_URL", "postgres://localhost/db")])).unwrap();
        assert_eq!(cfg.auth.argon2.memory_kib, 19_456);
        assert_eq!(cfg.auth.argon2.iterations, 2);
        assert_eq!(cfg.auth.argon2.parallelism, 1);
        assert_eq!(cfg.auth.jwt.issuer, "keystone");
        assert_eq!(cfg.auth.jwt.audience, "keystone-api");
        assert_eq!(cfg.auth.jwt.access_expiration_secs, 900);
        assert_eq!(cfg.auth.jwt.refresh_expiration_secs, 604_800);
        assert_eq!(cfg.auth.jwt.private_key_b64, None);
        assert_eq!(cfg.auth.jwt.private_key_path, None);
    }

    #[test]
    fn auth_values_are_parsed() {
        let cfg = Config::from_source(&env(&[
            ("DATABASE_URL", "postgres://localhost/db"),
            ("ARGON2_MEMORY", "65536"),
            ("ARGON2_ITERATIONS", "3"),
            ("ARGON2_PARALLELISM", "4"),
            ("JWT_ISSUER", "keystone-prod"),
            ("JWT_AUDIENCE", "keystone-api-prod"),
            ("JWT_ACCESS_EXPIRATION", "600"),
            ("JWT_REFRESH_EXPIRATION", "1209600"),
            ("JWT_PRIVATE_KEY_B64", "c2VjcmV0"),
            ("JWT_PRIVATE_KEY_PATH", "./keys/private.pem"),
        ]))
        .unwrap();
        assert_eq!(cfg.auth.argon2.memory_kib, 65_536);
        assert_eq!(cfg.auth.jwt.issuer, "keystone-prod");
        assert_eq!(cfg.auth.jwt.access_expiration_secs, 600);
        assert_eq!(cfg.auth.jwt.private_key_b64.as_deref(), Some("c2VjcmV0"));
        assert_eq!(
            cfg.auth.jwt.private_key_path.as_deref(),
            Some("./keys/private.pem")
        );
    }

    #[test]
    fn invalid_expiration_is_rejected() {
        let err = Config::from_source(&env(&[
            ("DATABASE_URL", "x"),
            ("JWT_ACCESS_EXPIRATION", "soon"),
        ]))
        .unwrap_err();
        assert!(err.to_string().contains("JWT_ACCESS_EXPIRATION"));
    }

    #[test]
    fn oauth_disabled_by_default() {
        let cfg =
            Config::from_source(&env(&[("DATABASE_URL", "postgres://localhost/db")])).unwrap();
        assert!(cfg.auth.oauth.google.is_none());
        assert_eq!(cfg.auth.oauth.post_login_redirect, "http://localhost:5173/");
    }

    #[test]
    fn oauth_enabled_with_both_credentials() {
        let cfg = Config::from_source(&env(&[
            ("DATABASE_URL", "postgres://localhost/db"),
            ("OAUTH_GOOGLE_CLIENT_ID", "client-123"),
            ("OAUTH_GOOGLE_CLIENT_SECRET", "secret-456"),
            ("API_URL", "https://api.example.com"),
            ("OAUTH_GOOGLE_SCOPES", "openid email"),
            ("APP_POST_LOGIN_REDIRECT", "https://app.example.com/welcome"),
        ]))
        .unwrap();
        let google = cfg.auth.oauth.google.expect("enabled");
        assert_eq!(google.client_id, "client-123");
        assert_eq!(google.client_secret, "secret-456");
        // Default redirect derived from the API URL.
        assert_eq!(
            google.redirect_uri,
            "https://api.example.com/api/v1/auth/oauth/google/callback"
        );
        // Google production defaults apply when unset.
        assert!(google.auth_url.contains("accounts.google.com"));
        assert!(google.token_url.contains("googleapis.com"));
        assert_eq!(google.scopes, vec!["openid", "email"]);
        assert_eq!(
            cfg.auth.oauth.post_login_redirect,
            "https://app.example.com/welcome"
        );
    }

    #[test]
    fn oauth_partial_config_fails_fast() {
        let err = Config::from_source(&env(&[
            ("DATABASE_URL", "postgres://localhost/db"),
            ("OAUTH_GOOGLE_CLIENT_ID", "client-123"),
        ]))
        .unwrap_err();
        assert!(err.to_string().contains("OAUTH_GOOGLE_CLIENT_SECRET"));
    }

    #[test]
    fn oauth_endpoints_are_configurable() {
        let cfg = Config::from_source(&env(&[
            ("DATABASE_URL", "postgres://localhost/db"),
            ("OAUTH_GOOGLE_CLIENT_ID", "c"),
            ("OAUTH_GOOGLE_CLIENT_SECRET", "s"),
            ("OAUTH_GOOGLE_AUTH_URL", "http://127.0.0.1:9000/auth"),
            ("OAUTH_GOOGLE_TOKEN_URL", "http://127.0.0.1:9000/token"),
            (
                "OAUTH_GOOGLE_USERINFO_URL",
                "http://127.0.0.1:9000/userinfo",
            ),
        ]))
        .unwrap();
        let google = cfg.auth.oauth.google.expect("enabled");
        assert_eq!(google.auth_url, "http://127.0.0.1:9000/auth");
        assert_eq!(google.token_url, "http://127.0.0.1:9000/token");
        assert_eq!(google.userinfo_url, "http://127.0.0.1:9000/userinfo");
    }
}
