//! Automatic backup of the whole data directory to Proton Drive.
//!
//! The server registers itself as a Drive *device* — a sync root with its own
//! share — and keeps that folder equal to `DIARY_DATA_DIR`. It is deliberately
//! one-way: this is a backup, not a two-master sync, so nothing that happens in
//! Proton Drive can ever reach back and change the diary.
//!
//! It is dormant until someone runs `narl-diary proton-login` once. Nothing
//! else in the application depends on it, and a Proton outage costs log lines,
//! never a write.
//!
//! Runs are change-driven with a timer as a backstop: entry and media routes
//! call [`Backup::signal`], the loop waits for the diary to fall quiet, and
//! mirrors once. An hour of writing is one backup, not sixty.

pub mod proton;
pub mod session;
pub mod sync;

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

use anyhow::{Context, Result};
use proton_drive_rs::ProtonDriveClient;
use proton_sdk::ids::NodeUid;
use serde::Serialize;
use sqlx::{Row, SqlitePool};
use tokio::sync::Notify;

use crate::config::Config;
use session::SessionStore;
pub use sync::Summary;

/// What the status endpoint reports and the loop keeps up to date.
#[derive(Debug, Default, Clone, Serialize)]
pub struct Status {
    /// Whether a Proton session has been stored at all.
    pub configured: bool,
    pub device: Option<String>,
    pub running: bool,
    /// A change is waiting for the quiet period to elapse.
    pub pending: bool,
    pub last_run_at: Option<i64>,
    pub last_success_at: Option<i64>,
    /// The last failure, kept until a run succeeds — a backup that fails
    /// silently is worse than no backup, so this is surfaced in the UI.
    pub last_error: Option<String>,
    pub last: Option<Summary>,
}

pub struct Backup {
    db: SqlitePool,
    config: Arc<Config>,
    store: SessionStore,
    /// Woken by [`signal`](Self::signal) and by [`run_now`](Self::run_now).
    wake: Notify,
    dirty: AtomicBool,
    status: Mutex<Status>,
    /// Held for the duration of a run, so a manual backup and the timer cannot
    /// upload the same snapshot twice at once.
    running: tokio::sync::Mutex<()>,
}

impl Backup {
    pub fn new(db: SqlitePool, config: Arc<Config>) -> Arc<Self> {
        let store = SessionStore::new(config.proton_session_path());
        let configured = matches!(store.load(), Ok(Some(_)));
        Arc::new(Self {
            db,
            config,
            store,
            wake: Notify::new(),
            // The diary may have been written to while this process was down,
            // and a restart is the cheapest moment to find out.
            dirty: AtomicBool::new(true),
            status: Mutex::new(Status {
                configured,
                ..Status::default()
            }),
            running: tokio::sync::Mutex::new(()),
        })
    }

    pub fn status(&self) -> Status {
        let mut status = self.status.lock().expect("backup status mutex poisoned").clone();
        status.pending = self.dirty.load(Ordering::Relaxed);
        status
    }

    /// Something changed. Cheap and non-blocking: safe to call from a handler.
    pub fn signal(&self) {
        self.dirty.store(true, Ordering::Relaxed);
        self.wake.notify_one();
    }

    /// Mirror now, skipping the quiet period. Returns `None` when no Proton
    /// session has been stored — asking an unconfigured backup to run is a
    /// no-op, not a failure.
    pub async fn run_now(&self) -> Result<Option<Summary>> {
        let _guard = self.running.lock().await;
        self.dirty.store(false, Ordering::Relaxed);
        self.set(|s| {
            s.running = true;
            s.last_run_at = Some(crate::now());
        });

        let outcome = self.mirror().await;

        match &outcome {
            Ok(Some(summary)) => {
                let summary = *summary;
                self.set(move |s| {
                    s.running = false;
                    s.configured = true;
                    s.last_error = None;
                    s.last_success_at = Some(crate::now());
                    s.last = Some(summary);
                });
            }
            Ok(None) => {
                // Nothing was mirrored, so the changes are still outstanding:
                // a login later today should still find them waiting.
                self.dirty.store(true, Ordering::Relaxed);
                self.set(|s| {
                    s.running = false;
                    s.configured = false;
                });
            }
            Err(e) => {
                // The run failed; leave the work outstanding so the next tick
                // retries it rather than waiting for the next change.
                self.dirty.store(true, Ordering::Relaxed);
                let message = format!("{e:#}");
                tracing::error!(error = %message, "Proton Drive backup failed");
                self.set(move |s| {
                    s.running = false;
                    s.last_error = Some(message.clone());
                });
            }
        }

        outcome
    }

    async fn mirror(&self) -> Result<Option<Summary>> {
        let Some(client) = proton::resume(&self.store).await? else {
            return Ok(None);
        };

        let (root, uploads) = self.device_folders(&client).await?;
        let summary = sync::run(&self.db, &self.config, &client, &root, &uploads).await?;
        tracing::info!(
            uploaded = summary.uploaded,
            skipped = summary.skipped,
            pruned = summary.pruned,
            bytes = summary.bytes,
            "Proton Drive backup finished"
        );
        Ok(Some(summary))
    }

