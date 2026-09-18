-- Mail is sent through an outbox rather than straight from the code that
-- decides a mail is due. Two reasons, both of which have bitten every reminder
-- that skips this step: a restart must not send yesterday's reminder again, and
-- a relay that is down for an hour must not lose the mail it refused.
--
-- `dedupe_key` carries the whole idea. It spells out *which* occurrence this is
-- ("reminder:2026-09-18", "card_due:41:2026-09-20:overdue"), so deciding
-- whether a mail was already sent is an insert that either takes or does not.

CREATE TABLE notifications (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    kind            TEXT    NOT NULL,
    dedupe_key      TEXT    NOT NULL UNIQUE,
    subject         TEXT    NOT NULL,
    body            TEXT    NOT NULL,
    created_at      INTEGER NOT NULL,
    sent_at         INTEGER,
    attempts        INTEGER NOT NULL DEFAULT 0,
    -- When the last attempt was made, which is what the backoff is measured
    -- from; a count alone cannot say whether it is time to try again.
    last_attempt_at INTEGER,
    last_error      TEXT
);

-- The sweep asks one question every minute: what is still unsent?
CREATE INDEX idx_notifications_pending ON notifications (created_at) WHERE sent_at IS NULL;

-- Which browsers have logged in. The hash is over the User-Agent, so this can
-- honestly say "a browser you have not used before", not "someone else" — and
-- it needs no proxy header to be trusted for it.
CREATE TABLE login_events (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    device_hash   TEXT    NOT NULL UNIQUE,
    user_agent    TEXT    NOT NULL,
    first_seen_at INTEGER NOT NULL,
    last_seen_at  INTEGER NOT NULL
);
