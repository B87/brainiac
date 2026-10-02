-- v0.2 core tables: the vault, note identities, tasks, and links from notes
-- to repositories (docs/architecture.md, Data model). Note bodies, the notes
-- search tables, and links between notes live in index.db; revisions and
-- drafts in history.db.

-- The origin fetch URL, refreshed on observation; restore and reconnection
-- match repositories on it.
ALTER TABLE repositories ADD COLUMN remote_url TEXT;

INSERT INTO pin_entity_types (name) VALUES ('note');

-- One active vault in v0.2. Older rows stay so that tasks keep their notes.
CREATE TABLE vaults (
  id         TEXT PRIMARY KEY,
  name       TEXT NOT NULL,
  root_path  TEXT NOT NULL,
  created_at TEXT NOT NULL,
  active     INTEGER NOT NULL DEFAULT 0
);
CREATE UNIQUE INDEX vaults_one_active ON vaults (active) WHERE active = 1;

-- Whether a note's text is indexed: a file over 5 MiB or not in UTF-8 is
-- found by its name only.
CREATE TABLE note_text_states (name TEXT PRIMARY KEY) WITHOUT ROWID;
INSERT INTO note_text_states (name) VALUES ('text'), ('too_large'), ('not_utf8');

-- A note is its vault plus its path inside the vault. A note whose file is
-- gone keeps its row with missing_at set (and trashed_at when Brainiac moved
-- it to its trash), so tasks and links keep their context. mtime is in
-- nanoseconds since the Unix epoch.
CREATE TABLE notes (
  id             TEXT PRIMARY KEY,
  vault_id       TEXT NOT NULL REFERENCES vaults (id) ON DELETE CASCADE,
  relative_path  TEXT NOT NULL,
  embedded_id    TEXT,
  title          TEXT NOT NULL,
  content_hash   TEXT NOT NULL,
  size           INTEGER NOT NULL,
  mtime          INTEGER NOT NULL,
  text_state     TEXT NOT NULL DEFAULT 'text' REFERENCES note_text_states (name) ON UPDATE CASCADE,
  created_at     TEXT NOT NULL,
  last_opened_at TEXT,
  missing_at     TEXT,
  trashed_at     TEXT
);
CREATE UNIQUE INDEX notes_live_path ON notes (vault_id, relative_path) WHERE missing_at IS NULL;
CREATE INDEX notes_embedded_id ON notes (embedded_id) WHERE embedded_id IS NOT NULL;
CREATE INDEX notes_last_opened ON notes (last_opened_at) WHERE last_opened_at IS NOT NULL;

CREATE TABLE task_statuses (name TEXT PRIMARY KEY) WITHOUT ROWID;
INSERT INTO task_statuses (name) VALUES ('todo'), ('in_progress'), ('done'), ('cancelled');

-- planned_date and due_date are local calendar dates (YYYY-MM-DD). A task is
-- to sort while triaged_at, planned_date, and due_date are all empty. seq is
-- the integer key the search tables need; id is what everything else uses.
-- linked_repository_id has no foreign key, like note_repository_links.
CREATE TABLE tasks (
  seq                  INTEGER PRIMARY KEY,
  id                   TEXT NOT NULL UNIQUE,
  title                TEXT NOT NULL,
  description          TEXT NOT NULL DEFAULT '',
  status               TEXT NOT NULL DEFAULT 'todo' REFERENCES task_statuses (name) ON UPDATE CASCADE,
  triaged_at           TEXT,
  planned_date         TEXT,
  due_date             TEXT,
  linked_note_id       TEXT REFERENCES notes (id) ON DELETE SET NULL,
  linked_repository_id TEXT,
  created_at           TEXT NOT NULL,
  updated_at           TEXT NOT NULL,
  completed_at         TEXT,
  version              INTEGER NOT NULL DEFAULT 1
);
CREATE INDEX tasks_status ON tasks (status);
CREATE INDEX tasks_note ON tasks (linked_note_id) WHERE linked_note_id IS NOT NULL;
CREATE INDEX tasks_repository ON tasks (linked_repository_id) WHERE linked_repository_id IS NOT NULL;

-- External-content FTS5 over tasks, kept in step by triggers in the task's
-- own transaction (docs/architecture.md, Search).
CREATE VIRTUAL TABLE task_search USING fts5(
  title, description, content = 'tasks', content_rowid = 'seq',
  tokenize = 'unicode61 remove_diacritics 2'
);
CREATE VIRTUAL TABLE task_title_search USING fts5(
  title, content = 'tasks', content_rowid = 'seq',
  tokenize = 'trigram remove_diacritics 1'
);
CREATE TRIGGER tasks_search_insert AFTER INSERT ON tasks BEGIN
  INSERT INTO task_search (rowid, title, description) VALUES (new.seq, new.title, new.description);
  INSERT INTO task_title_search (rowid, title) VALUES (new.seq, new.title);
END;
CREATE TRIGGER tasks_search_delete AFTER DELETE ON tasks BEGIN
  INSERT INTO task_search (task_search, rowid, title, description) VALUES ('delete', old.seq, old.title, old.description);
  INSERT INTO task_title_search (task_title_search, rowid, title) VALUES ('delete', old.seq, old.title);
END;
CREATE TRIGGER tasks_search_update AFTER UPDATE OF title, description ON tasks BEGIN
  INSERT INTO task_search (task_search, rowid, title, description) VALUES ('delete', old.seq, old.title, old.description);
  INSERT INTO task_title_search (task_title_search, rowid, title) VALUES ('delete', old.seq, old.title);
  INSERT INTO task_search (rowid, title, description) VALUES (new.seq, new.title, new.description);
  INSERT INTO task_title_search (rowid, title) VALUES (new.seq, new.title);
END;

-- No foreign key to repositories: removing a repository keeps the link, with
-- the name and remote URL it had when the link was made.
CREATE TABLE note_repository_links (
  note_id         TEXT NOT NULL REFERENCES notes (id) ON DELETE CASCADE,
  repository_id   TEXT NOT NULL,
  repository_name TEXT NOT NULL,
  remote_url      TEXT,
  created_at      TEXT NOT NULL,
  PRIMARY KEY (note_id, repository_id)
) WITHOUT ROWID;
CREATE INDEX note_repository_links_repository ON note_repository_links (repository_id);

-- Suggestions the user dismissed: a repository a note mentions but should not link to.
CREATE TABLE dismissed_suggestions (
  note_id       TEXT NOT NULL REFERENCES notes (id) ON DELETE CASCADE,
  repository_id TEXT NOT NULL,
  PRIMARY KEY (note_id, repository_id)
) WITHOUT ROWID;
