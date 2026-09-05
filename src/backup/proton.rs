//! Talking to Proton: logging in once, resuming forever, owning one device.

use anyhow::{anyhow, Context, Result};
use proton_drive_rs::{Device, DeviceType, ProtonDriveClient};
use proton_sdk::config::ProtonClientConfiguration;
use proton_sdk::session::ProtonApiSession;

use super::session::{SessionStore, StoredSession};

/// What Proton asks a third-party client to tell anyone it takes credentials
/// from. Shown by `narl-diary proton-login` before it asks for anything.
pub const DISCLOSURE: &str =
    "This is a third-party application not officially supported by Proton.";

/// How this client identifies itself in `x-pm-appversion`.
///
/// Proton's third-party rules require the shape
/// `external-drive-{name}@{semver}-{channel}`, where the name is lowercase
/// letters and underscores, and require that the value honestly describe the
/// application — a header that misrepresents its client, or imitates a
/// first-party Proton one, is a rule violation, not a cosmetic detail, and a
/// malformed one is answered with a 422 "unusual activity" that no login
/// survives. So: `narl_diary` is the application, the version is the crate's
/// own, and the channel follows it — a `0.x` release is not a stable one, and
/// saying otherwise would be the same misrepresentation in miniature.
const APP_NAME: &str = "narl_diary";

fn app_version() -> String {
    let version = env!("CARGO_PKG_VERSION");
    let channel = if version.starts_with("0.") { "alpha" } else { "stable" };
    format!("external-drive-{APP_NAME}@{version}-{channel}")
}

/// The identification headers. Overridable only for the case the shape itself
/// changes on Proton's side; the honest default is the one above.
fn client_config() -> ProtonClientConfiguration {
    let version = std::env::var("DIARY_PROTON_APP_VERSION")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(app_version);
    let agent = format!("narl-diary/{}", env!("CARGO_PKG_VERSION"));
    ProtonClientConfiguration::new(version).with_user_agent(agent)
}

/// Run an SRP login, complete 2FA if the account asks for one, and persist the
/// result. `get_totp` is only called when a second factor is actually required,
/// so accounts without one are never prompted.
pub async fn login(
    store: &SessionStore,
    username: &str,
    password: &str,
    get_totp: impl FnOnce() -> Result<String>,
) -> Result<()> {
    let mut session = ProtonApiSession::begin(client_config(), username, password.as_bytes())
        .await
        .context("Proton rejected the login")?;

    if session.is_waiting_for_second_factor() {
        let code = get_totp()?;
        session
            .apply_second_factor_code(code.trim())
            .await
            .context("Proton rejected the second-factor code")?;
    }

    // Salts are fetched here and only here: this access token still carries the
    // `locked` scope, and no refreshed one ever will.
    let client = ProtonDriveClient::new(&session, password.as_bytes().to_vec());
    let key_salts = client
        .account()
        .key_salts()
        .await
        .context("could not read the account key salts")?;

    let tokens = session.current_tokens().await;
    store.save(&StoredSession::from_session(
        &session,
        tokens,
        password,
        key_salts,
    ))?;
    Ok(())
}

/// Rebuild a Drive client from the stored session, or `None` when nothing has
/// been stored yet — that is the ordinary "backups are not set up" state, not
/// an error.
pub async fn resume(store: &SessionStore) -> Result<Option<ProtonDriveClient>> {
    let Some(stored) = store.load()? else {
        return Ok(None);
    };

    let session = ProtonApiSession::resume(client_config(), stored.resume_parameters())
        .context("the stored Proton session could not be resumed")?;

    if stored.key_salts.is_empty() {
        return Err(anyhow!(
            "the stored Proton session has no key salts — run `narl-diary proton-login` again"
        ));
    }

    // Every 401 refresh mints a new refresh token and invalidates the old one,
    // so the rotation has to reach disk before the process can next restart.
    let store_for_hook = store.clone();
    let stored_for_hook = stored.clone();
    session.http().set_on_tokens_refreshed(move |tokens| {
        let refreshed = StoredSession {
            access_token: tokens.access_token,
            refresh_token: tokens.refresh_token,
            ..stored_for_hook.clone()
        };
        if let Err(e) = store_for_hook.save(&refreshed) {
            tracing::error!(error = %e, "could not persist refreshed Proton tokens — the next start will need a new login");
        }
    });

    let client = ProtonDriveClient::with_key_salts(
        &session,
        stored.mailbox_password.clone().into_bytes(),
        stored.key_salts.clone(),
    )
    // A diary is mostly small files; one atomic request beats the
    // draft/block/commit dance for anything that fits in a single block.
    .with_small_file_upload(true);

    Ok(Some(client))
}

/// The device this server owns, registered on first use.
///
/// A device is a sync root with its own share, which is what makes the diary
/// appear in Proton Drive as a machine rather than as a folder someone dropped
/// in My Files — and keeps it out of the way of everything else in the account.
pub async fn ensure_device(client: &ProtonDriveClient, name: &str) -> Result<Device> {
    let devices = client
        .enumerate_devices()
        .await
        .context("could not list the account's devices")?;

    // A device whose name failed to decrypt is not ours to claim: adopting it
    // would mirror the diary into a stranger of a folder.
    if let Some(device) = devices
        .into_iter()
        .find(|d| d.name.as_deref().map(|n| n == name).unwrap_or(false))
    {
        return Ok(device);
    }

    tracing::info!(device = name, "registering a new Proton Drive device");
    client
        .create_device(name, DeviceType::Linux)
        .await
        .with_context(|| format!("could not register the device {name:?}"))
}

#[cfg(test)]
mod tests {
    use super::app_version;

    #[test]
    fn identifies_itself_the_way_proton_requires() {
        let version = app_version();
        let (prefix, rest) = version.split_once('@').expect("name@version");
        // Lowercase letters and underscores after the fixed `external-drive-`.
        let name = prefix.strip_prefix("external-drive-").expect("the required prefix");
        assert!(!name.is_empty());
        assert!(name.chars().all(|c| c.is_ascii_lowercase() || c == '_'), "{name}");

        let (semver, channel) = rest.rsplit_once('-').expect("version-channel");
        assert!(matches!(channel, "stable" | "beta" | "alpha"), "{channel}");
        assert_eq!(semver.split('.').count(), 3, "{semver}");
        assert!(semver.split('.').all(|part| part.parse::<u32>().is_ok()), "{semver}");
    }
}
