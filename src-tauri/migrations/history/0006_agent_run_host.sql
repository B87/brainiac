-- v0.5: which host a run executed on (SPEC.md, Remote hosts). Existing rows
-- are this Mac. The name is a snapshot, so a renamed host does not rewrite it.
ALTER TABLE agent_runs ADD COLUMN host_id TEXT NOT NULL DEFAULT 'local';
ALTER TABLE agent_runs ADD COLUMN host_name TEXT NOT NULL DEFAULT 'This Mac';
