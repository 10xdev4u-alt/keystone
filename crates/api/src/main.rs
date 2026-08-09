//! keystone API binary entrypoint.
#![forbid(unsafe_code)]

use keystone_api::{router, AppState};
use std::time::Instant;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load `.env` if present; real secrets are injected by the environment.
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,keystone=debug")),
        )
        .init();

    let database_url = env("DATABASE_URL");
    let max_connections: u32 = env_parse("DATABASE_MAX_CONNECTIONS").unwrap_or(10);
    let host = env_default("HOST", "0.0.0.0");
    let port = env_default("PORT", "4000");

    let pool = keystone_db::connect(&database_url, max_connections).await?;
    keystone_db::migrate(&pool).await?;
    tracing::info!("database connected and migrated");

    let addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("listening on {addr}");

    axum::serve(
        listener,
        router(AppState {
            pool,
            started_at: Instant::now(),
        }),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;

    Ok(())
}

fn env(key: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| panic!("required environment variable {key} is not set"))
}

fn env_default(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_owned())
}

fn env_parse<T: std::str::FromStr>(key: &str) -> Option<T> {
    std::env::var(key).ok().and_then(|v| v.parse().ok())
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to install ctrl-c handler");
    tracing::info!("shutdown signal received");
}
