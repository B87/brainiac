-- v0.1 schema: repositories, workspaces, pins, settings.
-- Timestamps are RFC 3339 UTC text. IDs are UUID text.

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

CREATE TABLE workspaces (
  id                     TEXT PRIMARY KEY,
  name                   TEXT NOT NULL,
  discovery_mode         TEXT NOT NULL CHECK (discovery_mode IN ('root_projects', 'manual')),
  root_repository_id     TEXT REFERENCES repositories (id) ON DELETE SET NULL,
  projects_relative_path TEXT,
  created_at             TEXT NOT NULL
);

CREATE TABLE workspace_folders (
  id             TEXT PRIMARY KEY,
  workspace_id   TEXT NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
  display_name   TEXT NOT NULL,
  canonical_path TEXT NOT NULL,
  role           TEXT NOT NULL CHECK (role IN ('root', 'project', 'folder')),
  origin         TEXT NOT NULL CHECK (origin IN ('discovered', 'manual')),
  repository_id  TEXT REFERENCES repositories (id) ON DELETE SET NULL,
  UNIQUE (workspace_id, canonical_path)
);

CREATE TABLE pins (
  entity_type TEXT NOT NULL CHECK (entity_type IN ('repository', 'workspace')),
  entity_id   TEXT NOT NULL,
  position    INTEGER NOT NULL,
  PRIMARY KEY (entity_type, entity_id)
);
