mod auth;
mod config;
mod error;
mod routes;
mod state;
mod static_files;

use std::sync::Arc;

use anyhow::{Context, Result};
use axum::Router;
use sqlx::sqlite::{
    SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous,
};
use axum::http::{header, HeaderValue};
use tower_http::{
    compression::CompressionLayer, set_header::SetResponseHeaderLayer, trace::TraceLayer,
};
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
                // The standard pairing for WAL: fsync at checkpoints rather
                // than on every commit. A crash can cost the last transaction,
                // never the database.
                .synchronous(SqliteSynchronous::Normal)
                .busy_timeout(std::time::Duration::from_secs(10))
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
        .layer(security_headers())
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

/// Content-Security-Policy for the app shell. `unsafe-inline` is granted to
/// styles only, because CodeMirror injects its theme as a <style> element at
/// runtime; scripts stay confined to the bundled files.
const CSP: &str = "default-src 'self'; img-src 'self' data:; media-src 'self'; \
    style-src 'self' 'unsafe-inline'; script-src 'self'; connect-src 'self'; \
    object-src 'none'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'";

type HeaderLayer = SetResponseHeaderLayer<HeaderValue>;

/// Headers every response carries. `if_not_present` throughout, so a handler
/// that has already said something stricter about itself — the media routes and
/// their sandbox CSP — keeps its own answer.
fn security_headers() -> (HeaderLayer, HeaderLayer, HeaderLayer, HeaderLayer) {
    let header = |name: header::HeaderName, value: &'static str| {
        SetResponseHeaderLayer::if_not_present(name, HeaderValue::from_static(value))
    };

    (
        // A share token lives in the URL path, so an outbound link from a shared
        // entry would otherwise hand the whole private link to a third party.
        header(header::REFERRER_POLICY, "no-referrer"),
        header(header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        header(header::X_FRAME_OPTIONS, "DENY"),
        header(header::CONTENT_SECURITY_POLICY, CSP),
    )
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("shutting down");
}
