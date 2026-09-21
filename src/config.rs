use std::{net::SocketAddr, path::PathBuf, time::Duration};

use anyhow::{Context, Result};
use chrono::Weekday;
use chrono_tz::Tz;

/// Runtime configuration. Everything comes from the environment (`.env` is
/// loaded on startup), because this is a deliberately single-user application.
#[derive(Debug, Clone)]
pub struct Config {
    pub username: String,
    pub password: String,
    pub secret: Vec<u8>,
    pub bind: SocketAddr,
    pub data_dir: PathBuf,
    pub session_days: i64,
    pub max_upload_bytes: usize,
    pub secure_cookie: bool,
    pub backup: BackupConfig,
    pub mail: MailConfig,
    pub sftp: SftpConfig,
}

/// The SFTP mount. Dormant until `WORKSPACE_SFTP_BIND` is set — same shape as
/// the mail relay, because a workspace with no key registered yet should not
/// refuse to start over a feature nobody has turned on.
#[derive(Debug, Clone)]
pub struct SftpConfig {
    pub bind: Option<SocketAddr>,
}

impl SftpConfig {
    fn from_env() -> Result<Self> {
        let bind = var("SFTP_BIND")
            .map(|v| v.parse())
            .transpose()
            .context("WORKSPACE_SFTP_BIND must look like 127.0.0.1:2222")?;
        Ok(Self { bind })
    }
}

/// Time of day in the configured timezone, as `HH:MM`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeOfDay {
    pub hour: u32,
    pub minute: u32,
}

/// Notification mail. Dormant until `WORKSPACE_SMTP_URL` is set — the same
/// dormant-until-configured shape the Proton mirror has, because a workspace
/// that refuses to start over an unconfigured mail relay would be a bad trade.
#[derive(Debug, Clone)]
pub struct MailConfig {
    /// `smtp://host:587` or `smtps://user:pass@host:465`. `None` turns every
    /// notification off, including the queue that would otherwise pile up.
    pub url: Option<String>,
    pub from: String,
    pub to: String,
    /// The timezone every "today" in the notifier is asked in.
    pub timezone: Tz,
    /// When to ask about the diary, if no document was written that day.
    pub reminder_at: Option<TimeOfDay>,
    /// When to mail about cards that are due or overdue.
    pub card_due_at: Option<TimeOfDay>,
    /// Weekday and time of the digest.
    pub digest_at: Option<(Weekday, TimeOfDay)>,
    /// The kinds that are allowed to be sent: `reminder`, `card_due`, `digest`,
    /// `login`. Listing them explicitly is how a single kind is switched off.
    pub kinds: Vec<String>,
}

impl MailConfig {
    /// Whether mail is configured at all.
    pub fn enabled(&self) -> bool {
        self.url.is_some()
    }

    pub fn sends(&self, kind: &str) -> bool {
        self.enabled() && self.kinds.iter().any(|k| k == kind)
    }
}

/// `20:00`, or `off` for nothing at all.
fn parse_time(key: &str, raw: &str) -> Result<Option<TimeOfDay>> {
    if raw.eq_ignore_ascii_case("off") || raw.eq_ignore_ascii_case("none") {
        return Ok(None);
    }
    let (hour, minute) = raw
        .split_once(':')
        .with_context(|| format!("{key} must look like 20:00, or be `off`"))?;
    let time = TimeOfDay {
        hour: hour.trim().parse().with_context(|| format!("{key}: {hour:?} is not an hour"))?,
        minute: minute
            .trim()
            .parse()
            .with_context(|| format!("{key}: {minute:?} is not a minute"))?,
    };
    if time.hour > 23 || time.minute > 59 {
        anyhow::bail!("{key} must be between 00:00 and 23:59");
    }
    Ok(Some(time))
}

fn parse_weekday(raw: &str) -> Option<Weekday> {
    match raw.to_ascii_lowercase().as_str() {
        "mon" | "monday" => Some(Weekday::Mon),
        "tue" | "tuesday" => Some(Weekday::Tue),
        "wed" | "wednesday" => Some(Weekday::Wed),
        "thu" | "thursday" => Some(Weekday::Thu),
        "fri" | "friday" => Some(Weekday::Fri),
        "sat" | "saturday" => Some(Weekday::Sat),
        "sun" | "sunday" => Some(Weekday::Sun),
        _ => None,
    }
}

