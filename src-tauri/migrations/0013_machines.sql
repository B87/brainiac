-- v0.5 machines (docs/architecture.md, Machines). A remote host's
-- connection and approved key belong to a machine, which another feature
-- can reach later as its own role; `agent_hosts` keeps only the run host
-- role on it. `agent_hosts` is rebuilt without the moved columns, so
-- foreign keys are turned off around this migration, as for 0011
-- (src-tauri/src/db.rs).
--
-- No release shipped 0008–0012, so only development data has SSH hosts.
-- Their approved key file moves to the machine's own folder, which a
-- migration cannot do: each one waits for the user to confirm its key
-- again, as a restored host does.

CREATE TABLE machines (
  id            TEXT PRIMARY KEY,
  name          TEXT NOT NULL,
  ssh_user      TEXT NOT NULL,
  ssh_host      TEXT NOT NULL,
  ssh_port      INTEGER NOT NULL,
  identity_path TEXT,
  -- The host key the user approved, `SHA256:…`. Never a private key.
  fingerprint   TEXT NOT NULL,
  -- A restored machine waits at 0 until the user confirms its key again.
  approved      INTEGER NOT NULL DEFAULT 1,
  version       INTEGER NOT NULL DEFAULT 1,
  created_at    TEXT NOT NULL,
  updated_at    TEXT NOT NULL,
  UNIQUE (ssh_user, ssh_host, ssh_port)
);

INSERT INTO machines (
  id, name, ssh_user, ssh_host, ssh_port, identity_path, fingerprint, approved,
  created_at, updated_at
)
SELECT id, name, ssh_user, ssh_host, ssh_port, identity_path, fingerprint, 0,
  created_at, updated_at
FROM agent_hosts
WHERE kind = 'ssh';

CREATE TABLE agent_hosts_v3 (
  id            TEXT PRIMARY KEY,
  kind          TEXT NOT NULL CHECK (kind IN ('local', 'ssh')),
  -- The machine a remote host runs on; NULL for this Mac.
  machine_id    TEXT UNIQUE REFERENCES machines (id),
  socket        TEXT,
  -- The controller's installation id, from its first Hello. Empty until Deploy.
  installation  TEXT,
  protocol      INTEGER,
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
  controller_build TEXT,
  controller_installed_at TEXT,
  version       INTEGER NOT NULL DEFAULT 1,
  created_at    TEXT NOT NULL,
  updated_at    TEXT NOT NULL,
  CHECK ((kind = 'ssh') = (machine_id IS NOT NULL))
);

-- A development host keeps its id, so its runs, token, and last job still
-- find it; its machine was given the same id above.
INSERT INTO agent_hosts_v3 (
  id, kind, machine_id, socket, installation, protocol, engine_name, loop_devices,
  image_id, image_recipe, image_built_at, test_passed_at, test_credential_revision,
  test_image_id, state_kept, controller_build, controller_installed_at, version,
  created_at, updated_at
)
SELECT id, kind, CASE WHEN kind = 'ssh' THEN id END, socket, installation, protocol,
  engine_name, loop_devices, image_id, image_recipe, image_built_at, test_passed_at,
  test_credential_revision, test_image_id, state_kept, controller_build,
  controller_installed_at, version, created_at, updated_at
FROM agent_hosts;

DROP TABLE agent_hosts;
ALTER TABLE agent_hosts_v3 RENAME TO agent_hosts;
