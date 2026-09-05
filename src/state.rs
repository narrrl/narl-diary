use std::sync::Arc;

use sqlx::SqlitePool;

use crate::{config::Config, throttle::LoginThrottle};

#[derive(Clone)]
pub struct AppState {
    pub db: SqlitePool,
    pub config: Arc<Config>,
    pub login_throttle: Arc<LoginThrottle>,
}
