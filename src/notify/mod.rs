//! What is worth a mail, and when.
//!
//! One task, one tick a minute, four questions asked on it: was anything
//! written in the diary today, is a card due or overdue, is it time for the
//! weekly digest, and — from the login route rather than the tick — has a
//! browser signed in that has not signed in before.
//!
//! Every question is answered in the configured timezone. "Today" is a local
//! day, and DST means a local day is not always 86400 seconds long, so the
//! boundaries are computed rather than rounded to.
//!
//! Nothing is sent from here. Everything found is enqueued into the outbox in
//! [`crate::mail`] under a key that names the occurrence, so asking twice —
//! which the tick does, once a minute, all day — costs one insert that does
//! nothing.

use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::{Datelike, Duration as ChronoDuration, NaiveDate, TimeZone, Utc, Weekday};
use chrono_tz::Tz;
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};

use crate::{
    config::{Config, TimeOfDay},
    mail::{self, Mailer},
};

pub struct Notifier {
    db: SqlitePool,
    config: Arc<Config>,
    /// `None` when no SMTP URL is configured; then nothing is enqueued either.
    mailer: Option<Mailer>,
}

impl Notifier {
    pub fn new(db: SqlitePool, config: Arc<Config>) -> Result<Arc<Self>> {
        let mailer = Mailer::new(&config.mail)?;
        Ok(Arc::new(Self { db, config, mailer }))
    }

    pub fn enabled(&self) -> bool {
        self.mailer.is_some()
    }

    /// One pass: find what is due, then drain the outbox.
    pub async fn tick(&self) -> Result<()> {
        let Some(mailer) = self.mailer.as_ref() else {
            return Ok(());
        };
        if let Err(e) = self.sweep(crate::now()).await {
            // A failing sweep must not stop the outbox from draining: the mails
            // already in it are the ones that matter most.
            tracing::error!(error = %format!("{e:#}"), "the notification sweep failed");
        }
        mail::send_pending(&self.db, mailer).await?;
        Ok(())
    }

    /// Everything the clock decides, at the instant `now`.
    pub async fn sweep(&self, now: i64) -> Result<()> {
        let tz = self.config.mail.timezone;
        let today = local_day(tz, now);

        if self.config.mail.sends("reminder") {
            if let Some(at) = self.config.mail.reminder_at {
                if time_reached(tz, now, today, at) {
                    self.sweep_reminder(today).await?;
                }
            }
        }

        if self.config.mail.sends("card_due") {
            if let Some(at) = self.config.mail.card_due_at {
                if time_reached(tz, now, today, at) {
                    self.sweep_cards(today).await?;
                }
            }
        }

        if self.config.mail.sends("digest") {
            if let Some((day, at)) = self.config.mail.digest_at {
                if today.weekday() == day && time_reached(tz, now, today, at) {
                    self.sweep_digest(today).await?;
                }
            }
        }

        Ok(())
    }

    /// "You have not written today." Asked once the reminder time has passed,
    /// and only if the diary space gained no document that local day.
    async fn sweep_reminder(&self, today: NaiveDate) -> Result<()> {
        let (start, end) = day_bounds(self.config.mail.timezone, today);
        let written: i64 = sqlx::query_scalar(
            "WITH RECURSIVE tree (id) AS (
                 SELECT id FROM nodes WHERE parent_id IS NULL AND slug = 'diary'
                 UNION ALL
                 SELECT n.id FROM nodes n JOIN tree t ON n.parent_id = t.id
             )
             SELECT count(*) FROM nodes
             WHERE kind = 'document' AND id IN (SELECT id FROM tree)
               AND created_at >= ? AND created_at < ?",
        )
        .bind(start)
        .bind(end)
        .fetch_one(&self.db)
        .await?;

        if written > 0 {
            return Ok(());
        }

