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
  -- The schedule, when there is one (docs/design/routines-ux.md). The host
  -- fires it in its own local time; `schedule_days` is daily, weekdays, or
  -- weekly:0 to weekly:6 (Monday is 0).
  schedule_days      TEXT,
  schedule_time      TEXT,
  late_start         INTEGER NOT NULL DEFAULT 0,
  paused             INTEGER NOT NULL DEFAULT 0,
  -- A run's cost ceiling in millionths of a US dollar.
  cost_ceiling_micros INTEGER NOT NULL DEFAULT 1000000,
  -- What the user approved, as JSON: a hash for each thing a schedule depends
  -- on. A change to any of them turns the schedule off until it is approved.
  approved_parts     TEXT,
  approved_at        TEXT,
  -- What the host said at the last contact.
  host_next_fire     TEXT,
  host_synced_at     TEXT,
  code_commit        TEXT,
  code_at            TEXT,
  -- The memory was edited here and the host has not been told.
  memory_unsent      INTEGER NOT NULL DEFAULT 0,
  -- Why the schedule was paused or turned off by Brainiac, in words.
  notice             TEXT,
  version            INTEGER NOT NULL DEFAULT 1,
  created_at         TEXT NOT NULL,
  updated_at         TEXT NOT NULL
);
