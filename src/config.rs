use std::{net::SocketAddr, path::PathBuf};

use anyhow::{Context, Result};

/// Runtime configuration. Everything comes from the environment (`.env` is
/// loaded on startup), because this is a deliberately single-user application.
#[derive(Debug, Clone)]
pub struct Config {
    pub username: String,
    pub password: String,
    pub secret: Vec<u8>,
    pub bind: SocketAddr,
    pub data_dir: PathBuf,
    pub session_days: i64,
    pub max_upload_bytes: usize,
    pub secure_cookie: bool,
}

fn var(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.trim().is_empty())
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let username = var("DIARY_USER").context("DIARY_USER must be set (see .env.example)")?;
        let password =
            var("DIARY_PASSWORD").context("DIARY_PASSWORD must be set (see .env.example)")?;
        let secret = var("DIARY_SECRET")
            .context("DIARY_SECRET must be set (see .env.example)")?
            .into_bytes();
        if secret.len() < 16 {
            anyhow::bail!("DIARY_SECRET must be at least 16 characters");
        }

        if password == "change-me" {
            tracing::warn!(
                "DIARY_PASSWORD is still the example value from .env.example — \
                 anyone who can reach this server can read the diary"
            );
        }

        let bind = var("DIARY_BIND")
            .unwrap_or_else(|| "127.0.0.1:4242".to_string())
            .parse()
            .context("DIARY_BIND must look like 127.0.0.1:4242")?;

        let data_dir = PathBuf::from(var("DIARY_DATA_DIR").unwrap_or_else(|| "./data".into()));

        let session_days = var("DIARY_SESSION_DAYS")
            .map(|v| v.parse())
            .transpose()
            .context("DIARY_SESSION_DAYS must be a number")?
            .unwrap_or(30);

        let max_upload_bytes = var("DIARY_MAX_UPLOAD_MB")
            .map(|v| v.parse::<usize>())
            .transpose()
            .context("DIARY_MAX_UPLOAD_MB must be a number")?
            .unwrap_or(64)
            * 1024
            * 1024;

        let secure_cookie = var("DIARY_SECURE_COOKIE")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);

        Ok(Self {
            username,
            password,
            secret,
            bind,
            data_dir,
            session_days,
            max_upload_bytes,
            secure_cookie,
        })
    }

    pub fn uploads_dir(&self) -> PathBuf {
        self.data_dir.join("uploads")
    }
}
