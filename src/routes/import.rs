//! Importing a folder as a tree of documents.
//!
//! A directory of markdown notes is the format writing arrives in — a repo's
//! `docs/`, a folder of lecture notes, an export of this very diary. Storing it
//! as opaque files would make it unsearchable and uneditable, so the importer
//! reproduces the directory as nodes: directories become folders, markdown
//! becomes documents, everything else becomes media. The links between the
//! files are rewritten as they land, because a set of documents that can no
//! longer reach each other is a worse copy of what was uploaded.

use std::collections::{BTreeSet, HashMap};

use axum::{
    extract::{Multipart, Path, State},
    Json,
};
use regex::Regex;
use serde::Serialize;
use sqlx::Row;
use std::sync::LazyLock;
use uuid::Uuid;

use crate::{
    auth::Session,
    error::{AppError, AppResult},
    routes::{
        media,
        nodes::{self, next_slug, slugify, DOCUMENT, FOLDER},
    },
    state::AppState,
};

/// Markdown inline links and images: `[text](target)`, `![alt](target "title")`,
/// `[text](<target with spaces>)`. Reference-style definitions are left alone —
/// they are rare in the wild and rewriting them wrongly is worse than not.
static LINK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(!?)\[([^\]\n]*)\]\(\s*<?([^)\s<>]*)>?([^)\n]*)\)").expect("static regex is valid")
});

/// What the client is told happened. Counts rather than a list of ids: the
/// status line has one line, and the tree is right there to look at.
#[derive(Serialize)]
pub struct ImportReport {
    pub folders: usize,
    pub documents: usize,
    pub files: usize,
    pub links: usize,
    /// Paths that were refused, so a silently missing file is never a surprise.
    pub skipped: Vec<String>,
}

/// One file of the upload, with the path it had inside the folder.
struct Upload {
    path: String,
    data: Vec<u8>,
    mime: String,
}

/// Where a path inside the import ended up, as a URL a document body can use.
#[derive(Clone)]
enum Landed {
    Node(String),
    Media(String),
}

/// A path that is safe to build a tree from: relative, no `.` or `..` segment,
/// no empty segment, no backslash separators smuggled in from Windows.
fn normalize_path(raw: &str) -> Option<String> {
    let raw = raw.trim().replace('\\', "/");
    if raw.is_empty() || raw.starts_with('/') || raw.contains('\0') {
        return None;
    }
    // A drive letter is an absolute path wearing a disguise.
    if raw.len() > 1 && raw.as_bytes()[1] == b':' {
        return None;
    }

    let mut parts = Vec::new();
    for part in raw.split('/') {
        match part {
            "" | "." => continue,
            ".." => return None,
            _ => parts.push(part),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// The directory a path sits in, `""` for the top level.
fn parent_of(path: &str) -> &str {
    path.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("")
}

fn file_name(path: &str) -> &str {
    path.rsplit_once('/').map(|(_, name)| name).unwrap_or(path)
}

fn is_markdown(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".md") || lower.ends_with(".markdown")
}

/// The title of a document and the body left over. A markdown file usually
/// opens with the heading that names it; a node keeps its name in its own
/// column, so carrying the heading in the body too would show it twice and
/// export it twice. Without a heading the file name has to do.
fn split_title(body: &str, path: &str) -> (String, String) {
    let mut lines = body.lines();
    let heading = lines
        .next()
        .and_then(|line| line.trim().strip_prefix("# "))
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(str::to_string);

    match heading {
        Some(title) => (title, lines.collect::<Vec<_>>().join("\n").trim_start().to_string()),
        None => {
            let name = file_name(path);
            let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
            (stem.to_string(), body.to_string())
        }
    }
}

/// `%20` and friends, because a link to a file with a space in its name is
/// written escaped but the file arrives named with the space.
fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(&value[i + 1..i + 3], 16) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| value.to_string())
}

