-- v0.6: explaining changes (SPEC.md, section 14; docs/architecture.md,
-- Explaining changes — v0.6, Storage). An explain run is an agent run of
-- its own kind, left out of Runs, its badge, and retention.
ALTER TABLE agent_runs ADD COLUMN kind TEXT NOT NULL DEFAULT 'run'
  CHECK (kind IN ('run', 'explain'));

-- One explanation per repository, subject, profile, and depth: Explain
-- again replaces it. The subject is a commit (its ID), a branch (its
-- name, explained at `tip` against the merge base `base`), or a collected
-- run's result (the run's ID). The checked explanation is JSON in the
-- shape of `Explanation` (models.rs), with each note's line hash; the
-- run's journal stays in agent-runs/<run_id>/trace.jsonl for How it was
-- written, and goes with the explanation.
--   state         working, ready, failed, or cancelled
--   step          while working: copying, starting, reading, checking,
--                 or follow_up
--   errors        the checker's errors on a failure (JSON array)
--   hidden        the indexes of disagreements marked Not a problem (JSON)
--   cost_micros   the cost the agent reported, in millionths of currency
CREATE TABLE explanations (
  id               TEXT PRIMARY KEY,
  repository_id    TEXT NOT NULL,
  repository_name  TEXT NOT NULL,
  subject_kind     TEXT NOT NULL CHECK (subject_kind IN ('commit', 'branch', 'run')),
  subject_ref      TEXT NOT NULL,
  title            TEXT NOT NULL,
  base             TEXT NOT NULL,
  tip              TEXT NOT NULL,
  profile_id       TEXT NOT NULL,
  agent            TEXT NOT NULL,
  provider         TEXT NOT NULL,
  payment          TEXT NOT NULL,
  host_id          TEXT NOT NULL,
  host_name        TEXT NOT NULL,
  model            TEXT NOT NULL,
  depth            TEXT NOT NULL CHECK (depth IN ('brief', 'teach_me', 'deep')),
  questions        INTEGER NOT NULL,
  time_limit_minutes INTEGER NOT NULL,
  state            TEXT NOT NULL CHECK (state IN ('working', 'ready', 'failed', 'cancelled')),
  step             TEXT,
  error            TEXT,
  errors           TEXT NOT NULL DEFAULT '[]',
  run_id           TEXT,
  explanation      TEXT,
  hidden           TEXT NOT NULL DEFAULT '[]',
  cost_micros      INTEGER,
  currency         TEXT,
  duration_secs    INTEGER,
  created_at       TEXT NOT NULL,
  ended_at         TEXT,
  UNIQUE (repository_id, subject_kind, subject_ref, profile_id, depth)
);
CREATE INDEX explanations_run ON explanations (run_id);
