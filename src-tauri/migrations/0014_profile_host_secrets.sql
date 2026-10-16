-- Which named secrets a profile's runs get (docs/design/scheduled-runs.md).
-- A run on a host receives the host's sealed copy of each, as the
-- environment variable the secret names. Deleting either side removes the row.
CREATE TABLE profile_host_secrets (
  profile_id TEXT NOT NULL REFERENCES agent_profiles (id) ON DELETE CASCADE,
  secret_id  TEXT NOT NULL REFERENCES named_secrets (id) ON DELETE CASCADE,
  PRIMARY KEY (profile_id, secret_id)
);