/// Split a link target into the part that names a file and the `#fragment` or
/// `?query` that follows it. `None` for anything that does not point inside the
/// import: a URL, an absolute path, a bare anchor, a mail address.
fn split_target(raw: &str) -> Option<(String, &str)> {
    let raw = raw.trim();
    if raw.is_empty() || raw.starts_with('#') || raw.starts_with('/') || raw.contains("://") {
        return None;
    }
    if raw.starts_with("mailto:") || raw.starts_with("tel:") || raw.starts_with("data:") {
        return None;
    }

    let cut = raw.find(['#', '?']).unwrap_or(raw.len());
    let (path, suffix) = raw.split_at(cut);
    (!path.is_empty()).then(|| (percent_decode(path), suffix))
}

/// Resolve a relative link against the directory its document sits in. `None`
/// when it climbs out of the import — that file was not uploaded, so there is
/// nothing to point at and the link is better left as it was written.
fn join_relative(dir: &str, target: &str) -> Option<String> {
    let mut parts: Vec<&str> = dir.split('/').filter(|p| !p.is_empty()).collect();
    for part in target.split('/') {
        match part {
            "" | "." => continue,
            ".." => {
                parts.pop()?;
            }
            _ => parts.push(part),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// Point every link that names an imported file at where that file now lives.
/// Returns the new body and how many links moved; anything unresolvable is left
/// exactly as written, so a link to the wider internet survives untouched.
fn rewrite_links(body: &str, dir: &str, landed: &HashMap<String, Landed>) -> (String, usize) {
    let mut count = 0;

    let out = LINK.replace_all(body, |caps: &regex::Captures| {
        let whole = caps[0].to_string();
        let Some((path, suffix)) = split_target(&caps[3]) else {
            return whole;
        };
        let Some(key) = join_relative(dir, &path) else {
            return whole;
        };

        // A link written without the extension (`see [notes](notes)`) still
        // names the markdown file next to it.
        let target = landed
            .get(&key)
            .or_else(|| landed.get(&format!("{key}.md")))
            .or_else(|| landed.get(&format!("{key}.markdown")));

        match target {
            Some(Landed::Node(url)) => {
                count += 1;
                format!("{}[{}]({url}{suffix})", &caps[1], &caps[2])
            }
            // A fragment means nothing to a file download, so it is dropped.
            Some(Landed::Media(url)) => {
                count += 1;
                format!("{}[{}]({url})", &caps[1], &caps[2])
            }
            None => whole,
        }
    });

    (out.into_owned(), count)
}

/// Read the multipart body into uploads. A field whose file name ends in `.zip`
/// is expanded, so dragging in an archive and picking a folder arrive the same
/// way — the exporter writes one of those archives.
async fn collect(
    state: &AppState,
    mut multipart: Multipart,
    skipped: &mut Vec<String>,
) -> AppResult<Vec<Upload>> {
    let mut uploads: Vec<Upload> = Vec::new();
    let mut total = 0usize;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("malformed upload: {e}")))?
    {
        let name = field
            .file_name()
            .map(str::to_string)
            .unwrap_or_else(|| "upload".to_string());
        let content_type = field.content_type().map(str::to_string);
        let data = field
            .bytes()
            .await
            .map_err(|e| AppError::BadRequest(format!("could not read upload: {e}")))?;

        total += data.len();
        if total > state.config.max_upload_bytes {
            return Err(AppError::BadRequest(format!(
                "the upload is larger than the {} MB limit",
                state.config.max_upload_bytes / 1024 / 1024
            )));
        }

        if name.to_ascii_lowercase().ends_with(".zip") {
            expand_zip(&data, &mut uploads, skipped)?;
            continue;
        }

        let Some(path) = normalize_path(&name) else {
            skipped.push(name);
            continue;
        };
        if data.is_empty() {
            skipped.push(path);
            continue;
        }

        uploads.push(Upload {
            mime: mime_of(&path, content_type),
            path,
            data: data.to_vec(),
        });
    }

    Ok(uploads)
}

fn mime_of(path: &str, declared: Option<String>) -> String {
    let raw = declared.filter(|m| !m.trim().is_empty()).unwrap_or_else(|| {
        mime_guess::from_path(path)
            .first_or_octet_stream()
            .to_string()
    });
    media::sanitize_mime(&raw)
}

fn expand_zip(data: &[u8], uploads: &mut Vec<Upload>, skipped: &mut Vec<String>) -> AppResult<()> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(data))
        .map_err(|e| AppError::BadRequest(format!("not a readable zip: {e}")))?;

    for index in 0..archive.len() {
        let mut file = archive
            .by_index(index)
            .map_err(|e| AppError::BadRequest(format!("damaged zip entry: {e}")))?;
        if file.is_dir() {
            continue;
        }

        let raw = file.name().to_string();
        // `enclosed_name` is the zip crate's own refusal of absolute paths,
        // `..` and symlink-style escapes; `normalize_path` then re-checks what
        // the tree builder relies on.
        let name = file
            .enclosed_name()
            .map(|path| path.to_string_lossy().to_string())
            .and_then(|path| normalize_path(&path));
        let Some(path) = name else {
            skipped.push(raw);
            continue;
        };

        let mut data = Vec::new();
        std::io::copy(&mut file, &mut data)?;
        if data.is_empty() {
            skipped.push(path);
            continue;
        }

        uploads.push(Upload {
            mime: mime_of(&path, None),
            path,
            data,
        });
    }

    Ok(())
}

