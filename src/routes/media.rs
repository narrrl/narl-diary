use std::sync::LazyLock;

use axum::{
    body::Body,
    extract::{Multipart, Path, State},
    http::header,
    response::{IntoResponse, Response},
    Json,
};
use regex::Regex;
use serde::Serialize;
use sqlx::{Row, SqlitePool};
use tokio_util::io::ReaderStream;
use uuid::Uuid;

use crate::{
    auth::Session,
    error::{AppError, AppResult},
    state::AppState,
};

/// Media referenced from an entry body always looks like `/api/media/<uuid>`,
/// which is how an entry claims ownership of the files it embeds.
static MEDIA_REF: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"/api/media/([0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12})")
        .expect("static regex is valid")
});

#[derive(Serialize)]
pub struct Media {
    pub id: String,
    pub filename: String,
    pub mime: String,
    pub size: i64,
    pub created_at: i64,
    pub url: String,
    pub entry_id: Option<i64>,
}

fn row_to_media(row: &sqlx::sqlite::SqliteRow) -> Media {
    let id: String = row.get("id");
    Media {
        url: format!("/api/media/{id}"),
        id,
        filename: row.get("filename"),
        mime: row.get("mime"),
        size: row.get("size"),
        created_at: row.get("created_at"),
        entry_id: row.get("entry_id"),
    }
}

pub async fn upload(
    _: Session,
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> AppResult<Json<Vec<Media>>> {
    let mut uploaded = Vec::new();

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("malformed upload: {e}")))?
    {
        let filename = field
            .file_name()
            .map(str::to_string)
            .unwrap_or_else(|| "upload".to_string());
        let mime = field
            .content_type()
            .map(str::to_string)
            .filter(|m| !m.is_empty())
            .unwrap_or_else(|| {
                mime_guess::from_path(&filename)
                    .first_or_octet_stream()
                    .to_string()
            });

        let data = field
            .bytes()
            .await
            .map_err(|e| AppError::BadRequest(format!("could not read upload: {e}")))?;

        if data.is_empty() {
            continue;
        }
        if data.len() > state.config.max_upload_bytes {
            return Err(AppError::BadRequest(format!(
                "{filename} is larger than the {} MB limit",
                state.config.max_upload_bytes / 1024 / 1024
            )));
        }

        let id = Uuid::new_v4().to_string();
        tokio::fs::write(state.config.uploads_dir().join(&id), &data).await?;

        let row = sqlx::query(
            "INSERT INTO media (id, filename, mime, size, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             RETURNING id, entry_id, filename, mime, size, created_at",
        )
        .bind(&id)
        .bind(&filename)
        .bind(&mime)
        .bind(data.len() as i64)
        .bind(crate::now())
        .fetch_one(&state.db)
        .await?;

        uploaded.push(row_to_media(&row));
    }

    if uploaded.is_empty() {
        return Err(AppError::BadRequest("no files in upload".into()));
    }
    Ok(Json(uploaded))
}

pub async fn list(_: Session, State(state): State<AppState>) -> AppResult<Json<Vec<Media>>> {
    let rows = sqlx::query(
        "SELECT id, entry_id, filename, mime, size, created_at
         FROM media ORDER BY created_at DESC LIMIT 500",
    )
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rows.iter().map(row_to_media).collect()))
}

pub async fn serve(
    _: Session,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Response> {
    stream_media(&state, &id).await
}

pub async fn remove(
    _: Session,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    delete_media(&state, &id).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// Shared by the private and the public-share media handlers.
pub async fn stream_media(state: &AppState, id: &str) -> AppResult<Response> {
    let row = sqlx::query("SELECT mime, filename, size FROM media WHERE id = ?1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(AppError::NotFound)?;

    let mime: String = row.get("mime");
    let filename: String = row.get("filename");
    let size: i64 = row.get("size");

    let file = tokio::fs::File::open(state.config.uploads_dir().join(id))
        .await
        .map_err(|_| AppError::NotFound)?;

    Ok((
        [
            (header::CONTENT_TYPE, mime),
            (header::CONTENT_LENGTH, size.to_string()),
            (header::CACHE_CONTROL, "private, max-age=31536000".into()),
            (
                header::CONTENT_DISPOSITION,
                format!("inline; filename*=UTF-8''{}", urlencode(&filename)),
            ),
        ],
        Body::from_stream(ReaderStream::new(file)),
    )
        .into_response())
}

pub async fn delete_media(state: &AppState, id: &str) -> AppResult<()> {
    sqlx::query("DELETE FROM media WHERE id = ?1")
        .bind(id)
        .execute(&state.db)
        .await?;
    let path = state.config.uploads_dir().join(id);
    if let Err(e) = tokio::fs::remove_file(&path).await {
        if e.kind() != std::io::ErrorKind::NotFound {
            return Err(e.into());
        }
    }
    Ok(())
}

/// Attach every media file the body embeds to this entry, and detach the ones
/// it no longer mentions so they stop being reachable through its share link.
pub async fn link_to_entry(db: &SqlitePool, entry_id: i64, body: &str) -> AppResult<()> {
    let referenced: Vec<String> = MEDIA_REF
        .captures_iter(body)
        .map(|c| c[1].to_lowercase())
        .collect();

    sqlx::query("UPDATE media SET entry_id = NULL WHERE entry_id = ?1")
        .bind(entry_id)
        .execute(db)
        .await?;

    for id in referenced {
        sqlx::query("UPDATE media SET entry_id = ?1 WHERE id = ?2")
            .bind(entry_id)
            .bind(id)
            .execute(db)
            .await?;
    }
    Ok(())
}

fn urlencode(value: &str) -> String {
    value
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}
