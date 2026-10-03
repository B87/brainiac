-- v0.3: review drafts (SPEC.md, Reviewing; docs/architecture.md, Data
-- model). Line comments written in Brainiac stay here until the review is
-- finished: the user's own text, so in the core database and backed up.
--
-- One row per draft, anchored to a line (path, side, line, an optional start
-- line for a range) of the commit it was written on. A row without a path is
-- the summary of a submission under way, with its verdict, kept so a
-- submission cut off midway can send what is left. remote_id is the
-- provider's comment ID once a row was sent (Bitbucket sends one request per
-- comment), so nothing is posted twice.
CREATE TABLE review_drafts (
  id         TEXT PRIMARY KEY,
  reference  TEXT NOT NULL,
  path       TEXT,
  side       TEXT,
  line       INTEGER,
  start_line INTEGER,
  commit_sha TEXT,
  body       TEXT NOT NULL,
  verdict    TEXT,
  origin     TEXT NOT NULL DEFAULT 'user',
  remote_id  TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE INDEX review_drafts_reference ON review_drafts (reference, created_at);
