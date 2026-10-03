-- v0.3: GitHub and Bitbucket Cloud accounts, one per provider
-- (docs/architecture.md, Pull requests — v0.3). The token is only in the
-- Keychain; this keeps what the last check found.

CREATE TABLE forge_kinds (name TEXT PRIMARY KEY) WITHOUT ROWID;
INSERT INTO forge_kinds (name) VALUES ('github'), ('bitbucket_cloud');

CREATE TABLE forge_token_kinds (name TEXT PRIMARY KEY) WITHOUT ROWID;
INSERT INTO forge_token_kinds (name) VALUES ('fine_grained'), ('classic'), ('api_token'), ('other');

-- user_id is the provider's stable ID for the user (GitHub's numeric ID,
-- Bitbucket's UUID), which pull requests name reviewers and authors by.
-- scopes_json is NULL when the provider does not reveal them.
CREATE TABLE forge_accounts (
  id           TEXT PRIMARY KEY,
  kind         TEXT NOT NULL UNIQUE REFERENCES forge_kinds (name) ON UPDATE CASCADE,
  host         TEXT NOT NULL,
  login        TEXT NOT NULL,
  user_id      TEXT NOT NULL,
  display_name TEXT,
  email        TEXT,
  token_kind   TEXT NOT NULL REFERENCES forge_token_kinds (name) ON UPDATE CASCADE,
  expires_at   TEXT,
  scopes_json  TEXT,
  missing_json TEXT NOT NULL DEFAULT '[]',
  read_only    INTEGER NOT NULL DEFAULT 0,
  checked_at   TEXT NOT NULL,
  created_at   TEXT NOT NULL
);
