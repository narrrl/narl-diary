//! The mirror itself: make the device folder look like `DIARY_DATA_DIR`.
//!
//! The remote layout is deliberately the local one, not a rendering of it:
//!
//! ```text
//! narl-diary/            the device's sync root — folders only, per Proton
//!   data/                the diary's data directory, copied
//!     RESTORE.txt        what this is and how to put it back
//!     diary.db           a consistent snapshot, one revision per change
//!     uploads/<uuid>     every uploaded file, under the name the database knows
//! ```
//!
//! so restoring is copying two things into an empty data directory, with no
//! tool in between. The readable-anywhere view of the diary is what `:export!`
//! is for; this is the copy that can be *restored*.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use proton_drive_rs::ProtonDriveClient;
use proton_sdk::ids::{LinkId, NodeUid, VolumeId};
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};

use crate::config::Config;

/// What one run did, for the log line and the status endpoint.
#[derive(Debug, Default, Clone, Copy, serde::Serialize)]
pub struct Summary {
    pub uploaded: usize,
    pub skipped: usize,
    pub pruned: usize,
    pub bytes: u64,
}

const RESTORE: &str = "\
This is a Proton Drive mirror of a ~/diary server. It is the contents of the
server's data directory, one folder down from the device root because Proton
does not allow files directly in it.

  diary.db          the whole diary: entries, media metadata, share tokens
  uploads/<uuid>    every uploaded file; diary.db gives each one its real name

To restore, stop the diary, empty its data directory, and copy both back:

  DIARY_DATA_DIR/diary.db
  DIARY_DATA_DIR/uploads/

The database is a snapshot taken with `VACUUM INTO`, so it is internally
consistent and carries no write-ahead log: it can be opened directly.

Each file above keeps its history — Proton Drive holds the previous revisions
of diary.db, so an accident that was mirrored can still be undone.
";

/// Mirror everything that has changed. Returns what it did.
pub async fn run(
    db: &SqlitePool,
    config: &Config,
    client: &ProtonDriveClient,
    data_folder: &NodeUid,
    uploads_folder: &NodeUid,
) -> Result<Summary> {
    let mut mirror = Mirror {
        db,
        client,
        summary: Summary::default(),
    };

    // Written once, and only ever re-read by a human who has lost everything
    // else — so it goes up before the data it explains.
    mirror
        .bytes(data_folder, "RESTORE.txt", "RESTORE.txt", "text/plain", RESTORE.as_bytes())
        .await?;

    mirror.database(config, data_folder).await?;
    mirror.media(config, uploads_folder).await?;

    if config.backup.prune {
        mirror.prune().await?;
    }

    Ok(mirror.summary)
}

struct Mirror<'a> {
    db: &'a SqlitePool,
    client: &'a ProtonDriveClient,
    summary: Summary,
}

