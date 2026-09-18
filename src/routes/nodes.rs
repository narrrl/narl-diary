use std::collections::BTreeSet;

use axum::{
    extract::{Path, Query, State},
    Json,
};
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};

use crate::{
    auth::Session,
    error::{AppError, AppResult},
    routes::media,
    state::AppState,
};

/// What a node is. Spaces are the roots (`diary`, `work`, `uni`, ...), folders
/// group things inside them to any depth, documents are the leaves that hold
/// text. One table, one recursion, so a space can nest exactly like a folder.
pub const SPACE: &str = "space";
pub const FOLDER: &str = "folder";
pub const DOCUMENT: &str = "document";

#[derive(Serialize)]
pub struct NodeSummary {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub kind: String,
    pub name: String,
    pub slug: String,
    pub excerpt: String,
    pub position: i64,
    pub has_board: bool,
    pub child_count: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub shared: bool,
    pub share_token: Option<String>,
}

#[derive(Serialize)]
pub struct Node {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub kind: String,
    pub name: String,
    pub slug: String,
    pub body: String,
    /// The same preview the list shows, so a client that has just saved can
    /// patch its list row without re-reading the whole list.
    pub excerpt: String,
    pub position: i64,
    pub has_board: bool,
    pub child_count: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub shared: bool,
    pub share_token: Option<String>,
    /// This node's ancestors, the space first and the node itself last — the
    /// breadcrumb, and everything needed to build its /n/... path.
    pub path: Vec<Crumb>,
}

#[derive(Serialize)]
pub struct Crumb {
    pub id: i64,
    pub kind: String,
    pub name: String,
    pub slug: String,
}

#[derive(Deserialize)]
pub struct ListQuery {
    /// Children of this node. Omitted together with `q` means the spaces.
    #[serde(default)]
    pub parent: Option<i64>,
    #[serde(default)]
    pub q: Option<String>,
    /// Confine a search to one space.
    #[serde(default)]
    pub space: Option<i64>,
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub offset: Option<i64>,
}

#[derive(Deserialize)]
pub struct CreateInput {
    pub parent_id: i64,
    /// `document` (the default) or `folder`.
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub body: String,
    /// Optional override so a document can be backdated to the day it is about.
    #[serde(default)]
    pub created_at: Option<i64>,
}

#[derive(Deserialize)]
pub struct UpdateInput {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub created_at: Option<i64>,
    /// Spaces only, and only when it is sent at all.
    #[serde(default)]
    pub has_board: Option<bool>,
}

#[derive(Deserialize)]
pub struct SpaceInput {
    pub name: String,
    /// Defaults to on: a space is usually a project, and the one place a board
    /// makes no sense — the diary — already exists.
    #[serde(default)]
    pub has_board: Option<bool>,
}

#[derive(Deserialize)]
pub struct MoveInput {
    pub parent_id: i64,
    #[serde(default)]
    pub position: Option<i64>,
}

/// How much of a body the list query reads. Comfortably more than an excerpt
/// needs, and a rounding error next to documents that run to thousands of words.
const HEAD_CHARS: usize = 1024;
const EXCERPT_CHARS: usize = 180;

/// The columns every list query selects, so a row always converts the same way.
const SUMMARY_COLUMNS: &str = "n.id, n.parent_id, n.kind, n.name, n.slug, n.position, n.has_board,
     n.created_at, n.updated_at, n.share_token,
     (SELECT COUNT(*) FROM nodes c WHERE c.parent_id = n.id) AS child_count";

