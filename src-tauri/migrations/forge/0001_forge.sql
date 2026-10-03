-- forge.db: pull requests, files, and checks as last read from GitHub and
-- Bitbucket Cloud (docs/architecture.md, Pull requests — v0.3). Rebuildable
-- from the providers: deleted and created again when a newer Brainiac wrote
-- it, never backed up.

-- One row per pull request, as the neutral model in JSON. version is the
-- provider's update time plus the head commit.
CREATE TABLE pull_requests (
  reference        TEXT PRIMARY KEY,
  forge_repository TEXT NOT NULL,
  number           INTEGER NOT NULL,
  state            TEXT NOT NULL,
  updated_at       TEXT NOT NULL,
  closed_at        TEXT,
  version          TEXT NOT NULL,
  json             TEXT NOT NULL,
  fetched_at       TEXT NOT NULL
);
CREATE INDEX pull_requests_repository ON pull_requests (forge_repository, state, updated_at);

-- When each repository's open (closed = 0) or recent closed (closed = 1)
-- list was last read, with the provider's ETag for a conditional request.
CREATE TABLE list_reads (
  forge_repository TEXT NOT NULL,
  closed           INTEGER NOT NULL,
  etag             TEXT,
  fetched_at       TEXT NOT NULL,
  PRIMARY KEY (forge_repository, closed)
);

-- Files and checks of a head commit; thrown away when the head moves.
CREATE TABLE pull_request_files (
  reference  TEXT PRIMARY KEY,
  head_sha   TEXT NOT NULL,
  json       TEXT NOT NULL,
  fetched_at TEXT NOT NULL
);
CREATE TABLE pull_request_checks (
  reference  TEXT PRIMARY KEY,
  head_sha   TEXT NOT NULL,
  json       TEXT NOT NULL,
  fetched_at TEXT NOT NULL
);
