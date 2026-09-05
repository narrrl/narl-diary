-- Bookkeeping for the Proton Drive mirror. It lives in the diary database so a
-- backup and the state describing it can never drift apart across a restore:
-- restore the database, and the mirror knows exactly what it already holds.

CREATE TABLE backup_state (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
) WITHOUT ROWID;

-- One row per file that exists remotely. `path` is the path inside the device
-- root ('diary.db', 'uploads/<uuid>'), `node_uid` the Drive '<volume>~<link>'
-- pair, so an unchanged file is skipped and a changed one becomes a new
-- revision of the node that is already there.
CREATE TABLE backup_files (
    path        TEXT PRIMARY KEY,
    node_uid    TEXT    NOT NULL,
    hash        TEXT    NOT NULL,
    size        INTEGER NOT NULL,
    uploaded_at INTEGER NOT NULL
) WITHOUT ROWID;
