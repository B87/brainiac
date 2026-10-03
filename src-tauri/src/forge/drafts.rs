//! The `review_drafts` table (docs/architecture.md, Data model): line
//! comments kept on the Mac until the review is finished, and the summary
//! of a submission under way. Only `PullRequestService` calls these, on the
//! core database's worker.

use rusqlite::{params, Connection, OptionalExtension};

use super::markdown;
use crate::db::{enum_name, parse_enum};
use crate::models::{
    AppResult, DiffSide, PendingReview, ReviewDraft, ReviewDrafts, ReviewVerdict, ThreadAnchor,
};

const COLUMNS: &str =
    "id, path, side, line, start_line, commit_sha, body, verdict, remote_id, created_at, updated_at";

/// One row: a draft on a line, or the summary row (no path).
struct Row {
    id: String,
    path: Option<String>,
    side: Option<String>,
    line: Option<u32>,
    start_line: Option<u32>,
    commit: Option<String>,
    body: String,
    verdict: Option<String>,
    remote_id: Option<String>,
    created_at: String,
    updated_at: String,
}

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Row> {
    Ok(Row {
        id: r.get(0)?,
        path: r.get(1)?,
        side: r.get(2)?,
        line: r.get(3)?,
        start_line: r.get(4)?,
        commit: r.get(5)?,
        body: r.get(6)?,
        verdict: r.get(7)?,
        remote_id: r.get(8)?,
        created_at: r.get(9)?,
        updated_at: r.get(10)?,
    })
}

fn rows(conn: &Connection, reference: &str) -> AppResult<Vec<Row>> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT {COLUMNS} FROM review_drafts WHERE reference = ?1 ORDER BY created_at, rowid"
    ))?;
    let out = stmt.query_map([reference], row)?;
    Ok(out.collect::<Result<_, _>>()?)
}

/// A pull request's drafts and any submission under way.
pub fn list(conn: &Connection, reference: &str) -> AppResult<ReviewDrafts> {
    let mut drafts = Vec::new();
    let mut pending = None;
    for r in rows(conn, reference)? {
        match r.path {
            Some(path) => drafts.push(ReviewDraft {
                id: r.id,
                reference: reference.to_string(),
                anchor: ThreadAnchor {
                    path,
                    side: match r.side.as_deref() {
                        Some("old") => DiffSide::Old,
                        _ => DiffSide::New,
                    },
                    line: r.line,
                    start_line: r.start_line,
                    commit: r.commit,
                },
                html: markdown::render(&r.body),
                body: r.body,
                remote_id: r.remote_id,
                created_at: r.created_at,
                updated_at: r.updated_at,
            }),
            None => {
                pending = Some(PendingReview {
                    body: r.body,
                    verdict: r
                        .verdict
                        .and_then(|v| parse_enum::<ReviewVerdict>(v).ok())
                        .unwrap_or(ReviewVerdict::Comment),
                    head_sha: r.commit.unwrap_or_default(),
                    summary_sent: r.remote_id.is_some(),
                })
            }
        }
    }
    Ok(ReviewDrafts {
        reference: reference.to_string(),
        drafts,
        pending,
    })
}

/// Insert a draft, or replace the text and anchor of one.
pub fn save(
    conn: &Connection,
    reference: &str,
    id: &str,
    anchor: &ThreadAnchor,
    body: &str,
    now: &str,
) -> AppResult<()> {
    conn.execute(
        "INSERT INTO review_drafts (id, reference, path, side, line, start_line, commit_sha, body, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)
         ON CONFLICT (id) DO UPDATE SET path = excluded.path, side = excluded.side, line = excluded.line,
             start_line = excluded.start_line, commit_sha = excluded.commit_sha, body = excluded.body,
             updated_at = excluded.updated_at",
        params![
            id,
            reference,
            anchor.path,
            enum_name(anchor.side)?,
            anchor.line,
            anchor.start_line,
            anchor.commit,
            body,
            now,
        ],
    )?;
    Ok(())
}

/// Whether the draft exists, unsent, for the pull request.
pub fn exists(conn: &Connection, reference: &str, id: &str) -> AppResult<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM review_drafts WHERE id = ?1 AND reference = ?2 AND path IS NOT NULL",
            params![id, reference],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

pub fn delete(conn: &Connection, reference: &str, id: &str) -> AppResult<()> {
    conn.execute(
        "DELETE FROM review_drafts WHERE id = ?1 AND reference = ?2",
        params![id, reference],
    )?;
    Ok(())
}

/// Stamp every unsent draft with the head the review goes on: the user
/// looked at the new commits and goes on reviewing on them.
pub fn move_to(conn: &Connection, reference: &str, head_sha: &str, now: &str) -> AppResult<()> {
    conn.execute(
        "UPDATE review_drafts SET commit_sha = ?2, updated_at = ?3
         WHERE reference = ?1 AND remote_id IS NULL",
        params![reference, head_sha, now],
    )?;
    Ok(())
}

/// Keep the summary and verdict of the submission under way (one per pull
/// request); a summary already posted keeps its remote ID.
pub fn set_pending(
    conn: &Connection,
    reference: &str,
    body: &str,
    verdict: ReviewVerdict,
    head_sha: &str,
    now: &str,
) -> AppResult<()> {
    let id = format!("summary:{reference}");
    conn.execute(
        "INSERT INTO review_drafts (id, reference, body, verdict, commit_sha, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)
         ON CONFLICT (id) DO UPDATE SET body = excluded.body, verdict = excluded.verdict,
             commit_sha = excluded.commit_sha, updated_at = excluded.updated_at",
        params![id, reference, body, enum_name(verdict)?, head_sha, now],
    )?;
    Ok(())
}

/// Record that a draft, or the summary (`id` of `summary:<reference>`), was posted.
pub fn mark_sent(conn: &Connection, id: &str, remote_id: &str, now: &str) -> AppResult<()> {
    conn.execute(
        "UPDATE review_drafts SET remote_id = ?2, updated_at = ?3 WHERE id = ?1",
        params![id, remote_id, now],
    )?;
    Ok(())
}

pub fn summary_id(reference: &str) -> String {
    format!("summary:{reference}")
}

/// The review was sent whole: nothing is left to keep.
pub fn clear(conn: &Connection, reference: &str) -> AppResult<()> {
    conn.execute(
        "DELETE FROM review_drafts WHERE reference = ?1",
        [reference],
    )?;
    Ok(())
}

/// How many pull requests have drafts, for the sidebar later; cheap.
#[allow(dead_code)]
pub fn count(conn: &Connection, reference: &str) -> AppResult<u32> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM review_drafts WHERE reference = ?1 AND path IS NOT NULL",
        [reference],
        |r| r.get(0),
    )?)
}
