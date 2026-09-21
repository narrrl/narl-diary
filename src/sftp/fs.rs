//! The virtual filesystem an SFTP client sees: `/<space>/<folder>/.../<doc>.md`.
//! Directories are spaces and folders; the only files are documents, always
//! `.md`. There is no on-disk mirror — every call reads or writes `nodes`
//! directly, so a save here is a save in the browser too.

use std::collections::HashMap;

use russh_sftp::protocol::{
    File, FileAttributes, Handle, Name, OpenFlags, StatusCode, Version,
};
use sqlx::Row;

use crate::{routes::media, routes::nodes, state::AppState};

type SftpResult<T> = Result<T, StatusCode>;

/// A node as this filesystem sees it: enough to answer `stat` and to list a
/// parent directory without a second query.
struct Entry {
    id: i64,
    kind: String,
    slug: String,
    updated_at: i64,
    size: i64,
}

fn filename(entry: &Entry) -> String {
    if entry.kind == nodes::DOCUMENT {
        format!("{}.md", entry.slug)
    } else {
        entry.slug.clone()
    }
}

fn dir_attrs(mtime: i64) -> FileAttributes {
    let mut attrs = FileAttributes::empty();
    attrs.set_dir(true);
    attrs.permissions = Some(0o755 | attrs.permissions.unwrap_or(0));
    attrs.mtime = Some(mtime as u32);
    attrs.atime = Some(mtime as u32);
    attrs.size = Some(0);
    attrs
}

fn file_attrs(size: u64, mtime: i64) -> FileAttributes {
    let mut attrs = FileAttributes::empty();
    attrs.set_regular(true);
    attrs.permissions = Some(0o644 | attrs.permissions.unwrap_or(0));
    attrs.mtime = Some(mtime as u32);
    attrs.atime = Some(mtime as u32);
    attrs.size = Some(size);
    attrs
}

fn attrs_of(entry: &Entry) -> FileAttributes {
    if entry.kind == nodes::DOCUMENT {
        file_attrs(entry.size as u64, entry.updated_at)
    } else {
        dir_attrs(entry.updated_at)
    }
}

/// A path split into segments, `/` and `.` collapsed away. `readlink`,
/// symlinks and anything OpenSSH probes for are simply not implemented —
/// there are none of those here.
fn segments(path: &str) -> Vec<&str> {
    path.split('/').filter(|s| !s.is_empty() && *s != ".").collect()
}

/// A document's slug is its filename without the trailing `.md`. A filename
/// that does not end in `.md` never resolves — this filesystem holds no other
/// kind of file yet.
fn doc_slug(filename: &str) -> Option<&str> {
    filename.strip_suffix(".md").filter(|base| !base.is_empty())
}

impl Default for Entry {
    fn default() -> Self {
        Entry {
            id: 0,
            kind: nodes::SPACE.to_string(),
            slug: String::new(),
            updated_at: 0,
            size: 0,
        }
    }
}

enum OpenHandle {
    /// Directory listing, already fetched: `readdir` just drains it.
    Dir(Vec<Entry>),
    /// A document's body, fetched once at `open`. Read-only handles never
    /// touch the database again after this.
    Read(Vec<u8>),
    /// A document being written. `id` is `None` for a file that does not
    /// exist yet — it is only created in `nodes` when the handle closes,
    /// so an editor's write-then-fsync-then-close never leaves a half
    /// document with the wrong name behind.
    Write {
        id: Option<i64>,
        parent_id: i64,
        slug: String,
        name: String,
        buf: Vec<u8>,
    },
}

pub struct SpaceFs {
    state: AppState,
    handles: HashMap<String, OpenHandle>,
    next_handle: u64,
}

impl SpaceFs {
    pub fn new(state: AppState) -> Self {
        Self {
            state,
            handles: HashMap::new(),
            next_handle: 0,
        }
    }

    fn allocate(&mut self, handle: OpenHandle) -> String {
        self.next_handle += 1;
        let key = self.next_handle.to_string();
        self.handles.insert(key.clone(), handle);
        key
    }

    /// Resolve a path of directory segments to a node id, `None` meaning the
    /// root — which holds the spaces and nothing else.
    async fn resolve_dir(&self, segs: &[&str]) -> SftpResult<Option<i64>> {
        let mut current: Option<i64> = None;
        for seg in segs {
            let row = sqlx::query(
                "SELECT id FROM nodes WHERE parent_id IS ?1 AND slug = ?2 AND kind <> 'document'",
            )
            .bind(current)
            .bind(seg)
            .fetch_optional(&self.state.db)
            .await
            .map_err(|_| StatusCode::Failure)?
            .ok_or(StatusCode::NoSuchFile)?;
            current = Some(row.get("id"));
        }
        Ok(current)
    }

