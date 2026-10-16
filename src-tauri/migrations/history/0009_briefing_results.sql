-- What a briefing's run left (docs/design/scheduled-runs.md): the checked
-- contents of `.brainiac/briefing.json`, or why there is none. Findings and
-- proposals are the app's own JSON; a result is rebuilt by running again.
CREATE TABLE briefing_results (
  id            TEXT PRIMARY KEY,
  briefing_id   TEXT NOT NULL,
  run_id        TEXT,
  state         TEXT NOT NULL CHECK (state IN ('working', 'ready', 'failed')),
  status        TEXT CHECK (status IN ('nothing_found', 'findings')),
  summary       TEXT NOT NULL DEFAULT '',
  findings      TEXT NOT NULL DEFAULT '[]',
  proposals     TEXT NOT NULL DEFAULT '[]',
  error         TEXT,
  created_at    TEXT NOT NULL,
  finished_at   TEXT,
  duration_secs INTEGER
);
CREATE INDEX briefing_results_by_briefing ON briefing_results (briefing_id, created_at DESC);
