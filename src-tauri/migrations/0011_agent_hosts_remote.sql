-- v0.5 remote hosts (SPEC.md, Remote hosts). `agent_hosts.kind` was
-- constrained to `local`. SQLite cannot alter a CHECK, so the table is
-- rebuilt. Foreign keys are turned off around this migration because
-- `agent_profiles.host_id` references it (src-tauri/src/db.rs).

CREATE TABLE agent_hosts_v2 (
  id            TEXT PRIMARY KEY,
  kind          TEXT NOT NULL CHECK (kind IN ('local', 'ssh')),
  name          TEXT NOT NULL DEFAULT '',
  socket        TEXT,
  ssh_user      TEXT,
  ssh_host      TEXT,
  ssh_port      INTEGER,
  identity_path TEXT,
  -- The host key the user approved, `SHA256:…`. Never a private key.
  fingerprint   TEXT,
  -- The controller's installation id, from its first Hello. Empty until Deploy.
  installation  TEXT,
  protocol      INTEGER,
  -- A restored ssh host waits at 0 and cannot deploy or start a run.
  approved      INTEGER NOT NULL DEFAULT 1,
  engine_name   TEXT,
  loop_devices  INTEGER NOT NULL DEFAULT 0,
  image_id      TEXT,
  image_recipe  TEXT,
  image_built_at TEXT,
  test_passed_at TEXT,
  test_credential_revision INTEGER,
  test_image_id TEXT,
  -- Remove finished, but the host still holds a container, volume, or ledger.
  state_kept    INTEGER NOT NULL DEFAULT 0,
  version       INTEGER NOT NULL DEFAULT 1,
  created_at    TEXT NOT NULL,
  updated_at    TEXT NOT NULL
);

INSERT INTO agent_hosts_v2 (
  id, kind, name, socket, approved, version, created_at, updated_at
)
SELECT id, kind, 'This Mac', socket, 1, version, created_at, updated_at
FROM agent_hosts;

DROP TABLE agent_hosts;
ALTER TABLE agent_hosts_v2 RENAME TO agent_hosts;
