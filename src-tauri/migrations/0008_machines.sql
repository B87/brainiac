-- v0.5: machines (docs/architecture.md, Machines). A computer Brainiac
-- reaches over SSH, and the host key the user approved for it. A feature
-- that puts something on a machine is a role with its own table and a
-- `machine_id`; v0.5's only role is a run host (`agent_hosts`). No private
-- key is stored; the approved key's lines are in the app-owned
-- `machines/<id>/known_hosts` in the data folder.
CREATE TABLE machines (
  id            TEXT PRIMARY KEY,
  name          TEXT NOT NULL,
  ssh_user      TEXT NOT NULL,
  ssh_host      TEXT NOT NULL,
  ssh_port      INTEGER NOT NULL,
  identity_path TEXT,
  -- The host key the user approved, `SHA256:…`.
  fingerprint   TEXT NOT NULL,
  -- 0 after a restore, or after its run host was removed with work kept
  -- there; nothing uses the machine until the user confirms its key again.
  approved      INTEGER NOT NULL DEFAULT 1,
  version       INTEGER NOT NULL DEFAULT 1,
  created_at    TEXT NOT NULL,
  updated_at    TEXT NOT NULL,
  UNIQUE (ssh_user, ssh_host, ssh_port)
);
