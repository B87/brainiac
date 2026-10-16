-- Two things a run records for its page (docs/design/scheduled-runs.md,
-- docs/design/routines-ux.md): the names (never values) of the host secrets it
-- was given, as a JSON array, and the routine it is a run of, when it is one.
ALTER TABLE agent_runs ADD COLUMN host_secrets TEXT NOT NULL DEFAULT '[]';
ALTER TABLE agent_runs ADD COLUMN routine_id TEXT;
