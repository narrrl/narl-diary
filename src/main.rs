mod auth;
mod config;
mod error;
mod routes;
mod state;
mod static_files;

use std::sync::Arc;

use anyhow::{Context, Result};
use axum::Router;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use tower_http::{compression::CompressionLayer, trace::TraceLayer};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

use crate::{config::Config, state::AppState};

/// Seconds since the Unix epoch. Every timestamp in the database uses this.
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock is after 1970")
        .as_secs() as i64
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();

    tracing_subscriber::registry()
        .with(EnvFilter::try_from_env("DIARY_LOG").unwrap_or_else(|_| EnvFilter::new("info")))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let config = Config::from_env()?;
    tokio::fs::create_dir_all(config.uploads_dir())
        .await
        .with_context(|| format!("could not create {}", config.uploads_dir().display()))?;

    let db = SqlitePoolOptions::new()
        .max_connections(8)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(config.data_dir.join("diary.db"))
                .create_if_missing(true)
                .journal_mode(SqliteJournalMode::Wal)
                .foreign_keys(true),
        )
        .await
        .context("could not open the diary database")?;

    sqlx::migrate!("./migrations")
        .run(&db)
        .await
        .context("could not apply database migrations")?;

    let bind = config.bind;
    let max_upload = config.max_upload_bytes;
    let state = AppState {
        db,
        config: Arc::new(config),
    };

    let app = Router::new()
        .nest("/api", routes::api_router())
        .fallback(static_files::handler)
        .layer(axum::extract::DefaultBodyLimit::max(max_upload + 1024 * 1024))
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .with_context(|| format!("could not bind {bind}"))?;

    tracing::info!("narl-diary listening on http://{bind}");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("shutting down");
}