/// `sun 18:00`, or `off`.
fn parse_digest(raw: &str) -> Result<Option<(Weekday, TimeOfDay)>> {
    let key = "WORKSPACE_DIGEST_AT";
    let Some((day, time)) = raw.split_once(char::is_whitespace) else {
        if raw.eq_ignore_ascii_case("off") || raw.eq_ignore_ascii_case("none") {
            return Ok(None);
        }
        anyhow::bail!("{key} must look like `sun 18:00`, or be `off`");
    };
    let day = parse_weekday(day.trim())
        .with_context(|| format!("{key}: {day:?} is not a weekday like `sun`"))?;
    Ok(parse_time(key, time.trim())?.map(|time| (day, time)))
}

/// The Proton Drive mirror. Dormant until a session has been stored by
/// `narl-workspace proton-login`; these values only shape how often it runs.
#[derive(Debug, Clone)]
pub struct BackupConfig {
    /// Name of the Drive device (its sync-root folder) this server owns.
    pub device_name: String,
    /// Backstop between runs. `None` disables the timer, leaving the mirror to
    /// whatever `POST /api/backup` asks for.
    pub interval: Option<Duration>,
    /// How long the workspace must be quiet after a change before it is mirrored,
    /// so a writing session is uploaded once rather than after every keystroke.
    pub debounce: Duration,
    /// Whether a file deleted here is trashed there. Off by default: a backup
    /// that forgets on command is one accident away from being no backup.
    pub prune: bool,
}

/// Look a setting up in the environment.
///
/// `WORKSPACE_*` is the name to use. `DIARY_*` is still read behind it, because
/// this application was called narl-diary until it grew spaces and the running
/// deployment's `.env` still says so; a rename that logs the owner out of their
/// own server would be a poor trade for a tidier prefix.
pub fn var(key: &str) -> Option<String> {
    std::env::var(format!("WORKSPACE_{key}"))
        .or_else(|_| std::env::var(format!("DIARY_{key}")))
        .ok()
        .filter(|v| !v.trim().is_empty())
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let username = var("USER").context("WORKSPACE_USER must be set (see .env.example)")?;
        let password =
            var("PASSWORD").context("WORKSPACE_PASSWORD must be set (see .env.example)")?;
        let secret = var("SECRET")
            .context("WORKSPACE_SECRET must be set (see .env.example)")?
            .into_bytes();
        if secret.len() < 16 {
            anyhow::bail!("WORKSPACE_SECRET must be at least 16 characters");
        }

        if password == "change-me" {
            tracing::warn!(
                "WORKSPACE_PASSWORD is still the example value from .env.example — \
                 anyone who can reach this server can read the workspace"
            );
        }

        let bind = var("BIND")
            .unwrap_or_else(|| "127.0.0.1:4242".to_string())
            .parse()
            .context("WORKSPACE_BIND must look like 127.0.0.1:4242")?;

        let data_dir = PathBuf::from(var("DATA_DIR").unwrap_or_else(|| "./data".into()));

        let session_days = var("SESSION_DAYS")
            .map(|v| v.parse())
            .transpose()
            .context("WORKSPACE_SESSION_DAYS must be a number")?
            .unwrap_or(30);

        let max_upload_bytes = var("MAX_UPLOAD_MB")
            .map(|v| v.parse::<usize>())
            .transpose()
            .context("WORKSPACE_MAX_UPLOAD_MB must be a number")?
            .unwrap_or(64)
            * 1024
            * 1024;

        let secure_cookie = var("SECURE_COOKIE")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);

        Ok(Self {
            username,
            password,
            secret,
            bind,
            data_dir,
            session_days,
            max_upload_bytes,
            secure_cookie,
            backup: BackupConfig::from_env()?,
            mail: MailConfig::from_env()?,
            sftp: SftpConfig::from_env()?,
        })
    }

    pub fn uploads_dir(&self) -> PathBuf {
        self.data_dir.join("uploads")
    }

    /// Where the Proton session blob lives. Next to the database, because the
    /// two are equally sensitive and equally worth putting on the same volume.
    pub fn proton_session_path(&self) -> PathBuf {
        self.data_dir.join("proton-session.json")
    }

    /// The SFTP server's own identity, generated once on first boot. Losing it
    /// only costs a host-key warning on every client, never data.
    pub fn sftp_host_key_path(&self) -> PathBuf {
        self.data_dir.join("sftp_host_key")
    }
}

