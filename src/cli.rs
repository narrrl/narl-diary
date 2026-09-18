//! The subcommands that set the Proton Drive backup up and inspect it.
//!
//! Logging in is interactive exactly once: SRP needs the password and, on an
//! account with a second factor, a code. Everything after that resumes from the
//! stored session, so the server itself never prompts for anything. In Docker:
//!
//! ```sh
//! docker compose exec -it workspace narl-workspace proton-login
//! ```

use std::{
    io::{IsTerminal, Write},
    sync::Arc,
};

use anyhow::{bail, Context, Result};
use sqlx::{Row, SqlitePool};

use crate::{backup, config::Config, notify};

pub const USAGE: &str = "\
narl-workspace — a terminal-themed personal workspace

    narl-workspace                  serve the workspace (the default)
    narl-workspace proton-login     log in to Proton Drive and enable backups
    narl-workspace proton-status    show whether backups are configured
    narl-workspace proton-logout    forget the stored Proton session
    narl-workspace backup-now       mirror to Proton Drive once and exit
    narl-workspace mail-test        send one mail through the configured relay
    narl-workspace mail-status      show what mail is configured and what is queued
";

pub async fn proton_login(config: &Config) -> Result<()> {
    let store = backup::session::SessionStore::new(config.proton_session_path());

    // Said before anything is asked for, not after: Proton requires a
    // third-party client to be honest about what it is at the point where
    // someone is about to hand it their account.
    println!("{}", backup::proton::DISCLOSURE);
    println!("It talks to Proton Drive with your account, and stores the session locally.");
    println!();

    let username = match env("PROTON_USERNAME") {
        Some(username) => username,
        None => prompt("Proton username: ")?,
    };
    let password = match env("PROTON_PASSWORD") {
        Some(password) => password,
        None => {
            if !std::io::stdin().is_terminal() {
                bail!("no terminal to ask for a password — set PROTON_PASSWORD, or run with `docker compose exec -it`");
            }
            rpassword::prompt_password("Proton password: ").context("could not read the password")?
        }
    };

    backup::proton::login(&store, username.trim(), &password, || {
        match env("PROTON_TOTP") {
            Some(code) => Ok(code),
            None => prompt("Two-factor code: "),
        }
    })
    .await?;

    println!("Logged in. The session is stored at {}.", store.path().display());
    println!("Backups start with the next server start, or run `narl-workspace backup-now`.");
    Ok(())
}

pub fn proton_logout(config: &Config) -> Result<()> {
    let store = backup::session::SessionStore::new(config.proton_session_path());
    store.clear()?;
    println!("Forgot the Proton session. Backups are off until the next login.");
    println!(
        "The mirror already in Proton Drive is untouched; delete the device there to remove it."
    );
    Ok(())
}

pub async fn proton_status(db: &SqlitePool, config: &Config) -> Result<()> {
    let store = backup::session::SessionStore::new(config.proton_session_path());
    let Some(stored) = store.load()? else {
        println!("Proton Drive backups are not configured.");
        println!("Run `narl-workspace proton-login` to enable them.");
        return Ok(());
    };

    println!("Account:  {}", stored.username);
    println!("Device:   {}", config.backup.device_name);
    println!("Session:  {}", store.path().display());
    match config.backup.interval {
        Some(interval) => println!("Interval: every {} minutes", interval.as_secs() / 60),
        None => println!("Interval: off (only on change and on request)"),
    }
    println!(
        "Quiet:    {}s after the last change",
        config.backup.debounce.as_secs()
    );
    println!("Prune:    {}", if config.backup.prune { "on" } else { "off" });

    let mirrored: i64 = sqlx::query_scalar("SELECT count(*) FROM backup_files")
        .fetch_one(db)
        .await
        .unwrap_or(0);
    let last: Option<i64> = sqlx::query_scalar("SELECT max(uploaded_at) FROM backup_files")
        .fetch_one(db)
        .await
        .unwrap_or(None);
    println!("Mirrored: {mirrored} files");
    match last {
        Some(at) => println!("Last put: {at} (unix seconds)"),
        None => println!("Last put: never"),
    }
    Ok(())
}

fn env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.trim().is_empty())
}

fn prompt(label: &str) -> Result<String> {
    if !std::io::stdin().is_terminal() {
        bail!("no terminal to ask for {label:?} — run with `docker compose exec -it`, or set the matching environment variable");
    }
    print!("{label}");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    let line = line.trim().to_string();
    if line.is_empty() {
        bail!("nothing entered");
    }
    Ok(line)
}

/// One mail, straight out. SMTP credentials are wrong the first time, always,
/// and finding that out from a reminder that quietly never arrived is worse
/// than finding it out here.
pub async fn mail_test(db: SqlitePool, config: Arc<Config>) -> Result<()> {
    let notifier = notify::Notifier::new(db, config)?;
    notifier
        .send_test()
        .await
        .context("the test mail could not be sent")?;
    println!("Sent. If it does not arrive, the relay accepted it and something after that did not.");
    Ok(())
}

pub async fn mail_status(db: &SqlitePool, config: &Config) -> Result<()> {
    let mail = &config.mail;
    let Some(url) = mail.url.as_deref() else {
        println!("Mail is off. Set WORKSPACE_SMTP_URL to enable notifications.");
        return Ok(());
    };

    // The URL may carry a password; only the part that identifies the relay is
    // worth printing, and printing the rest into a terminal log is not.
    let relay = url.split('@').next_back().unwrap_or(url);
    println!("Relay:    {relay}");
    println!("From:     {}", mail.from);
    println!("To:       {}", mail.to);
    println!("Timezone: {}", mail.timezone);
    match mail.reminder_at {
        Some(at) if mail.sends("reminder") => {
            println!("Reminder: {:02}:{:02} when nothing was written", at.hour, at.minute)
        }
        _ => println!("Reminder: off"),
    }
    match mail.card_due_at {
        Some(at) if mail.sends("card_due") => {
            println!("Cards:    {:02}:{:02} for due and overdue cards", at.hour, at.minute)
        }
        _ => println!("Cards:    off"),
    }
    match mail.digest_at {
        Some((day, at)) if mail.sends("digest") => println!(
            "Digest:   {} {:02}:{:02}",
            notify::weekday_name(day),
            at.hour,
            at.minute
        ),
        _ => println!("Digest:   off"),
    }
    println!("Logins:   {}", if mail.sends("login") { "on" } else { "off" });

    let row = sqlx::query(
        "SELECT count(*) AS queued,
                sum(CASE WHEN sent_at IS NULL THEN 1 ELSE 0 END) AS pending,
                max(sent_at) AS last_sent
         FROM notifications",
    )
    .fetch_one(db)
    .await?;
    let queued: i64 = row.get("queued");
    let pending: Option<i64> = row.get("pending");
    let last_sent: Option<i64> = row.get("last_sent");
    println!("Outbox:   {queued} mails, {} unsent", pending.unwrap_or(0));
    match last_sent {
        Some(at) => println!(
            "Last out: {}",
            crate::notify::local_stamp(config.mail.timezone, at)
        ),
        None => println!("Last out: never"),
    }

    let failed = sqlx::query(
        "SELECT dedupe_key, attempts, last_error FROM notifications
         WHERE sent_at IS NULL AND last_error IS NOT NULL ORDER BY created_at LIMIT 5",
    )
    .fetch_all(db)
    .await?;
    for row in failed {
        let key: String = row.get("dedupe_key");
        let attempts: i64 = row.get("attempts");
        let error: String = row.get("last_error");
        println!("  {key}: {attempts} attempt(s), {error}");
    }
    Ok(())
}
