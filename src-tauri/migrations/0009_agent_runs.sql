-- v0.5: agent runs (SPEC.md, section 13; docs/architecture.md, Agent runs —
-- v0.5). The runs themselves are in history.db (history/0004_agent_runs).

-- Where runs execute: `local`, this Mac's Docker engine chosen by its
-- socket (NULL until the user chooses one), and one `ssh` row per remote
-- run host, on its machine. A host's name and connection are its
-- machine's; this Mac is named in code.
--   installation, protocol   the controller's installation id and protocol,
--                            from its first Hello; NULL until Install
--   controller_build         the build Install put there, so Settings can
--                            offer Upgrade; and when it was installed
--   engine_name, loop_devices  the host's engine, and whether it can attach
--                            the workspace's loop devices
--   image_*                  the image built on the host's engine: its ID,
--                            the recipe (hash of the Dockerfile and its
--                            files) it was built from, and when; this
--                            Mac's is cleared when its engine changes
--   state_kept               Remove finished, but the host still holds a
--                            container, volume, or ledger
CREATE TABLE agent_hosts (
  id                       TEXT PRIMARY KEY,
  kind                     TEXT NOT NULL CHECK (kind IN ('local', 'ssh')),
  machine_id               TEXT UNIQUE REFERENCES machines (id),
  socket                   TEXT,
  installation             TEXT,
  protocol                 INTEGER,
  controller_build         TEXT,
  controller_installed_at  TEXT,
  engine_name              TEXT,
  loop_devices             INTEGER NOT NULL DEFAULT 0,
  image_id                 TEXT,
  image_recipe             TEXT,
  image_built_at           TEXT,
  state_kept               INTEGER NOT NULL DEFAULT 0,
  version                  INTEGER NOT NULL DEFAULT 1,
  created_at               TEXT NOT NULL,
  updated_at               TEXT NOT NULL,
  CHECK ((kind = 'ssh') = (machine_id IS NOT NULL))
);

-- How one agent's runs are paid for and started: an agent, Claude Code
-- or OpenCode, with one model provider. Each profile has its own token or
-- key, never here: like an account's (0007_secret_sources), the row keeps
-- where it is read from, under the credential owner `agent:<id>`;
-- `{"kind":"none"}` means none yet.
--   payment              'claude_plan' (a `claude setup-token` token, Claude
--                        Code only) or 'api_key' (the provider's API key)
--   credential_saved_at  when the token or key was last saved, for the expiry warning
--   sends_code_agreed    the user agreed that runs send code and prompts to the
--                        provider under this payment; cleared when it changes
--   model                the model new runs ask for; empty is the agent's
--                        default (Claude Code only: OpenCode needs one)
CREATE TABLE agent_profiles (
  id                       TEXT PRIMARY KEY,
  agent                    TEXT NOT NULL CHECK (agent IN ('claude_code', 'opencode')),
  provider                 TEXT NOT NULL CHECK (provider IN ('anthropic', 'openai', 'openrouter')),
  payment                  TEXT NOT NULL DEFAULT 'api_key'
                           CHECK (payment IN ('claude_plan', 'api_key')),
  secret_source            TEXT NOT NULL DEFAULT '{"kind":"none"}',
  credential_revision      INTEGER NOT NULL DEFAULT 1,
  source_approved          INTEGER NOT NULL DEFAULT 1,
  credential_pending       TEXT CHECK (credential_pending IN ('save', 'cleanup', 'removal')),
  credential_saved_at      TEXT,
  sends_code_agreed        INTEGER NOT NULL DEFAULT 0,
  permissions              TEXT NOT NULL DEFAULT 'ask' CHECK (permissions IN ('ask', 'act')),
  time_limit_minutes       INTEGER NOT NULL DEFAULT 60,
  cpus                     INTEGER NOT NULL DEFAULT 4,
  memory_mib               INTEGER NOT NULL DEFAULT 8192,
  workspace_gib            INTEGER NOT NULL DEFAULT 20,
  model                    TEXT NOT NULL DEFAULT '',
  version                  INTEGER NOT NULL DEFAULT 1,
  created_at               TEXT NOT NULL,
  updated_at               TEXT NOT NULL,
  CHECK (payment = 'api_key' OR agent = 'claude_code'),
  CHECK (provider = 'anthropic' OR agent = 'opencode')
);

-- The last passed test of a profile on a host, with what it ran with: the
-- credential revision, the image, and on this Mac the engine. A run of
-- that profile cannot start on that host until they match the current ones.
CREATE TABLE agent_tests (
  host_id             TEXT NOT NULL REFERENCES agent_hosts (id) ON DELETE CASCADE,
  profile_id          TEXT NOT NULL REFERENCES agent_profiles (id) ON DELETE CASCADE,
  passed_at           TEXT NOT NULL,
  credential_revision INTEGER NOT NULL,
  image_id            TEXT NOT NULL,
  engine_socket       TEXT,
  PRIMARY KEY (host_id, profile_id)
);

INSERT INTO agent_hosts (id, kind, created_at, updated_at)
VALUES ('local', 'local', strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), strftime('%Y-%m-%dT%H:%M:%SZ', 'now'));

INSERT INTO agent_profiles (id, agent, provider, created_at, updated_at)
VALUES
  ('claude-code', 'claude_code', 'anthropic', strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
  ('opencode-anthropic', 'opencode', 'anthropic', strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
  ('opencode-openai', 'opencode', 'openai', strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
  ('opencode-openrouter', 'opencode', 'openrouter', strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), strftime('%Y-%m-%dT%H:%M:%SZ', 'now'));
