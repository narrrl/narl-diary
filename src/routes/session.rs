use axum::{
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::json;

use crate::{
    auth::{self, Session},
    throttle::FREE_ATTEMPTS,
    error::{AppError, AppResult},
    state::AppState,
};

#[derive(Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

pub async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<LoginRequest>,
) -> AppResult<impl IntoResponse> {
    state.login_throttle.check().map_err(|wait| {
        tracing::warn!("login refused, {wait}s of backoff remaining");
        AppError::TooManyRequests(wait)
    })?;

    if !auth::credentials_match(&state.config, &body.username, &body.password) {
        let failures = state.login_throttle.record_failure();
        // Worth a mail once someone is past guessing and into trying.
        if failures > FREE_ATTEMPTS {
            state.notify.note_failed_logins(failures).await;
        }
        return Err(AppError::Unauthorized);
    }

    state.login_throttle.record_success();
    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown");
    state.notify.note_login(user_agent).await;

    let token = auth::issue_token(&state.config);
    Ok((
        StatusCode::OK,
        [(
            header::SET_COOKIE,
            auth::session_cookie(&state.config, &token),
        )],
        Json(json!({ "user": state.config.username })),
    ))
}

pub async fn logout(State(state): State<AppState>) -> impl IntoResponse {
    (
        StatusCode::OK,
        [(header::SET_COOKIE, auth::clear_cookie(&state.config))],
        Json(json!({ "ok": true })),
    )
}

pub async fn me(_: Session, State(state): State<AppState>) -> impl IntoResponse {
    Json(json!({ "user": state.config.username }))
}
