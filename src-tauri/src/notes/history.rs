//! `history.db`: revisions of notes and drafts of unsaved edits (SPEC.md,
//! Delete and recovery). Functions run on the history database worker.

use rusqlite::{params, Connection, OptionalExtension};

use crate::db::{enum_name, parse_enum};
use crate::models::{AppResult, NoteDraft, NoteRevision, RevisionReason};

/// Revisions kept per note, how long, and how much in total.
pub const KEEP_PER_NOTE: i64 = 20;
pub const KEEP_DAYS: i64 = 30;
pub const KEEP_BYTES: i64 = 250 * 1024 * 1024;
/// Brainiac's own saves within this many minutes of the last count as one version.
pub const SESSION_MINUTES: i64 = 10;

pub fn put_draft(
    conn: &Connection,
    note_id: &str,
    base: &str,
    text: &str,
    now: &str,
) -> AppResult<()> {
    conn.execute(
        "INSERT INTO drafts (note_id, base_hash, content, updated_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(note_id) DO UPDATE SET base_hash = excluded.base_hash,
           content = excluded.content, updated_at = excluded.updated_at",
        params![note_id, base, text, now],
    )?;
    Ok(())
}

pub fn get_draft(conn: &Connection, note_id: &str) -> AppResult<Option<NoteDraft>> {
    Ok(conn
        .query_row(
            "SELECT content, base_hash, updated_at FROM drafts WHERE note_id = ?1",
            [note_id],
            |r| {
                Ok(NoteDraft {
                    text: r.get(0)?,
                    base_version: r.get(1)?,
                    updated_at: r.get(2)?,
                })
            },
        )
        .optional()?)
}

pub fn delete_draft(conn: &Connection, note_id: &str) -> AppResult<()> {
    conn.execute("DELETE FROM drafts WHERE note_id = ?1", [note_id])?;
    Ok(())
}

/// Whether the note's newest revision is one of Brainiac's own saves from
/// the last `SESSION_MINUTES`, so a typing session counts as one version.
pub fn in_save_session(
    conn: &Connection,
    note_id: &str,
    now: &chrono::DateTime<chrono::Utc>,
) -> AppResult<bool> {
    let newest: Option<(String, String)> = conn
        .query_row(
            "SELECT reason, created_at FROM note_revisions WHERE note_id = ?1
             ORDER BY created_at DESC LIMIT 1",
            [note_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    Ok(match newest {
        Some((reason, at)) if reason == "app_save" => chrono::DateTime::parse_from_rfc3339(&at)
            .is_ok_and(|t| (*now - t.with_timezone(&chrono::Utc)).num_minutes() < SESSION_MINUTES),
        _ => false,
    })
}

/// A revision to store: the text a note had before it changed.
#[derive(Debug, Clone)]
pub struct NewRevision {
    pub note_id: String,
    pub content: String,
    pub content_hash: String,
    pub reason: RevisionReason,
}

/// Store revisions, skipping one identical to the note's newest, then prune.
pub fn add_revisions(conn: &mut Connection, revisions: &[NewRevision], now: &str) -> AppResult<()> {
    if revisions.is_empty() {
        return Ok(());
    }
    let tx = conn.transaction()?;
    {
        let mut newest = tx.prepare_cached(
            "SELECT content_hash FROM note_revisions WHERE note_id = ?1 ORDER BY created_at DESC LIMIT 1",
        )?;
        let mut insert = tx.prepare_cached(
            "INSERT INTO note_revisions (id, note_id, content, content_hash, created_at, reason)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?;
        for r in revisions {
            let last: Option<String> = newest
                .query_row([&r.note_id], |row| row.get(0))
                .optional()?;
            if last.as_deref() == Some(r.content_hash.as_str()) {
                continue;
            }
            insert.execute(params![
                uuid::Uuid::new_v4().to_string(),
                r.note_id,
                r.content,
                r.content_hash,
                now,
                enum_name(r.reason)?
            ])?;
        }
    }
    prune(&tx, revisions.iter().map(|r| r.note_id.as_str()), now)?;
    tx.commit()?;
    Ok(())
}

/// Keep at most `KEEP_PER_NOTE` revisions per note, none older than
/// `KEEP_DAYS`, and `KEEP_BYTES` in all, dropping the oldest first. Drafts
/// are a separate table and never pruned.
fn prune<'a>(conn: &Connection, notes: impl Iterator<Item = &'a str>, now: &str) -> AppResult<()> {
    let cutoff = chrono::DateTime::parse_from_rfc3339(now)
        .map(|t| t - chrono::Duration::days(KEEP_DAYS))
        .map(|t| {
            t.with_timezone(&chrono::Utc)
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
        })
        .unwrap_or_default();
    conn.execute(
        "DELETE FROM note_revisions WHERE created_at < ?1",
        [&cutoff],
    )?;
    let mut per_note = conn.prepare_cached(
        "DELETE FROM note_revisions WHERE note_id = ?1 AND id NOT IN
           (SELECT id FROM note_revisions WHERE note_id = ?1 ORDER BY created_at DESC LIMIT ?2)",
    )?;
    for note in notes {
        per_note.execute(params![note, KEEP_PER_NOTE])?;
    }
    let total: i64 = conn.query_row(
        "SELECT COALESCE(SUM(length(CAST(content AS BLOB))), 0) FROM note_revisions",
        [],
        |r| r.get(0),
    )?;
    if total > KEEP_BYTES {
        let mut excess = total - KEEP_BYTES;
        let mut oldest = conn.prepare(
            "SELECT id, length(CAST(content AS BLOB)) FROM note_revisions ORDER BY created_at",
        )?;
        let victims: Vec<(String, i64)> = oldest
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?;
        let mut delete = conn.prepare_cached("DELETE FROM note_revisions WHERE id = ?1")?;
        for (id, size) in victims {
            if excess <= 0 {
                break;
            }
            delete.execute([&id])?;
            excess -= size;
        }
    }
    Ok(())
}

pub fn list(conn: &Connection, note_id: &str) -> AppResult<Vec<NoteRevision>> {
    let mut stmt = conn.prepare(
        "SELECT id, created_at, reason, length(CAST(content AS BLOB)) FROM note_revisions
         WHERE note_id = ?1 ORDER BY created_at DESC",
    )?;
    let rows = stmt.query_map([note_id], |r| {
        Ok(NoteRevision {
            id: r.get(0)?,
            created_at: r.get(1)?,
            reason: parse_enum(r.get(2)?)?,
            size: r.get::<_, i64>(3)? as u64,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// A revision's note and text.
pub fn get(conn: &Connection, revision_id: &str) -> AppResult<Option<(String, String)>> {
    Ok(conn
        .query_row(
            "SELECT note_id, content FROM note_revisions WHERE id = ?1",
            [revision_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?)
}

/// The newest revision's text, for restoring a missing note as a new file.
pub fn newest_text(conn: &Connection, note_id: &str) -> AppResult<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT content FROM note_revisions WHERE note_id = ?1 ORDER BY created_at DESC LIMIT 1",
            [note_id],
            |r| r.get(0),
        )
        .optional()?)
}
