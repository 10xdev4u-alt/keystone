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

    let state = AppState {
        pool,
        started_at: Instant::now(),
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

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to install ctrl-c handler");
    tracing::info!("shutdown signal received");
}
