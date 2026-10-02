//! The core database's tables for v0.2 notes: vaults, note identities, links
//! from notes to repositories, and dismissed suggestions. Functions run on
//! the core database worker.

use rusqlite::{params, Connection, OptionalExtension};

use super::files::{self, mtime_rfc3339};
use crate::db::{enum_name, parse_enum};
use crate::models::{AppResult, NoteSummary, NoteTextState};

/// A vault row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultRow {
    pub id: String,
    pub name: String,
    pub root_path: String,
}

pub fn active_vault(conn: &Connection) -> AppResult<Option<VaultRow>> {
    Ok(conn
        .query_row(
            "SELECT id, name, root_path FROM vaults WHERE active = 1",
            [],
            |r| {
                Ok(VaultRow {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    root_path: r.get(2)?,
                })
            },
        )
        .optional()?)
}

/// Make the vault at `root_path` the active one, reusing its row when the
/// folder was a vault before, so its notes keep their identities.
pub fn activate_vault(
    conn: &mut Connection,
    root_path: &str,
    name: &str,
    now: &str,
) -> AppResult<VaultRow> {
    let tx = conn.transaction()?;
    let existing: Option<String> = tx
        .query_row(
            "SELECT id FROM vaults WHERE root_path = ?1",
            [root_path],
            |r| r.get(0),
        )
        .optional()?;
    tx.execute("UPDATE vaults SET active = 0 WHERE active = 1", [])?;
    let id = match existing {
        Some(id) => {
            tx.execute(
                "UPDATE vaults SET active = 1, name = ?2 WHERE id = ?1",
                params![id, name],
            )?;
            id
        }
        None => {
            let id = uuid::Uuid::new_v4().to_string();
            tx.execute(
                "INSERT INTO vaults (id, name, root_path, created_at, active) VALUES (?1, ?2, ?3, ?4, 1)",
                params![id, name, root_path, now],
            )?;
            id
        }
    };
    tx.commit()?;
    Ok(VaultRow {
        id,
        name: name.to_string(),
        root_path: root_path.to_string(),
    })
}

/// A note row as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteRow {
    pub id: String,
    pub vault_id: String,
    pub relative_path: String,
    pub embedded_id: Option<String>,
    pub title: String,
    pub content_hash: String,
    pub size: i64,
    pub mtime: i64,
    pub text_state: NoteTextState,
    pub created_at: String,
    pub last_opened_at: Option<String>,
    pub missing_at: Option<String>,
    pub trashed_at: Option<String>,
    /// False for a note of a vault chosen before the current one: it counts as missing.
    pub in_active_vault: bool,
}

impl NoteRow {
    /// Its file is in the active vault, as far as Brainiac knows.
    pub fn is_live(&self) -> bool {
        self.missing_at.is_none() && self.in_active_vault
    }
}

const NOTE_COLUMNS: &str = "n.id, n.vault_id, n.relative_path, n.embedded_id, n.title, n.content_hash, n.size, n.mtime, n.text_state, n.created_at, n.last_opened_at, n.missing_at, n.trashed_at,
    COALESCE((SELECT v.active FROM vaults v WHERE v.id = n.vault_id), 0) = 1";

/// Whether another live note of the vault carries the same `brainiac_id`.
const CONFLICT_COLUMN: &str = "CASE WHEN n.embedded_id IS NULL OR n.missing_at IS NOT NULL THEN 0
     ELSE (SELECT COUNT(*) FROM notes d WHERE d.vault_id = n.vault_id AND d.embedded_id = n.embedded_id
           AND d.missing_at IS NULL) > 1 END";

fn row_to_note(r: &rusqlite::Row<'_>) -> rusqlite::Result<NoteRow> {
    Ok(NoteRow {
        id: r.get(0)?,
        vault_id: r.get(1)?,
        relative_path: r.get(2)?,
        embedded_id: r.get(3)?,
        title: r.get(4)?,
        content_hash: r.get(5)?,
        size: r.get(6)?,
        mtime: r.get(7)?,
        text_state: parse_enum(r.get(8)?)?,
        created_at: r.get(9)?,
        last_opened_at: r.get(10)?,
        missing_at: r.get(11)?,
        trashed_at: r.get(12)?,
        in_active_vault: r.get(13)?,
    })
}

