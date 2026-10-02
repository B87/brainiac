//! What is derived from the vault: parsing a note, its row in `index.db`
//! (body, search tables, links between notes), and keyword search over notes
//! and tasks (docs/architecture.md, Notes, indexing, and backups; Search).
//!
//! Functions taking a `Connection` run on a database worker: writes on the
//! one indexing worker that owns `index.db`, reads on its read-only worker.

use std::collections::{HashMap, HashSet};

use pulldown_cmark::{Event, LinkType, Options, Parser, Tag, TagEnd};
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};

use crate::models::{AppResult, NoteLinkKind, TextPart};

/// Notes larger than this are found by name only (SPEC.md, The vault).
pub const EDIT_LIMIT: u64 = 5 * 1024 * 1024;

/// One transaction of the indexing worker holds at most this many notes or
/// bytes of text, so a save's update never waits long behind a scan
/// (docs/architecture.md, Storage layout).
pub const BATCH_NOTES: usize = 200;
pub const BATCH_BYTES: usize = 4 * 1024 * 1024;

/// SHA-256 of a file's bytes, as lowercase hex: a note's version.
pub fn content_hash(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for b in digest {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// The fields Brainiac reads from a note's frontmatter, which it otherwise
/// leaves alone.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Frontmatter {
    pub title: Option<String>,
    pub brainiac_id: Option<String>,
    /// Byte offset just after the closing `---` line, or 0 without frontmatter.
    pub end: usize,
}

/// Read the YAML frontmatter block at the top of `text`, if any. Only
/// top-level `title:` and `brainiac_id:` are read; nothing is rewritten.
pub fn frontmatter(text: &str) -> Frontmatter {
    let body = text.strip_prefix('\u{feff}').unwrap_or(text);
    let bom = text.len() - body.len();
    let Some(first_len) = ["---\n", "---\r\n"]
        .iter()
        .find(|p| body.starts_with(**p))
        .map(|p| p.len())
    else {
        return Frontmatter::default();
    };
    let mut fm = Frontmatter::default();
    let mut offset = bom + first_len;
    for line in text[offset..].split_inclusive('\n') {
        let content = line.trim_end_matches(['\n', '\r']);
        offset += line.len();
        if content == "---" || content == "..." {
            fm.end = offset;
            return fm;
        }
        if let Some(v) = content.strip_prefix("title:") {
            fm.title = Some(unquote(v));
        } else if let Some(v) = content.strip_prefix("brainiac_id:") {
            fm.brainiac_id = Some(unquote(v)).filter(|v| !v.is_empty());
        }
    }
    // No closing line: not frontmatter.
    Frontmatter::default()
}

fn unquote(value: &str) -> String {
    let v = value.trim();
    let v = v
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .or_else(|| v.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
        .unwrap_or(v);
    v.trim().to_string()
}

/// `text` with `brainiac_id: <id>` added to its frontmatter, creating the
/// block when there is none. Everything else, line endings included, stays
/// as it was (SPEC.md, Note identity).
pub fn with_embedded_id(text: &str, id: &str) -> String {
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let line = format!("brainiac_id: {id}{newline}");
    let fm = frontmatter(text);
    if fm.end > 0 {
        // Insert just before the closing `---` line.
        let closing_start = text[..fm.end]
            .trim_end_matches(['\n', '\r'])
            .rfind('\n')
            .map_or(0, |i| i + 1);
        let mut out = String::with_capacity(text.len() + line.len());
        out.push_str(&text[..closing_start]);
        out.push_str(&line);
        out.push_str(&text[closing_start..]);
        return out;
    }
    let bom = if text.starts_with('\u{feff}') { 3 } else { 0 };
    let mut out = String::with_capacity(text.len() + line.len() + 10);
    out.push_str(&text[..bom]);
    out.push_str("---");
    out.push_str(newline);
    out.push_str(&line);
    out.push_str("---");
    out.push_str(newline);
    out.push_str(&text[bom..]);
    out
}

/// `text` with the value of its frontmatter `brainiac_id` replaced by `id`;
/// unchanged when it has none. A copy of a note must not carry the
/// original's identity.
pub fn replace_embedded_id(text: &str, id: &str) -> String {
    let fm = frontmatter(text);
    if fm.brainiac_id.is_none() {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    for line in text[..fm.end].split_inclusive('\n') {
        if line.starts_with("brainiac_id:") {
            let ending = &line[line.trim_end_matches(['\n', '\r']).len()..];
            out.push_str("brainiac_id: ");
            out.push_str(id);
            out.push_str(ending);
        } else {
            out.push_str(line);
        }
    }
    out.push_str(&text[fm.end..]);
    out
}

/// A link to another note found in a note's text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedLink {
    /// The target as written: a Markdown link's destination, or a wikilink's target.
    pub raw: String,
    pub kind: NoteLinkKind,
    /// Byte range of the whole link in the note.
    pub start: usize,
    pub end: usize,
    /// 1-based line of the link.
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedNote {
    pub title: String,
    pub embedded_id: Option<String>,
    pub links: Vec<ParsedLink>,
}

/// The file name of a vault path without `.md`.
pub fn file_stem(relative_path: &str) -> &str {
    let name = relative_path.rsplit('/').next().unwrap_or(relative_path);
    strip_md(name)
}

fn strip_md(name: &str) -> &str {
    if name.len() > 3 && name[name.len() - 3..].eq_ignore_ascii_case(".md") {
        &name[..name.len() - 3]
    } else {
        name
    }
}

/// Whether a vault path names a Markdown note.
pub fn is_note_path(path: &str) -> bool {
    path.len() > 3 && path[path.len() - 3..].eq_ignore_ascii_case(".md")
}

/// Title, `brainiac_id`, and links of a note. The title is the frontmatter
/// `title`, else the first heading, else the file name (SPEC.md, Note identity).
pub fn parse_note(text: &str, relative_path: &str) -> ParsedNote {
    let fm = frontmatter(text);
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS
        | Options::ENABLE_WIKILINKS;
    let mut heading: Option<String> = None;
    let mut in_first_heading = false;
    let mut links = Vec::new();
    // Lines are counted incrementally: events arrive in source order.
    let (mut line, mut counted_to) = (1u32, 0usize);
    for (event, range) in Parser::new_ext(text, options).into_offset_iter() {
        match event {
            Event::Start(Tag::Heading { .. }) if heading.is_none() => {
                in_first_heading = true;
                heading = Some(String::new());
            }
            Event::End(TagEnd::Heading(_)) => in_first_heading = false,
            Event::Text(t) | Event::Code(t) if in_first_heading => {
                if let Some(h) = heading.as_mut() {
                    h.push_str(&t);
                }
            }
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                ..
            }) => {
                let kind = match link_type {
                    LinkType::WikiLink { .. } => NoteLinkKind::Wikilink,
                    _ => NoteLinkKind::Markdown,
                };
                if !links_to_note(&dest_url, kind) {
                    continue;
                }
                if range.start >= counted_to {
                    line += text[counted_to..range.start].matches('\n').count() as u32;
                    counted_to = range.start;
                }
                links.push(ParsedLink {
                    raw: dest_url.into_string(),
                    kind,
                    start: range.start,
                    end: range.end,
                    line,
                });
            }
            _ => {}
        }
    }
    let title = fm
        .title
        .filter(|t| !t.is_empty())
        .or_else(|| {
            heading
                .map(|h| h.trim().to_string())
                .filter(|h| !h.is_empty())
        })
        .unwrap_or_else(|| file_stem(relative_path).to_string());
    ParsedNote {
        title,
        embedded_id: fm.brainiac_id,
        links,
    }
}

/// Whether a link can point at a note: any wikilink, and a Markdown link to
/// a `.md` file or a path without an extension. Web links, anchors within
/// the note, and attachments are not links between notes.
fn links_to_note(dest: &str, kind: NoteLinkKind) -> bool {
    let target = strip_fragment(dest);
    if target.is_empty() {
        return false;
    }
    if kind == NoteLinkKind::Wikilink {
        return true;
    }
    if target.contains("://") || target.starts_with("mailto:") || target.starts_with('#') {
        return false;
    }
    let name = target.rsplit('/').next().unwrap_or(target);
    is_note_path(name) || !name.contains('.')
}

fn strip_fragment(dest: &str) -> &str {
    dest.split(['#', '?']).next().unwrap_or("")
}

// ---------------------------------------------------------------------------
// Resolving links
// ---------------------------------------------------------------------------

/// Lowercased path, the key links resolve against (macOS volumes ignore case).
pub fn path_key(relative_path: &str) -> String {
    relative_path.to_lowercase()
}

pub fn stem_key(relative_path: &str) -> String {
    file_stem(relative_path).to_lowercase()
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Some(v) = std::str::from_utf8(&b[i + 1..i + 3])
                .ok()
                .and_then(|h| u8::from_str_radix(h, 16).ok())
            {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The vault path a link points at, before checking that a note exists
/// there: for a Markdown link, the destination resolved against the linking
/// note's folder; for a wikilink with a folder, that path. A bare wikilink
/// returns `None`; it resolves by file name.
pub fn link_path(from: &str, raw: &str, kind: NoteLinkKind) -> Option<String> {
    let target = strip_fragment(raw).trim();
    if target.is_empty() {
        return None;
    }
    match kind {
        NoteLinkKind::Wikilink => {
            if !target.contains('/') {
                return None;
            }
            let path = target.trim_start_matches('/');
            Some(if is_note_path(path) {
                path.to_string()
            } else {
                format!("{path}.md")
            })
        }
        NoteLinkKind::Markdown => {
            let decoded = percent_decode(target);
            let mut parts: Vec<&str> = if decoded.starts_with('/') {
                Vec::new()
            } else {
                let mut folder: Vec<&str> = from.split('/').collect();
                folder.pop();
                folder
            };
            for part in decoded.split('/') {
                match part {
                    "" | "." => {}
                    ".." => {
                        parts.pop()?;
                    }
                    p => parts.push(p),
                }
            }
            let joined = parts.join("/");
            if joined.is_empty() {
                return None;
            }
            Some(if is_note_path(&joined) {
                joined
            } else {
                format!("{joined}.md")
            })
        }
    }
}

/// Every indexed note by lowercased path and by lowercased file name. When
/// several notes share a file name, a bare wikilink resolves to the one with
/// the shortest path, then the first in order.
#[derive(Debug, Default)]
pub struct NameTable {
    paths: HashMap<String, String>,
    stems: HashMap<String, (String, String)>,
}

impl NameTable {
    pub fn insert(&mut self, relative_path: &str, note_id: &str) {
        self.paths
            .insert(path_key(relative_path), note_id.to_string());
        let candidate = (relative_path.to_string(), note_id.to_string());
        self.stems
            .entry(stem_key(relative_path))
            .and_modify(|best| {
                if (candidate.0.len(), &candidate.0) < (best.0.len(), &best.0) {
                    *best = candidate.clone();
                }
            })
            .or_insert(candidate);
    }

    pub fn resolve(&self, from: &str, raw: &str, kind: NoteLinkKind) -> Option<String> {
        if let Some(path) = link_path(from, raw, kind) {
            return self.paths.get(&path_key(&path)).cloned();
        }
        let name = strip_fragment(raw).trim();
        self.stems
            .get(&strip_md(name).to_lowercase())
            .map(|(_, id)| id.clone())
    }

    /// All notes as they are in `note_bodies`.
    pub fn load(conn: &Connection) -> AppResult<Self> {
        let mut table = NameTable::default();
        let mut stmt = conn.prepare_cached("SELECT relative_path, note_id FROM note_bodies")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        for row in rows {
            let (path, id) = row?;
            table.insert(&path, &id);
        }
        Ok(table)
    }
}

/// Where **Create** would put the note an unresolved link names.
pub fn suggested_path(from: &str, raw: &str, kind: NoteLinkKind) -> String {
    if let Some(path) = link_path(from, raw, kind) {
        return path;
    }
    let name = strip_md(strip_fragment(raw).trim());
    match from.rfind('/') {
        Some(i) => format!("{}/{name}.md", &from[..i]),
        None => format!("{name}.md"),
    }
}

// ---------------------------------------------------------------------------
// Writing index.db (indexing worker only)
// ---------------------------------------------------------------------------

/// A note as it goes into `index.db`.
#[derive(Debug, Clone)]
pub struct IndexDoc {
    pub note_id: String,
    pub content_hash: String,
    pub title: String,
    pub relative_path: String,
    /// Empty for a note that is not searchable text.
    pub body: String,
    pub links: Vec<ParsedLink>,
}

/// Insert or replace notes in one transaction, then resolve their outgoing links.
pub fn upsert_docs(conn: &mut Connection, docs: &[IndexDoc]) -> AppResult<()> {
    let tx = conn.transaction()?;
    {
        let mut upsert = tx.prepare_cached(
            "INSERT INTO note_bodies (note_id, content_hash, title, relative_path, path_key, stem_key, body)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(note_id) DO UPDATE SET content_hash = excluded.content_hash,
               title = excluded.title, relative_path = excluded.relative_path,
               path_key = excluded.path_key, stem_key = excluded.stem_key, body = excluded.body",
        )?;
        let mut clear = tx.prepare_cached("DELETE FROM note_links WHERE source_note_id = ?1")?;
        let mut link = tx.prepare_cached(
            "INSERT INTO note_links (source_note_id, raw_target, kind, start_offset, end_offset, line)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?;
        for doc in docs {
            upsert.execute(params![
                doc.note_id,
                doc.content_hash,
                doc.title,
                doc.relative_path,
                path_key(&doc.relative_path),
                stem_key(&doc.relative_path),
                doc.body
            ])?;
            clear.execute([&doc.note_id])?;
            for l in &doc.links {
                link.execute(params![
                    doc.note_id,
                    l.raw,
                    link_kind_name(l.kind),
                    l.start as i64,
                    l.end as i64,
                    l.line
                ])?;
            }
        }
    }
    resolve_links_from(&tx, docs.iter().map(|d| d.note_id.as_str()))?;
    tx.commit()?;
    Ok(())
}

fn link_kind_name(kind: NoteLinkKind) -> &'static str {
    match kind {
        NoteLinkKind::Markdown => "markdown",
        NoteLinkKind::Wikilink => "wikilink",
    }
}

fn parse_link_kind(name: &str) -> NoteLinkKind {
    if name == "wikilink" {
        NoteLinkKind::Wikilink
    } else {
        NoteLinkKind::Markdown
    }
}

/// Remove notes from search and their outgoing links. Links pointing at them
/// become unresolved.
pub fn remove_docs(conn: &mut Connection, note_ids: &[String]) -> AppResult<()> {
    let tx = conn.transaction()?;
    {
        let mut body = tx.prepare_cached("DELETE FROM note_bodies WHERE note_id = ?1")?;
        let mut out = tx.prepare_cached("DELETE FROM note_links WHERE source_note_id = ?1")?;
        let mut into = tx.prepare_cached(
            "UPDATE note_links SET target_note_id = NULL WHERE target_note_id = ?1",
        )?;
        for id in note_ids {
            body.execute([id])?;
            out.execute([id])?;
            into.execute([id])?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// Resolve the outgoing links of some notes against the notes indexed now.
fn resolve_links_from<'a>(
    conn: &Connection,
    note_ids: impl Iterator<Item = &'a str>,
) -> AppResult<()> {
    let mut by_path = conn.prepare_cached(
        "SELECT note_id FROM note_bodies WHERE path_key = ?1 ORDER BY relative_path LIMIT 1",
    )?;
    let mut by_stem = conn.prepare_cached(
        "SELECT note_id FROM note_bodies WHERE stem_key = ?1
         ORDER BY length(relative_path), relative_path LIMIT 1",
    )?;
    let mut links = conn.prepare_cached(
        "SELECT l.rowid, l.raw_target, l.kind, b.relative_path FROM note_links l
         JOIN note_bodies b ON b.note_id = l.source_note_id WHERE l.source_note_id = ?1",
    )?;
    let mut set =
        conn.prepare_cached("UPDATE note_links SET target_note_id = ?2 WHERE rowid = ?1")?;
    for id in note_ids {
        let rows: Vec<(i64, String, String, String)> = links
            .query_map([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
            .collect::<Result<_, _>>()?;
        for (rowid, raw, kind, from) in rows {
            let target = resolve_with(
                &mut by_path,
                &mut by_stem,
                &from,
                &raw,
                parse_link_kind(&kind),
            )?;
            set.execute(params![rowid, target])?;
        }
    }
    Ok(())
}

fn resolve_with(
    by_path: &mut rusqlite::CachedStatement<'_>,
    by_stem: &mut rusqlite::CachedStatement<'_>,
    from: &str,
    raw: &str,
    kind: NoteLinkKind,
) -> AppResult<Option<String>> {
    Ok(match link_path(from, raw, kind) {
        Some(path) => by_path
            .query_row([path_key(&path)], |r| r.get(0))
            .optional()?,
        None => by_stem
            .query_row([strip_md(strip_fragment(raw).trim()).to_lowercase()], |r| {
                r.get(0)
            })
            .optional()?,
    })
}

/// The note a link written in the note at `from` points at now, if any.
pub fn resolve_one(
    conn: &Connection,
    from: &str,
    raw: &str,
    kind: NoteLinkKind,
) -> AppResult<Option<String>> {
    let mut by_path = conn.prepare_cached(
        "SELECT note_id FROM note_bodies WHERE path_key = ?1 ORDER BY relative_path LIMIT 1",
    )?;
    let mut by_stem = conn.prepare_cached(
        "SELECT note_id FROM note_bodies WHERE stem_key = ?1
         ORDER BY length(relative_path), relative_path LIMIT 1",
    )?;
    resolve_with(&mut by_path, &mut by_stem, from, raw, kind)
}

/// Re-resolve every link from its raw target, after a scan or rename that
/// added, moved, or removed notes. Returns how many targets changed.
pub fn resolve_all_links(conn: &mut Connection) -> AppResult<usize> {
    let names = NameTable::load(conn)?;
    let tx = conn.transaction()?;
    let mut changed = 0;
    {
        let mut stmt = tx.prepare(
            "SELECT l.rowid, l.raw_target, l.kind, b.relative_path, l.target_note_id FROM note_links l
             JOIN note_bodies b ON b.note_id = l.source_note_id",
        )?;
        let rows: Vec<(i64, String, String, String, Option<String>)> = stmt
            .query_map([], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })?
            .collect::<Result<_, _>>()?;
        let mut set =
            tx.prepare_cached("UPDATE note_links SET target_note_id = ?2 WHERE rowid = ?1")?;
        for (rowid, raw, kind, from, current) in rows {
            let target = names.resolve(&from, &raw, parse_link_kind(&kind));
            if target != current {
                set.execute(params![rowid, target])?;
                changed += 1;
            }
        }
    }
    tx.commit()?;
    Ok(changed)
}

/// Point an indexed note at a new path (a rename in Brainiac or outside).
pub fn move_doc(conn: &Connection, note_id: &str, relative_path: &str) -> AppResult<()> {
    conn.execute(
        "UPDATE note_bodies SET relative_path = ?2, path_key = ?3, stem_key = ?4 WHERE note_id = ?1",
        params![
            note_id,
            relative_path,
            path_key(relative_path),
            stem_key(relative_path)
        ],
    )?;
    Ok(())
}

/// The content hash each indexed note was built from.
pub fn indexed_hashes(conn: &Connection) -> AppResult<HashMap<String, String>> {
    let mut stmt = conn.prepare("SELECT note_id, content_hash FROM note_bodies")?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// The indexed text of some notes, for revisions of what an outside change replaced.
pub fn bodies(
    conn: &Connection,
    note_ids: &[String],
) -> AppResult<HashMap<String, (String, String)>> {
    let mut stmt =
        conn.prepare_cached("SELECT content_hash, body FROM note_bodies WHERE note_id = ?1")?;
    let mut out = HashMap::new();
    for id in note_ids {
        if let Some(row) = stmt
            .query_row([id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .optional()?
        {
            out.insert(id.clone(), row);
        }
    }
    Ok(out)
}

/// Empty the index: every body, search row, and link. A full scan fills it again.
pub fn clear(conn: &mut Connection) -> AppResult<()> {
    let tx = conn.transaction()?;
    tx.execute_batch(
        "DELETE FROM note_links;
         DELETE FROM note_bodies;
         INSERT INTO note_search (note_search) VALUES ('rebuild');
         INSERT INTO note_name_search (note_name_search) VALUES ('rebuild');",
    )?;
    tx.commit()?;
    Ok(())
}

/// Reclaim pages freed by a scan that rewrote many notes.
pub fn reclaim_space(conn: &Connection) -> AppResult<()> {
    let free: i64 = conn.query_row("PRAGMA freelist_count", [], |r| r.get(0))?;
    if free > 1024 {
        conn.execute_batch("PRAGMA incremental_vacuum;")?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Links read back
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredLink {
    pub source_note_id: String,
    pub target_note_id: Option<String>,
    pub raw_target: String,
    pub kind: NoteLinkKind,
    pub start: usize,
    pub end: usize,
    pub line: u32,
}

fn row_to_link(r: &rusqlite::Row<'_>) -> rusqlite::Result<StoredLink> {
    Ok(StoredLink {
        source_note_id: r.get(0)?,
        target_note_id: r.get(1)?,
        raw_target: r.get(2)?,
        kind: parse_link_kind(&r.get::<_, String>(3)?),
        start: r.get::<_, i64>(4)? as usize,
        end: r.get::<_, i64>(5)? as usize,
        line: r.get(6)?,
    })
}

const LINK_COLUMNS: &str =
    "source_note_id, target_note_id, raw_target, kind, start_offset, end_offset, line";

/// Links from other notes into `note_id`.
pub fn links_into(conn: &Connection, note_id: &str) -> AppResult<Vec<StoredLink>> {
    let sql = format!(
        "SELECT {LINK_COLUMNS} FROM note_links WHERE target_note_id = ?1 AND source_note_id <> ?1
         ORDER BY source_note_id, start_offset"
    );
    let mut stmt = conn.prepare_cached(&sql)?;
    let rows = stmt.query_map([note_id], row_to_link)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Links out of `note_id`.
pub fn links_from(conn: &Connection, note_id: &str) -> AppResult<Vec<StoredLink>> {
    let sql = format!(
        "SELECT {LINK_COLUMNS} FROM note_links WHERE source_note_id = ?1 ORDER BY start_offset"
    );
    let mut stmt = conn.prepare_cached(&sql)?;
    let rows = stmt.query_map([note_id], row_to_link)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// The indexed text of one note.
pub fn body(conn: &Connection, note_id: &str) -> AppResult<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT body FROM note_bodies WHERE note_id = ?1",
            [note_id],
            |r| r.get(0),
        )
        .optional()?)
}

/// The text of line `line` (1-based) of `text`, trimmed and shortened.
pub fn line_excerpt(text: &str, line: u32) -> String {
    let content = text
        .lines()
        .nth(line.saturating_sub(1) as usize)
        .unwrap_or("")
        .trim();
    shorten(content, 160)
}

fn shorten(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max_chars).collect();
    out.push('…');
    out
}

/// Rewrite the links in `text` that point at a renamed note, from the end
/// backwards so earlier offsets stay valid. `links` are the stored links of
/// the linking note `from` into the renamed note, now at `new_path`.
pub fn rewrite_links(
    text: &str,
    from: &str,
    links: &[StoredLink],
    new_path: &str,
) -> Option<String> {
    let mut out = text.to_string();
    let mut sorted: Vec<&StoredLink> = links.iter().collect();
    sorted.sort_by_key(|l| std::cmp::Reverse(l.start));
    let mut changed = false;
    for link in sorted {
        let span = out.get(link.start..link.end)?;
        let target_text = strip_fragment(&link.raw_target);
        let fragment = &link.raw_target[target_text.len()..];
        let replacement = match link.kind {
            NoteLinkKind::Wikilink => {
                let new_target = if target_text.contains('/') {
                    strip_md(new_path).to_string()
                } else {
                    file_stem(new_path).to_string()
                };
                format!("{new_target}{fragment}")
            }
            NoteLinkKind::Markdown => {
                let relative = relative_link(from, new_path);
                let encoded = if target_text.contains("%20") || !link_uses_angle(span) {
                    relative.replace(' ', "%20")
                } else {
                    relative
                };
                format!("{encoded}{fragment}")
            }
        };
        // The target follows `](` in a Markdown link and `[[` in a wikilink;
        // the same words in the link's text are not the target.
        let after = match link.kind {
            NoteLinkKind::Markdown => span.rfind("](").map_or(0, |i| i + 2),
            NoteLinkKind::Wikilink => span.find("[[").map_or(0, |i| i + 2),
        };
        let Some(pos) = span[after..]
            .find(link.raw_target.as_str())
            .map(|p| p + after)
        else {
            continue;
        };
        let at = link.start + pos;
        out.replace_range(at..at + link.raw_target.len(), &replacement);
        changed = true;
    }
    changed.then_some(out)
}

fn link_uses_angle(span: &str) -> bool {
    span.contains("](<")
}

/// The relative link from the note at `from` to the note at `to`.
pub fn relative_link(from: &str, to: &str) -> String {
    let from_dir: Vec<&str> = {
        let mut parts: Vec<&str> = from.split('/').collect();
        parts.pop();
        parts
    };
    let to_parts: Vec<&str> = to.split('/').collect();
    let common = from_dir
        .iter()
        .zip(to_parts.iter())
        .take_while(|(a, b)| a == b)
        .count();
    let mut out: Vec<&str> = vec![".."; from_dir.len() - common];
    out.extend(&to_parts[common..]);
    out.join("/")
}

// ---------------------------------------------------------------------------
// Search
// ---------------------------------------------------------------------------

/// User input compiled into safe FTS5 expressions (docs/architecture.md, Search).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CompiledQuery {
    /// For the `unicode61` tables: each word quoted with `*`, each phrase quoted, joined with AND.
    pub text: Option<String>,
    /// For the trigram tables: the terms of three characters or more, quoted.
    pub names: Option<String>,
    /// Every term folded as the tokenizer folds it, for checking names and highlighting.
    pub terms: Vec<String>,
    /// Folded word parts (split as `unicode61` splits identifiers), for highlighting text.
    pub parts: Vec<String>,
    pub has_phrase: bool,
}

/// Compile literal user input. Never fails: control characters are removed,
/// unbalanced quotes are ignored, and input without word characters compiles
/// to nothing.
pub fn compile_query(input: &str) -> CompiledQuery {
    let clean: String = input.chars().filter(|c| !c.is_control()).collect();
    let mut words: Vec<String> = Vec::new();
    let mut phrases: Vec<String> = Vec::new();
    let quotes = clean.matches('"').count();
    // With an odd number of quotes, the last one is unbalanced and ignored.
    let usable = quotes - quotes % 2;
    let mut seen = 0;
    let mut in_phrase = false;
    let mut current = String::new();
    for c in clean.chars() {
        if c == '"' {
            seen += 1;
            if seen > usable {
                continue;
            }
            if in_phrase {
                phrases.push(std::mem::take(&mut current));
            } else {
                words.extend(current.split_whitespace().map(str::to_string));
                current.clear();
            }
            in_phrase = !in_phrase;
            continue;
        }
        current.push(c);
    }
    words.extend(current.split_whitespace().map(str::to_string));

    let has_word_char = |s: &str| s.chars().any(char::is_alphanumeric);
    let words: Vec<String> = words.into_iter().filter(|w| has_word_char(w)).collect();
    let phrases: Vec<String> = phrases
        .into_iter()
        .map(|p| p.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|p| has_word_char(p))
        .collect();

    let mut text_terms: Vec<String> = Vec::new();
    for w in &words {
        text_terms.push(format!("\"{}\"*", w.replace('"', "")));
    }
    for p in &phrases {
        text_terms.push(format!("\"{}\"", p.replace('"', "")));
    }
    let terms: Vec<String> = words
        .iter()
        .chain(phrases.iter())
        .map(|t| fold_str(t))
        .collect();
    let name_terms: Vec<String> = words
        .iter()
        .chain(phrases.iter())
        .filter(|t| t.chars().count() >= 3)
        .map(|t| format!("\"{}\"", t.replace('"', "")))
        .collect();
    let mut parts: Vec<String> = Vec::new();
    for t in &terms {
        for part in t
            .split(|c: char| !c.is_alphanumeric())
            .filter(|p| !p.is_empty())
        {
            if !parts.iter().any(|p| p == part) {
                parts.push(part.to_string());
            }
        }
    }
    CompiledQuery {
        text: (!text_terms.is_empty()).then(|| text_terms.join(" AND ")),
        names: (!name_terms.is_empty()).then(|| name_terms.join(" AND ")),
        terms,
        parts,
        has_phrase: !phrases.is_empty(),
    }
}

/// Lowercase and drop accents, close to `unicode61 remove_diacritics 2` for
/// Latin text. One character maps to one, so offsets line up.
pub fn fold(c: char) -> char {
    const FROM: &str = "àáâäãåāăąçćčďđèéêëēėęěìíîïīįłñńňòóôöõøōőŕřśšşßťùúûüūůűųýÿźżž";
    const TO: &str = "aaaaaaaaacccddeeeeeeeeiiiiiilnnnoooooooorrsssstuuuuuuuuyyzzz";
    let c = c.to_lowercase().next().unwrap_or(c);
    if c.is_ascii() {
        return c;
    }
    FROM.chars()
        .position(|f| f == c)
        .and_then(|i| TO.chars().nth(i))
        .unwrap_or(c)
}

pub fn fold_str(s: &str) -> String {
    s.chars().map(fold).collect()
}

/// Whether every term occurs in `name` (folded), as the trigram tables match.
pub fn name_contains_all(name: &str, terms: &[String]) -> bool {
    let folded = fold_str(name);
    terms.iter().all(|t| folded.contains(t.as_str()))
}

/// `text` split into parts, highlighting every occurrence of each term
/// anywhere in a word: for titles and paths, which match any part of a word.
pub fn highlight_substrings(text: &str, terms: &[String]) -> Vec<TextPart> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let folded: Vec<char> = chars.iter().map(|(_, c)| fold(*c)).collect();
    let mut marked = vec![false; chars.len()];
    for term in terms {
        let t: Vec<char> = term.chars().collect();
        if t.is_empty() || t.len() > folded.len() {
            continue;
        }
        for start in 0..=folded.len() - t.len() {
            if folded[start..start + t.len()] == t[..] {
                for m in marked.iter_mut().skip(start).take(t.len()) {
                    *m = true;
                }
            }
        }
    }
    let byte_at = |i: usize| chars.get(i).map_or(text.len(), |(b, _)| *b);
    let mut parts = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let state = marked[i];
        let mut j = i;
        while j < chars.len() && marked[j] == state {
            j += 1;
        }
        parts.push(TextPart {
            text: text[byte_at(i)..byte_at(j)].to_string(),
            highlight: state,
        });
        i = j;
    }
    parts
}

/// A snippet of `text` in linear time: the window of `width` words holding
/// the most distinct query parts, each matched at the start of a word as the
/// `unicode61` table matches. Scans at most a bounded number of matches, so
/// a 5 MiB note whose every line matches stays fast (Decisions, 2 Oct 2026).
pub fn snippet(text: &str, parts: &[String], width: usize) -> Vec<TextPart> {
    const MAX_HITS: usize = 2000;
    let mut words: Vec<(usize, usize)> = Vec::new();
    let mut hits: Vec<(usize, usize)> = Vec::new();
    let mut start: Option<usize> = None;
    let mut buf = String::new();
    for (i, ch) in text
        .char_indices()
        .chain(std::iter::once((text.len(), ' ')))
    {
        if ch.is_alphanumeric() && i < text.len() {
            if start.is_none() {
                start = Some(i);
                buf.clear();
            }
            buf.push(fold(ch));
        } else if let Some(s) = start.take() {
            if let Some(t) = parts.iter().position(|p| buf.starts_with(p.as_str())) {
                hits.push((words.len(), t));
            }
            words.push((s, i));
            if hits.len() >= MAX_HITS && words.len() > hits[hits.len() - 1].0 + width {
                break;
            }
        }
    }
    if words.is_empty() {
        return Vec::new();
    }
    let (mut best, mut best_n) = (hits.first().map_or(0, |h| h.0), 0);
    let mut j = 0;
    for i in 0..hits.len() {
        while hits[i].0 - hits[j].0 >= width {
            j += 1;
        }
        let n = hits[j..=i]
            .iter()
            .map(|h| h.1)
            .collect::<HashSet<_>>()
            .len();
        if n > best_n {
            best_n = n;
            best = hits[j].0;
        }
    }
    let from = best.saturating_sub(width / 4);
    let to = (from + width).min(words.len());
    let hit_set: HashSet<usize> = hits.iter().map(|h| h.0).collect();
    let mut out: Vec<TextPart> = Vec::new();
    let mut push = |text: &str, highlight: bool| match out.last_mut() {
        Some(last) if last.highlight == highlight => last.text.push_str(text),
        _ => out.push(TextPart {
            text: text.to_string(),
            highlight,
        }),
    };
    if from > 0 {
        push("…", false);
    }
    for (w, &(a, b)) in words.iter().enumerate().take(to).skip(from) {
        if w > from {
            push(" ", false);
        }
        push(&text[a..b], hit_set.contains(&w));
    }
    if to < words.len() {
        push(" …", false);
    }
    out
}

/// One candidate note from the search tables.
#[derive(Debug, Clone)]
pub struct NoteCandidate {
    pub note_id: String,
    pub title: String,
    pub relative_path: String,
    /// `bm25` of the text table (lower is better), when the text matched.
    pub text_rank: Option<f64>,
}

/// Most candidates taken from each table before merging.
const MAX_CANDIDATES: i64 = 2000;

/// Notes matching `query`, best first: titles that contain every term, then
/// the rest, each by text rank (SPEC.md, Search).
pub fn search_notes(conn: &Connection, query: &CompiledQuery) -> AppResult<Vec<NoteCandidate>> {
    let mut by_id: HashMap<String, NoteCandidate> = HashMap::new();
    if let Some(text) = &query.text {
        let mut stmt = conn.prepare_cached(
            "SELECT b.note_id, b.title, b.relative_path, bm25(note_search, 10.0, 1.0) AS rank
             FROM note_search JOIN note_bodies b ON b.seq = note_search.rowid
             WHERE note_search MATCH ?1 ORDER BY rank LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![text, MAX_CANDIDATES], |r| {
            Ok(NoteCandidate {
                note_id: r.get(0)?,
                title: r.get(1)?,
                relative_path: r.get(2)?,
                text_rank: Some(r.get(3)?),
            })
        })?;
        for row in rows {
            let c = row?;
            by_id.insert(c.note_id.clone(), c);
        }
    }
    if let Some(names) = &query.names {
        let mut stmt = conn.prepare_cached(
            "SELECT b.note_id, b.title, b.relative_path FROM note_name_search
             JOIN note_bodies b ON b.seq = note_name_search.rowid
             WHERE note_name_search MATCH ?1 ORDER BY bm25(note_name_search, 10.0, 1.0) LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![names, MAX_CANDIDATES], |r| {
            Ok(NoteCandidate {
                note_id: r.get(0)?,
                title: r.get(1)?,
                relative_path: r.get(2)?,
                text_rank: None,
            })
        })?;
        for row in rows {
            let c = row?;
            // Terms under three characters were left out of the trigram query.
            let name = format!("{} {}", c.title, c.relative_path);
            if name_contains_all(&name, &query.terms) {
                by_id.entry(c.note_id.clone()).or_insert(c);
            }
        }
    }
    Ok(rank(by_id.into_values().collect(), &query.terms, |c| {
        (&c.title, c.text_rank)
    }))
}

/// Order candidates: a title containing every term first, then by text rank,
/// then by title.
fn rank<T>(
    mut items: Vec<T>,
    terms: &[String],
    key: impl Fn(&T) -> (&String, Option<f64>),
) -> Vec<T> {
    items.sort_by(|a, b| {
        let (ta, ra) = key(a);
        let (tb, rb) = key(b);
        let tier = |t: &String| u8::from(!name_contains_all(t, terms));
        tier(ta)
            .cmp(&tier(tb))
            .then_with(|| {
                ra.unwrap_or(f64::MAX)
                    .partial_cmp(&rb.unwrap_or(f64::MAX))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| ta.to_lowercase().cmp(&tb.to_lowercase()))
    });
    items
}

/// How many match in all, for Show all N: the candidates, unless the text
/// table had more than were taken, which are then counted.
fn total_matches(
    conn: &Connection,
    table: &str,
    query: &CompiledQuery,
    candidates: usize,
) -> AppResult<u64> {
    let Some(text) = query
        .text
        .as_ref()
        .filter(|_| candidates as i64 >= MAX_CANDIDATES)
    else {
        return Ok(candidates as u64);
    };
    let sql = format!("SELECT count(*) FROM {table} WHERE {table} MATCH ?1");
    let count: i64 = conn.query_row(&sql, [text], |r| r.get(0))?;
    Ok((count as u64).max(candidates as u64))
}

/// One candidate task from the core database's search tables.
#[derive(Debug, Clone)]
pub struct TaskCandidate {
    pub task_id: String,
    pub title: String,
    pub description: String,
    pub text_rank: Option<f64>,
}

/// Tasks matching `query`, ranked like notes. Runs on the core database.
pub fn search_tasks(conn: &Connection, query: &CompiledQuery) -> AppResult<Vec<TaskCandidate>> {
    let mut by_id: HashMap<String, TaskCandidate> = HashMap::new();
    if let Some(text) = &query.text {
        let mut stmt = conn.prepare_cached(
            "SELECT t.id, t.title, t.description, bm25(task_search, 10.0, 1.0) AS rank
             FROM task_search JOIN tasks t ON t.seq = task_search.rowid
             WHERE task_search MATCH ?1 ORDER BY rank LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![text, MAX_CANDIDATES], |r| {
            Ok(TaskCandidate {
                task_id: r.get(0)?,
                title: r.get(1)?,
                description: r.get(2)?,
                text_rank: Some(r.get(3)?),
            })
        })?;
        for row in rows {
            let c = row?;
            by_id.insert(c.task_id.clone(), c);
        }
    }
    if let Some(names) = &query.names {
        let mut stmt = conn.prepare_cached(
            "SELECT t.id, t.title, t.description FROM task_title_search
             JOIN tasks t ON t.seq = task_title_search.rowid
             WHERE task_title_search MATCH ?1 LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![names, MAX_CANDIDATES], |r| {
            Ok(TaskCandidate {
                task_id: r.get(0)?,
                title: r.get(1)?,
                description: r.get(2)?,
                text_rank: None,
            })
        })?;
        for row in rows {
            let c = row?;
            if name_contains_all(&c.title, &query.terms) {
                by_id.entry(c.task_id.clone()).or_insert(c);
            }
        }
    }
    Ok(rank(by_id.into_values().collect(), &query.terms, |c| {
        (&c.title, c.text_rank)
    }))
}

/// The text after the frontmatter, where snippets are cut from.
pub fn after_frontmatter(text: &str) -> &str {
    &text[frontmatter(text).end..]
}

/// Keyword search over notes and tasks (SPEC.md, Search). Input is literal;
/// a failure of the notes index reports search as unavailable instead of an
/// error, so repository names can still match in the palette.
pub async fn search(
    service: &crate::notes::NoteService,
    request: crate::models::SearchRequest,
) -> AppResult<crate::models::SearchResults> {
    use crate::models::{IndexState, SearchGroup, SearchHit, SearchKind, SearchResults};
    let query = compile_query(&request.query);
    let limit = request.limit.unwrap_or(8).clamp(1, 200) as usize;
    let wants = |k: SearchKind| request.kinds.as_ref().is_none_or(|ks| ks.contains(&k));
    let mut status = service.index_status();
    let empty = || SearchGroup {
        hits: Vec::new(),
        total: 0,
    };
    let (mut notes, mut tasks) = (empty(), empty());
    if query.text.is_none() && query.names.is_none() {
        return Ok(SearchResults {
            query: request.query,
            notes,
            tasks,
            index: status,
        });
    }

    if wants(SearchKind::Note) && service.vault().is_some() {
        let q = query.clone();
        let found = service
            .stores
            .reader
            .call(move |conn| {
                let candidates = search_notes(conn, &q)?;
                let total = total_matches(conn, "note_search", &q, candidates.len())?;
                let top: Vec<NoteCandidate> = candidates.into_iter().take(limit).collect();
                let ids: Vec<String> = top.iter().map(|c| c.note_id.clone()).collect();
                let texts = bodies(conn, &ids)?;
                let hits: Vec<SearchHit> = top
                    .into_iter()
                    .map(|c| {
                        let body = texts.get(&c.note_id).map(|(_, b)| b.as_str()).unwrap_or("");
                        SearchHit {
                            kind: SearchKind::Note,
                            title: highlight_substrings(&c.title, &q.terms),
                            detail: c.relative_path,
                            snippet: snippet(after_frontmatter(body), &q.parts, 16),
                            id: c.note_id,
                        }
                    })
                    .collect();
                Ok(SearchGroup { hits, total })
            })
            .await;
        match found {
            Ok(group) => notes = group,
            Err(e) => {
                tracing::warn!(error = %e, details = ?e.details, "note search failed");
                status.state = IndexState::Unavailable;
                status.message = Some("Search is unavailable. Rebuild the index to fix it.".into());
                service.set_status(|s| {
                    s.state = IndexState::Unavailable;
                    s.message = status.message.clone();
                });
            }
        }
    }

    if wants(SearchKind::Task) {
        let q = query.clone();
        tasks = service
            .core()
            .call(move |conn| {
                let candidates = search_tasks(conn, &q)?;
                let total = total_matches(conn, "task_search", &q, candidates.len())?;
                let hits = candidates
                    .into_iter()
                    .take(limit)
                    .map(|c| SearchHit {
                        kind: SearchKind::Task,
                        title: highlight_substrings(&c.title, &q.terms),
                        detail: String::new(),
                        snippet: snippet(&c.description, &q.parts, 16),
                        id: c.task_id,
                    })
                    .collect();
                Ok(SearchGroup { hits, total })
            })
            .await?;
    }
    Ok(SearchResults {
        query: request.query,
        notes,
        tasks,
        index: status,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_comes_from_frontmatter_then_heading_then_file_name() {
        let fm = "---\ntitle: \"Plan\"\nbrainiac_id: abc\ntags: [x]\n---\n# Heading\n";
        let p = parse_note(fm, "a/Note.md");
        assert_eq!(p.title, "Plan");
        assert_eq!(p.embedded_id.as_deref(), Some("abc"));
        assert_eq!(
            parse_note("text\n\n## Second `code`\n", "Note.md").title,
            "Second code"
        );
        assert_eq!(parse_note("just text\n", "dir/My Note.md").title, "My Note");
        // A thematic break is not frontmatter, and `---` without a closing line is not either.
        assert_eq!(frontmatter("---\nnot closed\n"), Frontmatter::default());
    }

    #[test]
    fn frontmatter_with_crlf_and_bom_is_read() {
        let text = "\u{feff}---\r\nbrainiac_id: x1\r\n---\r\nbody";
        let fm = frontmatter(text);
        assert_eq!(fm.brainiac_id.as_deref(), Some("x1"));
        assert_eq!(&text[fm.end..], "body");
    }

    #[test]
    fn an_embedded_id_is_added_without_touching_anything_else() {
        assert_eq!(
            with_embedded_id("# T\n", "id1"),
            "---\nbrainiac_id: id1\n---\n# T\n"
        );
        assert_eq!(
            with_embedded_id("---\r\ntags: [a]\r\n---\r\nx", "id1"),
            "---\r\ntags: [a]\r\nbrainiac_id: id1\r\n---\r\nx"
        );
        let added = with_embedded_id("---\nk: v\n...\nbody", "z");
        assert_eq!(added, "---\nk: v\nbrainiac_id: z\n...\nbody");
        assert_eq!(frontmatter(&added).brainiac_id.as_deref(), Some("z"));
    }

    #[test]
    fn links_to_notes_are_found_with_their_lines() {
        let text = "intro\n[a](Other.md) [web](https://x.y/a.md) ![img](p.png)\n\n[[Wiki Page|alias]] [[folder/Deep#h]] [file](report.pdf) [rel](../up)\n";
        let links = parse_note(text, "dir/Note.md").links;
        let raws: Vec<(&str, NoteLinkKind, u32)> = links
            .iter()
            .map(|l| (l.raw.as_str(), l.kind, l.line))
            .collect();
        assert_eq!(
            raws,
            vec![
                ("Other.md", NoteLinkKind::Markdown, 2),
                ("Wiki Page", NoteLinkKind::Wikilink, 4),
                ("folder/Deep#h", NoteLinkKind::Wikilink, 4),
                ("../up", NoteLinkKind::Markdown, 4),
            ]
        );
        assert!(text[links[0].start..links[0].end].starts_with("[a]"));
    }

    #[test]
    fn links_resolve_by_path_and_by_file_name() {
        let mut names = NameTable::default();
        names.insert("Projects/Plan.md", "plan");
        names.insert("Archive/Old/Plan.md", "old-plan");
        names.insert("My Note.md", "mine");
        let md = NoteLinkKind::Markdown;
        let wiki = NoteLinkKind::Wikilink;
        assert_eq!(
            names.resolve("Projects/A.md", "Plan.md", md).as_deref(),
            Some("plan")
        );
        assert_eq!(
            names
                .resolve("Projects/A.md", "../My%20Note.md#x", md)
                .as_deref(),
            Some("mine")
        );
        assert_eq!(
            names.resolve("Projects/A.md", "/my note", md).as_deref(),
            Some("mine")
        );
        assert_eq!(names.resolve("x.md", "plan", wiki).as_deref(), Some("plan"));
        assert_eq!(
            names.resolve("x.md", "Archive/Old/Plan", wiki).as_deref(),
            Some("old-plan")
        );
        assert_eq!(names.resolve("x.md", "Missing", wiki), None);
        assert_eq!(names.resolve("a.md", "../../escape.md", md), None);
        assert_eq!(
            suggested_path("Projects/A.md", "New Idea", wiki),
            "Projects/New Idea.md"
        );
        assert_eq!(suggested_path("A.md", "sub/Idea", wiki), "sub/Idea.md");
    }

    #[test]
    fn renamed_links_are_rewritten_in_place() {
        let text = "See [plan](Plan.md#goals) and [[Plan|the plan]] and [[Projects/Plan]].\n";
        let links: Vec<StoredLink> = parse_note(text, "Projects/Index.md")
            .links
            .into_iter()
            .map(|l| StoredLink {
                source_note_id: "s".into(),
                target_note_id: Some("t".into()),
                raw_target: l.raw,
                kind: l.kind,
                start: l.start,
                end: l.end,
                line: l.line,
            })
            .collect();
        let out = rewrite_links(
            text,
            "Projects/Index.md",
            &links,
            "Projects/Done/Final Plan.md",
        )
        .unwrap();
        assert_eq!(
            out,
            "See [plan](Done/Final%20Plan.md#goals) and [[Final Plan|the plan]] and [[Projects/Done/Final Plan]].\n"
        );
        assert_eq!(relative_link("a/b/c.md", "a/d.md"), "../d.md");

        // The link's text is not its target, even when they read the same.
        let text = "See [Plan](Plan) and [Plan.md](Plan.md) and [[Plan|Plan]].";
        let links: Vec<StoredLink> = parse_note(text, "Index.md")
            .links
            .into_iter()
            .map(|l| StoredLink {
                source_note_id: "s".into(),
                target_note_id: Some("t".into()),
                raw_target: l.raw,
                kind: l.kind,
                start: l.start,
                end: l.end,
                line: l.line,
            })
            .collect();
        assert_eq!(
            rewrite_links(text, "Index.md", &links, "Final.md").unwrap(),
            "See [Plan](Final.md) and [Plan.md](Final.md) and [[Final|Plan]]."
        );
    }

    #[test]
    fn hostile_input_compiles_to_safe_expressions() {
        for input in [
            "\"unbalanced",
            "a\"b\"c\"",
            "NOT OR AND",
            "title:secret",
            "col:* ^start",
            "*",
            "^",
            "\u{0}nul\u{1}",
            "((( )))",
            "-- ++ ::",
            "\"\"",
            "",
            "NEAR(a b)",
        ] {
            let q = compile_query(input);
            if let Some(t) = &q.text {
                // Every term is quoted, so FTS5 operators stay literal.
                assert!(
                    t.split(" AND ").all(|p| p.starts_with('"')),
                    "{input:?} -> {t}"
                );
                assert!(!t.contains('\u{0}'));
            }
        }
        assert_eq!(compile_query("*").text, None);
        assert_eq!(compile_query("migrat").text.as_deref(), Some("\"migrat\"*"));
        assert_eq!(
            compile_query("async \"run time\"").text.as_deref(),
            Some("\"async\"* AND \"run time\"")
        );
        assert_eq!(compile_query("io plan").names.as_deref(), Some("\"plan\""));
        assert_eq!(
            compile_query("fetch_with_backoff").parts,
            vec!["fetch", "with", "backoff"]
        );
        assert_eq!(compile_query("Café").terms, vec!["cafe"]);
    }

    #[test]
    fn hostile_queries_never_fail_in_sqlite() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!("../migrations/index/0001_index.sql"))
            .unwrap();
        upsert_docs(
            &mut conn,
            &[IndexDoc {
                note_id: "n1".into(),
                content_hash: "h".into(),
                title: "Retry with backoff".into(),
                relative_path: "Code/fetchWithBackoff.md".into(),
                body: "Call fetch_with_backoff from tokio::spawn in src/main.rs (v0.2). Café."
                    .into(),
                links: vec![],
            }],
        )
        .unwrap();
        for input in [
            "\"unbalanced",
            "NOT",
            "title:x",
            "^a",
            "*",
            "a OR b",
            "\"\"",
            "\u{0}",
            "NEAR(x y)",
            "backoff",
            "fetch_with_backoff",
            "tokio::spawn",
            "src/main.rs",
            "v0.2",
            "cafe",
            "Backoff",
        ] {
            let q = compile_query(input);
            let found = search_notes(&conn, &q).unwrap();
            if [
                "backoff",
                "fetch_with_backoff",
                "tokio::spawn",
                "src/main.rs",
                "v0.2",
                "cafe",
                "Backoff",
            ]
            .contains(&input)
            {
                assert_eq!(found.len(), 1, "{input:?} should find the note");
            }
        }
        let q = compile_query("Backoff");
        assert!(search_notes(&conn, &q).unwrap()[0].text_rank.is_some());
    }

    #[test]
    fn snippets_highlight_word_starts_and_stay_fast_on_huge_notes() {
        let parts = compile_query("retry back").parts;
        let s = snippet(
            "We retry the request with backoff after a failure.",
            &parts,
            12,
        );
        let highlighted: Vec<&str> = s
            .iter()
            .filter(|p| p.highlight)
            .map(|p| p.text.as_str())
            .collect();
        assert_eq!(highlighted, vec!["retry", "backoff"]);

        let line = "request failed, retrying request\n";
        let huge = line.repeat((5 * 1024 * 1024) / line.len());
        let started = std::time::Instant::now();
        let s = snippet(&huge, &compile_query("request").parts, 12);
        assert!(s.iter().any(|p| p.highlight));
        assert!(started.elapsed() < std::time::Duration::from_millis(50) || cfg!(debug_assertions));

        let t = highlight_substrings("fetchWithBackoff.md", &["backoff".into()]);
        assert_eq!(t[1].text, "Backoff");
        assert!(t[1].highlight);
    }
}
