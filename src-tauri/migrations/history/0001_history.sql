-- history.db: note revisions and unsaved drafts. Recovery data, never the
-- saved note itself, and never inside the vault.

CREATE TABLE revision_reasons (name TEXT PRIMARY KEY) WITHOUT ROWID;
INSERT INTO revision_reasons (name) VALUES ('app_save'), ('external_change'), ('restore');

CREATE TABLE note_revisions (
  id           TEXT PRIMARY KEY,
  note_id      TEXT NOT NULL,
  content      TEXT NOT NULL,
  content_hash TEXT NOT NULL,
  created_at   TEXT NOT NULL,
  reason       TEXT NOT NULL REFERENCES revision_reasons (name) ON UPDATE CASCADE
);
CREATE INDEX note_revisions_note ON note_revisions (note_id, created_at);

-- One draft per note with edits that are not saved yet; base_hash is the
-- version of the file the edits started from. Drafts are never pruned.
CREATE TABLE drafts (
  note_id    TEXT PRIMARY KEY,
  base_hash  TEXT NOT NULL,
  content    TEXT NOT NULL,
  updated_at TEXT NOT NULL
) WITHOUT ROWID;