pub fn summary_of(row: &NoteRow, id_conflict: bool) -> NoteSummary {
    NoteSummary {
        id: row.id.clone(),
        relative_path: row.relative_path.clone(),
        title: row.title.clone(),
        text_state: row.text_state,
        missing: !row.is_live(),
        trashed: row.trashed_at.is_some(),
        id_conflict,
        has_embedded_id: row.embedded_id.is_some(),
        modified_at: mtime_rfc3339(row.mtime),
        last_opened_at: row.last_opened_at.clone(),
        title_file_name: title_file_name(row),
    }
}

/// The file name a live text note's title would give, when its own differs.
fn title_file_name(row: &NoteRow) -> Option<String> {
    if !row.is_live() || row.text_state != NoteTextState::Text {
        return None;
    }
    let stem = crate::index::file_stem(&row.relative_path);
    // A title taken from the file name always matches, whatever it contains.
    if row.title == stem {
        return None;
    }
    let name = files::file_name_for(&row.title);
    (!files::name_matches(stem, &name)).then(|| format!("{name}.md"))
}

fn row_to_summary(r: &rusqlite::Row<'_>) -> rusqlite::Result<NoteSummary> {
    let row = row_to_note(r)?;
    let conflict: bool = r.get(14)?;
    Ok(summary_of(&row, conflict))
}

pub fn get_note(conn: &Connection, id: &str) -> AppResult<Option<NoteRow>> {
    let sql = format!("SELECT {NOTE_COLUMNS} FROM notes n WHERE n.id = ?1");
    Ok(conn.query_row(&sql, [id], row_to_note).optional()?)
}

pub fn get_summary(conn: &Connection, id: &str) -> AppResult<Option<NoteSummary>> {
    let sql = format!("SELECT {NOTE_COLUMNS}, {CONFLICT_COLUMN} FROM notes n WHERE n.id = ?1");
    Ok(conn.query_row(&sql, [id], row_to_summary).optional()?)
}

/// Summaries of the given notes, in the order asked; unknown IDs are skipped.
pub fn summaries(conn: &Connection, ids: &[String]) -> AppResult<Vec<NoteSummary>> {
    let sql = format!("SELECT {NOTE_COLUMNS}, {CONFLICT_COLUMN} FROM notes n WHERE n.id = ?1");
    let mut stmt = conn.prepare_cached(&sql)?;
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(s) = stmt.query_row([id], row_to_summary).optional()? {
            out.push(s);
        }
    }
    Ok(out)
}

