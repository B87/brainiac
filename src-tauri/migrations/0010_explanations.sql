-- v0.6: explaining changes (SPEC.md, section 14, Boundaries and Settings →
-- Explanations). The explanations themselves are in history.db.

-- Whether a repository's code may be sent to a provider for explanations,
-- answered for one repository or for every repository in a workspace. A
-- repository's own answer wins over its workspaces', and a No from any of
-- its workspaces wins over a Yes.
CREATE TABLE explain_answers (
  scope        TEXT NOT NULL CHECK (scope IN ('repository', 'workspace')),
  scope_id     TEXT NOT NULL,
  provider     TEXT NOT NULL,
  allowed      INTEGER NOT NULL,
  answered_at  TEXT NOT NULL,
  PRIMARY KEY (scope, scope_id, provider)
);

-- The concepts the reader knows (Got it). `key` is the name folded to
-- lowercase with spaces and punctuation collapsed. A project pattern
-- belongs to the repository it was learned in; other kinds have ''.
-- Merge points one concept at another.
CREATE TABLE known_concepts (
  id             TEXT PRIMARY KEY,
  kind           TEXT NOT NULL CHECK (kind IN ('language', 'library', 'system', 'project_pattern')),
  name           TEXT NOT NULL,
  key            TEXT NOT NULL,
  repository_id  TEXT NOT NULL DEFAULT '',
  merged_into    TEXT REFERENCES known_concepts (id) ON DELETE SET NULL,
  learned_at     TEXT NOT NULL,
  -- The explanation's own words for it, and where it was learned: the
  -- subject's short label ("287bdc9", "feature/x", "#42", "run 4f2a91c0"),
  -- its repository, and the explanation (in history.db, so not a key).
  description    TEXT NOT NULL DEFAULT '',
  learned_from   TEXT NOT NULL DEFAULT '',
  learned_in     TEXT NOT NULL DEFAULT '',
  explanation_id TEXT,
  UNIQUE (kind, key, repository_id),
  CHECK ((kind = 'project_pattern') = (repository_id <> ''))
);
