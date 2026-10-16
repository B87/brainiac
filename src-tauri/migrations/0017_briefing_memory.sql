-- A briefing's memory and how many earlier results its next run sees
-- (docs/design/scheduled-runs.md, Memory). The memory is text the agent
-- rewrites after each run and the user can edit; it is bounded and goes into
-- the run's prompt, never into the container as a file.
ALTER TABLE briefings ADD COLUMN memory TEXT NOT NULL DEFAULT '';
ALTER TABLE briefings ADD COLUMN memory_updated_at TEXT;
ALTER TABLE briefings ADD COLUMN previous_results INTEGER NOT NULL DEFAULT 3;
