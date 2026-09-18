-- A board per space, so "what am I working on" can be asked per space instead
-- of across everything at once. There is no board table: a space *is* its
-- board, and `nodes.has_board` decides whether the view exists. The diary is a
-- journal rather than a project and keeps that flag off.

CREATE TABLE board_lists (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    -- A space. Dropping the space drops its board with it.
    space_id   INTEGER NOT NULL REFERENCES nodes (id) ON DELETE CASCADE,
    name       TEXT    NOT NULL,
    position   INTEGER NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE INDEX idx_board_lists_space ON board_lists (space_id, position);

CREATE TABLE cards (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    list_id    INTEGER NOT NULL REFERENCES board_lists (id) ON DELETE CASCADE,
    -- The document this card is about, if any. Deleting that document must not
    -- delete the card: the work is still open, only its notes are gone.
    node_id    INTEGER REFERENCES nodes (id) ON DELETE SET NULL,
    title      TEXT    NOT NULL,
    body       TEXT    NOT NULL DEFAULT '',
    due_at     INTEGER,
    done_at    INTEGER,
    position   INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX idx_cards_list ON cards (list_id, position);
CREATE INDEX idx_cards_node ON cards (node_id);
-- The reminder mails in the next phase ask "what is due", which is a question
-- about every board at once.
CREATE INDEX idx_cards_due ON cards (due_at) WHERE due_at IS NOT NULL AND done_at IS NULL;

-- The three columns every board starts with, for the spaces that already exist.
INSERT INTO board_lists (space_id, name, position, created_at)
SELECT n.id, l.name, l.position, strftime('%s', 'now')
FROM nodes n
CROSS JOIN (SELECT 'backlog' AS name, 0 AS position
            UNION ALL SELECT 'doing', 1
            UNION ALL SELECT 'done', 2) l
WHERE n.parent_id IS NULL AND n.kind = 'space' AND n.has_board <> 0;