impl Mirror<'_> {
    /// Snapshot the database and mirror it.
    ///
    /// `VACUUM INTO` is the only honest way to copy a live SQLite database: it
    /// takes a read transaction, so the copy is a single point in time with the
    /// write-ahead log already folded in. Copying the file by hand while the
    /// diary is being written to would produce something that may or may not
    /// open.
    async fn database(&mut self, config: &Config, data_folder: &NodeUid) -> Result<()> {
        let snapshot = config.data_dir.join(".diary-backup.db");
        // A snapshot left behind by a killed run would fail the next one:
        // VACUUM INTO refuses to write over an existing file.
        let _ = tokio::fs::remove_file(&snapshot).await;

        sqlx::query("VACUUM INTO ?")
            .bind(snapshot.to_string_lossy().as_ref())
            .execute(self.db)
            .await
            .context("could not snapshot the database")?;

        let result = self
            .file(data_folder, "diary.db", "diary.db", "application/vnd.sqlite3", &snapshot)
            .await;

        let _ = tokio::fs::remove_file(&snapshot).await;
        result
    }

    /// Mirror every uploaded file that is not up there yet.
    ///
    /// Uploads are immutable — a uuid names one set of bytes forever — so a
    /// file already recorded at the same size is not read, let alone hashed. A
    /// diary with a decade of photographs in it must not re-read the lot every
    /// hour.
    async fn media(&mut self, config: &Config, uploads_folder: &NodeUid) -> Result<()> {
        let media: Vec<(String, i64)> =
            sqlx::query("SELECT id, size FROM media ORDER BY created_at")
                .fetch_all(self.db)
                .await?
                .into_iter()
                .map(|row| (row.get("id"), row.get("size")))
                .collect();

        for (id, size) in media {
            let key = format!("uploads/{id}");
            if let Some(tracked) = self.tracked(&key).await? {
                if tracked.size == size {
                    self.summary.skipped += 1;
                    continue;
                }
            }

            let path = config.uploads_dir().join(&id);
            if !path.exists() {
                // A row whose file has gone missing is a local inconsistency,
                // not a reason to abandon the rest of the backup.
                tracing::warn!(media = %id, "media row has no file on disk; skipping");
                continue;
            }

            self.file(uploads_folder, &key, &id, "application/octet-stream", &path)
                .await?;
        }

        Ok(())
    }

    /// Trash remote files whose local rows are gone. Off unless asked for.
    async fn prune(&mut self) -> Result<()> {
        let orphans: Vec<(String, String)> = sqlx::query(
            "SELECT path, node_uid FROM backup_files
              WHERE path LIKE 'uploads/%'
                AND substr(path, 9) NOT IN (SELECT id FROM media)",
        )
        .fetch_all(self.db)
        .await?
        .into_iter()
        .map(|row| (row.get("path"), row.get("node_uid")))
        .collect();

        if orphans.is_empty() {
            return Ok(());
        }

        let uids: Vec<NodeUid> = orphans.iter().filter_map(|(_, uid)| parse_uid(uid)).collect();
        for (uid, result) in self.client.trash_nodes(&uids).await? {
            if let Err(e) = result {
                tracing::warn!(node = %uid, error = %e, "could not trash a pruned file");
            }
        }

        for (path, _) in &orphans {
            sqlx::query("DELETE FROM backup_files WHERE path = ?")
                .bind(path)
                .execute(self.db)
                .await?;
            self.summary.pruned += 1;
        }

        Ok(())
    }

    /// Mirror bytes held in memory, through a temporary file so the upload path
    /// is the same one everything else takes.
    async fn bytes(
        &mut self,
        parent: &NodeUid,
        key: &str,
        name: &str,
        mime: &str,
        bytes: &[u8],
    ) -> Result<()> {
        let hash = hex::encode(Sha256::digest(bytes));
        if self.is_current(key, &hash).await? {
            return Ok(());
        }

        let temporary = tempfile::NamedTempFile::new()?;
        tokio::fs::write(temporary.path(), bytes).await?;
        self.upload(parent, key, name, mime, temporary.path(), &hash)
            .await
    }

    /// Hash a local file and mirror it if the remote copy is not already it.
    async fn file(
        &mut self,
        parent: &NodeUid,
        key: &str,
        name: &str,
        mime: &str,
        path: &Path,
    ) -> Result<()> {
        let hash = hash_file(path.to_path_buf()).await?;
        if self.is_current(key, &hash).await? {
            return Ok(());
        }

        self.upload(parent, key, name, mime, path, &hash).await
    }

    /// Upload `path`, as a new revision of the node already tracked for `key`
    /// when there is one, and as a new file otherwise.
    ///
    /// A tracked node that has been deleted in the Drive UI is not an error:
    /// the mirror notices the node is gone and re-creates the file.
    async fn upload(
        &mut self,
        parent: &NodeUid,
        key: &str,
        name: &str,
        mime: &str,
        path: &Path,
        hash: &str,
    ) -> Result<()> {
        let size = tokio::fs::metadata(path).await?.len();
        let modified = crate::now();

        let existing = match self.tracked(key).await? {
            Some(tracked) => self
                .client
                .get_node(&tracked.uid)
                .await
                .ok()
                .flatten()
                .map(|_| tracked.uid),
            None => None,
        };

        let uid = match existing {
            Some(uid) => {
                let file = std::fs::File::open(path)?;
                self.client
                    .upload_new_revision_from(&uid, file, size as i64, Vec::new(), Some(modified))
                    .await
                    .with_context(|| format!("could not upload a new revision of {name}"))?;
                uid
            }
            None => {
                let file = std::fs::File::open(path)?;
                self.client
                    .upload_file_from(
                        parent,
                        name,
                        mime,
                        file,
                        size as i64,
                        Vec::new(),
                        Some(modified),
                        false,
                    )
                    .await
                    .with_context(|| format!("could not upload {name}"))?
            }
        };

        self.record(key, &uid, hash, size as i64).await?;
        self.summary.uploaded += 1;
        self.summary.bytes += size;
        tracing::info!(file = key, bytes = size, "mirrored to Proton Drive");
        Ok(())
    }

    /// Whether the remote copy of `key` is already these bytes.
    async fn is_current(&mut self, key: &str, hash: &str) -> Result<bool> {
        let current = self
            .tracked(key)
            .await?
            .is_some_and(|tracked| tracked.hash == hash);
        if current {
            self.summary.skipped += 1;
        }
        Ok(current)
    }

    async fn tracked(&self, path: &str) -> Result<Option<Tracked>> {
        let row = sqlx::query("SELECT node_uid, hash, size FROM backup_files WHERE path = ?")
            .bind(path)
            .fetch_optional(self.db)
            .await?;

        Ok(row.and_then(|row| {
            let uid: String = row.get("node_uid");
            parse_uid(&uid).map(|uid| Tracked {
                uid,
                hash: row.get("hash"),
                size: row.get("size"),
            })
        }))
    }

    async fn record(&self, path: &str, uid: &NodeUid, hash: &str, size: i64) -> Result<()> {
        sqlx::query(
            "INSERT INTO backup_files (path, node_uid, hash, size, uploaded_at)
             VALUES (?, ?, ?, ?, ?)
             ON CONFLICT (path) DO UPDATE SET
                 node_uid = excluded.node_uid,
                 hash = excluded.hash,
                 size = excluded.size,
                 uploaded_at = excluded.uploaded_at",
        )
        .bind(path)
        .bind(uid.to_string())
        .bind(hash)
        .bind(size)
        .bind(crate::now())
        .execute(self.db)
        .await?;
        Ok(())
    }
}