/// Live notes of a vault inside `folder` (not below it), by path.
pub fn summaries_in_folder(
    conn: &Connection,
    vault_id: &str,
    folder: &str,
) -> AppResult<std::collections::HashMap<String, NoteSummary>> {
    let sql = format!(
        "SELECT {NOTE_COLUMNS}, {CONFLICT_COLUMN} FROM notes n
         WHERE n.vault_id = ?1 AND n.missing_at IS NULL
           AND (?2 = '' AND instr(n.relative_path, '/') = 0
                OR substr(n.relative_path, 1, length(?2) + 1) = ?2 || '/'
                   AND instr(substr(n.relative_path, length(?2) + 2), '/') = 0)"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![vault_id, folder], row_to_summary)?;
    let mut out = std::collections::HashMap::new();
    for row in rows {
        let s = row?;
        out.insert(s.relative_path.clone(), s);
    }
    Ok(out)
}

/// Every live note of a vault.
pub fn live_notes(conn: &Connection, vault_id: &str) -> AppResult<Vec<NoteRow>> {
    let sql = format!(
        "SELECT {NOTE_COLUMNS} FROM notes n WHERE n.vault_id = ?1 AND n.missing_at IS NULL"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([vault_id], row_to_note)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn live_note_at(conn: &Connection, vault_id: &str, path: &str) -> AppResult<Option<NoteRow>> {
    let sql = format!(
        "SELECT {NOTE_COLUMNS} FROM notes n
         WHERE n.vault_id = ?1 AND n.relative_path = ?2 AND n.missing_at IS NULL"
    );
    Ok(conn
        .query_row(&sql, params![vault_id, path], row_to_note)
        .optional()?)
}

/// Missing notes that are not in the trash and carry one of these `brainiac_id`s.
pub fn missing_by_embedded_id(
    conn: &Connection,
    vault_id: &str,
    ids: &[String],
) -> AppResult<Vec<NoteRow>> {
    let sql = format!(
        "SELECT {NOTE_COLUMNS} FROM notes n WHERE n.vault_id = ?1 AND n.embedded_id = ?2
           AND n.missing_at IS NOT NULL AND n.trashed_at IS NULL"
    );
    let mut stmt = conn.prepare_cached(&sql)?;
    let mut out = Vec::new();
    for id in ids {
        let rows = stmt.query_map(params![vault_id, id], row_to_note)?;
        for row in rows {
            out.push(row?);
        }
    }
    Ok(out)
}

/// The most recent missing note (not in the trash) at each of these paths.
pub fn missing_at_paths(
    conn: &Connection,
    vault_id: &str,
    paths: &[String],
) -> AppResult<std::collections::HashMap<String, NoteRow>> {
    let sql = format!(
        "SELECT {NOTE_COLUMNS} FROM notes n WHERE n.vault_id = ?1 AND n.relative_path = ?2
           AND n.missing_at IS NOT NULL AND n.trashed_at IS NULL
         ORDER BY n.missing_at DESC LIMIT 1"
    );
    let mut stmt = conn.prepare_cached(&sql)?;
    let mut out = std::collections::HashMap::new();
    for path in paths {
        if let Some(row) = stmt
            .query_row(params![vault_id, path], row_to_note)
            .optional()?
        {
            out.insert(path.clone(), row);
        }
    }
    Ok(out)
}

/// What a scan or save learned about a note's file.
#[derive(Debug, Clone)]
pub struct FileState {
    pub relative_path: String,
    pub embedded_id: Option<String>,
    pub title: String,
    pub content_hash: String,
    pub size: i64,
    pub mtime: i64,
    pub text_state: NoteTextState,
}

/// Point a note at a file: its path and content now, and live again if it was missing.
pub fn set_file(conn: &Connection, id: &str, f: &FileState) -> AppResult<()> {
    conn.prepare_cached(
        "UPDATE notes SET relative_path = ?2, embedded_id = ?3, title = ?4, content_hash = ?5,
           size = ?6, mtime = ?7, text_state = ?8, missing_at = NULL, trashed_at = NULL
         WHERE id = ?1",
    )?
    .execute(params![
        id,
        f.relative_path,
        f.embedded_id,
        f.title,
        f.content_hash,
        f.size,
        f.mtime,
        enum_name(f.text_state)?
    ])?;
    Ok(())
}

pub fn insert_note(
    conn: &Connection,
    id: &str,
    vault_id: &str,
    f: &FileState,
    now: &str,
) -> AppResult<()> {
    conn.prepare_cached(
        "INSERT INTO notes (id, vault_id, relative_path, embedded_id, title, content_hash, size, mtime, text_state, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
    )?
    .execute(params![
        id,
        vault_id,
        f.relative_path,
        f.embedded_id,
        f.title,
        f.content_hash,
        f.size,
        f.mtime,
        enum_name(f.text_state)?,
        now
    ])?;
    Ok(())
}

/// Same content, new size or modification time (a touch): no re-index.
pub fn touch(conn: &Connection, id: &str, size: i64, mtime: i64) -> AppResult<()> {
    conn.prepare_cached("UPDATE notes SET size = ?2, mtime = ?3 WHERE id = ?1")?
        .execute(params![id, size, mtime])?;
    Ok(())
}

pub fn mark_missing(conn: &Connection, id: &str, at: &str, trashed: bool) -> AppResult<()> {
    conn.prepare_cached(
        "UPDATE notes SET missing_at = COALESCE(missing_at, ?2),
           trashed_at = CASE WHEN ?3 THEN ?2 ELSE trashed_at END
         WHERE id = ?1",
    )?
    .execute(params![id, at, trashed])?;
    Ok(())
}

/// Move a note into a vault, as when a note of an earlier vault is restored into the current one.
pub fn set_vault(conn: &Connection, id: &str, vault_id: &str) -> AppResult<()> {
    conn.execute(
        "UPDATE notes SET vault_id = ?2 WHERE id = ?1",
        params![id, vault_id],
    )?;
    Ok(())
}

pub fn set_path(conn: &Connection, id: &str, path: &str) -> AppResult<()> {
    conn.execute(
        "UPDATE notes SET relative_path = ?2 WHERE id = ?1",
        params![id, path],
    )?;
    Ok(())
}

pub fn mark_opened(conn: &Connection, id: &str, at: &str) -> AppResult<()> {
    conn.execute(
        "UPDATE notes SET last_opened_at = ?2 WHERE id = ?1",
        params![id, at],
    )?;
    Ok(())
}

/// Recently opened live notes of a vault.
pub fn recent(conn: &Connection, vault_id: &str, limit: usize) -> AppResult<Vec<NoteSummary>> {
    let sql = format!(
        "SELECT {NOTE_COLUMNS}, {CONFLICT_COLUMN} FROM notes n
         WHERE n.vault_id = ?1 AND n.missing_at IS NULL AND n.last_opened_at IS NOT NULL
         ORDER BY n.last_opened_at DESC LIMIT ?2"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![vault_id, limit as i64], row_to_summary)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Pinned notes in pin order.
pub fn pinned(conn: &Connection) -> AppResult<Vec<NoteSummary>> {
    let sql = format!(
        "SELECT {NOTE_COLUMNS}, {CONFLICT_COLUMN} FROM pins p JOIN notes n ON n.id = p.entity_id
         WHERE p.entity_type = 'note' ORDER BY p.position"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], row_to_summary)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Notes in the trash, newest first.
pub fn trashed(conn: &Connection, vault_id: &str) -> AppResult<Vec<(NoteSummary, String)>> {
    let sql = format!(
        "SELECT {NOTE_COLUMNS}, {CONFLICT_COLUMN} FROM notes n
         WHERE n.vault_id = ?1 AND n.trashed_at IS NOT NULL ORDER BY n.trashed_at DESC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([vault_id], |r| {
        Ok((row_to_summary(r)?, r.get::<_, String>(12)?))
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Whether anything refers to a note: tasks, repository links, or a pin.
pub fn has_context(conn: &Connection, id: &str) -> AppResult<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM tasks WHERE linked_note_id = ?1)
             OR EXISTS (SELECT 1 FROM note_repository_links WHERE note_id = ?1)
             OR EXISTS (SELECT 1 FROM pins WHERE entity_type = 'note' AND entity_id = ?1)",
        [id],
        |r| r.get(0),
    )?)
}

pub fn delete_note(conn: &Connection, id: &str) -> AppResult<()> {
    conn.execute(
        "DELETE FROM pins WHERE entity_type = 'note' AND entity_id = ?1",
        [id],
    )?;
    conn.execute("DELETE FROM notes WHERE id = ?1", [id])?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Links to repositories
// ---------------------------------------------------------------------------

/// A note-to-repository link as stored, with whether the repository is still registered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryLink {
    pub note_id: String,
    pub repository_id: String,
    pub repository_name: String,
    pub remote_url: Option<String>,
    pub registered: bool,
}

pub fn repository_links(conn: &Connection, note_id: &str) -> AppResult<Vec<RepositoryLink>> {
    let mut stmt = conn.prepare(
        "SELECT l.note_id, l.repository_id, l.repository_name, l.remote_url, r.id IS NOT NULL
         FROM note_repository_links l LEFT JOIN repositories r ON r.id = l.repository_id
         WHERE l.note_id = ?1 ORDER BY l.repository_name COLLATE NOCASE",
    )?;
    let rows = stmt.query_map([note_id], |r| {
        Ok(RepositoryLink {
            note_id: r.get(0)?,
            repository_id: r.get(1)?,
            repository_name: r.get(2)?,
            remote_url: r.get(3)?,
            registered: r.get(4)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Live notes linked to a repository, by title.
pub fn notes_linked_to(conn: &Connection, repository_id: &str) -> AppResult<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT n.id FROM note_repository_links l JOIN notes n ON n.id = l.note_id
         WHERE l.repository_id = ?1 ORDER BY n.missing_at IS NOT NULL, n.title COLLATE NOCASE",
    )?;
    let rows = stmt.query_map([repository_id], |r| r.get(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn link_repository(
    conn: &Connection,
    note_id: &str,
    repository_id: &str,
    name: &str,
    remote_url: Option<&str>,
    now: &str,
) -> AppResult<bool> {
    conn.execute(
        "DELETE FROM dismissed_suggestions WHERE note_id = ?1 AND repository_id = ?2",
        params![note_id, repository_id],
    )?;
    Ok(conn.execute(
        "INSERT OR IGNORE INTO note_repository_links (note_id, repository_id, repository_name, remote_url, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![note_id, repository_id, name, remote_url, now],
    )? > 0)
}

pub fn unlink_repository(conn: &Connection, note_id: &str, repository_id: &str) -> AppResult<bool> {
    Ok(conn.execute(
        "DELETE FROM note_repository_links WHERE note_id = ?1 AND repository_id = ?2",
        params![note_id, repository_id],
    )? > 0)
}

pub fn dismiss_suggestion(conn: &Connection, note_id: &str, repository_id: &str) -> AppResult<()> {
    conn.execute(
        "INSERT OR IGNORE INTO dismissed_suggestions (note_id, repository_id) VALUES (?1, ?2)",
        params![note_id, repository_id],
    )?;
    Ok(())
}

pub fn dismissed(conn: &Connection, note_id: &str) -> AppResult<Vec<String>> {
    let mut stmt =
        conn.prepare("SELECT repository_id FROM dismissed_suggestions WHERE note_id = ?1")?;
    let rows = stmt.query_map([note_id], |r| r.get(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Move every link from a removed repository to a registered one with the
/// same remote (Reconnect). Links the note already has are kept once.
pub fn reconnect(
    conn: &mut Connection,
    from: &str,
    to: &str,
    name: &str,
    remote_url: Option<&str>,
) -> AppResult<usize> {
    let tx = conn.transaction()?;
    let moved = tx.execute(
        "UPDATE OR IGNORE note_repository_links SET repository_id = ?2, repository_name = ?3, remote_url = ?4
         WHERE repository_id = ?1",
        params![from, to, name, remote_url],
    )?;
    tx.execute(
        "DELETE FROM note_repository_links WHERE repository_id = ?1",
        [from],
    )?;
    let tasks = tx.execute(
        "UPDATE tasks SET linked_repository_id = ?2, version = version + 1, updated_at = ?3
         WHERE linked_repository_id = ?1",
        params![from, to, crate::models::now_rfc3339()],
    )?;
    tx.commit()?;
    Ok(moved + tasks)
}

/// Registered repositories: ID, folder name, remote URL.
pub fn repositories(conn: &Connection) -> AppResult<Vec<(String, String, Option<String>)>> {
    let mut stmt = conn.prepare("SELECT id, canonical_root, remote_url FROM repositories")?;
    let rows = stmt.query_map([], |r| {
        let root: String = r.get(1)?;
        let name = std::path::Path::new(&root)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or(root.clone());
        Ok((r.get(0)?, name, r.get(2)?))
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}
