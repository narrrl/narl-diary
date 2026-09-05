//! The subcommands that set the Proton Drive backup up and inspect it.
//!
//! Logging in is interactive exactly once: SRP needs the password and, on an
//! account with a second factor, a code. Everything after that resumes from the
//! stored session, so the server itself never prompts for anything. In Docker:
//!
//! ```sh
//! docker compose exec -it diary narl-diary proton-login
//! ```

use std::io::{IsTerminal, Write};

use anyhow::{bail, Context, Result};
use sqlx::SqlitePool;

use crate::{backup, config::Config};

pub const USAGE: &str = "\
narl-diary — a terminal-themed personal diary

    narl-diary                  serve the diary (the default)
    narl-diary proton-login     log in to Proton Drive and enable backups
    narl-diary proton-status    show whether backups are configured
    narl-diary proton-logout    forget the stored Proton session
    narl-diary backup-now       mirror to Proton Drive once and exit
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
    println!("Backups start with the next server start, or run `narl-diary backup-now`.");
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
        println!("Run `narl-diary proton-login` to enable them.");
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