    /// Split a path into the directory that must already exist and the
    /// trailing segment, still unresolved.
    fn split_path(path: &str) -> (Vec<&str>, Option<&str>) {
        let mut segs = segments(path);
        let leaf = segs.pop();
        (segs, leaf)
    }

    async fn entry_at(&self, path: &str) -> SftpResult<Entry> {
        let segs = segments(path);
        if segs.is_empty() {
            return Ok(Entry::default());
        }
        let (dir, leaf) = Self::split_path(path);
        let Some(leaf) = leaf else {
            return Ok(Entry::default());
        };
        let parent = self.resolve_dir(&dir).await;

        // A directory: space or folder, matched by slug under its parent.
        if let Ok(parent_id) = parent {
            let dir_row = sqlx::query(
                "SELECT id, kind, slug, updated_at FROM nodes
                 WHERE parent_id IS ?1 AND slug = ?2 AND kind <> 'document'",
            )
            .bind(parent_id)
            .bind(leaf)
            .fetch_optional(&self.state.db)
            .await
            .map_err(|_| StatusCode::Failure)?;
            if let Some(row) = dir_row {
                return Ok(Entry {
                    id: row.get("id"),
                    kind: row.get("kind"),
                    slug: row.get("slug"),
                    updated_at: row.get("updated_at"),
                    size: 0,
                });
            }

            if let Some(slug) = doc_slug(leaf) {
                let doc_row = sqlx::query(
                    "SELECT id, slug, updated_at, length(body) AS size FROM nodes
                     WHERE parent_id IS ?1 AND slug = ?2 AND kind = 'document'",
                )
                .bind(parent_id)
                .bind(slug)
                .fetch_optional(&self.state.db)
                .await
                .map_err(|_| StatusCode::Failure)?;
                if let Some(row) = doc_row {
                    return Ok(Entry {
                        id: row.get("id"),
                        kind: nodes::DOCUMENT.to_string(),
                        slug: row.get("slug"),
                        updated_at: row.get("updated_at"),
                        size: row.get("size"),
                    });
                }
            }
        }

        Err(StatusCode::NoSuchFile)
    }

    async fn children(&self, parent_id: Option<i64>) -> SftpResult<Vec<Entry>> {
        let rows = sqlx::query(
            "SELECT id, kind, slug, updated_at, length(body) AS size FROM nodes
             WHERE parent_id IS ?1 ORDER BY kind, slug",
        )
        .bind(parent_id)
        .fetch_all(&self.state.db)
        .await
        .map_err(|_| StatusCode::Failure)?;

        Ok(rows
            .iter()
            .map(|row| Entry {
                id: row.get("id"),
                kind: row.get("kind"),
                slug: row.get("slug"),
                updated_at: row.get("updated_at"),
                size: row.try_get("size").unwrap_or(0),
            })
            .collect())
    }
}

impl russh_sftp::server::Handler for SpaceFs {
    type Error = StatusCode;

    fn unimplemented(&self) -> Self::Error {
        StatusCode::OpUnsupported
    }

    async fn init(
        &mut self,
        _version: u32,
        _extensions: HashMap<String, String>,
    ) -> SftpResult<Version> {
        Ok(Version::new())
    }

    async fn realpath(&mut self, id: u32, path: String) -> SftpResult<Name> {
        let clean = segments(&path).join("/");
        Ok(Name {
            id,
            files: vec![File::dummy(format!("/{clean}"))],
        })
    }

    async fn lstat(&mut self, id: u32, path: String) -> SftpResult<russh_sftp::protocol::Attrs> {
        self.stat(id, path).await
    }

    async fn stat(&mut self, id: u32, path: String) -> SftpResult<russh_sftp::protocol::Attrs> {
        let entry = self.entry_at(&path).await?;
        Ok(russh_sftp::protocol::Attrs {
            id,
            attrs: attrs_of(&entry),
        })
    }

    async fn fstat(&mut self, id: u32, handle: String) -> SftpResult<russh_sftp::protocol::Attrs> {
        match self.handles.get(&handle) {
            Some(OpenHandle::Read(data)) => Ok(russh_sftp::protocol::Attrs {
                id,
                attrs: file_attrs(data.len() as u64, crate::now()),
            }),
            Some(OpenHandle::Write { buf, .. }) => Ok(russh_sftp::protocol::Attrs {
                id,
                attrs: file_attrs(buf.len() as u64, crate::now()),
            }),
            Some(OpenHandle::Dir(_)) => Ok(russh_sftp::protocol::Attrs {
                id,
                attrs: dir_attrs(crate::now()),
            }),
            None => Err(StatusCode::Failure),
        }
    }

    async fn opendir(&mut self, id: u32, path: String) -> SftpResult<Handle> {
        let segs = segments(&path);
        let parent_id = self.resolve_dir(&segs).await?;
        let entries = self.children(parent_id).await?;
        let handle = self.allocate(OpenHandle::Dir(entries));
        let _ = id;
        Ok(Handle { id, handle })
    }

