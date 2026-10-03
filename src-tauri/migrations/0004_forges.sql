-- v0.3: which hosted repository a repository's pull requests come from, and
-- which workspaces track pull requests (docs/architecture.md, Data model).

-- Pull requests are off per workspace until turned on in its Pull requests tab.
ALTER TABLE workspaces ADD COLUMN pull_requests INTEGER NOT NULL DEFAULT 0;

-- Only the repositories whose pull requests live somewhere other than their
-- origin (an upstream, a fork): everything else is derived from remote_url
-- when read, so it needs no refresh. Kept when origin changes.
CREATE TABLE repository_forges (
  repository_id TEXT PRIMARY KEY REFERENCES repositories (id) ON DELETE CASCADE,
  kind          TEXT NOT NULL REFERENCES forge_kinds (name) ON UPDATE CASCADE,
  owner         TEXT NOT NULL,
  name          TEXT NOT NULL,
  created_at    TEXT NOT NULL
);
