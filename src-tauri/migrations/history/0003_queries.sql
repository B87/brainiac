-- v0.4: databases (SPEC.md, Databases: History). What ran, never the rows,
-- and the open query tabs with their text, so tabs survive a restart.
CREATE TABLE query_runs (
  id            TEXT PRIMARY KEY,
  connection_id TEXT NOT NULL,
  sql           TEXT NOT NULL,
  ran_at        TEXT NOT NULL,
  elapsed_ms    INTEGER NOT NULL,
  row_count     INTEGER,
  error         TEXT
);
CREATE INDEX query_runs_connection ON query_runs (connection_id, ran_at);

CREATE TABLE query_tabs (
  id             TEXT PRIMARY KEY,
  position       INTEGER NOT NULL,
  connection_id  TEXT,
  saved_query_id TEXT,
  title          TEXT NOT NULL,
  text           TEXT NOT NULL,
  saved_version  INTEGER,
  dirty          INTEGER NOT NULL DEFAULT 0,
  mode           TEXT NOT NULL DEFAULT 'read_only'
);
