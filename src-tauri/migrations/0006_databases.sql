-- v0.4: databases (SPEC.md, section 11; docs/architecture.md, Databases).
-- Saved connections to SQLite files and PostgreSQL servers. A password is
-- never stored here: it is in the Keychain item `brainiac/db:<id>`, or asked
-- for once per run.
CREATE TABLE db_connections (
  id                        TEXT PRIMARY KEY,
  name                      TEXT NOT NULL,
  kind                      TEXT NOT NULL CHECK (kind IN ('sqlite', 'postgres')),
  environment               TEXT NOT NULL
                            CHECK (environment IN ('local', 'development', 'staging', 'production')),
  access                    TEXT NOT NULL DEFAULT 'read_only'
                            CHECK (access IN ('read_only', 'read_write')),
  -- SQLite
  file_path                 TEXT,
  -- PostgreSQL
  host                      TEXT,
  port                      INTEGER,
  database                  TEXT,
  user_name                 TEXT,
  tls                       TEXT CHECK (tls IN ('verify', 'require', 'off')),
  ca_file                   TEXT,
  password                  TEXT NOT NULL DEFAULT 'none'
                            CHECK (password IN ('keychain', 'ask', 'none')),
  statement_timeout_seconds INTEGER NOT NULL DEFAULT 30,
  -- Health: where the server runs, for its memory, CPU, and disk (JSON).
  runs_on                   TEXT,
  version                   INTEGER NOT NULL DEFAULT 1,
  created_at                TEXT NOT NULL,
  updated_at                TEXT NOT NULL
);

-- Connections shown in a repository's side panel, like a note's links.
CREATE TABLE db_connection_repositories (
  connection_id TEXT NOT NULL REFERENCES db_connections (id) ON DELETE CASCADE,
  repository_id TEXT NOT NULL REFERENCES repositories (id) ON DELETE CASCADE,
  PRIMARY KEY (connection_id, repository_id)
);

-- Saved queries: the user's own SQL, so here and backed up. The last
-- parameter values are kept with the query because they rarely change.
CREATE TABLE saved_queries (
  id              TEXT PRIMARY KEY,
  name            TEXT NOT NULL,
  folder          TEXT NOT NULL DEFAULT '',
  description     TEXT NOT NULL DEFAULT '',
  connection_id   TEXT REFERENCES db_connections (id) ON DELETE SET NULL,
  sql             TEXT NOT NULL,
  parameters_json TEXT NOT NULL DEFAULT '[]',
  version         INTEGER NOT NULL DEFAULT 1,
  created_at      TEXT NOT NULL,
  updated_at      TEXT NOT NULL
);
