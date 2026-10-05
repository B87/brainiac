-- v0.5: agent runs (SPEC.md, section 13; docs/architecture.md, Agent runs — v0.5).

-- Where runs execute. Phase 1 has one row, `local`: this Mac's Docker
-- engine, chosen by its socket (NULL until the user chooses one).
CREATE TABLE agent_hosts (
  id         TEXT PRIMARY KEY,
  kind       TEXT NOT NULL CHECK (kind IN ('local')),
  socket     TEXT,
  version    INTEGER NOT NULL DEFAULT 1,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

-- How one agent's runs are paid for and started. Phase 1 has one profile,
-- Claude Code. Its token or key is never here: like an account's
-- (0007_secret_sources), the row keeps where it is read from, under the
-- credential owner `agent:<id>`; `{"kind":"none"}` means none yet.
--   payment              'claude_plan' (a `claude setup-token` token) or 'api_key'
--   credential_saved_at  when the token or key was last saved, for the expiry warning
--   sends_code_agreed    the user agreed that runs send code and prompts to the
--                        provider under this payment; cleared when it changes
--   image_*              the image built on the host's engine: its ID, the
--                        recipe (hash of the Dockerfile and its files) it was
--                        built from, and when; cleared when the engine changes
CREATE TABLE agent_profiles (
  id                  TEXT PRIMARY KEY,
  agent               TEXT NOT NULL CHECK (agent IN ('claude_code')),
  host_id             TEXT NOT NULL REFERENCES agent_hosts (id),
  payment             TEXT NOT NULL DEFAULT 'api_key'
                      CHECK (payment IN ('claude_plan', 'api_key')),
  secret_source       TEXT NOT NULL DEFAULT '{"kind":"none"}',
  credential_revision INTEGER NOT NULL DEFAULT 1,
  source_approved     INTEGER NOT NULL DEFAULT 1,
  credential_pending  TEXT CHECK (credential_pending IN ('save', 'cleanup', 'removal')),
  credential_saved_at TEXT,
  sends_code_agreed   INTEGER NOT NULL DEFAULT 0,
  permissions         TEXT NOT NULL DEFAULT 'ask' CHECK (permissions IN ('ask', 'act')),
  time_limit_minutes  INTEGER NOT NULL DEFAULT 60,
  cpus                INTEGER NOT NULL DEFAULT 4,
  memory_mib          INTEGER NOT NULL DEFAULT 8192,
  workspace_gib       INTEGER NOT NULL DEFAULT 20,
  image_id            TEXT,
  image_recipe        TEXT,
  image_built_at      TEXT,
  version             INTEGER NOT NULL DEFAULT 1,
  created_at          TEXT NOT NULL,
  updated_at          TEXT NOT NULL
);

INSERT INTO agent_hosts (id, kind, created_at, updated_at)
VALUES ('local', 'local', strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), strftime('%Y-%m-%dT%H:%M:%SZ', 'now'));

INSERT INTO agent_profiles (id, agent, host_id, created_at, updated_at)
VALUES ('claude-code', 'claude_code', 'local', strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), strftime('%Y-%m-%dT%H:%M:%SZ', 'now'));