    async fn readdir(&mut self, id: u32, handle: String) -> SftpResult<Name> {
        match self.handles.get_mut(&handle) {
            Some(OpenHandle::Dir(entries)) if !entries.is_empty() => {
                let files = entries
                    .drain(..)
                    .map(|entry| File::new(filename(&entry), attrs_of(&entry)))
                    .collect();
                Ok(Name { id, files })
            }
            Some(OpenHandle::Dir(_)) => Err(StatusCode::Eof),
            _ => Err(StatusCode::Failure),
        }
    }

    async fn open(
        &mut self,
        id: u32,
        filename: String,
        pflags: OpenFlags,
        _attrs: FileAttributes,
    ) -> SftpResult<Handle> {
        let (dir, leaf) = Self::split_path(&filename);
        let leaf = leaf.ok_or(StatusCode::PermissionDenied)?;
        let slug = doc_slug(leaf).ok_or(StatusCode::PermissionDenied)?;
        let parent_id = self.resolve_dir(&dir).await?;

        let existing = sqlx::query(
            "SELECT id, body FROM nodes WHERE parent_id IS ?1 AND slug = ?2 AND kind = 'document'",
        )
        .bind(parent_id)
        .bind(slug)
        .fetch_optional(&self.state.db)
        .await
        .map_err(|_| StatusCode::Failure)?;

        if pflags.contains(OpenFlags::WRITE) {
            if existing.is_none() && !pflags.contains(OpenFlags::CREATE) {
                return Err(StatusCode::NoSuchFile);
            }
            if existing.is_some() && pflags.contains(OpenFlags::EXCLUDE) {
                return Err(StatusCode::Failure);
            }
            let Some(parent_id) = parent_id else {
                // A document cannot live loose at the root — only spaces do.
                return Err(StatusCode::PermissionDenied);
            };
            let buf = match &existing {
                Some(row) if !pflags.contains(OpenFlags::TRUNCATE) => {
                    row.get::<String, _>("body").into_bytes()
                }
                _ => Vec::new(),
            };
            let handle = self.allocate(OpenHandle::Write {
                id: existing.map(|row| row.get("id")),
                parent_id,
                slug: slug.to_string(),
                name: slug.to_string(),
                buf,
            });
            return Ok(Handle { id, handle });
        }

        let row = existing.ok_or(StatusCode::NoSuchFile)?;
        let body: String = row.get("body");
        let handle = self.allocate(OpenHandle::Read(body.into_bytes()));
        Ok(Handle { id, handle })
    }

    async fn read(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        len: u32,
    ) -> SftpResult<russh_sftp::protocol::Data> {
        let data = match self.handles.get(&handle) {
            Some(OpenHandle::Read(data)) => data,
            _ => return Err(StatusCode::Failure),
        };
        let offset = offset as usize;
        if offset >= data.len() {
            return Err(StatusCode::Eof);
        }
        let end = (offset + len as usize).min(data.len());
        Ok(russh_sftp::protocol::Data {
            id,
            data: data[offset..end].to_vec(),
        })
    }

    async fn write(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        data: Vec<u8>,
    ) -> SftpResult<russh_sftp::protocol::Status> {
        match self.handles.get_mut(&handle) {
            Some(OpenHandle::Write { buf, .. }) => {
                let offset = offset as usize;
                if buf.len() < offset {
                    buf.resize(offset, 0);
                }
                let end = offset + data.len();
                if buf.len() < end {
                    buf.resize(end, 0);
                }
                buf[offset..end].copy_from_slice(&data);
                Ok(ok_status(id))
            }
            _ => Err(StatusCode::Failure),
        }
    }

    async fn close(&mut self, id: u32, handle: String) -> SftpResult<russh_sftp::protocol::Status> {
        let Some(open) = self.handles.remove(&handle) else {
            return Err(StatusCode::Failure);
        };

        let OpenHandle::Write { id: node_id, parent_id, slug, name, buf } = open else {
            return Ok(ok_status(id));
        };

        let body = String::from_utf8(buf).map_err(|_| StatusCode::Failure)?;
        let now = crate::now();

        match node_id {
            Some(node_id) => {
                sqlx::query("UPDATE nodes SET body = ?1, updated_at = ?2 WHERE id = ?3")
                    .bind(&body)
                    .bind(now)
                    .bind(node_id)
                    .execute(&self.state.db)
                    .await
                    .map_err(|_| StatusCode::Failure)?;
                media::link_to_node(&self.state.db, node_id, &body)
                    .await
                    .ok();
            }
            None => {
                let slug = nodes::unique_slug(&self.state.db, Some(parent_id), &slug, None)
                    .await
                    .map_err(|_| StatusCode::Failure)?;
                let new_id: i64 = sqlx::query(
                    "INSERT INTO nodes (parent_id, kind, name, slug, body, created_at, updated_at)
                     VALUES (?1, 'document', ?2, ?3, ?4, ?5, ?5) RETURNING id",
                )
                .bind(parent_id)
                .bind(&name)
                .bind(&slug)
                .bind(&body)
                .bind(now)
                .fetch_one(&self.state.db)
                .await
                .map_err(|_| StatusCode::Failure)?
                .get("id");
                media::link_to_node(&self.state.db, new_id, &body).await.ok();
            }
        }

        self.state.backup.signal();
        Ok(ok_status(id))
    }

