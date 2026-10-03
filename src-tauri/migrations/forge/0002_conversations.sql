-- A pull request's conversation (its threads and comments, as the neutral
-- model in JSON), read again when it is older than the caller allows.
CREATE TABLE pull_request_conversations (
  reference  TEXT PRIMARY KEY,
  json       TEXT NOT NULL,
  fetched_at TEXT NOT NULL
);

-- The provider's diff of a pull request, split per file, for a head commit
-- that is not on the Mac; thrown away when the head moves. A set with no row
-- for a path means the provider's diff has none (the file is in no hunk).
CREATE TABLE pull_request_patch_sets (
  reference  TEXT PRIMARY KEY,
  head_sha   TEXT NOT NULL,
  fetched_at TEXT NOT NULL
);
CREATE TABLE pull_request_patches (
  reference TEXT NOT NULL,
  path      TEXT NOT NULL,
  patch     TEXT NOT NULL,
  PRIMARY KEY (reference, path)
);
