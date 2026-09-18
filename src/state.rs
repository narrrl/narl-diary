use std::sync::Arc;

use sqlx::SqlitePool;

use crate::{backup::Backup, config::Config, notify::Notifier, throttle::LoginThrottle};

#[derive(Clone)]
pub struct AppState {
    pub db: SqlitePool,
    pub config: Arc<Config>,
    pub login_throttle: Arc<LoginThrottle>,
    /// The Proton Drive mirror. Always present; dormant until it has a session.
    pub backup: Arc<Backup>,
    /// Notification mail. Always present; dormant until WORKSPACE_SMTP_URL is set.
    pub notify: Arc<Notifier>,
}
