-- Public keys allowed to mount the workspace over SFTP. A single-user
-- application still wants more than one key: one per machine that mounts it,
-- so losing a laptop means revoking one row rather than a shared secret.
CREATE TABLE ssh_keys (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    name        TEXT    NOT NULL,
    -- `ssh-ed25519 AAAA...`, comment stripped: the comment is not part of the
    -- key and would otherwise make the same key look different twice.
    public_key  TEXT    NOT NULL UNIQUE,
    fingerprint TEXT    NOT NULL UNIQUE,
    created_at  INTEGER NOT NULL,
    last_used_at INTEGER
);
