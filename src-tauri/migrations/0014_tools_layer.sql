-- The tools layer (docs/design/scheduled-runs.md, The tools layer): Python and
-- the Google Cloud CLI on top of the run image, built with it on every host;
-- a run or routine that asks for the tools uses it. A host keeps the layer it
-- built like its run image: ID, recipe (a hash of the base recipe and the
-- layer's Dockerfile), and time.
ALTER TABLE agent_hosts ADD COLUMN tools_image_id TEXT;
ALTER TABLE agent_hosts ADD COLUMN tools_image_recipe TEXT;
ALTER TABLE agent_hosts ADD COLUMN tools_image_built_at TEXT;
