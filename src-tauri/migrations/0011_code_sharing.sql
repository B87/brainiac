-- v0.6: code sharing (SPEC.md, section 13, Code sharing). Whether a
-- repository's code may be sent to a provider, by a run or an explanation,
-- answered for one repository or for every repository in a workspace. A
-- repository's own answer wins over its workspaces', and a No from any of
-- its workspaces wins over a Yes.
CREATE TABLE code_answers (
  scope        TEXT NOT NULL CHECK (scope IN ('repository', 'workspace')),
  scope_id     TEXT NOT NULL,
  provider     TEXT NOT NULL,
  allowed      INTEGER NOT NULL,
  answered_at  TEXT NOT NULL,
  PRIMARY KEY (scope, scope_id, provider)
);

-- The answer above replaces each agent profile's agreement to send code.
ALTER TABLE agent_profiles DROP COLUMN sends_code_agreed;
