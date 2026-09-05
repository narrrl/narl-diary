//! The stored Proton session: everything needed to come back up unattended.
//!
//! A login is worth persisting whole, because none of it can be re-derived
//! without a human: the tokens (the session itself), the mailbox password
//! (which rebuilds the key chain on every start — the SDK needs it in memory to
//! decrypt anything), and the account's key salts.
//!
//! The salts matter more than they look. `core/v4/keys/salts` requires the
//! `locked` scope, and only the access token minted by a *password* login
//! carries it; once that token has been rotated through `auth/v4/refresh` — as
//! it is on every restart after the first — the endpoint answers 403 and the
//! key chain can no longer be unlocked. So the salts are captured at login and
//! handed back on resume, exactly as the desktop client does it.
//!
//! Refresh tokens are single-use: a rotation that is not written back leaves a
//! dead token on disk and the next resume fails. Hence the refresh hook in
//! [`super::proton`], which persists on every rotation.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use proton_drive_rs::KeySalt;
use proton_sdk::session::{PasswordMode, ProtonApiSession, ResumeParameters};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone)]
pub struct StoredSession {
    pub session_id: String,
    pub username: String,
    pub user_id: String,
    pub access_token: String,
    pub refresh_token: String,
    pub scopes: Vec<String>,
    /// `1` = single password, `2` = dual (Proton's wire values).
    pub password_mode: u8,
    /// Mailbox (data) password. `ProtonDriveClient` derives the key chain from
    /// it on every construction, so a backup that runs unattended has to hold
    /// it — there is no token-only mode.
    pub mailbox_password: String,
    /// Captured at login, because a refreshed token may no longer fetch them.
    #[serde(default)]
    pub key_salts: Vec<KeySalt>,
}

impl StoredSession {
    pub fn from_session(
        session: &ProtonApiSession,
        tokens: proton_sdk::http::Tokens,
        mailbox_password: &str,
        key_salts: Vec<KeySalt>,
    ) -> Self {
        Self {
            session_id: session.session_id().as_str().to_owned(),
            username: session.username().to_owned(),
            user_id: session.user_id().as_str().to_owned(),
            access_token: tokens.access_token,
            refresh_token: tokens.refresh_token,
            scopes: session.scopes().to_vec(),
            password_mode: match session.password_mode() {
                PasswordMode::Single => 1,
                PasswordMode::Dual => 2,
            },
            mailbox_password: mailbox_password.to_owned(),
            key_salts,
        }
    }

    pub fn resume_parameters(&self) -> ResumeParameters {
        ResumeParameters {
            session_id: self.session_id.clone().into(),
            username: self.username.clone(),
            user_id: self.user_id.clone().into(),
            access_token: self.access_token.clone(),
            refresh_token: self.refresh_token.clone(),
            scopes: self.scopes.clone(),
            is_waiting_for_second_factor_code: false,
            password_mode: match self.password_mode {
                1 => PasswordMode::Single,
                _ => PasswordMode::Dual,
            },
        }
    }
}

/// Where a session is kept.
///
/// The OS keyring is the right home for this and is what the desktop client
/// uses — but it is a Secret Service on D-Bus, and the diary's supported
/// deployment is a container, where there is neither. So the default is a
/// `0600` file next to the database: on the same volume as the diary it is
/// protecting, and no weaker than the thing it sits beside. Build with
/// `--features keyring` on a host that has a session bus to use the keyring
/// instead, with the file as the fallback when it cannot be reached.
#[derive(Clone)]
pub struct SessionStore {
    path: PathBuf,
}

impl SessionStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<Option<StoredSession>> {
        #[cfg(feature = "keyring")]
        if let Some(stored) = keyring_load()? {
            return Ok(Some(stored));
        }

        let json = match std::fs::read_to_string(&self.path) {
            Ok(json) => json,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => {
                return Err(e).with_context(|| format!("could not read {}", self.path.display()))
            }
        };
        let stored = serde_json::from_str(&json)
            .with_context(|| format!("{} is not a session this build understands", self.path.display()))?;
        Ok(Some(stored))
    }

    pub fn save(&self, stored: &StoredSession) -> Result<()> {
        let json = serde_json::to_string(stored)?;

        #[cfg(feature = "keyring")]
        if keyring_save(&json).is_ok() {
            return Ok(());
        }

        write_private(&self.path, json.as_bytes())
    }

    pub fn clear(&self) -> Result<()> {
        #[cfg(feature = "keyring")]
        let _ = keyring_clear();

        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e).with_context(|| format!("could not remove {}", self.path.display())),
        }
    }
}

/// Write `bytes` to `path` as `0600`, and atomically: a crash mid-write must
/// not leave a truncated session behind, because a half-written refresh token
/// is indistinguishable from a revoked one and costs an interactive re-login.
fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;

    let temporary = path.with_extension("tmp");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    let mut file = options
        .open(&temporary)
        .with_context(|| format!("could not write {}", temporary.display()))?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);

    std::fs::rename(&temporary, path)
        .with_context(|| format!("could not replace {}", path.display()))?;
    Ok(())
}

#[cfg(feature = "keyring")]
const KEYRING_SERVICE: &str = "narl-diary";
#[cfg(feature = "keyring")]
const KEYRING_USER: &str = "proton-session";

#[cfg(feature = "keyring")]
fn keyring_load() -> Result<Option<StoredSession>> {
    match keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).and_then(|e| e.get_password()) {
        Ok(json) => Ok(Some(serde_json::from_str(&json)?)),
        // No entry, no keyring, locked keyring: all mean "ask the file".
        Err(_) => Ok(None),
    }
}

#[cfg(feature = "keyring")]
fn keyring_save(json: &str) -> Result<()> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)?.set_password(json)?;
    Ok(())
}

#[cfg(feature = "keyring")]
fn keyring_clear() -> Result<()> {
    match keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).map(|e| e.delete_credential()) {
        Ok(Ok(())) | Ok(Err(keyring::Error::NoEntry)) | Err(_) => Ok(()),
        Ok(Err(e)) => Err(e.into()),
    }
}
