-- Routines (docs/design/routines-ux.md): a saved definition of a run whose job
-- is to leave a report, called a briefing. Run now starts it; the briefings are
-- in history.db (`briefing_results`). The profile and repository must exist;
-- deleting either removes the routine.
CREATE TABLE routines (
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
  -- Text the agent rewrites after each run and the user can edit; part of the
  -- next run's prompt, as data. Bounded; never a file in the container.
  memory             TEXT NOT NULL DEFAULT '',
  memory_updated_at  TEXT,
  -- How many earlier briefings the next run is shown (0 to 10).
  previous_results   INTEGER NOT NULL DEFAULT 3,
  version            INTEGER NOT NULL DEFAULT 1,
  created_at         TEXT NOT NULL,
  updated_at         TEXT NOT NULL
);
