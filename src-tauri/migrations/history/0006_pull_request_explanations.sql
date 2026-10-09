-- v0.6: pull requests are explained too (SPEC.md, section 14, Pull
-- requests). SQLite cannot change a CHECK constraint, so the table is
-- copied into one that also allows the subject kind `pull_request`, whose
-- reference is the pull request's (`github.com/acme/api#42`), explained at
-- `tip` (its head) against `base` (the merge base with its target branch).
CREATE TABLE explanations_new (
  id               TEXT PRIMARY KEY,
  repository_id    TEXT NOT NULL,
  repository_name  TEXT NOT NULL,
  subject_kind     TEXT NOT NULL
                   CHECK (subject_kind IN ('commit', 'branch', 'run', 'pull_request')),
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
INSERT INTO explanations_new SELECT * FROM explanations;
DROP TABLE explanations;
ALTER TABLE explanations_new RENAME TO explanations;
CREATE INDEX explanations_run ON explanations (run_id);
-- A branch and a pull request with the same changes share one explanation:
-- found by the commits they compare.
CREATE INDEX explanations_range ON explanations (repository_id, tip, base);