impl MailConfig {
    fn from_env() -> Result<Self> {
        let url = var("SMTP_URL");

        // Both addresses are the owner's: this mails the workspace to its writer.
        let to = var("MAIL_TO").unwrap_or_default();
        let from = var("MAIL_FROM").unwrap_or_else(|| to.clone());
        if url.is_some() && (from.is_empty() || to.is_empty()) {
            anyhow::bail!("WORKSPACE_SMTP_URL is set, so WORKSPACE_MAIL_TO must be set too");
        }

        let timezone = var("TIMEZONE")
            .unwrap_or_else(|| "Europe/Berlin".to_string())
            .parse::<Tz>()
            .map_err(|e| anyhow::anyhow!("WORKSPACE_TIMEZONE: {e}"))?;

        let reminder_at = parse_time(
            "WORKSPACE_REMINDER_AT",
            &var("REMINDER_AT").unwrap_or_else(|| "20:00".into()),
        )?;
        let card_due_at = parse_time(
            "WORKSPACE_CARD_DUE_AT",
            &var("CARD_DUE_AT").unwrap_or_else(|| "08:00".into()),
        )?;
        let digest_at = parse_digest(&var("DIGEST_AT").unwrap_or_else(|| "sun 18:00".into()))?;

        let kinds = var("MAIL_KINDS")
            .unwrap_or_else(|| "reminder,card_due,digest,login".into())
            .split(',')
            .map(|k| k.trim().to_ascii_lowercase())
            .filter(|k| !k.is_empty() && k != "off" && k != "none")
            .collect::<Vec<_>>();
        if let Some(unknown) = kinds
            .iter()
            .find(|k| !["reminder", "card_due", "digest", "login"].contains(&k.as_str()))
        {
            anyhow::bail!(
                "WORKSPACE_MAIL_KINDS: {unknown:?} is not one of reminder, card_due, digest, login"
            );
        }

        Ok(Self {
            url,
            from,
            to,
            timezone,
            reminder_at,
            card_due_at,
            digest_at,
            kinds,
        })
    }
}

impl BackupConfig {
    fn from_env() -> Result<Self> {
        // The default keeps the old name on purpose: it names a device that
        // already exists in Proton Drive, and a new name there would mean a
        // second device and a fresh upload of everything in it.
        let device_name = var("PROTON_DEVICE").unwrap_or_else(|| "narl-diary".to_string());

        let minutes = var("BACKUP_INTERVAL_MIN")
            .map(|v| v.parse::<u64>())
            .transpose()
            .context("WORKSPACE_BACKUP_INTERVAL_MIN must be a number of minutes")?
            .unwrap_or(60);
        let interval = (minutes > 0).then(|| Duration::from_secs(minutes * 60));

        let debounce = Duration::from_secs(
            var("BACKUP_DEBOUNCE_SEC")
                .map(|v| v.parse::<u64>())
                .transpose()
                .context("WORKSPACE_BACKUP_DEBOUNCE_SEC must be a number of seconds")?
                .unwrap_or(300),
        );

        let prune = var("BACKUP_PRUNE")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);

        Ok(Self {
            device_name,
            interval,
            debounce,
            prune,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_digest, parse_time, TimeOfDay};
    use chrono::Weekday;

    #[test]
    fn a_time_of_day_is_read_as_written() {
        assert_eq!(
            parse_time("T", "20:00").unwrap(),
            Some(TimeOfDay { hour: 20, minute: 0 })
        );
        assert_eq!(
            parse_time("T", "07:45").unwrap(),
            Some(TimeOfDay { hour: 7, minute: 45 })
        );
        assert_eq!(parse_time("T", "off").unwrap(), None);
    }

    #[test]
    fn a_time_that_does_not_exist_is_refused_rather_than_rounded() {
        for raw in ["24:00", "20:60", "8", "eight", "20:00:00"] {
            assert!(parse_time("T", raw).is_err(), "{raw:?}");
        }
    }

    #[test]
    fn a_digest_is_a_weekday_and_a_time() {
        assert_eq!(
            parse_digest("sun 18:00").unwrap(),
            Some((Weekday::Sun, TimeOfDay { hour: 18, minute: 0 }))
        );
        assert_eq!(
            parse_digest("Monday 09:30").unwrap(),
            Some((Weekday::Mon, TimeOfDay { hour: 9, minute: 30 }))
        );
        assert_eq!(parse_digest("off").unwrap(), None);
        assert!(parse_digest("someday 18:00").is_err());
        assert!(parse_digest("sun").is_err());
    }
}
