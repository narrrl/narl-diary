use axum::{
    extract::{Path, State},
    response::Response,
    Json,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64, Engine};
use rand::RngCore;
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::Row;
use subtle::ConstantTimeEq;

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

fn random_secret() -> String {
    let mut bytes = [0u8; 24];
    rand::thread_rng().fill_bytes(&mut bytes);
    B64.encode(bytes)
}

fn hash_key(key: &str) -> String {
    B64.encode(Sha256::digest(key.as_bytes()))
}

/// Constant-time check of a presented key against the stored hash. A share with
/// no hash at all is unreadable rather than open: that combination should not
/// exist, and failing closed is the safe reading of it.
fn key_matches(stored: Option<&str>, presented: &str) -> bool {
    let Some(stored) = stored else { return false };
    bool::from(hash_key(presented).as_bytes().ct_eq(stored.as_bytes()))
}

/// Publish an entry, or mint a fresh key for one that is already published.
///
/// The plaintext key is returned exactly once, here. Nothing but its hash is
/// stored, so a link that the author loses cannot be recovered — only replaced,
/// which is what calling this again does. Rotating the key kills the old link
/// while leaving the token, so a revoked reader loses access even if they kept
/// the path.
pub async fn enable(
    _: Session,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> AppResult<Json<serde_json::Value>> {
    let existing: Option<Option<String>> =
        sqlx::query("SELECT share_token FROM entries WHERE id = ?1")
            .bind(id)
            .fetch_optional(&state.db)
            .await?
            .map(|row| row.get("share_token"));

    let token = match existing {
        None => return Err(AppError::NotFound),
        Some(Some(token)) => token,
        Some(None) => random_secret(),
    };
    let key = random_secret();

    sqlx::query("UPDATE entries SET share_token = ?1, share_key_hash = ?2 WHERE id = ?3")
        .bind(&token)
        .bind(hash_key(&key))
        .bind(id)
        .execute(&state.db)
        .await?;

    state.backup.signal();
    Ok(Json(json!({
        "token": token,
        "key": key,
        "path": format!("/s/{token}#{key}"),
    })))
}

pub async fn disable(
    _: Session,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> AppResult<Json<serde_json::Value>> {
    let affected =
        sqlx::query("UPDATE entries SET share_token = NULL, share_key_hash = NULL WHERE id = ?1")
            .bind(id)
            .execute(&state.db)
            .await?
            .rows_affected();
    if affected == 0 {
        return Err(AppError::NotFound);
    }
    state.backup.signal();
    Ok(Json(json!({ "ok": true })))
}

/// Public: no session required. The credential is the pair — the token from the
/// link's path and the key the reader's browser lifted out of its fragment.
pub async fn read(
    State(state): State<AppState>,
    Path((token, key)): Path<(String, String)>,
) -> AppResult<Json<SharedEntry>> {
    let row = sqlx::query(
        "SELECT title, body, created_at, updated_at, share_key_hash
         FROM entries WHERE share_token = ?1",
    )
    .bind(&token)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;

    let stored: Option<String> = row.get("share_key_hash");
    if !key_matches(stored.as_deref(), &key) {
        // Deliberately the same answer as an unknown token: a wrong key must not
        // confirm that the token itself is live.
        return Err(AppError::NotFound);
    }

    Ok(Json(SharedEntry {
        title: row.get("title"),
        body: row.get("body"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        token,
    }))
}

/// Public: serves a media file only while it is embedded in a shared entry, and
/// only to a caller holding that entry's key.
pub async fn serve_media(
    State(state): State<AppState>,
    Path((token, key, id)): Path<(String, String, String)>,
) -> AppResult<Response> {
    let stored: Option<String> = sqlx::query(
        "SELECT e.share_key_hash
         FROM entry_media em
         JOIN entries e ON e.id = em.entry_id
         WHERE em.media_id = ?1 AND e.share_token = ?2",
    )
    .bind(&id)
    .bind(&token)
    .fetch_optional(&state.db)
    .await?
    .and_then(|row| row.get("share_key_hash"));

    if !key_matches(stored.as_deref(), &key) {
        return Err(AppError::NotFound);
    }
    media::stream_media(&state, &id).await
}

#[cfg(test)]
mod tests {
    use super::{hash_key, key_matches, random_secret};

    #[test]
    fn a_key_matches_only_its_own_hash() {
        let key = random_secret();
        assert!(key_matches(Some(&hash_key(&key)), &key));
        assert!(!key_matches(Some(&hash_key(&key)), &random_secret()));
        assert!(!key_matches(Some(&hash_key(&key)), ""));
    }

    #[test]
    fn the_stored_hash_is_not_the_key() {
        let key = random_secret();
        let stored = hash_key(&key);
        assert_ne!(stored, key);
        // The hash itself must not open the link either.
        assert!(!key_matches(Some(&stored), &stored));
    }

    #[test]
    fn a_share_without_a_hash_stays_shut() {
        assert!(!key_matches(None, &random_secret()));
        assert!(!key_matches(None, ""));
    }

    #[test]
    fn secrets_do_not_repeat() {
        assert_ne!(random_secret(), random_secret());
    }
}
