//! Kanban boards, one per space.
//!
//! The question a board answers here is "what am I working on in *this* part of
//! my life", so there is exactly one board per space and no board table: a
//! space with `has_board` set has a board, and the lists hang off the space
//! node directly. That keeps the deployment one binary and one SQLite file, and
//! the Proton mirror keeps covering everything without a second service.
//!
//! Cards may point at a document. The link is deliberately weak — deleting the
//! document sets `node_id` to NULL rather than deleting the card, because the
//! work is still open once its notes are gone.

use axum::{
    extract::{Path, State},
    Json,
};
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};

use crate::{
    auth::Session,
    error::{AppError, AppResult},
    routes::nodes::SPACE,
    state::AppState,
};

/// What a board starts with. Three columns is the smallest arrangement that
/// still says something: not started, in hand, finished.
const DEFAULT_LISTS: [&str; 3] = ["backlog", "doing", "done"];

#[derive(Serialize)]
pub struct Board {
    pub space_id: i64,
    pub space_name: String,
    pub lists: Vec<BoardList>,
}

#[derive(Serialize)]
pub struct BoardList {
    pub id: i64,
    pub name: String,
    pub position: i64,
    pub cards: Vec<Card>,
}

#[derive(Serialize)]
pub struct Card {
    pub id: i64,
    pub list_id: i64,
    pub title: String,
    pub body: String,
    pub node_id: Option<i64>,
    /// The linked document's name, so a card can say what it points at without
    /// the client reading every document on the board.
    pub node_name: Option<String>,
    pub due_at: Option<i64>,
    pub done_at: Option<i64>,
    pub position: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Deserialize)]
pub struct ListInput {
    pub name: String,
}

#[derive(Deserialize)]
pub struct CardInput {
    pub title: String,
    #[serde(default)]
    pub body: String,
    /// The document the card is about, if it has one.
    #[serde(default)]
    pub node_id: Option<i64>,
    #[serde(default)]
    pub due_at: Option<i64>,
}

/// Every field is optional: the client sends what changed. `null` and "absent"
/// are different for the two that can be cleared, so they are doubly wrapped —
/// `"due_at": null` drops the date, leaving it out keeps it.
#[derive(Deserialize)]
pub struct CardPatch {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default, deserialize_with = "double_option")]
    pub node_id: Option<Option<i64>>,
    #[serde(default, deserialize_with = "double_option")]
    pub due_at: Option<Option<i64>>,
    pub done: Option<bool>,
}

fn double_option<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    Option::deserialize(deserializer).map(Some)
}

#[derive(Deserialize)]
pub struct MoveInput {
    pub list_id: i64,
    /// Where among the cards of that list it lands. Past the end means last.
    #[serde(default)]
    pub position: Option<i64>,
}

/// `ids` with `card` taken out and put back at `to`. Pulled out of the move
/// handler because off-by-one in a reorder is silent: the card lands next to
/// where it was asked to go and nobody notices until the board reads wrong.
fn reordered(ids: &[i64], card: i64, to: usize) -> Vec<i64> {
    let mut rest: Vec<i64> = ids.iter().copied().filter(|id| *id != card).collect();
    rest.insert(to.min(rest.len()), card);
    rest
}

