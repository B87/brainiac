-- Secrets a run host keeps for runs it starts without the Mac
-- (docs/design/scheduled-runs.md, Credentials on the host).
--
-- named_secrets: a secret that is neither an account's token nor a
-- connection's password, such as a Sentry token. Like them, the row keeps
-- where the value is read from (`secret_source`, under the credential owner
-- `named:<id>`), never the value. `env_name` is the environment variable the
-- agent sees it as, and the name it has on a host.
--
-- host_secret_shares: what was sent to which host. `shared_revision` is the
-- credential revision that was sent, so a later change to the source reads as
-- out of date; `failure` is the safe sentence for a Share or Remove that did
-- not finish. The host's own list is the truth about what it holds; this is
-- what Brainiac did.
CREATE TABLE named_secrets (
  id                  TEXT PRIMARY KEY,
  label               TEXT NOT NULL,
  env_name            TEXT NOT NULL UNIQUE,
  note                TEXT NOT NULL DEFAULT '',
  -- A run gets a file holding the value, the variable holding its path (a key
  -- file a tool reads, like GOOGLE_APPLICATION_CREDENTIALS).
  as_file             INTEGER NOT NULL DEFAULT 0,
  secret_source       TEXT NOT NULL DEFAULT '{"kind":"none"}',
  credential_revision INTEGER NOT NULL DEFAULT 1,
  source_approved     INTEGER NOT NULL DEFAULT 1,
  credential_pending  TEXT CHECK (credential_pending IN ('save', 'cleanup', 'removal')),
  credential_saved_at TEXT,
  version             INTEGER NOT NULL DEFAULT 1,
  created_at          TEXT NOT NULL,
  updated_at          TEXT NOT NULL
);

CREATE TABLE host_secret_shares (
  host_id         TEXT NOT NULL REFERENCES agent_hosts (id) ON DELETE CASCADE,
  secret_id       TEXT NOT NULL REFERENCES named_secrets (id),
  shared_revision INTEGER NOT NULL,
  shared_at       TEXT NOT NULL,
  failure         TEXT,
  PRIMARY KEY (host_id, secret_id)
);
