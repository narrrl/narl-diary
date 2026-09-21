use axum::{
    extract::{Path, State},
    Json,
};
use serde::{Deserialize, Serialize};
use sqlx::Row;

use crate::{
    auth::Session,
    error::{AppError, AppResult},
    sftp::fingerprint_of,
    state::AppState,
};

#[derive(Serialize)]
pub struct SshKeySummary {
    pub id: i64,
    pub name: String,
    pub fingerprint: String,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
}

#[derive(Deserialize)]
pub struct AddInput {
    pub name: String,
    /// A line from `authorized_keys`: `ssh-ed25519 AAAA... comment`. The
    /// comment is accepted but not kept — it is not part of the key, and
    /// keeping it would make the same key compare unequal to itself.
    pub public_key: String,
}

pub async fn list(_: Session, State(state): State<AppState>) -> AppResult<Json<Vec<SshKeySummary>>> {
    let rows = sqlx::query(
        "SELECT id, name, fingerprint, created_at, last_used_at FROM ssh_keys ORDER BY created_at",
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(
        rows.iter()
            .map(|row| SshKeySummary {
                id: row.get("id"),
                name: row.get("name"),
                fingerprint: row.get("fingerprint"),
                created_at: row.get("created_at"),
                last_used_at: row.get("last_used_at"),
            })
            .collect(),
    ))
}

pub async fn add(
    _: Session,
    State(state): State<AppState>,
    Json(input): Json<AddInput>,
) -> AppResult<Json<SshKeySummary>> {
    let name = input.name.trim();
    if name.is_empty() {
        return Err(AppError::BadRequest("a key needs a name".into()));
    }

    let (normalized, fingerprint) = fingerprint_of(&input.public_key)
        .map_err(|e| AppError::BadRequest(format!("not a public key: {e}")))?;

    let now = crate::now();
    let id: i64 = sqlx::query(
        "INSERT INTO ssh_keys (name, public_key, fingerprint, created_at) VALUES (?1, ?2, ?3, ?4)
         RETURNING id",
    )
    .bind(name)
    .bind(&normalized)
    .bind(&fingerprint)
    .bind(now)
    .fetch_one(&state.db)
    .await
    .map_err(|e| match e {
        sqlx::Error::Database(db) if db.is_unique_violation() => {
            AppError::BadRequest("that key is already registered".into())
        }
        e => e.into(),
    })?
    .get("id");

    Ok(Json(SshKeySummary {
        id,
        name: name.to_string(),
        fingerprint,
        created_at: now,
        last_used_at: None,
    }))
}

pub async fn remove(
    _: Session,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> AppResult<Json<serde_json::Value>> {
    let affected = sqlx::query("DELETE FROM ssh_keys WHERE id = ?1")
        .bind(id)
        .execute(&state.db)
        .await?
        .rows_affected();

    if affected == 0 {
        return Err(AppError::NotFound);
    }
    Ok(Json(serde_json::json!({ "ok": true })))
}
