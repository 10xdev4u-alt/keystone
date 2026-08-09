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
        })
    }
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
}