    /// The two folders the mirror writes into: `data/` on the device, and
    /// `data/uploads/` inside it. Both are registered on first use and then
    /// remembered.
    ///
    /// Nothing is written to the device's own root, because Proton refuses it —
    /// a device root holds folders, and a file there is answered with 422
    /// "Cannot create file at the root of a device". `data/` is that folder,
    /// and it is also the honest name for what it holds: the diary's data
    /// directory, copied.
    ///
    /// Remembering them is the point. Resolving either by name means listing a
    /// folder's children and decrypting every name in it, and Proton asks
    /// third-party clients not to traverse the tree repeatedly for things that
    /// do not change. So each uid is cached and confirmed with a single node
    /// fetch; only a uid that has genuinely gone — the folder deleted in the
    /// Drive UI — costs a lookup, and then it is re-created rather than
    /// searched for.
    async fn device_folders(&self, client: &ProtonDriveClient) -> Result<(NodeUid, NodeUid)> {
        let name = self.config.backup.device_name.clone();
        let cached_name = self.state_get("device_name").await?;
        let mut fresh = cached_name.as_deref() != Some(name.as_str());

        let root = match self.cached_node(client, "device_root", fresh).await? {
            Some(uid) => uid,
            None => {
                let device = proton::ensure_device(client, &name).await?;
                self.state_set("device_name", &name).await?;
                self.state_set("device_uid", device.uid.as_str()).await?;
                self.state_set("device_root", &device.root_folder_uid.to_string())
                    .await?;
                // Folders remembered inside the old root mean nothing in a new
                // one, whichever of them still resolves.
                fresh = true;
                device.root_folder_uid
            }
        };

        let data = match self.cached_node(client, "device_data", fresh).await? {
            Some(uid) => uid,
            None => {
                let uid = client
                    .create_folder_path(&root, "data")
                    .await
                    .context("could not create the data folder on the device")?;
                self.state_set("device_data", &uid.to_string()).await?;
                uid
            }
        };

        let uploads = match self.cached_node(client, "device_uploads", fresh).await? {
            Some(uid) => uid,
            None => {
                let uid = client
                    .create_folder_path(&data, "uploads")
                    .await
                    .context("could not create the uploads folder on the device")?;
                self.state_set("device_uploads", &uid.to_string()).await?;
                uid
            }
        };

        self.set(move |s| s.device = Some(name.clone()));
        Ok((data, uploads))
    }

    /// A remembered node, if it is still remembered and still there.
    async fn cached_node(
        &self,
        client: &ProtonDriveClient,
        key: &str,
        stale: bool,
    ) -> Result<Option<NodeUid>> {
        if stale {
            return Ok(None);
        }
        let Some(uid) = self.state_get(key).await?.as_deref().and_then(sync::parse_uid) else {
            return Ok(None);
        };
        // A failed lookup is not a missing node — a network error must not be
        // read as "the folder is gone" and answered by making a second one.
        match client.get_node(&uid).await {
            Ok(Some(_)) => Ok(Some(uid)),
            Ok(None) => Ok(None),
            Err(e) => Err(e).with_context(|| format!("could not confirm the {key} folder")),
        }
    }

    async fn state_get(&self, key: &str) -> Result<Option<String>> {
        let row = sqlx::query("SELECT value FROM backup_state WHERE key = ?")
            .bind(key)
            .fetch_optional(&self.db)
            .await?;
        Ok(row.map(|row| row.get("value")))
    }

    async fn state_set(&self, key: &str, value: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO backup_state (key, value) VALUES (?, ?)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        )
        .bind(key)
        .bind(value)
        .execute(&self.db)
        .await?;
        Ok(())
    }

    fn set(&self, edit: impl FnOnce(&mut Status)) {
        let mut status = self.status.lock().expect("backup status mutex poisoned");
        edit(&mut status);
    }
}

/// Run the mirror in the background for as long as the server lives.
pub fn spawn(backup: Arc<Backup>) {
    if backup.store.load().ok().flatten().is_none() {
        tracing::info!(
            "Proton Drive backups are not configured — run `narl-diary proton-login` to enable them"
        );
        return;
    }

    tokio::spawn(async move {
        let interval = backup.config.backup.interval;
        let debounce = backup.config.backup.debounce;

        loop {
            // Nothing to do: wait for a change, or for the backstop timer.
            while !backup.dirty.load(Ordering::Relaxed) {
                match interval {
                    Some(interval) => {
                        tokio::select! {
                            _ = backup.wake.notified() => {}
                            _ = tokio::time::sleep(interval) => break,
                        }
                    }
                    None => backup.wake.notified().await,
                }
            }

            // Let the diary fall quiet. Every further change restarts the wait,
            // so a writing session is mirrored once, when it is over.
            loop {
                tokio::select! {
                    _ = backup.wake.notified() => continue,
                    _ = tokio::time::sleep(debounce) => break,
                }
            }

            let _ = backup.run_now().await;
        }
    });
}

/// One-shot mirror for `narl-diary backup-now`, without starting a server.
pub async fn run_once(db: SqlitePool, config: Arc<Config>) -> Result<Summary> {
    let backup = Backup::new(db, config);
    backup
        .run_now()
        .await?
        .context("no Proton session stored — run `narl-diary proton-login` first")
}
