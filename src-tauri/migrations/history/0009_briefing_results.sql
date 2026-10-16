-- What a routine's run left, a briefing (docs/design/routines-ux.md): the checked
-- contents of `.brainiac/briefing.json`, or why there is none. Findings and
-- proposals are the app's own JSON; a result is rebuilt by running again.
CREATE TABLE briefing_results (
  id            TEXT PRIMARY KEY,
  routine_id   TEXT NOT NULL,
  run_id        TEXT,
  state         TEXT NOT NULL CHECK (state IN ('working', 'ready', 'failed')),
  status        TEXT CHECK (status IN ('nothing_found', 'findings')),
  summary       TEXT NOT NULL DEFAULT '',
  findings      TEXT NOT NULL DEFAULT '[]',
  proposals     TEXT NOT NULL DEFAULT '[]',
  error         TEXT,
  created_at    TEXT NOT NULL,
  finished_at   TEXT,
  duration_secs INTEGER,
  -- What the agent reported the run cost, when it did, in millionths of a
  -- unit of `cost_currency`.
  cost_micros   INTEGER,
  cost_currency TEXT
);
CREATE INDEX briefing_results_by_briefing ON briefing_results (routine_id, created_at DESC);
