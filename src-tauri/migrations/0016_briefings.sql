-- Briefings (docs/design/scheduled-runs.md, The briefing result): a saved
-- definition of a run whose job is to leave a short report. Run now starts
-- it; results are in history.db (`briefing_results`). The profile and
-- repository must exist; deleting either removes the definition.
CREATE TABLE briefings (
  id                 TEXT PRIMARY KEY,
  name               TEXT NOT NULL,
  repository_id      TEXT NOT NULL REFERENCES repositories (id) ON DELETE CASCADE,
  profile_id         TEXT NOT NULL REFERENCES agent_profiles (id) ON DELETE CASCADE,
  -- Empty: this Mac.
  host_id            TEXT NOT NULL DEFAULT '',
  -- A branch or commit, resolved at each start.
  start              TEXT NOT NULL DEFAULT 'HEAD',
  prompt             TEXT NOT NULL,
  time_limit_minutes INTEGER NOT NULL,
  version            INTEGER NOT NULL DEFAULT 1,
  created_at         TEXT NOT NULL,
  updated_at         TEXT NOT NULL
);
