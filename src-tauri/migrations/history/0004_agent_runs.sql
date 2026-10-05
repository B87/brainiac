-- v0.5: agent runs (SPEC.md, section 13; docs/architecture.md, Agent runs —
-- v0.5, Storage). A run's immutable start, what it was started with, the
-- run controller's last confirmed state (the controller's own records are
-- authoritative), the journal cursor the Mac mirrored, the collected
-- result, pending client actions, and safe errors. Prompts and the
-- conversation live in the mirrored journal, agent-runs/<id>/trace.jsonl,
-- filtered by the controller; the token or key is never here.
CREATE TABLE agent_runs (
  id                  TEXT PRIMARY KEY,
  repository_id       TEXT NOT NULL,
  repository_name     TEXT NOT NULL,
  title               TEXT NOT NULL,
  start_commit        TEXT NOT NULL,
  start_subject       TEXT NOT NULL DEFAULT '',
  profile_id          TEXT NOT NULL,
  payment             TEXT NOT NULL,
  credential_source   TEXT NOT NULL,
  engine_socket       TEXT NOT NULL,
  engine_name         TEXT NOT NULL,
  image_name          TEXT NOT NULL,
  image_id            TEXT NOT NULL,
  permissions         TEXT NOT NULL,
  time_limit_minutes  INTEGER NOT NULL,
  cpus                INTEGER NOT NULL,
  memory_mib          INTEGER NOT NULL,
  workspace_gib       INTEGER NOT NULL,
  attempt             INTEGER NOT NULL DEFAULT 1,
  -- The controller's projection: phase, activity, turn, outcome, and
  -- whether it confirmed the stop and still keeps the container.
  phase               TEXT NOT NULL DEFAULT 'preparing',
  activity            TEXT NOT NULL DEFAULT 'preparing',
  turn                INTEGER NOT NULL DEFAULT 0,
  outcome             TEXT,
  stop_confirmed      INTEGER NOT NULL DEFAULT 0,
  kept                INTEGER NOT NULL DEFAULT 1,
  session_id          TEXT,
  accepted_at         TEXT,
  deadline_at         TEXT,
  ended_at            TEXT,
  expired_asleep      INTEGER NOT NULL DEFAULT 0,
  error               TEXT,
  -- Pending permissions, as the controller last showed them (JSON).
  pending_permissions TEXT NOT NULL DEFAULT '[]',
  -- The journal sequence mirrored to trace.jsonl.
  cursor              INTEGER NOT NULL DEFAULT 0,
  -- When the controller last answered, and whether Cancel waits to be sent.
  reported_at         TEXT,
  cancel_requested    INTEGER NOT NULL DEFAULT 0,
  -- Collection: none, collecting, ready, no_changes, or failed; the result
  -- commit in Brainiac's bare repository, the files it changed, the new
  -- files left out (JSON), and whether the snapshot was accepted.
  collection          TEXT NOT NULL DEFAULT 'none',
  collection_error    TEXT,
  result_commit       TEXT,
  changed_files       INTEGER,
  left_out            TEXT NOT NULL DEFAULT '[]',
  left_out_more       INTEGER NOT NULL DEFAULT 0,
  snapshot_accepted   INTEGER NOT NULL DEFAULT 0,
  -- A container, volume, or file that could not be removed, in words.
  cleanup_pending     TEXT,
  version             INTEGER NOT NULL DEFAULT 1,
  created_at          TEXT NOT NULL,
  updated_at          TEXT NOT NULL
);
CREATE INDEX agent_runs_repository ON agent_runs (repository_id, created_at);
CREATE INDEX agent_runs_ended ON agent_runs (ended_at);
