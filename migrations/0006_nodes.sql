-- The diary grows sub-structures. A flat list of dated entries becomes a tree:
-- top-level spaces (diary, work, uni, ...), folders inside them to any depth,
-- and documents as the leaves. One recursive table holds all three, so a space
-- is simply a node without a parent and nesting costs nothing extra.
--
-- Ids are carried over unchanged. Share tokens already handed out, the
-- entry/media links and the `diary:draft:<id>` keys browsers hold in
-- localStorage all key on the entry id, and none of them can be rewritten from
-- here. The `diary` space therefore takes an id above every existing entry.

CREATE TABLE nodes (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    parent_id      INTEGER REFERENCES nodes (id) ON DELETE CASCADE,
    kind           TEXT    NOT NULL CHECK (kind IN ('space', 'folder', 'document')),
    -- A document's title, a folder's or space's name.
    name           TEXT    NOT NULL DEFAULT '',
    -- URL-safe, unique among siblings: the path segment in /n/<space>/<slug>.
    slug           TEXT    NOT NULL,
    -- Documents only; folders and spaces keep it empty.
    body           TEXT    NOT NULL DEFAULT '',
    position       INTEGER NOT NULL DEFAULT 0,
    -- Spaces only. The diary is a journal, not a project, so it has no board.
    has_board      INTEGER NOT NULL DEFAULT 0,
    created_at     INTEGER NOT NULL,
    updated_at     INTEGER NOT NULL,
    share_token    TEXT    UNIQUE,
    share_key_hash TEXT
);

CREATE UNIQUE INDEX idx_nodes_parent_slug ON nodes (parent_id, slug);
CREATE INDEX idx_nodes_parent ON nodes (parent_id, position, created_at DESC);
CREATE INDEX idx_nodes_share_token ON nodes (share_token);

-- Same shape and same reasoning as entry_media in 0002: a file may be embedded
-- by more than one document, and deleting one of them must not take the file
-- away from the others.
CREATE TABLE node_media (
    node_id  INTEGER NOT NULL REFERENCES nodes (id) ON DELETE CASCADE,
    media_id TEXT    NOT NULL REFERENCES media (id) ON DELETE CASCADE,
    PRIMARY KEY (node_id, media_id)
) WITHOUT ROWID;

CREATE INDEX idx_node_media_media_id ON node_media (media_id);

-- The three spaces to start from. Their ids sit above every entry id so the
-- documents below can keep theirs.
INSERT INTO nodes (id, parent_id, kind, name, slug, position, has_board, created_at, updated_at)
VALUES
    ((SELECT COALESCE(MAX(id), 0) FROM entries) + 1, NULL, 'space', 'diary', 'diary', 0, 0,
     strftime('%s', 'now'), strftime('%s', 'now')),
    ((SELECT COALESCE(MAX(id), 0) FROM entries) + 2, NULL, 'space', 'work', 'work', 1, 1,
     strftime('%s', 'now'), strftime('%s', 'now')),
    ((SELECT COALESCE(MAX(id), 0) FROM entries) + 3, NULL, 'space', 'uni', 'uni', 2, 1,
     strftime('%s', 'now'), strftime('%s', 'now'));

-- Every entry becomes a document in the diary space, id intact. The slug
-- carries the day the entry is about and its id, which reads well in a URL and
-- cannot collide with a second entry written on the same day.
INSERT INTO nodes (id, parent_id, kind, name, slug, body, position,
                   created_at, updated_at, share_token, share_key_hash)
SELECT e.id,
       (SELECT id FROM nodes WHERE parent_id IS NULL AND slug = 'diary'),
       'document',
       e.title,
       date(e.created_at, 'unixepoch') || '-' || e.id,
       e.body,
       0,
       e.created_at,
       e.updated_at,
       e.share_token,
       e.share_key_hash
FROM entries e;

INSERT INTO node_media (node_id, media_id)
SELECT entry_id, media_id FROM entry_media;

-- Search follows. Folder and space names are indexed alongside document bodies,
-- because "where did I put that folder" is the same question as "where did I
-- write that sentence".
CREATE VIRTUAL TABLE nodes_fts USING fts5 (
    name, body, content = 'nodes', content_rowid = 'id', tokenize = 'unicode61'
);

CREATE TRIGGER nodes_ai AFTER INSERT ON nodes BEGIN
    INSERT INTO nodes_fts (rowid, name, body) VALUES (new.id, new.name, new.body);
END;
CREATE TRIGGER nodes_ad AFTER DELETE ON nodes BEGIN
    INSERT INTO nodes_fts (nodes_fts, rowid, name, body) VALUES ('delete', old.id, old.name, old.body);
END;
CREATE TRIGGER nodes_au AFTER UPDATE ON nodes BEGIN
    INSERT INTO nodes_fts (nodes_fts, rowid, name, body) VALUES ('delete', old.id, old.name, old.body);
    INSERT INTO nodes_fts (rowid, name, body) VALUES (new.id, new.name, new.body);
END;

-- The rows above were inserted before the triggers existed.
INSERT INTO nodes_fts (nodes_fts) VALUES ('rebuild');

DROP TRIGGER entries_ai;
DROP TRIGGER entries_ad;
DROP TRIGGER entries_au;
DROP TABLE entries_fts;
DROP TABLE entry_media;
DROP TABLE entries;
