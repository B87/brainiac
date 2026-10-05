-- v0.5: a run's model (SPEC.md, New run): what the user asked Claude Code
-- for (empty: its default), and the model the session reported it opened
-- with, once the agent is ready.
ALTER TABLE agent_runs ADD COLUMN model TEXT NOT NULL DEFAULT '';
ALTER TABLE agent_runs ADD COLUMN model_used TEXT;