fn row_to_card(row: &sqlx::sqlite::SqliteRow) -> Card {
    Card {
        id: row.get("id"),
        list_id: row.get("list_id"),
        title: row.get("title"),
        body: row.get("body"),
        node_id: row.get("node_id"),
        node_name: row.get("node_name"),
        due_at: row.get("due_at"),
        done_at: row.get("done_at"),
        position: row.get("position"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

/// The space behind an id, refusing anything that is not a space with a board.
async fn board_space(db: &SqlitePool, space_id: i64) -> AppResult<String> {
    let row = sqlx::query("SELECT kind, name, has_board FROM nodes WHERE id = ?1")
        .bind(space_id)
        .fetch_optional(db)
        .await?
        .ok_or(AppError::NotFound)?;

    if row.get::<String, _>("kind") != SPACE {
        return Err(AppError::BadRequest("only a space has a board".into()));
    }
    if row.get::<i64, _>("has_board") == 0 {
        // The diary is the case this exists for: a journal has no backlog.
        return Err(AppError::BadRequest(
            "this space has no board — :set board turns one on".into(),
        ));
    }
    Ok(row.get("name"))
}

/// The space a list belongs to, and the same board check along the way.
async fn list_space(db: &SqlitePool, list_id: i64) -> AppResult<i64> {
    let space_id: i64 = sqlx::query("SELECT space_id FROM board_lists WHERE id = ?1")
        .bind(list_id)
        .fetch_optional(db)
        .await?
        .ok_or(AppError::NotFound)?
        .get("space_id");
    board_space(db, space_id).await?;
    Ok(space_id)
}

/// Seed a board that has no lists yet. A space created before this feature
/// existed — or one whose board was switched on later — gets its columns the
/// first time the board is looked at, which is the only moment it matters.
async fn ensure_lists(db: &SqlitePool, space_id: i64) -> AppResult<()> {
    let existing: i64 = sqlx::query("SELECT COUNT(*) AS n FROM board_lists WHERE space_id = ?1")
        .bind(space_id)
        .fetch_one(db)
        .await?
        .get("n");
    if existing > 0 {
        return Ok(());
    }

    let now = crate::now();
    for (position, name) in DEFAULT_LISTS.iter().enumerate() {
        sqlx::query(
            "INSERT INTO board_lists (space_id, name, position, created_at) VALUES (?1, ?2, ?3, ?4)",
        )
        .bind(space_id)
        .bind(name)
        .bind(position as i64)
        .bind(now)
        .execute(db)
        .await?;
    }
    Ok(())
}

pub async fn board(
    _: Session,
    State(state): State<AppState>,
    Path(space_id): Path<i64>,
) -> AppResult<Json<Board>> {
    let space_name = board_space(&state.db, space_id).await?;
    ensure_lists(&state.db, space_id).await?;
    load(&state, space_id, space_name).await.map(Json)
}

async fn load(state: &AppState, space_id: i64, space_name: String) -> AppResult<Board> {
    let list_rows = sqlx::query(
        "SELECT id, name, position FROM board_lists WHERE space_id = ?1 ORDER BY position, id",
    )
    .bind(space_id)
    .fetch_all(&state.db)
    .await?;

    // One query for every card on the board rather than one per list: a board
    // is small, and the round trips are what would be felt.
    let card_rows = sqlx::query(
        "SELECT c.id, c.list_id, c.title, c.body, c.node_id, c.due_at, c.done_at,
                c.position, c.created_at, c.updated_at,
                (SELECT n.name FROM nodes n WHERE n.id = c.node_id) AS node_name
         FROM cards c
         JOIN board_lists l ON l.id = c.list_id
         WHERE l.space_id = ?1
         ORDER BY c.position, c.id",
    )
    .bind(space_id)
    .fetch_all(&state.db)
    .await?;

    let lists = list_rows
        .iter()
        .map(|row| {
            let id: i64 = row.get("id");
            BoardList {
                id,
                name: row.get("name"),
                position: row.get("position"),
                cards: card_rows
                    .iter()
                    .filter(|card| card.get::<i64, _>("list_id") == id)
                    .map(row_to_card)
                    .collect(),
            }
        })
        .collect();

    Ok(Board {
        space_id,
        space_name,
        lists,
    })
}

pub async fn create_list(
    _: Session,
    State(state): State<AppState>,
    Path(space_id): Path<i64>,
    Json(input): Json<ListInput>,
) -> AppResult<Json<Board>> {
    let space_name = board_space(&state.db, space_id).await?;
    let name = input.name.trim();
    if name.is_empty() {
        return Err(AppError::BadRequest("a list needs a name".into()));
    }

    let position: i64 = sqlx::query(
        "SELECT COALESCE(MAX(position), -1) + 1 AS next FROM board_lists WHERE space_id = ?1",
    )
    .bind(space_id)
    .fetch_one(&state.db)
    .await?
    .get("next");

    sqlx::query(
        "INSERT INTO board_lists (space_id, name, position, created_at) VALUES (?1, ?2, ?3, ?4)",
    )
    .bind(space_id)
    .bind(name)
    .bind(position)
    .bind(crate::now())
    .execute(&state.db)
    .await?;

    state.backup.signal();
    load(&state, space_id, space_name).await.map(Json)
}

pub async fn rename_list(
    _: Session,
    State(state): State<AppState>,
    Path(list_id): Path<i64>,
    Json(input): Json<ListInput>,
) -> AppResult<Json<Board>> {
    let space_id = list_space(&state.db, list_id).await?;
    let name = input.name.trim();
    if name.is_empty() {
        return Err(AppError::BadRequest("a list needs a name".into()));
    }

    sqlx::query("UPDATE board_lists SET name = ?1 WHERE id = ?2")
        .bind(name)
        .bind(list_id)
        .execute(&state.db)
        .await?;

    state.backup.signal();
    let space_name = board_space(&state.db, space_id).await?;
    load(&state, space_id, space_name).await.map(Json)
}

/// Delete a list and the cards on it. A board must keep one list, or there is
/// nowhere left to put a card.
pub async fn remove_list(
    _: Session,
    State(state): State<AppState>,
    Path(list_id): Path<i64>,
) -> AppResult<Json<Board>> {
    let space_id = list_space(&state.db, list_id).await?;

    let remaining: i64 = sqlx::query("SELECT COUNT(*) AS n FROM board_lists WHERE space_id = ?1")
        .bind(space_id)
        .fetch_one(&state.db)
        .await?
        .get("n");
    if remaining <= 1 {
        return Err(AppError::BadRequest(
            "a board needs at least one list".into(),
        ));
    }

    sqlx::query("DELETE FROM board_lists WHERE id = ?1")
        .bind(list_id)
        .execute(&state.db)
        .await?;

    state.backup.signal();
    let space_name = board_space(&state.db, space_id).await?;
    load(&state, space_id, space_name).await.map(Json)
}

pub async fn create_card(
    _: Session,
    State(state): State<AppState>,
    Path(list_id): Path<i64>,
    Json(input): Json<CardInput>,
) -> AppResult<Json<Card>> {
    list_space(&state.db, list_id).await?;
    let title = input.title.trim();
    if title.is_empty() {
        return Err(AppError::BadRequest("a card needs a title".into()));
    }

    let position: i64 = sqlx::query(
        "SELECT COALESCE(MAX(position), -1) + 1 AS next FROM cards WHERE list_id = ?1",
    )
    .bind(list_id)
    .fetch_one(&state.db)
    .await?
    .get("next");

    let now = crate::now();
    let id: i64 = sqlx::query(
        "INSERT INTO cards (list_id, node_id, title, body, due_at, position, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)
         RETURNING id",
    )
    .bind(list_id)
    .bind(input.node_id)
    .bind(title)
    .bind(&input.body)
    .bind(input.due_at)
    .bind(position)
    .bind(now)
    .fetch_one(&state.db)
    .await?
    .get("id");

    state.backup.signal();
    card(&state.db, id).await.map(Json)
}

pub async fn update_card(
    _: Session,
    State(state): State<AppState>,
    Path(card_id): Path<i64>,
    Json(patch): Json<CardPatch>,
) -> AppResult<Json<Card>> {
    let current = card(&state.db, card_id).await?;
    let now = crate::now();

    let title = match patch.title {
        Some(title) if title.trim().is_empty() => {
            return Err(AppError::BadRequest("a card needs a title".into()))
        }
        Some(title) => title.trim().to_string(),
        None => current.title,
    };
    // `done` is a flag on the way in and a timestamp in the table: when it was
    // finished is worth more than that it was, and the weekly digest needs it.
    let done_at = match patch.done {
        Some(true) => current.done_at.or(Some(now)),
        Some(false) => None,
        None => current.done_at,
    };

    sqlx::query(
        "UPDATE cards SET title = ?1, body = ?2, node_id = ?3, due_at = ?4, done_at = ?5,
                          updated_at = ?6
         WHERE id = ?7",
    )
    .bind(&title)
    .bind(patch.body.unwrap_or(current.body))
    .bind(patch.node_id.unwrap_or(current.node_id))
    .bind(patch.due_at.unwrap_or(current.due_at))
    .bind(done_at)
    .bind(now)
    .bind(card_id)
    .execute(&state.db)
    .await?;

    state.backup.signal();
    card(&state.db, card_id).await.map(Json)
}

pub async fn remove_card(
    _: Session,
    State(state): State<AppState>,
    Path(card_id): Path<i64>,
) -> AppResult<Json<serde_json::Value>> {
    let affected = sqlx::query("DELETE FROM cards WHERE id = ?1")
        .bind(card_id)
        .execute(&state.db)
        .await?
        .rows_affected();
    if affected == 0 {
        return Err(AppError::NotFound);
    }

    state.backup.signal();
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// Move a card to a list and a place in it. Positions are rewritten densely
/// afterwards, so they stay the small numbers the board reads them as instead
/// of drifting into gaps nobody can reason about.
pub async fn move_card(
    _: Session,
    State(state): State<AppState>,
    Path(card_id): Path<i64>,
    Json(input): Json<MoveInput>,
) -> AppResult<Json<Card>> {
    let current = card(&state.db, card_id).await?;
    let from = list_space(&state.db, current.list_id).await?;
    let to = list_space(&state.db, input.list_id).await?;
    if from != to {
        // Boards are per space on purpose; a card that could cross would make
        // "what am I working on in work" a lie.
        return Err(AppError::BadRequest(
            "a card cannot move to another space's board".into(),
        ));
    }

    let mut tx = state.db.begin().await?;

    sqlx::query("UPDATE cards SET list_id = ?1, updated_at = ?2 WHERE id = ?3")
        .bind(input.list_id)
        .bind(crate::now())
        .bind(card_id)
        .execute(&mut *tx)
        .await?;

    let ids: Vec<i64> = sqlx::query("SELECT id FROM cards WHERE list_id = ?1 ORDER BY position, id")
        .bind(input.list_id)
        .fetch_all(&mut *tx)
        .await?
        .iter()
        .map(|row| row.get("id"))
        .collect();

    let target = input.position.unwrap_or(i64::MAX).max(0) as usize;
    for (position, id) in reordered(&ids, card_id, target).iter().enumerate() {
        sqlx::query("UPDATE cards SET position = ?1 WHERE id = ?2")
            .bind(position as i64)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }

    // The list it came from keeps a hole where the card was. Harmless for
    // ordering, but it would grow with every move.
    let left: Vec<i64> = sqlx::query("SELECT id FROM cards WHERE list_id = ?1 ORDER BY position, id")
        .bind(current.list_id)
        .fetch_all(&mut *tx)
        .await?
        .iter()
        .map(|row| row.get("id"))
        .collect();
    for (position, id) in left.iter().enumerate() {
        sqlx::query("UPDATE cards SET position = ?1 WHERE id = ?2")
            .bind(position as i64)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }

    tx.commit().await?;
    state.backup.signal();
    card(&state.db, card_id).await.map(Json)
}

async fn card(db: &SqlitePool, id: i64) -> AppResult<Card> {
    let row = sqlx::query(
        "SELECT c.id, c.list_id, c.title, c.body, c.node_id, c.due_at, c.done_at,
                c.position, c.created_at, c.updated_at,
                (SELECT n.name FROM nodes n WHERE n.id = c.node_id) AS node_name
         FROM cards c WHERE c.id = ?1",
    )
    .bind(id)
    .fetch_optional(db)
    .await?
    .ok_or(AppError::NotFound)?;

    Ok(row_to_card(&row))
}

#[cfg(test)]
mod tests {
    use super::reordered;

    #[test]
    fn a_card_lands_where_it_was_asked_to() {
        let ids = [1, 2, 3, 4];
        assert_eq!(reordered(&ids, 4, 0), vec![4, 1, 2, 3]);
        assert_eq!(reordered(&ids, 1, 2), vec![2, 3, 1, 4]);
        // Past the end is the end, not an error: dragging below the last card
        // is how a card is sent to the bottom.
        assert_eq!(reordered(&ids, 1, 99), vec![2, 3, 4, 1]);
    }

    #[test]
    fn a_card_from_another_list_is_inserted_rather_than_moved() {
        // The card is not in `ids` yet — the usual case when it comes from the
        // list next door.
        assert_eq!(reordered(&[1, 2], 9, 1), vec![1, 9, 2]);
        assert_eq!(reordered(&[], 9, 0), vec![9]);
    }

    #[test]
    fn a_card_kept_in_place_does_not_drift() {
        let ids = [1, 2, 3];
        assert_eq!(reordered(&ids, 2, 1), vec![1, 2, 3]);
    }
}