        let subject = format!("workspace — nothing written on {today}");
        let body = format!(
            "No entry in the diary for {today}.\n\n\
             Open the diary, press o, and write the day down while it is still today.\n"
        );
        self.enqueue("reminder", &format!("reminder:{today}"), &subject, &body)
            .await
    }

    /// Cards whose due date has arrived, and cards that have slipped past it.
    /// The due date is part of the key, so moving a card's date asks again.
    async fn sweep_cards(&self, today: NaiveDate) -> Result<()> {
        let tz = self.config.mail.timezone;
        let (_, end_of_today) = day_bounds(tz, today);

        let rows = sqlx::query(
            "SELECT c.id, c.title, c.due_at, l.name AS list_name, s.name AS space_name
             FROM cards c
             JOIN board_lists l ON l.id = c.list_id
             JOIN nodes s ON s.id = l.space_id
             WHERE c.done_at IS NULL AND c.due_at IS NOT NULL AND c.due_at < ?
             ORDER BY c.due_at",
        )
        .bind(end_of_today)
        .fetch_all(&self.db)
        .await?;

        for row in rows {
            let id: i64 = row.get("id");
            let title: String = row.get("title");
            let space: String = row.get("space_name");
            let list: String = row.get("list_name");
            let due = local_day(tz, row.get("due_at"));

            let (stage, subject, opening) = if due < today {
                let days = (today - due).num_days();
                (
                    "overdue",
                    format!("{space} — overdue: {title}"),
                    format!("\"{title}\" was due on {due}, {days} day(s) ago."),
                )
            } else {
                (
                    "due",
                    format!("{space} — due today: {title}"),
                    format!("\"{title}\" is due today, {due}."),
                )
            };

            let body = format!("{opening}\n\nSpace: {space}\nList:  {list}\n");
            self.enqueue(
                "card_due",
                &format!("card_due:{id}:{due}:{stage}"),
                &subject,
                &body,
            )
            .await?;
        }

        Ok(())
    }

    /// The week just gone: what was written, what was finished, what is open.
    async fn sweep_digest(&self, today: NaiveDate) -> Result<()> {
        let tz = self.config.mail.timezone;
        let (_, end) = day_bounds(tz, today);
        let (start, _) = day_bounds(tz, today - ChronoDuration::days(6));

        let written = sqlx::query(
            "WITH RECURSIVE tree (space_id, id) AS (
                 SELECT id, id FROM nodes WHERE parent_id IS NULL AND kind = 'space'
                 UNION ALL
                 SELECT t.space_id, n.id FROM nodes n JOIN tree t ON n.parent_id = t.id
             )
             SELECT s.name AS space_name, count(*) AS written
             FROM nodes n
             JOIN tree t ON t.id = n.id
             JOIN nodes s ON s.id = t.space_id
             WHERE n.kind = 'document' AND n.created_at >= ? AND n.created_at < ?
             GROUP BY s.id ORDER BY s.position",
        )
        .bind(start)
        .bind(end)
        .fetch_all(&self.db)
        .await?;

        let finished = sqlx::query(
            "SELECT s.name AS space_name, c.title AS title
             FROM cards c
             JOIN board_lists l ON l.id = c.list_id
             JOIN nodes s ON s.id = l.space_id
             WHERE c.done_at >= ? AND c.done_at < ?
             ORDER BY s.position, c.done_at",
        )
        .bind(start)
        .bind(end)
        .fetch_all(&self.db)
        .await?;

        let open = sqlx::query(
            "SELECT s.name AS space_name, c.title AS title, c.due_at AS due_at
             FROM cards c
             JOIN board_lists l ON l.id = c.list_id
             JOIN nodes s ON s.id = l.space_id
             WHERE c.done_at IS NULL AND c.due_at IS NOT NULL AND c.due_at < ?
             ORDER BY c.due_at",
        )
        .bind(end)
        .fetch_all(&self.db)
        .await?;

        let mut body = format!("The week to {today}.\n\nWritten\n");
        if written.is_empty() {
            body.push_str("  nothing\n");
        }
        for row in &written {
            let space: String = row.get("space_name");
            let count: i64 = row.get("written");
            body.push_str(&format!("  {space}: {count} document(s)\n"));
        }

        body.push_str("\nFinished\n");
        if finished.is_empty() {
            body.push_str("  nothing\n");
        }
        for row in &finished {
            let space: String = row.get("space_name");
            let title: String = row.get("title");
            body.push_str(&format!("  {space}: {title}\n"));
        }

        body.push_str("\nStill open, due or overdue\n");
        if open.is_empty() {
            body.push_str("  nothing\n");
        }
        for row in &open {
            let space: String = row.get("space_name");
            let title: String = row.get("title");
            let due = local_day(tz, row.get("due_at"));
            body.push_str(&format!("  {space}: {title} (due {due})\n"));
        }

        let week = iso_week_key(today);
        self.enqueue(
            "digest",
            &format!("digest:{week}"),
            &format!("workspace — week {week}"),
            &body,
        )
        .await
    }

    /// A login happened. The browser is remembered by a hash of its User-Agent,
    /// and one that has not been seen before is worth a mail.
    ///
    /// That is all the hash can honestly claim: a browser, not a person. It
    /// needs no proxy header to be trusted, which is the reason to prefer it
    /// over an address that a reverse proxy could be lying about.
    pub async fn note_login(&self, user_agent: &str) {
        if !self.config.mail.sends("login") {
            return;
        }
        if let Err(e) = self.record_login(user_agent).await {
            tracing::error!(error = %format!("{e:#}"), "could not record a login");
        }
    }

    async fn record_login(&self, user_agent: &str) -> Result<()> {
        let hash = device_hash(user_agent);
        let now = crate::now();
        let inserted = sqlx::query(
            "INSERT INTO login_events (device_hash, user_agent, first_seen_at, last_seen_at)
             VALUES (?, ?, ?, ?)
             ON CONFLICT (device_hash) DO UPDATE SET last_seen_at = excluded.last_seen_at",
        )
        .bind(&hash)
        .bind(user_agent)
        .bind(now)
        .bind(now)
        .execute(&self.db)
        .await?
        .rows_affected();

        // The upsert touches a row either way, so "new" is asked separately.
        let first_seen: i64 = sqlx::query_scalar(
            "SELECT first_seen_at FROM login_events WHERE device_hash = ?",
        )
        .bind(&hash)
        .fetch_one(&self.db)
        .await?;
        if inserted == 0 || first_seen != now {
            return Ok(());
        }

        let when = local_stamp(self.config.mail.timezone, now);
        self.enqueue(
            "login",
            &format!("login:{hash}"),
            "workspace — signed in from a new browser",
            &format!(
                "A browser that has not signed in before signed in at {when}.\n\n\
                 User-Agent: {user_agent}\n\n\
                 If that was not you, change WORKSPACE_PASSWORD and rotate WORKSPACE_SECRET — \
                 rotating the secret signs every device out.\n"
            ),
        )
        .await
    }

    /// A run of wrong passwords. Enqueued at most once an hour, because the
    /// point is to notice an attempt, not to be mailed by the attacker.
    pub async fn note_failed_logins(&self, failures: u32) {
        if !self.config.mail.sends("login") {
            return;
        }
        let hour = local_stamp(self.config.mail.timezone, crate::now());
        let hour = &hour[..13.min(hour.len())];
        let outcome = self
            .enqueue(
                "login",
                &format!("login_failed:{hour}"),
                "workspace — repeated failed logins",
                &format!(
                    "{failures} failed login attempts. The login is being answered more \
                     slowly with every one of them.\n"
                ),
            )
            .await;
        if let Err(e) = outcome {
            tracing::error!(error = %format!("{e:#}"), "could not record failed logins");
        }
    }

    async fn enqueue(&self, kind: &str, key: &str, subject: &str, body: &str) -> Result<()> {
        if mail::enqueue(&self.db, kind, key, subject, body).await? {
            tracing::info!(kind, key, "queued a notification");
        }
        Ok(())
    }

    /// One mail, sent straight out, for `narl-workspace mail-test`. It skips the
    /// outbox on purpose: the question it answers is whether the relay works,
    /// and a queued row would answer it later or not at all.
    pub async fn send_test(&self) -> Result<()> {
        let mailer = self
            .mailer
            .as_ref()
            .context("WORKSPACE_SMTP_URL is not set — mail is off")?;
        let when = local_stamp(self.config.mail.timezone, crate::now());
        mailer
            .send(
                "workspace — test",
                &format!("This is narl-workspace checking its SMTP settings at {when}.\n"),
            )
            .await
    }
}

