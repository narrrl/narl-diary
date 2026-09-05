CREATE TABLE entries (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    title       TEXT    NOT NULL DEFAULT '',
    body        TEXT    NOT NULL DEFAULT '',
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    share_token TEXT    UNIQUE
);

CREATE INDEX idx_entries_created_at ON entries (created_at DESC);
CREATE INDEX idx_entries_share_token ON entries (share_token);

CREATE TABLE media (
    id         TEXT    PRIMARY KEY,
    entry_id   INTEGER REFERENCES entries (id) ON DELETE SET NULL,
    filename   TEXT    NOT NULL,
    mime       TEXT    NOT NULL,
    size       INTEGER NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE INDEX idx_media_entry_id ON media (entry_id);

CREATE VIRTUAL TABLE entries_fts USING fts5 (
    title, body, content = 'entries', content_rowid = 'id', tokenize = 'unicode61'
);

CREATE TRIGGER entries_ai AFTER INSERT ON entries BEGIN
    INSERT INTO entries_fts (rowid, title, body) VALUES (new.id, new.title, new.body);
END;
CREATE TRIGGER entries_ad AFTER DELETE ON entries BEGIN
    INSERT INTO entries_fts (entries_fts, rowid, title, body) VALUES ('delete', old.id, old.title, old.body);
END;
CREATE TRIGGER entries_au AFTER UPDATE ON entries BEGIN
    INSERT INTO entries_fts (entries_fts, rowid, title, body) VALUES ('delete', old.id, old.title, old.body);
    INSERT INTO entries_fts (rowid, title, body) VALUES (new.id, new.title, new.body);
END;
