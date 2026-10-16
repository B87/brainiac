-- The names (never values) of the host secrets a run was given
-- (docs/design/scheduled-runs.md), as a JSON array, so its page can say so.
ALTER TABLE agent_runs ADD COLUMN host_secrets TEXT NOT NULL DEFAULT '[]';
