-- `media.entry_id` claimed a file for exactly one entry. Embedding the same
-- file in a second entry silently moved the claim, and deleting that second
-- entry then deleted a file the first one still displayed. A join table lets a
-- file belong to as many entries as embed it.

CREATE TABLE entry_media (
    entry_id INTEGER NOT NULL REFERENCES entries (id) ON DELETE CASCADE,
    media_id TEXT    NOT NULL REFERENCES media (id)   ON DELETE CASCADE,
    PRIMARY KEY (entry_id, media_id)
) WITHOUT ROWID;

CREATE INDEX idx_entry_media_media_id ON entry_media (media_id);

INSERT INTO entry_media (entry_id, media_id)
    SELECT entry_id, id FROM media WHERE entry_id IS NOT NULL;

DROP INDEX idx_media_entry_id;
ALTER TABLE media DROP COLUMN entry_id;
