-- index.db: everything derived from the vault. It can be deleted at any time
-- and is rebuilt from the notes (docs/architecture.md, Storage layout). Each
-- body records the content hash it was built from.

-- path_key and stem_key are the lowercased relative path and file name
-- without `.md`, which links are resolved against.
CREATE TABLE note_bodies (
  seq           INTEGER PRIMARY KEY,
  note_id       TEXT NOT NULL UNIQUE,
  content_hash  TEXT NOT NULL,
  title         TEXT NOT NULL,
  relative_path TEXT NOT NULL,
  path_key      TEXT NOT NULL,
  stem_key      TEXT NOT NULL,
  body          TEXT NOT NULL
);
CREATE INDEX note_bodies_path_key ON note_bodies (path_key);
CREATE INDEX note_bodies_stem_key ON note_bodies (stem_key);

CREATE VIRTUAL TABLE note_search USING fts5(
  title, body, content = 'note_bodies', content_rowid = 'seq',
  tokenize = 'unicode61 remove_diacritics 2'
);
CREATE VIRTUAL TABLE note_name_search USING fts5(
  title, relative_path, content = 'note_bodies', content_rowid = 'seq',
  tokenize = 'trigram remove_diacritics 1'
);
CREATE TRIGGER note_bodies_insert AFTER INSERT ON note_bodies BEGIN
  INSERT INTO note_search (rowid, title, body) VALUES (new.seq, new.title, new.body);
  INSERT INTO note_name_search (rowid, title, relative_path) VALUES (new.seq, new.title, new.relative_path);
END;
CREATE TRIGGER note_bodies_delete AFTER DELETE ON note_bodies BEGIN
  INSERT INTO note_search (note_search, rowid, title, body) VALUES ('delete', old.seq, old.title, old.body);
  INSERT INTO note_name_search (note_name_search, rowid, title, relative_path) VALUES ('delete', old.seq, old.title, old.relative_path);
END;
CREATE TRIGGER note_bodies_update AFTER UPDATE OF title, relative_path, body ON note_bodies BEGIN
  INSERT INTO note_search (note_search, rowid, title, body) VALUES ('delete', old.seq, old.title, old.body);
  INSERT INTO note_name_search (note_name_search, rowid, title, relative_path) VALUES ('delete', old.seq, old.title, old.relative_path);
  INSERT INTO note_search (rowid, title, body) VALUES (new.seq, new.title, new.body);
  INSERT INTO note_name_search (rowid, title, relative_path) VALUES (new.seq, new.title, new.relative_path);
END;

CREATE TABLE link_kinds (name TEXT PRIMARY KEY) WITHOUT ROWID;
INSERT INTO link_kinds (name) VALUES ('markdown'), ('wikilink');

-- Links between notes, kept with their raw target and re-resolved from it
-- after any scan that adds, moves, or removes notes. A NULL target is an
-- unresolved link. Offsets are bytes in the source note; line is 1-based.
CREATE TABLE note_links (
  source_note_id TEXT NOT NULL,
  target_note_id TEXT,
  raw_target     TEXT NOT NULL,
  kind           TEXT NOT NULL REFERENCES link_kinds (name),
  start_offset   INTEGER NOT NULL,
  end_offset     INTEGER NOT NULL,
  line           INTEGER NOT NULL
);
CREATE INDEX note_links_source ON note_links (source_note_id);
CREATE INDEX note_links_target ON note_links (target_note_id) WHERE target_note_id IS NOT NULL;
