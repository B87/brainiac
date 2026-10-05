-- v0.5: the model new runs ask Claude Code for (SPEC.md, Settings → Agents,
-- New runs). Empty means Claude Code's own default.
ALTER TABLE agent_profiles ADD COLUMN model TEXT NOT NULL DEFAULT '';