/// A short, plain-ish preview of the body for the list pane. `head` is only the
/// leading `HEAD_CHARS` of the document, so truncation is inferred from its
/// length rather than measured against the whole body.
fn excerpt(head: &str) -> String {
    let flat: String = head
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("![") && !line.starts_with("```"))
        .map(|line| line.trim_start_matches(['#', '>', '-', '*', ' ']))
        .collect::<Vec<_>>()
        .join(" ");
    let mut out: String = flat.chars().take(EXCERPT_CHARS).collect();
    if flat.chars().count() > EXCERPT_CHARS || head.chars().count() >= HEAD_CHARS {
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

/// A name as a path segment: lowercase, ASCII-ish, no surprises in a URL. An
/// empty result becomes `untitled`, because every node needs a slug and a
/// document usually gets its name minutes after it is created.
pub fn slugify(name: &str) -> String {
    let mut slug = String::new();
    let mut dash = false;
    for ch in name.trim().chars().flat_map(|c| c.to_lowercase()) {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch);
            dash = false;
        } else if matches!(ch, 'ä' | 'ö' | 'ü' | 'ß') {
            // The diary is written in two languages; transliterating rather
            // than dropping keeps a German heading readable in its URL.
            slug.push_str(match ch {
                'ä' => "ae",
                'ö' => "oe",
                'ü' => "ue",
                _ => "ss",
            });
            dash = false;
        } else if !slug.is_empty() && !dash {
            slug.push('-');
            dash = true;
        }
    }
    let slug = slug.trim_end_matches('-').to_string();
    let slug: String = slug.chars().take(80).collect();
    let slug = slug.trim_end_matches('-').to_string();
    if slug.is_empty() {
        "untitled".to_string()
    } else {
        slug
    }
}

/// `base`, or `base-2`, `base-3`… until one is free. Kept separate from the
/// database so the importer, which creates a whole tree of siblings before any
/// of them is queryable, can allocate against the set it is building.
pub fn next_slug(taken: &BTreeSet<String>, base: &str) -> AppResult<String> {
    for suffix in 1..1000 {
        let candidate = if suffix == 1 {
            base.to_string()
        } else {
            format!("{base}-{suffix}")
        };
        if !taken.contains(&candidate) {
            return Ok(candidate);
        }
    }
    Err(AppError::BadRequest(
        "too many siblings with that name".into(),
    ))
}

/// The slugs already spoken for under `parent_id`. `exclude` is the node being
/// renamed, which must not collide with itself.
pub async fn sibling_slugs(
    db: &SqlitePool,
    parent_id: Option<i64>,
    exclude: Option<i64>,
) -> AppResult<BTreeSet<String>> {
    let rows = sqlx::query(
        "SELECT slug FROM nodes
         WHERE parent_id IS ?1 AND (?2 IS NULL OR id <> ?2)",
    )
    .bind(parent_id)
    .bind(exclude)
    .fetch_all(db)
    .await?;

    Ok(rows.iter().map(|row| row.get("slug")).collect())
}

/// `base`, or `base-2`, `base-3`… until no sibling holds it.
pub async fn unique_slug(
    db: &SqlitePool,
    parent_id: Option<i64>,
    base: &str,
    exclude: Option<i64>,
) -> AppResult<String> {
    next_slug(&sibling_slugs(db, parent_id, exclude).await?, base)
}