/// Run the notifier for as long as the server lives.
pub fn spawn(notifier: Arc<Notifier>) {
    if !notifier.enabled() {
        tracing::info!("mail is off — set WORKSPACE_SMTP_URL to enable notifications");
        return;
    }

    tokio::spawn(async move {
        // A minute is fine-grained enough for a configuration written as HH:MM
        // and coarse enough to be free.
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(60));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            if let Err(e) = notifier.tick().await {
                tracing::error!(error = %format!("{e:#}"), "the notification tick failed");
            }
        }
    });
}

/// SHA-256 of the User-Agent, hex. Not a secret — a stable name for a browser.
fn device_hash(user_agent: &str) -> String {
    let digest = Sha256::digest(user_agent.as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// The local date an instant falls on.
fn local_day(tz: Tz, at: i64) -> NaiveDate {
    Utc.timestamp_opt(at, 0)
        .single()
        .unwrap_or_else(Utc::now)
        .with_timezone(&tz)
        .date_naive()
}

/// The instant a local wall-clock time happens at.
///
/// DST makes this a question with two awkward answers: the hour that occurs
/// twice, where the earlier one is meant, and the hour that does not occur at
/// all, where the clock skipping past it counts as the time having passed.
fn local_instant(tz: Tz, day: NaiveDate, hour: u32, minute: u32) -> i64 {
    let Some(naive) = day.and_hms_opt(hour, minute, 0) else {
        return i64::MAX;
    };
    match tz.from_local_datetime(&naive).earliest() {
        Some(at) => at.timestamp(),
        // The wall clock never showed this time; the moment it jumped over it
        // is the honest answer.
        None => tz
            .from_local_datetime(&(naive + ChronoDuration::hours(1)))
            .earliest()
            .map(|at| at.timestamp())
            .unwrap_or(i64::MAX),
    }
}

/// Start (inclusive) and end (exclusive) of a local day, in Unix seconds.
fn day_bounds(tz: Tz, day: NaiveDate) -> (i64, i64) {
    (
        local_instant(tz, day, 0, 0),
        local_instant(tz, day + ChronoDuration::days(1), 0, 0),
    )
}

/// Whether `at` has already come round on the local day `day`.
fn time_reached(tz: Tz, now: i64, day: NaiveDate, at: TimeOfDay) -> bool {
    now >= local_instant(tz, day, at.hour, at.minute)
}

/// `2026-W38`. The digest belongs to a week, and the ISO week is the one that
/// does not change its mind about which year the first days of January are in.
fn iso_week_key(day: NaiveDate) -> String {
    let week = day.iso_week();
    format!("{}-W{:02}", week.year(), week.week())
}

/// `2026-09-18 20:00`, in the configured timezone — for the body of a mail,
/// where a Unix timestamp would be no use to a reader.
pub fn local_stamp(tz: Tz, at: i64) -> String {
    Utc.timestamp_opt(at, 0)
        .single()
        .unwrap_or_else(Utc::now)
        .with_timezone(&tz)
        .format("%Y-%m-%d %H:%M %Z")
        .to_string()
}

/// The weekday name the digest configuration uses, for the status output.
pub fn weekday_name(day: Weekday) -> &'static str {
    match day {
        Weekday::Mon => "Monday",
        Weekday::Tue => "Tuesday",
        Weekday::Wed => "Wednesday",
        Weekday::Thu => "Thursday",
        Weekday::Fri => "Friday",
        Weekday::Sat => "Saturday",
        Weekday::Sun => "Sunday",
    }
}

#[cfg(test)]
mod tests {
    use super::{day_bounds, device_hash, iso_week_key, local_day, time_reached};
    use crate::config::TimeOfDay;
    use chrono::NaiveDate;
    use chrono_tz::Europe::Berlin;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).expect("a real date")
    }

    const AT: TimeOfDay = TimeOfDay {
        hour: 20,
        minute: 0,
    };

    #[test]
    fn a_local_day_is_not_the_utc_day_it_overlaps() {
        // 22:30 UTC on the 17th is already the 18th in Berlin.
        let at = day(2026, 9, 17)
            .and_hms_opt(22, 30, 0)
            .unwrap()
            .and_utc()
            .timestamp();
        assert_eq!(local_day(Berlin, at), day(2026, 9, 18));
    }

    #[test]
    fn the_reminder_waits_for_the_local_evening() {
        let today = day(2026, 9, 18);
        let (start, end) = day_bounds(Berlin, today);
        assert_eq!(end - start, 86_400);
        // 19:59 local is 17:59 UTC in summer time.
        assert!(!time_reached(Berlin, start + 19 * 3600 + 59 * 60, today, AT));
        assert!(time_reached(Berlin, start + 20 * 3600, today, AT));
    }

    #[test]
    fn the_day_the_clocks_go_forward_is_an_hour_short() {
        // 29 March 2026: 02:00 never happens in Berlin.
        let (start, end) = day_bounds(Berlin, day(2026, 3, 29));
        assert_eq!(end - start, 82_800);
        // And the evening still arrives, an hour earlier in UTC than usual.
        assert!(time_reached(
            Berlin,
            start + 19 * 3600,
            day(2026, 3, 29),
            AT
        ));
    }

    #[test]
    fn the_day_the_clocks_go_back_is_an_hour_long() {
        let (start, end) = day_bounds(Berlin, day(2026, 10, 25));
        assert_eq!(end - start, 90_000);
    }

    #[test]
    fn a_week_key_names_the_iso_week() {
        assert_eq!(iso_week_key(day(2026, 9, 18)), "2026-W38");
        // The first days of January belong to the week that holds most of them.
        assert_eq!(iso_week_key(day(2027, 1, 1)), "2026-W53");
    }

    #[test]
    fn a_device_hash_is_stable_and_says_nothing_by_itself() {
        let hash = device_hash("Mozilla/5.0");
        assert_eq!(hash.len(), 64);
        assert_eq!(hash, device_hash("Mozilla/5.0"));
        assert_ne!(hash, device_hash("Mozilla/5.1"));
    }
}
