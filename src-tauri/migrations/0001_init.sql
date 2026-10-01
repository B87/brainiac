-- v0.1 schema: repositories, workspaces, pins, settings.
-- Timestamps are RFC 3339 UTC text. IDs are UUID text.
--
-- Enumerated columns reference a lookup table instead of carrying a CHECK
-- constraint. Adding a value is an INSERT; renaming one is an UPDATE on the
-- lookup row, which ON UPDATE CASCADE carries into every referencing row.
-- Neither needs the referencing table to be recreated. Values match the
-- snake_case serde names in models.rs.

CREATE TABLE discovery_modes (name TEXT PRIMARY KEY) WITHOUT ROWID;
INSERT INTO discovery_modes (name) VALUES ('discovered'), ('manual');

CREATE TABLE member_origins (name TEXT PRIMARY KEY) WITHOUT ROWID;
INSERT INTO member_origins (name) VALUES ('discovered'), ('manual');

CREATE TABLE pin_entity_types (name TEXT PRIMARY KEY) WITHOUT ROWID;
INSERT INTO pin_entity_types (name) VALUES ('repository'), ('workspace');

CREATE TABLE settings (
  key        TEXT PRIMARY KEY,
  value_json TEXT NOT NULL,
  version    INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE repositories (
  id              TEXT PRIMARY KEY,
  canonical_root  TEXT NOT NULL,
  display_path    TEXT NOT NULL,
  git_dir         TEXT NOT NULL,
  common_git_dir  TEXT NOT NULL,
  created_at      TEXT NOT NULL,
  last_opened_at  TEXT,
  last_checked_at TEXT,
  last_tab        TEXT,
  status_json     TEXT,
  error_json      TEXT,
  UNIQUE (display_path)
);
CREATE INDEX repositories_canonical_root ON repositories (canonical_root);

-- discovery_root and discovery_path are set for discovered workspaces only;
-- a NULL discovery_path means the discovery root itself is scanned.
CREATE TABLE workspaces (
  id                 TEXT PRIMARY KEY,
  name               TEXT NOT NULL,
  discovery_mode     TEXT NOT NULL REFERENCES discovery_modes (name) ON UPDATE CASCADE,
  root_repository_id TEXT REFERENCES repositories (id) ON DELETE SET NULL,
  discovery_root     TEXT,
  discovery_path     TEXT,
  created_at         TEXT NOT NULL
);

-- One row per member. repository_id is NULL for a non-Git folder; the root is
-- the member whose repository_id equals workspaces.root_repository_id.
CREATE TABLE workspace_members (
  id             TEXT PRIMARY KEY,
  workspace_id   TEXT NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
  display_name   TEXT NOT NULL,
  canonical_path TEXT NOT NULL,
  origin         TEXT NOT NULL REFERENCES member_origins (name) ON UPDATE CASCADE,
  repository_id  TEXT REFERENCES repositories (id) ON DELETE SET NULL,
  UNIQUE (workspace_id, canonical_path)
);

CREATE TABLE pins (
  entity_type TEXT NOT NULL REFERENCES pin_entity_types (name) ON UPDATE CASCADE,
  entity_id   TEXT NOT NULL,
  position    INTEGER NOT NULL,
  PRIMARY KEY (entity_type, entity_id)
);