fn row_to_summary(row: &sqlx::sqlite::SqliteRow, head: &str) -> NodeSummary {
    let share_token: Option<String> = row.get("share_token");
    NodeSummary {
        id: row.get("id"),
        parent_id: row.get("parent_id"),
        kind: row.get("kind"),
        name: row.get("name"),
        slug: row.get("slug"),
        excerpt: excerpt(head),
        position: row.get("position"),
        has_board: row.get::<i64, _>("has_board") != 0,
        child_count: row.get("child_count"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        shared: share_token.is_some(),
        share_token,
    }
}

/// The spaces, in display order. Their child counts are what the switcher shows.
pub async fn spaces(_: Session, State(state): State<AppState>) -> AppResult<Json<Vec<NodeSummary>>> {
    let rows = sqlx::query(&format!(
        "SELECT {SUMMARY_COLUMNS} FROM nodes n
         WHERE n.parent_id IS NULL AND n.kind = 'space'
         ORDER BY n.position, n.name"
    ))
    .fetch_all(&state.db)
    .await?;

    Ok(Json(
        rows.iter().map(|row| row_to_summary(row, "")).collect(),
    ))
}

pub async fn create_space(
    _: Session,
    State(state): State<AppState>,
    Json(input): Json<SpaceInput>,
) -> AppResult<Json<Node>> {
    let name = input.name.trim();
    if name.is_empty() {
        return Err(AppError::BadRequest("a space needs a name".into()));
    }

    let slug = unique_slug(&state.db, None, &slugify(name), None).await?;
    let now = crate::now();
    let position: i64 = sqlx::query("SELECT COALESCE(MAX(position), -1) + 1 AS next FROM nodes WHERE parent_id IS NULL")
        .fetch_one(&state.db)
        .await?
        .get("next");

    let id: i64 = sqlx::query(
        "INSERT INTO nodes (parent_id, kind, name, slug, position, has_board, created_at, updated_at)
         VALUES (NULL, 'space', ?1, ?2, ?3, ?4, ?5, ?5)
         RETURNING id",
    )
    .bind(name)
    .bind(&slug)
    .bind(position)
    .bind(i64::from(input.has_board.unwrap_or(true)))
    .bind(now)
    .fetch_one(&state.db)
    .await?
    .get("id");

    state.backup.signal();
    load(&state, id).await.map(Json)
}

pub async fn list(
    _: Session,
    State(state): State<AppState>,
    Query(params): Query<ListQuery>,
) -> AppResult<Json<Vec<NodeSummary>>> {
    let limit = params.limit.unwrap_or(500).clamp(1, 2000);
    let offset = params.offset.unwrap_or(0).max(0);
    let search = params.q.as_deref().map(str::trim).filter(|q| !q.is_empty());

    let rows = match search.and_then(fts_query) {
        // A search reaches through the whole tree, optionally confined to one
        // space: the recursive term collects that space and everything under it.
        Some(query) => {
            sqlx::query(&format!(
                "WITH RECURSIVE sub (id) AS (
                     SELECT id FROM nodes WHERE id = ?4
                     UNION ALL
                     SELECT n.id FROM nodes n JOIN sub ON n.parent_id = sub.id
                 )
                 SELECT {SUMMARY_COLUMNS}, substr(n.body, 1, {HEAD_CHARS}) AS head
                 FROM nodes_fts
                 JOIN nodes n ON n.id = nodes_fts.rowid
                 WHERE nodes_fts MATCH ?1
                   AND n.kind <> 'space'
                   AND (?4 IS NULL OR n.id IN (SELECT id FROM sub))
                 ORDER BY rank
                 LIMIT ?2 OFFSET ?3"
            ))
            .bind(query)
            .bind(limit)
            .bind(offset)
            .bind(params.space)
            .fetch_all(&state.db)
            .await?
        }
        // Browsing: the children of one node, folders before documents.
        None => {
            let parent = params.parent;
            sqlx::query(&format!(
                "SELECT {SUMMARY_COLUMNS}, substr(n.body, 1, {HEAD_CHARS}) AS head
                 FROM nodes n
                 WHERE n.parent_id IS ?1
                 ORDER BY CASE n.kind WHEN 'folder' THEN 0 ELSE 1 END,
                          n.position, n.created_at DESC, n.id DESC
                 LIMIT ?2 OFFSET ?3"
            ))
            .bind(parent)
            .bind(limit)
            .bind(offset)
            .fetch_all(&state.db)
            .await?
        }
    };

    Ok(Json(
        rows.iter()
            .map(|row| {
                let head: String = row.try_get("head").unwrap_or_default();
                row_to_summary(row, &head)
            })
            .collect(),
    ))
}

