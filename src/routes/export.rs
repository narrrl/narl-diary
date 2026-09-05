use std::io::{Seek, SeekFrom, Write};

use axum::{
    body::Body,
    extract::State,
    http::header,
    response::{IntoResponse, Response},
};
use sqlx::Row;
use tokio_util::io::ReaderStream;
use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

use crate::{auth::Session, error::AppResult, state::AppState};

/// One entry, flattened out of the database ahead of the blocking zip work.
struct ExportEntry {
    id: i64,
    title: String,
    body: String,
    created_at: i64,
}

/// `2026-09-05-first-light-12.md`. The id is always on the end, so two entries
/// written on the same day under the same title cannot collide.
fn entry_filename(entry: &ExportEntry) -> String {
    let slug: String = entry
        .title
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let slug = slug.trim_matches('-').split('-').filter(|s| !s.is_empty()).collect::<Vec<_>>().join("-");
    let slug: String = slug.chars().take(60).collect();
    let day = day_string(entry.created_at);
    if slug.is_empty() {
        format!("{day}-entry-{}.md", entry.id)
    } else {
        format!("{day}-{slug}-{}.md", entry.id)
    }
}

/// `YYYY-MM-DD` in UTC, from a Unix timestamp, without pulling in a date crate.
fn day_string(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    // Civil-from-days, Howard Hinnant's algorithm.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

/// Uploads are stored under a uuid; the archive gives them their name back.
/// Only characters that are safe in a path *and* need no escaping inside a
/// markdown link survive, because the entries link to these names directly.
fn media_filename(id: &str, original: &str) -> String {
    let safe: String = original
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' })
        .collect();
    let safe = safe.trim_matches(['.', '_']).to_string();
    if safe.is_empty() {
        id.to_string()
    } else {
        format!("{id}-{safe}")
    }
}

const README: &str = "\
This is a complete copy of a ~/diary.

  entries/   one markdown file per entry, named by the day it is about
  media/     every file the entries embed

Embedded media is linked relatively, so the entries render correctly in any
markdown reader as long as the two folders stay next to each other. Nothing
here needs the diary application to read it.
";

/// Everything, as one zip. A diary you cannot get out of is not a diary you
/// can trust, so this is deliberately a plain archive of plain files.
pub async fn export(_: Session, State(state): State<AppState>) -> AppResult<Response> {
    let entries: Vec<ExportEntry> = sqlx::query(
        "SELECT id, title, body, created_at FROM entries ORDER BY created_at, id",
    )
    .fetch_all(&state.db)
    .await?
    .into_iter()
    .map(|row| ExportEntry {
        id: row.get("id"),
        title: row.get("title"),
        body: row.get("body"),
        created_at: row.get("created_at"),
    })
    .collect();

    let media: Vec<(String, String)> = sqlx::query("SELECT id, filename FROM media")
        .fetch_all(&state.db)
        .await?
        .into_iter()
        .map(|row| (row.get("id"), row.get("filename")))
        .collect();

    let uploads = state.config.uploads_dir();

    // Zip writing is synchronous and reads every upload off disk, so it happens
    // on the blocking pool, into a temporary file rather than into memory: a
    // diary with photographs in it does not fit comfortably in a Vec.
    let file = tokio::task::spawn_blocking(move || -> anyhow::Result<std::fs::File> {
        let mut zip = ZipWriter::new(tempfile::tempfile()?);
        let text = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        // Media is overwhelmingly already-compressed; deflating it again costs
        // a lot of CPU to save nothing.
        let blob = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);

        zip.start_file("README.txt", text)?;
        zip.write_all(README.as_bytes())?;

        let names: Vec<(String, String)> = media
            .iter()
            .map(|(id, filename)| (id.clone(), media_filename(id, filename)))
            .collect();

        for entry in &entries {
            let mut body = entry.body.clone();
            for (id, name) in &names {
                body = body.replace(&format!("/api/media/{id}"), &format!("../media/{name}"));
            }
            let document = if entry.title.trim().is_empty() {
                body
            } else {
                format!("# {}\n\n{body}", entry.title.trim())
            };

            zip.start_file(format!("entries/{}", entry_filename(entry)), text)?;
            zip.write_all(document.as_bytes())?;
        }

        for (id, name) in &names {
            // A row whose file has gone missing must not fail the whole backup.
            let Ok(bytes) = std::fs::read(uploads.join(id)) else {
                continue;
            };
            zip.start_file(format!("media/{name}"), blob)?;
            zip.write_all(&bytes)?;
        }

        let mut file = zip.finish()?;
        file.seek(SeekFrom::Start(0))?;
        Ok(file)
    })
    .await??;

    let size = file.metadata()?.len();
    let name = format!("diary-{}.zip", day_string(crate::now()));

    Ok((
        [
            (header::CONTENT_TYPE, "application/zip".to_string()),
            (header::CONTENT_LENGTH, size.to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{name}\""),
            ),
            (header::CACHE_CONTROL, "no-store".to_string()),
        ],
        Body::from_stream(ReaderStream::new(tokio::fs::File::from_std(file))),
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::{day_string, entry_filename, media_filename, ExportEntry};

    #[test]
    fn renders_days_without_a_date_crate() {
        assert_eq!(day_string(0), "1970-01-01");
        assert_eq!(day_string(1_757_030_400), "2025-09-05");
        assert_eq!(day_string(951_782_400), "2000-02-29"); // a leap day the 100-year rule skips
        assert_eq!(day_string(-1), "1969-12-31");
    }

    fn entry(title: &str, id: i64) -> ExportEntry {
        ExportEntry { id, title: title.into(), body: String::new(), created_at: 0 }
    }

    #[test]
    fn entry_names_are_readable_and_unique() {
        assert_eq!(entry_filename(&entry("First Light", 12)), "1970-01-01-first-light-12.md");
        assert_eq!(entry_filename(&entry("", 3)), "1970-01-01-entry-3.md");
        assert_eq!(entry_filename(&entry("!!! ???", 4)), "1970-01-01-entry-4.md");
        // Same day, same title, different entries.
        assert_ne!(entry_filename(&entry("a", 1)), entry_filename(&entry("a", 2)));
    }

    #[test]
    fn media_names_cannot_escape_their_folder() {
        assert_eq!(media_filename("abc", "../../etc/passwd"), "abc-etc_passwd");
        // Nothing that would need escaping in the markdown link that points at it.
        assert_eq!(media_filename("abc", "my photo (1).png"), "abc-my_photo__1_.png");
        assert_eq!(media_filename("abc", "photo.jpg"), "abc-photo.jpg");
        assert_eq!(media_filename("abc", "..."), "abc");
    }
}
