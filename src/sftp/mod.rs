//! Spaces and folders, mounted over SFTP instead of the browser. A separate
//! authentication path on purpose: the cookie session is for one browser tab
//! at a time, and a mount is a standing thing a laptop keeps open for weeks.
//! Trust here comes from a public key registered in `ssh_keys`, not a login.

mod fs;

use std::{net::SocketAddr, sync::Arc};

use anyhow::{Context, Result};
use russh::{
    keys::{Algorithm, PrivateKey, PublicKey},
    server::{
        Auth, ChannelOpenHandle, Config as ServerConfig, Handler as ServerHandler, Msg,
        Server as _, Session,
    },
    Channel, ChannelId,
};
use sqlx::Row;

use crate::state::AppState;

/// Normalise an `authorized_keys` line to `algorithm base64` (no comment) and
/// its SHA-256 fingerprint, the two things stored for a registered key.
pub fn fingerprint_of(line: &str) -> Result<(String, String)> {
    let key = PublicKey::from_openssh(line.trim())?;
    let normalized = key.to_openssh()?;
    let fingerprint = key.fingerprint(russh::keys::HashAlg::Sha256).to_string();
    Ok((normalized, fingerprint))
}

/// Load the host key from disk, generating and persisting one on first boot.
/// Losing this file only costs every client a host-key warning, never data,
/// so it lives next to the database rather than anywhere more precious.
fn load_or_create_host_key(path: &std::path::Path) -> Result<PrivateKey> {
    if path.exists() {
        return PrivateKey::read_openssh_file(path)
            .with_context(|| format!("could not read {}", path.display()));
    }
    let key = PrivateKey::random(&mut rand10::rng(), Algorithm::Ed25519)?;
    key.write_openssh_file(path, russh::keys::ssh_key::LineEnding::LF)
        .with_context(|| format!("could not write {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(key)
}

pub async fn serve(state: AppState, bind: SocketAddr) -> Result<()> {
    let host_key = load_or_create_host_key(&state.config.sftp_host_key_path())?;

    let config = Arc::new(ServerConfig {
        keys: vec![host_key],
        ..Default::default()
    });

    let mut server = SftpServer { state };
    tracing::info!("sftp mount listening on {bind}");
    server.run_on_address(config, bind).await?;
    Ok(())
}

#[derive(Clone)]
struct SftpServer {
    state: AppState,
}

impl russh::server::Server for SftpServer {
    type Handler = SshSession;

    fn new_client(&mut self, _addr: Option<SocketAddr>) -> Self::Handler {
        SshSession {
            state: self.state.clone(),
            channel: None,
        }
    }
}

struct SshSession {
    state: AppState,
    /// The one channel a mount opens for its sftp subsystem. A second
    /// `channel_open_session` would mean a shell or a second subsystem,
    /// neither of which this server offers.
    channel: Option<Channel<Msg>>,
}

impl ServerHandler for SshSession {
    type Error = anyhow::Error;

    async fn auth_publickey(
        &mut self,
        _user: &str,
        public_key: &PublicKey,
    ) -> Result<Auth, Self::Error> {
        let fingerprint = public_key.fingerprint(russh::keys::HashAlg::Sha256).to_string();
        let matched = sqlx::query("SELECT id FROM ssh_keys WHERE fingerprint = ?1")
            .bind(&fingerprint)
            .fetch_optional(&self.state.db)
            .await?;

        match matched {
            Some(row) => {
                let id: i64 = row.get("id");
                sqlx::query("UPDATE ssh_keys SET last_used_at = ?1 WHERE id = ?2")
                    .bind(crate::now())
                    .bind(id)
                    .execute(&self.state.db)
                    .await?;
                Ok(Auth::Accept)
            }
            None => Ok(Auth::reject()),
        }
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.channel = Some(channel);
        reply.accept().await;
        Ok(())
    }

    async fn subsystem_request(
        &mut self,
        channel_id: ChannelId,
        name: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if name != "sftp" {
            session.channel_failure(channel_id)?;
            return Ok(());
        }
        let Some(channel) = self.channel.take() else {
            session.channel_failure(channel_id)?;
            return Ok(());
        };
        session.channel_success(channel_id)?;
        let handler = fs::SpaceFs::new(self.state.clone());
        tokio::spawn(russh_sftp::server::run(channel.into_stream(), handler));
        Ok(())
    }
}
