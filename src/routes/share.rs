use axum::{
    extract::{Path, State},
    response::Response,
    Json,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64, Engine};
use rand::RngCore;
use serde::Serialize;
use serde_json::json;
use sqlx::Row;

use crate::{
    auth::Session,
    error::{AppError, AppResult},
    routes::media,
    state::AppState,
};

#[derive(Serialize)]
pub struct SharedEntry {
    pub title: String,
    pub body: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub token: String,
}

fn new_token() -> String {
    let mut bytes = [0u8; 24];
    rand::thread_rng().fill_bytes(&mut bytes);
    B64.encode(bytes)
}

pub async fn enable(
    _: Session,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> AppResult<Json<serde_json::Value>> {
    let existing: Option<Option<String>> = sqlx::query("SELECT share_token FROM entries WHERE id = ?1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .map(|row| row.get("share_token"));

    let token = match existing {
        None => return Err(AppError::NotFound),
        Some(Some(token)) => token,
        Some(None) => {
            let token = new_token();
            sqlx::query("UPDATE entries SET share_token = ?1 WHERE id = ?2")
                .bind(&token)
                .bind(id)
                .execute(&state.db)
                .await?;
            token
        }
    };

    Ok(Json(json!({ "token": token, "path": format!("/s/{token}") })))
}

pub async fn disable(
    _: Session,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> AppResult<Json<serde_json::Value>> {
    let affected = sqlx::query("UPDATE entries SET share_token = NULL WHERE id = ?1")
        .bind(id)
        .execute(&state.db)
        .await?
        .rows_affected();
    if affected == 0 {
        return Err(AppError::NotFound);
    }
    Ok(Json(json!({ "ok": true })))
}

/// Public: no session required, the unguessable token is the credential.
pub async fn read(
    State(state): State<AppState>,
    Path(token): Path<String>,
) -> AppResult<Json<SharedEntry>> {
    let row = sqlx::query(
        "SELECT title, body, created_at, updated_at FROM entries WHERE share_token = ?1",
    )
    .bind(&token)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;

    Ok(Json(SharedEntry {
        title: row.get("title"),
        body: row.get("body"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        token,
    }))
}

/// Public: serves a media file only while it is embedded in a shared entry.
pub async fn serve_media(
    State(state): State<AppState>,
    Path((token, id)): Path<(String, String)>,
) -> AppResult<Response> {
    let allowed = sqlx::query(
        "SELECT 1 AS ok
         FROM media m
         JOIN entries e ON e.id = m.entry_id
         WHERE m.id = ?1 AND e.share_token = ?2",
    )
    .bind(&id)
    .bind(&token)
    .fetch_optional(&state.db)
    .await?
    .is_some();

    if !allowed {
        return Err(AppError::NotFound);
    }
    media::stream_media(&state, &id).await
}