/// `POST /api/nodes/{id}/import` — a folder, or a zip of one, becomes a tree
/// under the given space or folder.
pub async fn import(
    _: Session,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    multipart: Multipart,
) -> AppResult<Json<ImportReport>> {
    let target = nodes::load(&state, id).await?;
    if target.kind == DOCUMENT {
        return Err(AppError::BadRequest(
            "a folder can only be imported into a space or a folder".into(),
        ));
    }

    let mut skipped = Vec::new();
    let uploads = collect(&state, multipart, &mut skipped).await?;
    if uploads.is_empty() {
        return Err(AppError::BadRequest(
            "nothing importable in that upload".into(),
        ));
    }

    // The files land on disk before the transaction so a failed import writes
    // nothing to the database; the orphans are swept up on the way out.
    let mut written: Vec<String> = Vec::new();
    let result = build(&state, &target, uploads, skipped, &mut written).await;
    if result.is_err() {
        for media_id in &written {
            let _ = tokio::fs::remove_file(state.config.uploads_dir().join(media_id)).await;
        }
    }

    let report = result?;
    state.backup.signal();
    Ok(Json(report))
}

async fn build(
    state: &AppState,
    target: &nodes::Node,
    mut uploads: Vec<Upload>,
    skipped: Vec<String>,
    written: &mut Vec<String>,
) -> AppResult<ImportReport> {
    // The path of the target in slugs, which every rewritten link starts with.
    let prefix = target
        .path
        .iter()
        .map(|crumb| crumb.slug.as_str())
        .collect::<Vec<_>>()
        .join("/");

    // Sorted so a parent directory is always created before its children, and
    // so two runs of the same folder produce the same slugs.
    uploads.sort_by(|a, b| a.path.cmp(&b.path));
    uploads.dedup_by(|a, b| a.path == b.path);

    let mut directories: BTreeSet<String> = BTreeSet::new();
    for upload in &uploads {
        let mut dir = parent_of(&upload.path);
        while !dir.is_empty() {
            directories.insert(dir.to_string());
            dir = parent_of(dir);
        }
    }

    let now = crate::now();
    let mut tx = state.db.begin().await?;

    // Slugs already taken under each parent, extended as the tree is built —
    // the nodes being created are not visible to a query until the commit.
    let mut siblings: HashMap<i64, BTreeSet<String>> = HashMap::new();
    let existing = sqlx::query("SELECT slug FROM nodes WHERE parent_id IS ?1")
        .bind(target.id)
        .fetch_all(&mut *tx)
        .await?;
    siblings.insert(
        target.id,
        existing.iter().map(|row| row.get("slug")).collect(),
    );

    let mut dir_ids: HashMap<String, i64> = HashMap::from([(String::new(), target.id)]);
    let mut dir_paths: HashMap<String, String> = HashMap::from([(String::new(), prefix)]);
    let mut landed: HashMap<String, Landed> = HashMap::new();

    for dir in &directories {
        let parent = dir_ids[parent_of(dir)];
        let name = file_name(dir);
        let taken = siblings.entry(parent).or_default();
        let slug = next_slug(taken, &slugify(name))?;
        taken.insert(slug.clone());

        let id: i64 = sqlx::query(
            "INSERT INTO nodes (parent_id, kind, name, slug, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)
             RETURNING id",
        )
        .bind(parent)
        .bind(FOLDER)
        .bind(name)
        .bind(&slug)
        .bind(now)
        .fetch_one(&mut *tx)
        .await?
        .get("id");

        let path = format!("{}/{slug}", dir_paths[parent_of(dir)]);
        dir_ids.insert(dir.clone(), id);
        dir_paths.insert(dir.clone(), path.clone());
        landed.insert(dir.clone(), Landed::Node(format!("/n/{path}")));
    }

    // Documents first, with their bodies as uploaded: the rewriting below needs
    // to know where every document landed before it can point anything at one.
    let mut documents: Vec<(i64, String, String)> = Vec::new();
    let mut files = 0usize;

    for upload in &uploads {
        let dir = parent_of(&upload.path);
        let parent = dir_ids[dir];

        if is_markdown(&upload.path) {
            let raw = String::from_utf8_lossy(&upload.data).to_string();
            let (name, body) = split_title(&raw, &upload.path);
            // The slug follows the file name rather than the title, so a link
            // someone wrote to `01-was-ist-eebus.md` keeps reading that way.
            let stem = file_name(&upload.path);
            let stem = stem.rsplit_once('.').map_or(stem, |(stem, _)| stem);

            let taken = siblings.entry(parent).or_default();
            let slug = next_slug(taken, &slugify(stem))?;
            taken.insert(slug.clone());

            let id: i64 = sqlx::query(
                "INSERT INTO nodes (parent_id, kind, name, slug, body, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)
                 RETURNING id",
            )
            .bind(parent)
            .bind(DOCUMENT)
            .bind(&name)
            .bind(&slug)
            .bind(&body)
            .bind(now)
            .fetch_one(&mut *tx)
            .await?
            .get("id");

            let path = format!("{}/{slug}", dir_paths[dir]);
            landed.insert(upload.path.clone(), Landed::Node(format!("/n/{path}")));
            documents.push((id, upload.path.clone(), body));
        } else {
            let media_id = Uuid::new_v4().to_string();
            tokio::fs::write(state.config.uploads_dir().join(&media_id), &upload.data).await?;
            written.push(media_id.clone());

            sqlx::query(
                "INSERT INTO media (id, filename, mime, size, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )
            .bind(&media_id)
            .bind(file_name(&upload.path))
            .bind(&upload.mime)
            .bind(upload.data.len() as i64)
            .bind(now)
            .execute(&mut *tx)
            .await?;

            landed.insert(
                upload.path.clone(),
                Landed::Media(format!("/api/media/{media_id}")),
            );
            files += 1;
        }
    }

    // Second pass: point the links at where things landed, and attach every
    // file a document embeds to it the way a hand-written document does.
    let mut links = 0usize;
    let mut attached: BTreeSet<String> = BTreeSet::new();

    for (id, path, body) in &documents {
        let (rewritten, count) = rewrite_links(body, parent_of(path), &landed);
        links += count;

        if rewritten != *body {
            sqlx::query("UPDATE nodes SET body = ?1 WHERE id = ?2")
                .bind(&rewritten)
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }

        for capture in media::MEDIA_REF.captures_iter(&rewritten) {
            let media_id = capture[1].to_lowercase();
            sqlx::query(
                "INSERT OR IGNORE INTO node_media (node_id, media_id)
                 SELECT ?1, id FROM media WHERE id = ?2",
            )
            .bind(id)
            .bind(&media_id)
            .execute(&mut *tx)
            .await?;
            attached.insert(media_id);
        }
    }

    // A file nobody links to still belongs to the folder it was uploaded in,
    // or deleting that folder would leave it behind with no way back to it.
    for upload in &uploads {
        if let Some(Landed::Media(url)) = landed.get(&upload.path) {
            let media_id = url.trim_start_matches("/api/media/").to_string();
            if attached.contains(&media_id) {
                continue;
            }
            sqlx::query("INSERT OR IGNORE INTO node_media (node_id, media_id) VALUES (?1, ?2)")
                .bind(dir_ids[parent_of(&upload.path)])
                .bind(&media_id)
                .execute(&mut *tx)
                .await?;
        }
    }

    tx.commit().await?;

    Ok(ImportReport {
        folders: directories.len(),
        documents: documents.len(),
        files,
        links,
        skipped,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::{
        is_markdown, join_relative, normalize_path, rewrite_links, split_target, split_title, Landed,
    };

    #[test]
    fn refuses_paths_that_escape_the_import() {
        assert_eq!(normalize_path("docs/readme.md").as_deref(), Some("docs/readme.md"));
        assert_eq!(normalize_path("./docs//readme.md").as_deref(), Some("docs/readme.md"));
        assert_eq!(normalize_path("docs\\readme.md").as_deref(), Some("docs/readme.md"));
        assert_eq!(normalize_path("../secrets"), None);
        assert_eq!(normalize_path("docs/../../etc/passwd"), None);
        assert_eq!(normalize_path("/etc/passwd"), None);
        assert_eq!(normalize_path("C:/windows/system32"), None);
        assert_eq!(normalize_path("   "), None);
    }

    #[test]
    fn a_title_comes_from_the_heading_then_the_file_name() {
        assert_eq!(
            split_title("# Was ist EEBUS?\n\ntext\n", "a/01.md"),
            ("Was ist EEBUS?".into(), "text".into())
        );
        // No heading, or one further down: the body is left exactly as it came.
        assert_eq!(
            split_title("no heading", "a/01-intro.md"),
            ("01-intro".into(), "no heading".into())
        );
        assert_eq!(
            split_title("#nope\n\n# Real\n", "x.md"),
            ("x".into(), "#nope\n\n# Real\n".into())
        );
    }

    #[test]
    fn markdown_is_recognised_by_extension() {
        assert!(is_markdown("a/README.MD"));
        assert!(is_markdown("notes.markdown"));
        assert!(!is_markdown("diagram.png"));
    }

    #[test]
    fn relative_links_resolve_against_their_document() {
        assert_eq!(join_relative("docs", "img/a.png").as_deref(), Some("docs/img/a.png"));
        assert_eq!(join_relative("docs/deep", "../a.md").as_deref(), Some("docs/a.md"));
        assert_eq!(join_relative("", "a.md").as_deref(), Some("a.md"));
        assert_eq!(join_relative("docs", "../../outside.md"), None);
    }

    #[test]
    fn external_targets_are_not_touched() {
        assert!(split_target("https://example.org/x").is_none());
        assert!(split_target("#section").is_none());
        assert!(split_target("/api/media/x").is_none());
        assert!(split_target("mailto:someone@example.org").is_none());
        assert_eq!(split_target("a%20b.md#top"), Some(("a b.md".into(), "#top")));
    }

    #[test]
    fn links_point_at_where_the_files_landed() {
        let landed = HashMap::from([
            ("einarbeiten/01.md".to_string(), Landed::Node("/n/work/einarbeiten/01".into())),
            ("einarbeiten/img/a.png".to_string(), Landed::Media("/api/media/uuid".into())),
        ]);

        let body = "see [one](01.md#ziele) and ![pic](img/a.png), \
                    plus [the web](https://example.org) and [gone](missing.md)";
        let (out, count) = rewrite_links(body, "einarbeiten", &landed);

        assert_eq!(count, 2);
        assert!(out.contains("[one](/n/work/einarbeiten/01#ziele)"), "{out}");
        assert!(out.contains("![pic](/api/media/uuid)"), "{out}");
        assert!(out.contains("[the web](https://example.org)"), "{out}");
        assert!(out.contains("[gone](missing.md)"), "{out}");
    }

    #[test]
    fn a_link_from_a_sibling_directory_still_resolves() {
        let landed = HashMap::from([(
            "a/one.md".to_string(),
            Landed::Node("/n/work/a/one".into()),
        )]);
        let (out, count) = rewrite_links("[x](../a/one.md)", "b", &landed);
        assert_eq!(count, 1);
        assert_eq!(out, "[x](/n/work/a/one)");
    }
}