pub async fn get(
    _: Session,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> AppResult<Json<Node>> {
    load(&state, id).await.map(Json)
}

#[derive(Deserialize)]
pub struct ResolveQuery {
    /// Slugs from a space downwards, `work/einarbeiten/01-was-ist-eebus`.
    pub path: String,
}

/// Resolve a slug path to the node it names. This is the other half of the
/// `/n/<space>/<slug>/...` addresses the client puts in the URL bar and the
/// importer writes into rewritten links: they stay readable because they are
/// resolved here rather than carrying ids around.
pub async fn resolve(
    _: Session,
    State(state): State<AppState>,
    Query(params): Query<ResolveQuery>,
) -> AppResult<Json<Node>> {
    let mut current: Option<i64> = None;
    for segment in params.path.split('/').filter(|s| !s.is_empty()) {
        let row = sqlx::query("SELECT id FROM nodes WHERE parent_id IS ?1 AND slug = ?2")
            .bind(current)
            .bind(segment)
            .fetch_optional(&state.db)
            .await?
            .ok_or(AppError::NotFound)?;
        current = Some(row.get("id"));
    }
    load(&state, current.ok_or(AppError::NotFound)?).await.map(Json)
}

pub async fn create(
    _: Session,
    State(state): State<AppState>,
    Json(input): Json<CreateInput>,
) -> AppResult<Json<Node>> {
    let kind = input.kind.as_deref().unwrap_or(DOCUMENT);
    if kind != DOCUMENT && kind != FOLDER {
        return Err(AppError::BadRequest(format!(
            "a node created here is a document or a folder, not a {kind}"
        )));
    }

    let parent_kind: String = sqlx::query("SELECT kind FROM nodes WHERE id = ?1")
        .bind(input.parent_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(AppError::NotFound)?
        .get("kind");
    if parent_kind == DOCUMENT {
        return Err(AppError::BadRequest(
            "a document cannot contain other nodes".into(),
        ));
    }

    let now = crate::now();
    let created_at = input.created_at.unwrap_or(now);
    let slug = unique_slug(
        &state.db,
        Some(input.parent_id),
        &slugify(&input.name),
        None,
    )
    .await?;

    let id: i64 = sqlx::query(
        "INSERT INTO nodes (parent_id, kind, name, slug, body, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         RETURNING id",
    )
    .bind(input.parent_id)
    .bind(kind)
    .bind(input.name.trim())
    .bind(&slug)
    .bind(&input.body)
    .bind(created_at)
    .bind(now)
    .fetch_one(&state.db)
    .await?
    .get("id");

    media::link_to_node(&state.db, id, &input.body).await?;
    state.backup.signal();
    load(&state, id).await.map(Json)
}

pub async fn update(
    _: Session,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(input): Json<UpdateInput>,
) -> AppResult<Json<Node>> {
    let row = sqlx::query("SELECT parent_id, kind, name, slug FROM nodes WHERE id = ?1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(AppError::NotFound)?;

    let parent_id: Option<i64> = row.get("parent_id");
    let kind: String = row.get("kind");
    let previous_name: String = row.get("name");
    let name = input.name.trim();

    // The slug follows the name, because a path nobody can read is no better
    // than an id. It only moves when the name does, so a link to an unrenamed
    // document keeps working.
    let slug = if name == previous_name {
        row.get::<String, _>("slug")
    } else {
        unique_slug(&state.db, parent_id, &slugify(name), Some(id)).await?
    };

    if input.has_board.is_some() && kind != SPACE {
        return Err(AppError::BadRequest("only a space has a board".into()));
    }

    let affected = sqlx::query(
        "UPDATE nodes
         SET name = ?1, body = ?2, slug = ?3, updated_at = ?4,
             created_at = COALESCE(?5, created_at),
             has_board = COALESCE(?6, has_board)
         WHERE id = ?7",
    )
    .bind(name)
    .bind(&input.body)
    .bind(&slug)
    .bind(crate::now())
    .bind(input.created_at)
    .bind(input.has_board.map(i64::from))
    .bind(id)
    .execute(&state.db)
    .await?
    .rows_affected();

    if affected == 0 {
        return Err(AppError::NotFound);
    }

    media::link_to_node(&state.db, id, &input.body).await?;
    state.backup.signal();
    load(&state, id).await.map(Json)
}

pub async fn remove(
    _: Session,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> AppResult<Json<serde_json::Value>> {
    // Everything under a folder goes with it, so the files to clean up are
    // those exclusive to the whole subtree. Collected before the delete, which
    // takes the node_media rows with it.
    let subtree = descendants(&state.db, id).await?;
    if subtree.is_empty() {
        return Err(AppError::NotFound);
    }
    let orphans = media::exclusive_media(&state.db, &subtree).await?;

    let affected = sqlx::query("DELETE FROM nodes WHERE id = ?1")
        .bind(id)
        .execute(&state.db)
        .await?
        .rows_affected();

    if affected == 0 {
        return Err(AppError::NotFound);
    }

    for media_id in orphans {
        media::delete_media(&state, &media_id).await?;
    }

    state.backup.signal();
    Ok(Json(serde_json::json!({ "ok": true, "removed": subtree.len() })))
}

/// Reparent a node, and optionally place it among its new siblings.
pub async fn move_node(
    _: Session,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(input): Json<MoveInput>,
) -> AppResult<Json<Node>> {
    if input.parent_id == id {
        return Err(AppError::BadRequest("a node cannot contain itself".into()));
    }

    let target_kind: String = sqlx::query("SELECT kind FROM nodes WHERE id = ?1")
        .bind(input.parent_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(AppError::NotFound)?
        .get("kind");
    if target_kind == DOCUMENT {
        return Err(AppError::BadRequest(
            "a document cannot contain other nodes".into(),
        ));
    }

    // Moving a folder into its own subtree would cut that subtree loose from
    // every root, where nothing could reach it again.
    if descendants(&state.db, id).await?.contains(&input.parent_id) {
        return Err(AppError::BadRequest(
            "a folder cannot be moved into itself".into(),
        ));
    }

    let name: String = sqlx::query("SELECT name FROM nodes WHERE id = ?1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(AppError::NotFound)?
        .get("name");

    // The slug has to be free where it lands, not where it came from.
    let slug = unique_slug(
        &state.db,
        Some(input.parent_id),
        &slugify(&name),
        Some(id),
    )
    .await?;

    sqlx::query(
        "UPDATE nodes
         SET parent_id = ?1, slug = ?2, position = COALESCE(?3, position), updated_at = ?4
         WHERE id = ?5",
    )
    .bind(input.parent_id)
    .bind(&slug)
    .bind(input.position)
    .bind(crate::now())
    .bind(id)
    .execute(&state.db)
    .await?;

    state.backup.signal();
    load(&state, id).await.map(Json)
}

/// A node and everything beneath it, the node itself first.
pub async fn descendants(db: &SqlitePool, id: i64) -> AppResult<Vec<i64>> {
    let rows = sqlx::query(
        "WITH RECURSIVE sub (id) AS (
             SELECT id FROM nodes WHERE id = ?1
             UNION ALL
             SELECT n.id FROM nodes n JOIN sub ON n.parent_id = sub.id
         )
         SELECT id FROM sub",
    )
    .bind(id)
    .fetch_all(db)
    .await?;

    Ok(rows.iter().map(|row| row.get("id")).collect())
}

/// The ancestors of a node, the space first and the node itself last.
pub async fn path_of(db: &SqlitePool, id: i64) -> AppResult<Vec<Crumb>> {
    let rows = sqlx::query(
        "WITH RECURSIVE up (id, parent_id, kind, name, slug, depth) AS (
             SELECT id, parent_id, kind, name, slug, 0 FROM nodes WHERE id = ?1
             UNION ALL
             SELECT n.id, n.parent_id, n.kind, n.name, n.slug, up.depth + 1
             FROM nodes n JOIN up ON n.id = up.parent_id
         )
         SELECT id, kind, name, slug FROM up ORDER BY depth DESC",
    )
    .bind(id)
    .fetch_all(db)
    .await?;

    Ok(rows
        .iter()
        .map(|row| Crumb {
            id: row.get("id"),
            kind: row.get("kind"),
            name: row.get("name"),
            slug: row.get("slug"),
        })
        .collect())
}

pub async fn load(state: &AppState, id: i64) -> AppResult<Node> {
    let row = sqlx::query(&format!(
        "SELECT {SUMMARY_COLUMNS}, n.body FROM nodes n WHERE n.id = ?1"
    ))
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;

    let body: String = row.get("body");
    let summary = row_to_summary(&row, &body.chars().take(HEAD_CHARS).collect::<String>());

    Ok(Node {
        id: summary.id,
        parent_id: summary.parent_id,
        kind: summary.kind,
        name: summary.name,
        slug: summary.slug,
        body,
        excerpt: summary.excerpt,
        position: summary.position,
        has_board: summary.has_board,
        child_count: summary.child_count,
        created_at: summary.created_at,
        updated_at: summary.updated_at,
        shared: summary.shared,
        share_token: summary.share_token,
        path: path_of(&state.db, id).await?,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{excerpt, fts_query, next_slug, slugify, EXCERPT_CHARS, HEAD_CHARS};

    #[test]
    fn strips_markdown_furniture() {
        let body = "# Title\n\n![pic](/api/media/x)\n\n> a quote\n- a point";
        assert_eq!(excerpt(body), "Title a quote a point");
    }

    #[test]
    fn marks_a_body_that_was_cut_short() {
        assert!(!excerpt("short").ends_with('…'));
        assert!(excerpt(&"word ".repeat(60)).ends_with('…'));
        // A head at the read limit is truncated even when it flattens small.
        assert!(excerpt(&"![x](/api/media/y)\n".repeat(HEAD_CHARS / 18)).ends_with('…'));
    }

    #[test]
    fn excerpt_never_exceeds_its_budget() {
        let out = excerpt(&"a".repeat(HEAD_CHARS));
        assert_eq!(out.chars().count(), EXCERPT_CHARS + 1);
    }

    #[test]
    fn fts_query_discards_operators() {
        // Anything that could be read as FTS syntax must not survive as syntax.
        assert_eq!(fts_query("hello"), Some("\"hello\"*".into()));
        assert_eq!(fts_query("a OR b"), Some("\"a\"* AND \"OR\"* AND \"b\"*".into()));
        assert_eq!(fts_query("foo\"NEAR/2\"bar"), Some("\"foo\"* AND \"NEAR\"* AND \"2\"* AND \"bar\"*".into()));
        assert_eq!(fts_query("  "), None);
        assert_eq!(fts_query("***"), None);
    }

    #[test]
    fn slugs_are_url_safe() {
        assert_eq!(slugify("01 — Was ist EEBUS?"), "01-was-ist-eebus");
        assert_eq!(slugify("  Trailing and   inner  "), "trailing-and-inner");
        assert_eq!(slugify("Übergrößen"), "uebergroessen");
        assert_eq!(slugify("README.md"), "readme-md");
    }

    #[test]
    fn a_nameless_node_still_gets_a_slug() {
        assert_eq!(slugify(""), "untitled");
        assert_eq!(slugify("***"), "untitled");
        assert_eq!(slugify("   "), "untitled");
    }

    #[test]
    fn slugs_stay_short_and_never_end_in_a_dash() {
        let slug = slugify(&"word ".repeat(40));
        assert!(slug.len() <= 80, "{slug}");
        assert!(!slug.ends_with('-'), "{slug}");
    }

    #[test]
    fn a_taken_slug_gets_a_number() {
        let taken: BTreeSet<String> = ["readme", "readme-2"].iter().map(|s| s.to_string()).collect();
        assert_eq!(next_slug(&taken, "readme").unwrap(), "readme-3");
        assert_eq!(next_slug(&taken, "notes").unwrap(), "notes");
        assert_eq!(next_slug(&BTreeSet::new(), "readme").unwrap(), "readme");
    }
}
