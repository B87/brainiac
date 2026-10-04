-- v0.4.x: where secrets come from (SPEC.md, Secrets; docs/architecture.md,
-- Credentials). Each account and connection keeps where its secret is read
-- from, never the secret:
--   secret_source        JSON: {"kind":"store"}, {"kind":"ask"}, {"kind":"none"},
--                        {"kind":"environment","name":"…"}, or
--                        {"kind":"command","program":"/abs/path","args":["…"]}
--   credential_revision  advanced when the source, where the secret is sent,
--                        or the stored secret changes
--   source_approved      0 after a restore, until the user allows the source
--   credential_pending   a Keychain write ('save'), deletion of an old item
--                        ('cleanup'), or removal ('removal') that did not finish
-- Existing accounts keep their Keychain item; existing connections keep
-- their meaning, and `db_connections.password` is no longer read.

ALTER TABLE forge_accounts ADD COLUMN secret_source TEXT NOT NULL DEFAULT '{"kind":"store"}';
ALTER TABLE forge_accounts ADD COLUMN credential_revision INTEGER NOT NULL DEFAULT 1;
ALTER TABLE forge_accounts ADD COLUMN source_approved INTEGER NOT NULL DEFAULT 1;
ALTER TABLE forge_accounts ADD COLUMN credential_pending TEXT
  CHECK (credential_pending IN ('save', 'cleanup', 'removal'));

ALTER TABLE db_connections ADD COLUMN secret_source TEXT NOT NULL DEFAULT '{"kind":"none"}';
ALTER TABLE db_connections ADD COLUMN credential_revision INTEGER NOT NULL DEFAULT 1;
ALTER TABLE db_connections ADD COLUMN source_approved INTEGER NOT NULL DEFAULT 1;
ALTER TABLE db_connections ADD COLUMN credential_pending TEXT
  CHECK (credential_pending IN ('save', 'cleanup', 'removal'));

UPDATE db_connections SET secret_source = CASE password
  WHEN 'keychain' THEN '{"kind":"store"}'
  WHEN 'ask' THEN '{"kind":"ask"}'
  ELSE '{"kind":"none"}'
END;
