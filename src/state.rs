use std::sync::Arc;

use sqlx::SqlitePool;

use crate::{backup::Backup, config::Config, throttle::LoginThrottle};

#[derive(Clone)]
pub struct AppState {
    pub db: SqlitePool,
    pub config: Arc<Config>,
    pub login_throttle: Arc<LoginThrottle>,
    /// The Proton Drive mirror. Always present; dormant until it has a session.
    pub backup: Arc<Backup>,
}
