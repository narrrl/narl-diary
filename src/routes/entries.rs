use axum::{
    extract::{Path, Query, State},
    Json,
};
use serde::{Deserialize, Serialize};
use sqlx::Row;

use crate::{
    auth::Session,
    error::{AppError, AppResult},
    routes::media,
    state::AppState,
};

#[derive(Serialize)]
pub struct EntrySummary {
    pub id: i64,
    pub title: String,
    pub excerpt: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub shared: bool,
    pub share_token: Option<String>,
}

#[derive(Serialize)]
pub struct Entry {
    pub id: i64,
    pub title: String,
    pub body: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub shared: bool,
    pub share_token: Option<String>,
}

#[derive(Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    pub q: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub offset: Option<i64>,
}

#[derive(Deserialize)]
pub struct EntryInput {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub body: String,
    /// Optional override so an entry can be backdated to the day it is about.
    #[serde(default)]
    pub created_at: Option<i64>,
}

/// A short, plain-ish preview of the body for the entry list.
fn excerpt(body: &str) -> String {
    let flat: String = body
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("![") && !line.starts_with("```"))
        .map(|line| line.trim_start_matches(['#', '>', '-', '*', ' ']))
        .collect::<Vec<_>>()
        .join(" ");
    let mut out: String = flat.chars().take(180).collect();
    if flat.chars().count() > 180 {
        out.push('…');
    }
    out
}

/// Turn free text into an FTS5 prefix query, discarding anything that could be
/// read as FTS syntax.
fn fts_query(raw: &str) -> Option<String> {
    let terms: Vec<String> = raw
        .split_whitespace()
        .map(|term| term.replace(|c: char| !c.is_alphanumeric(), " "))
        .flat_map(|term| {
            term.split_whitespace()
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .map(|term| format!("\"{term}\"*"))
        .collect();
    (!terms.is_empty()).then(|| terms.join(" AND "))
}

pub async fn list(
    _: Session,
    State(state): State<AppState>,
    Query(params): Query<ListQuery>,
) -> AppResult<Json<Vec<EntrySummary>>> {
    let limit = params.limit.unwrap_or(200).clamp(1, 1000);
    let offset = params.offset.unwrap_or(0).max(0);

    let search = params.q.as_deref().map(str::trim).filter(|q| !q.is_empty());

    let rows = match search.and_then(fts_query) {
        Some(query) => {
            sqlx::query(
                "SELECT e.id, e.title, e.body, e.created_at, e.updated_at, e.share_token
                 FROM entries_fts
                 JOIN entries e ON e.id = entries_fts.rowid
                 WHERE entries_fts MATCH ?1
                 ORDER BY rank
                 LIMIT ?2 OFFSET ?3",
            )
            .bind(query)
            .bind(limit)
            .bind(offset)
            .fetch_all(&state.db)
            .await?
        }
        None => {
            sqlx::query(
                "SELECT id, title, body, created_at, updated_at, share_token
                 FROM entries
                 ORDER BY created_at DESC, id DESC
                 LIMIT ?1 OFFSET ?2",
            )
            .bind(limit)
            .bind(offset)
            .fetch_all(&state.db)
            .await?
        }
    };

    let entries = rows
        .into_iter()
        .map(|row| {
            let body: String = row.get("body");
            let share_token: Option<String> = row.get("share_token");
            EntrySummary {
                id: row.get("id"),
                title: row.get("title"),
                excerpt: excerpt(&body),
                created_at: row.get("created_at"),
                updated_at: row.get("updated_at"),
                shared: share_token.is_some(),
                share_token,
            }
        })
        .collect();

    Ok(Json(entries))
}

pub async fn get(
    _: Session,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> AppResult<Json<Entry>> {
    load(&state, id).await.map(Json)
}

pub async fn create(
    _: Session,
    State(state): State<AppState>,
    Json(input): Json<EntryInput>,
) -> AppResult<Json<Entry>> {
    let now = crate::now();
    let created_at = input.created_at.unwrap_or(now);
    let id: i64 = sqlx::query(
        "INSERT INTO entries (title, body, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4)
         RETURNING id",
    )
    .bind(&input.title)
    .bind(&input.body)
    .bind(created_at)
    .bind(now)
    .fetch_one(&state.db)
    .await?
    .get("id");

    media::link_to_entry(&state.db, id, &input.body).await?;
    load(&state, id).await.map(Json)
}

pub async fn update(
    _: Session,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(input): Json<EntryInput>,
) -> AppResult<Json<Entry>> {
    let affected = sqlx::query(
        "UPDATE entries
         SET title = ?1, body = ?2, updated_at = ?3, created_at = COALESCE(?4, created_at)
         WHERE id = ?5",
    )
    .bind(&input.title)
    .bind(&input.body)
    .bind(crate::now())
    .bind(input.created_at)
    .bind(id)
    .execute(&state.db)
    .await?
    .rows_affected();

    if affected == 0 {
        return Err(AppError::NotFound);
    }

    media::link_to_entry(&state.db, id, &input.body).await?;
    load(&state, id).await.map(Json)
}

pub async fn remove(
    _: Session,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> AppResult<Json<serde_json::Value>> {
    let media_ids: Vec<String> = sqlx::query("SELECT id FROM media WHERE entry_id = ?1")
        .bind(id)
        .fetch_all(&state.db)
        .await?
        .into_iter()
        .map(|row| row.get("id"))
        .collect();

    let affected = sqlx::query("DELETE FROM entries WHERE id = ?1")
        .bind(id)
        .execute(&state.db)
        .await?
        .rows_affected();

    if affected == 0 {
        return Err(AppError::NotFound);
    }

    for media_id in media_ids {
        media::delete_media(&state, &media_id).await?;
    }

    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn load(state: &AppState, id: i64) -> AppResult<Entry> {
    let row = sqlx::query(
        "SELECT id, title, body, created_at, updated_at, share_token FROM entries WHERE id = ?1",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;

    let share_token: Option<String> = row.get("share_token");
    Ok(Entry {
        id: row.get("id"),
        title: row.get("title"),
        body: row.get("body"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        shared: share_token.is_some(),
        share_token,
    })
}
