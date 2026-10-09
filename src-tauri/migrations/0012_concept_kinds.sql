-- v0.6.1: a known concept's kind `system` ("system tool") becomes two:
-- `protocol` (a protocol, format, or standard: HTTP, DKIM, SPF) and `tool` (a
-- program or service: git, Docker, PostgreSQL); and `technique` is added (a
-- way of solving a problem that means the same in any codebase: idempotency). SQLite cannot change a CHECK
-- constraint, so the table is copied into one that allows the new kinds.
--
-- An old `system` row cannot be told apart by its name, so every one becomes
-- `tool`. A merge group has one kind, so every group survives; a concept that
-- is really a protocol is changed by the reader. Explanations already stored
-- in history.db read `system` as `tool` too (an alias in the model).
CREATE TABLE known_concepts_new (
  id             TEXT PRIMARY KEY,
  kind           TEXT NOT NULL CHECK (kind IN ('language', 'library', 'protocol', 'tool', 'technique', 'project_pattern')),
  name           TEXT NOT NULL,
  key            TEXT NOT NULL,
  repository_id  TEXT NOT NULL DEFAULT '',
  merged_into    TEXT REFERENCES known_concepts_new (id) ON DELETE SET NULL,
  learned_at     TEXT NOT NULL,
  description    TEXT NOT NULL DEFAULT '',
  learned_from   TEXT NOT NULL DEFAULT '',
  learned_in     TEXT NOT NULL DEFAULT '',
  explanation_id TEXT,
  UNIQUE (kind, key, repository_id),
  CHECK ((kind = 'project_pattern') = (repository_id <> ''))
);
INSERT INTO known_concepts_new
  SELECT id,
         CASE kind WHEN 'system' THEN 'tool' ELSE kind END,
         name, key, repository_id, merged_into, learned_at,
         description, learned_from, learned_in, explanation_id
  FROM known_concepts;
DROP TABLE known_concepts;
ALTER TABLE known_concepts_new RENAME TO known_concepts;
