-- v0.1 schema: repositories, workspaces, pins, settings, and the activity feed.
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

CREATE TABLE activity_kinds (name TEXT PRIMARY KEY) WITHOUT ROWID;
INSERT INTO activity_kinds (name) VALUES ('advanced'), ('rewritten'), ('created'), ('tagged');

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
  -- Outcome of Brainiac's own fetches (SPEC.md, Fetching).
  last_fetch_at         TEXT,
  last_fetch_error_json TEXT,
  UNIQUE (display_path)
);
CREATE INDEX repositories_canonical_root ON repositories (canonical_root);

-- discovery_root and discovery_path are set for discovered workspaces only;
-- a NULL discovery_path means the discovery root itself is scanned. The
-- remaining columns are activity settings (SPEC.md, Workspace activity);
-- watched patterns are JSON arrays of strings, and last_digest_on is the
-- local date (YYYY-MM-DD) of the last morning digest check.
CREATE TABLE workspaces (
  id                 TEXT PRIMARY KEY,
  name               TEXT NOT NULL,
  discovery_mode     TEXT NOT NULL REFERENCES discovery_modes (name) ON UPDATE CASCADE,
  root_repository_id TEXT REFERENCES repositories (id) ON DELETE SET NULL,
  discovery_root     TEXT,
  discovery_path     TEXT,
  created_at         TEXT NOT NULL,
  watched_branches_json TEXT NOT NULL DEFAULT '["main","master","develop"]',
  watched_tags_json     TEXT NOT NULL DEFAULT '["v*"]',
  auto_fetch            INTEGER NOT NULL DEFAULT 0,
  notify_moves          INTEGER NOT NULL DEFAULT 0,
  morning_digest        INTEGER NOT NULL DEFAULT 0,
  warn_conflicts        INTEGER NOT NULL DEFAULT 1,
  -- {"b:<pattern>"|"t:<pattern>": RFC 3339}: since when each pattern is
  -- watched; a missing entry means since created_at.
  watched_since_json    TEXT NOT NULL DEFAULT '{}',
  last_digest_on        TEXT
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

-- Ref tracking is keyed by the shared Git directory (repositories.common_git_dir),
-- so a checkout and its linked worktrees share one set of tips and one feed.
-- Tips and baselines are derived data, rebuilt from a new baseline.

-- The union of watched patterns the stored tips were taken with, as JSON.
-- Refs that start matching later join the baseline silently.
CREATE TABLE ref_baselines (
  git_store     TEXT PRIMARY KEY,
  watched_json  TEXT NOT NULL,
  taken_at      TEXT NOT NULL
) WITHOUT ROWID;

CREATE TABLE ref_tips (
  git_store  TEXT NOT NULL,
  ref_name   TEXT NOT NULL,
  target_id  TEXT NOT NULL,
  PRIMARY KEY (git_store, ref_name)
) WITHOUT ROWID;

-- One row per moved watched ref. ref_name is the full name
-- (refs/remotes/origin/main, refs/tags/v1); match_name is the part workspace
-- patterns match: the branch without its remote, or the tag name.
-- detail_json holds commits, authors, overlapping paths, and drift
-- (models::ActivityDetail).
CREATE TABLE activity_events (
  id           TEXT PRIMARY KEY,
  git_store    TEXT NOT NULL,
  kind         TEXT NOT NULL REFERENCES activity_kinds (name) ON UPDATE CASCADE,
  ref_name     TEXT NOT NULL,
  match_name   TEXT NOT NULL,
  old_id       TEXT,
  new_id       TEXT NOT NULL,
  observed_at  TEXT NOT NULL,
  seen_at      TEXT,
  detail_json  TEXT NOT NULL
);
CREATE INDEX activity_events_store_time ON activity_events (git_store, observed_at);
CREATE INDEX activity_events_unseen ON activity_events (git_store) WHERE seen_at IS NULL;
