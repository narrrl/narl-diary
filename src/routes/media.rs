use std::{collections::BTreeSet, sync::LazyLock};

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

/// An upload is attacker-controlled content served from our own origin, so the
/// browser is only ever told it is a type that cannot execute script. Anything
/// else — `text/html`, `image/svg+xml`, an unrecognised type — is stored and
/// served as an opaque download instead.
fn sanitize_mime(raw: &str) -> String {
    let base = raw
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();

    let (top, sub) = base.split_once('/').unwrap_or_default();
    let inlineable = match top {
        // SVG is a document: it can carry <script> and same-origin markup.
        "image" => sub != "svg+xml" && !sub.is_empty(),
        "video" | "audio" => !sub.is_empty(),
        "text" => sub == "plain",
        "application" => sub == "pdf",
        _ => false,
    };

    if inlineable {
        base
    } else {
        "application/octet-stream".to_string()
    }
}

fn is_inline(mime: &str) -> bool {
    mime != "application/octet-stream"
}

#[derive(Serialize)]
pub struct Media {
    pub id: String,
    pub filename: String,
    pub mime: String,
    pub size: i64,
    pub created_at: i64,
    pub url: String,
    /// Every entry that embeds this file, which may be none or several.
    pub entry_ids: Vec<i64>,
}

fn row_to_media(row: &sqlx::sqlite::SqliteRow) -> Media {
    let id: String = row.get("id");
    // `group_concat` gives "3,7" or NULL; queries that do not ask for it at all
    // (a fresh upload) get an empty list.
    let entry_ids = row
        .try_get::<Option<String>, _>("entry_ids")
        .ok()
        .flatten()
        .map(|joined| joined.split(',').filter_map(|n| n.parse().ok()).collect())
        .unwrap_or_default();

    Media {
        url: format!("/api/media/{id}"),
        id,
        filename: row.get("filename"),
        mime: row.get("mime"),
        size: row.get("size"),
        created_at: row.get("created_at"),
        entry_ids,
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
        let mime = sanitize_mime(&mime);

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
             RETURNING id, filename, mime, size, created_at",
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
    state.backup.signal();
    Ok(Json(uploaded))
}

pub async fn list(_: Session, State(state): State<AppState>) -> AppResult<Json<Vec<Media>>> {
    let rows = sqlx::query(
        "SELECT m.id, m.filename, m.mime, m.size, m.created_at,
                (SELECT group_concat(em.entry_id)
                 FROM entry_media em WHERE em.media_id = m.id) AS entry_ids
         FROM media m ORDER BY m.created_at DESC LIMIT 500",
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
    state.backup.signal();
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// Shared by the private and the public-share media handlers.
pub async fn stream_media(state: &AppState, id: &str) -> AppResult<Response> {
    let row = sqlx::query("SELECT mime, filename, size FROM media WHERE id = ?1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(AppError::NotFound)?;

    let filename: String = row.get("filename");
    let size: i64 = row.get("size");
    // Re-checked on the way out, not just on the way in, so rows written before
    // the allowlist existed cannot be served as something executable either.
    let mime = sanitize_mime(&row.get::<String, _>("mime"));
    let disposition = if is_inline(&mime) { "inline" } else { "attachment" };

    let file = tokio::fs::File::open(state.config.uploads_dir().join(id))
        .await
        .map_err(|_| AppError::NotFound)?;

    Ok((
        [
            (header::CONTENT_TYPE, mime),
            (header::CONTENT_LENGTH, size.to_string()),
            (header::CACHE_CONTROL, "private, max-age=31536000".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!(
                    "{disposition}; filename*=UTF-8''{}",
                    urlencode(&filename)
                ),
            ),
            // Belt and braces: never let the browser sniff past the type above,
            // and strip the ambient authority of the origin from the response.
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_string()),
            (
                header::CONTENT_SECURITY_POLICY,
                "sandbox; default-src 'none'".to_string(),
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
/// Other entries keep whatever they embed; this only rewrites one entry's row.
pub async fn link_to_entry(db: &SqlitePool, entry_id: i64, body: &str) -> AppResult<()> {
    let referenced: BTreeSet<String> = MEDIA_REF
        .captures_iter(body)
        .map(|c| c[1].to_lowercase())
        .collect();

    // One transaction, so an entry is never momentarily attached to nothing —
    // which would blank its images for anyone reading its share link.
    let mut tx = db.begin().await?;

    sqlx::query("DELETE FROM entry_media WHERE entry_id = ?1")
        .bind(entry_id)
        .execute(&mut *tx)
        .await?;

    for id in &referenced {
        // Selecting from `media` rather than binding the id directly means a
        // body that still mentions a since-deleted file saves fine.
        sqlx::query(
            "INSERT OR IGNORE INTO entry_media (entry_id, media_id)
             SELECT ?1, id FROM media WHERE id = ?2",
        )
        .bind(entry_id)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(())
}

/// The files this entry embeds that no other entry does — the ones that become
/// unreachable once it is gone.
pub async fn exclusive_media(db: &SqlitePool, entry_id: i64) -> AppResult<Vec<String>> {
    let rows = sqlx::query(
        "SELECT em.media_id AS id
         FROM entry_media em
         WHERE em.entry_id = ?1
           AND NOT EXISTS (
               SELECT 1 FROM entry_media other
               WHERE other.media_id = em.media_id AND other.entry_id <> ?1
           )",
    )
    .bind(entry_id)
    .fetch_all(db)
    .await?;

    Ok(rows.iter().map(|row| row.get("id")).collect())
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

#[cfg(test)]
mod tests {
    use super::sanitize_mime;

    #[test]
    fn keeps_types_that_cannot_execute() {
        for mime in ["image/png", "image/jpeg", "video/mp4", "audio/ogg", "text/plain", "application/pdf"] {
            assert_eq!(sanitize_mime(mime), mime);
        }
    }

    #[test]
    fn neutralises_script_capable_types() {
        for mime in [
            "text/html",
            "image/svg+xml",
            "application/xhtml+xml",
            "application/javascript",
            "text/html; charset=utf-8",
            "",
            "nonsense",
        ] {
            assert_eq!(sanitize_mime(mime), "application/octet-stream", "{mime}");
        }
    }

    #[test]
    fn normalises_case_and_parameters() {
        assert_eq!(sanitize_mime("IMAGE/PNG; charset=binary"), "image/png");
    }
}
