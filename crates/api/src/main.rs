//! Keystone API binary entrypoint.
#![forbid(unsafe_code)]

use keystone_api::{cors_layer, router, AppState};
use keystone_config::Config;
use std::time::Instant;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load `.env` if present; real secrets are injected by the environment.
    dotenvy::dotenv().ok();

    let config = Config::from_env()?;

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new(&config.log.filter))
        .init();

    let pool = keystone_db::connect(
        &config.database.url,
        config.database.max_connections,
        config.database.connect_timeout,
    )
    .await?;
    keystone_db::migrate(&pool).await?;
    tracing::info!("database connected and migrated");

    // Fail-fast auth construction: no JWT key, no boot. Keys load once here.
    let jwt_keys = keystone_auth::jwt::JwtKeys::from_config(&config.auth.jwt)
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let jwt = keystone_auth::jwt::AccessTokenService::new(&config.auth.jwt, jwt_keys);
    let password = keystone_auth::password::PasswordHasher::from_config(&config.auth.argon2)
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let lockout = keystone_auth::service::LockoutPolicy::new(
        5,
        std::time::Duration::from_secs(300),
        std::time::Duration::from_secs(60),
    );
    let auth = keystone_api::auth::AuthServices {
        password: std::sync::Arc::new(password),
        jwt: std::sync::Arc::new(jwt),
        lockout,
        access_ttl: std::time::Duration::from_secs(
            config.auth.jwt.access_expiration_secs.max(1) as u64
        ),
        refresh_ttl: std::time::Duration::from_secs(
            config.auth.jwt.refresh_expiration_secs.max(1) as u64
        ),
        secure_cookies: config.app.app_url.starts_with("https://"),
    };

    let oauth = match config.auth.oauth.google {
        Some(provider) => {
            let service = keystone_api::oauth::OAuthService::new(
                provider,
                config.auth.oauth.post_login_redirect.clone(),
            )
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
            tracing::info!("OAuth login enabled (Google)");
            Some(service)
        }
        None => None,
    };

    // Object storage: S3/MinIO when configured, in-process memory otherwise.
    // Fail-fast on an s3 backend with missing credentials — silent fallback
    // to memory would lose uploads on restart.
    let storage: std::sync::Arc<dyn keystone_db::storage::StorageBackend> =
        match config.storage.backend.as_str() {
            "s3" => std::sync::Arc::new(
                keystone_db::storage::S3Storage::from_env(&config.storage.bucket)
                    .map_err(|e| anyhow::anyhow!(e.to_string()))?,
            ),
            _ => std::sync::Arc::new(keystone_db::storage::MemoryStorage::new()),
        };

    let state = AppState {
        pool,
        started_at: Instant::now(),
        auth,
        rate_limit: std::sync::Arc::new(keystone_api::middleware::RateLimiter::new()),
        realtime: std::sync::Arc::new(keystone_api::realtime::RealtimeHub::new()),
        oauth,
        storage,
    };
    let mut app = router(state);
    if !config.app.cors_origins.is_empty() {
        app = app.layer(cors_layer(&config.app.cors_origins));
        tracing::info!(origins = ?config.app.cors_origins, "CORS enabled");
    }

    let addr = format!("{}:{}", config.server.host, config.server.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("listening on {addr}");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

/// Wait for Ctrl-C (interactive) or SIGTERM (container orchestrators), then
/// let axum's graceful shutdown drain in-flight requests.
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install ctrl-c handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => tracing::info!("ctrl-c received, shutting down"),
        _ = terminate => tracing::info!("SIGTERM received, shutting down"),
    }
}
