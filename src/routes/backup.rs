//! Looking at the Proton Drive mirror, and asking it to run now.

use axum::{extract::State, Json};

use crate::{auth::Session, backup::Status, error::AppResult, state::AppState};

pub async fn status(_: Session, State(state): State<AppState>) -> AppResult<Json<Status>> {
    Ok(Json(state.backup.status()))
}

/// Mirror now rather than when the diary next falls quiet. Answers with the
/// status the run produced, so `:backup` can report what it did.
pub async fn run(_: Session, State(state): State<AppState>) -> AppResult<Json<Status>> {
    state.backup.run_now().await?;
    Ok(Json(state.backup.status()))
}
