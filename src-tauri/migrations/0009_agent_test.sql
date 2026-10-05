-- v0.5: Settings → Agents, Test (SPEC.md, Settings → Agents). A run cannot
-- start until a test passed for the current token or key, image, and
-- engine; the row records what the last passed test ran with.
ALTER TABLE agent_profiles ADD COLUMN test_passed_at TEXT;
ALTER TABLE agent_profiles ADD COLUMN test_credential_revision INTEGER;
ALTER TABLE agent_profiles ADD COLUMN test_image_id TEXT;
ALTER TABLE agent_profiles ADD COLUMN test_engine_socket TEXT;