    async fn remove(&mut self, id: u32, filename: String) -> SftpResult<russh_sftp::protocol::Status> {
        let entry = self.entry_at(&filename).await?;
        if entry.kind != nodes::DOCUMENT {
            return Err(StatusCode::PermissionDenied);
        }
        sqlx::query("DELETE FROM nodes WHERE id = ?1")
            .bind(entry.id)
            .execute(&self.state.db)
            .await
            .map_err(|_| StatusCode::Failure)?;
        self.state.backup.signal();
        Ok(ok_status(id))
    }

    async fn mkdir(
        &mut self,
        id: u32,
        path: String,
        _attrs: FileAttributes,
    ) -> SftpResult<russh_sftp::protocol::Status> {
        let (dir, leaf) = Self::split_path(&path);
        let name = leaf.ok_or(StatusCode::PermissionDenied)?;
        let Some(parent_id) = self.resolve_dir(&dir).await? else {
            // A new space is made in the browser, not by mounting the root
            // and mkdir-ing into it — the root has no `has_board` to ask about.
            return Err(StatusCode::PermissionDenied);
        };
        let slug = nodes::unique_slug(&self.state.db, Some(parent_id), &nodes::slugify(name), None)
            .await
            .map_err(|_| StatusCode::Failure)?;
        let now = crate::now();
        sqlx::query(
            "INSERT INTO nodes (parent_id, kind, name, slug, created_at, updated_at)
             VALUES (?1, 'folder', ?2, ?3, ?4, ?4)",
        )
        .bind(parent_id)
        .bind(name)
        .bind(&slug)
        .bind(now)
        .execute(&self.state.db)
        .await
        .map_err(|_| StatusCode::Failure)?;
        self.state.backup.signal();
        Ok(ok_status(id))
    }

    async fn rmdir(&mut self, id: u32, path: String) -> SftpResult<russh_sftp::protocol::Status> {
        let entry = self.entry_at(&path).await?;
        if entry.kind == nodes::DOCUMENT {
            return Err(StatusCode::PermissionDenied);
        }
        let children = self.children(Some(entry.id)).await?;
        if !children.is_empty() {
            return Err(StatusCode::Failure);
        }
        sqlx::query("DELETE FROM nodes WHERE id = ?1")
            .bind(entry.id)
            .execute(&self.state.db)
            .await
            .map_err(|_| StatusCode::Failure)?;
        self.state.backup.signal();
        Ok(ok_status(id))
    }

    async fn rename(
        &mut self,
        id: u32,
        oldpath: String,
        newpath: String,
    ) -> SftpResult<russh_sftp::protocol::Status> {
        let entry = self.entry_at(&oldpath).await?;
        let (new_dir, new_leaf) = Self::split_path(&newpath);
        let new_name = match entry.kind.as_str() {
            k if k == nodes::DOCUMENT => new_leaf
                .and_then(doc_slug)
                .ok_or(StatusCode::PermissionDenied)?,
            _ => new_leaf.ok_or(StatusCode::PermissionDenied)?,
        };
        let new_parent = self.resolve_dir(&new_dir).await?;
        let Some(new_parent) = new_parent else {
            return Err(StatusCode::PermissionDenied);
        };
        let slug = nodes::unique_slug(
            &self.state.db,
            Some(new_parent),
            &nodes::slugify(new_name),
            Some(entry.id),
        )
        .await
        .map_err(|_| StatusCode::Failure)?;

        sqlx::query(
            "UPDATE nodes SET parent_id = ?1, name = ?2, slug = ?3, updated_at = ?4 WHERE id = ?5",
        )
        .bind(new_parent)
        .bind(new_name)
        .bind(&slug)
        .bind(crate::now())
        .bind(entry.id)
        .execute(&self.state.db)
        .await
        .map_err(|_| StatusCode::Failure)?;

        self.state.backup.signal();
        Ok(ok_status(id))
    }
}

fn ok_status(id: u32) -> russh_sftp::protocol::Status {
    russh_sftp::protocol::Status {
        id,
        status_code: StatusCode::Ok,
        error_message: "Ok".to_string(),
        language_tag: "en-US".to_string(),
    }
}