struct Tracked {
    uid: NodeUid,
    hash: String,
    size: i64,
}

/// `<volume>~<link>`, the form [`NodeUid`] prints. Anything else is a row this
/// build did not write and is treated as untracked rather than trusted.
pub(crate) fn parse_uid(text: &str) -> Option<NodeUid> {
    let (volume, link) = text.split_once('~')?;
    if volume.is_empty() || link.is_empty() {
        return None;
    }
    Some(NodeUid::new(VolumeId::new(volume), LinkId::new(link)))
}

/// Streaming sha256 of a file, on the blocking pool: the database snapshot is
/// as large as the diary, and photographs are larger.
async fn hash_file(path: PathBuf) -> Result<String> {
    tokio::task::spawn_blocking(move || -> Result<String> {
        let mut file = std::fs::File::open(&path)
            .with_context(|| format!("could not read {}", path.display()))?;
        let mut hasher = Sha256::new();
        std::io::copy(&mut file, &mut hasher)?;
        Ok(hex::encode(hasher.finalize()))
    })
    .await?
}

#[cfg(test)]
mod tests {
    use super::parse_uid;

    #[test]
    fn node_uids_survive_a_round_trip() {
        let uid = parse_uid("vol-1~link-2").expect("a well-formed uid parses");
        assert_eq!(uid.volume_id.as_str(), "vol-1");
        assert_eq!(uid.link_id.as_str(), "link-2");
        assert_eq!(uid.to_string(), "vol-1~link-2");
    }

    #[test]
    fn a_row_this_build_did_not_write_is_untracked() {
        assert!(parse_uid("").is_none());
        assert!(parse_uid("no-separator").is_none());
        assert!(parse_uid("~link").is_none());
        assert!(parse_uid("volume~").is_none());
    }
}
