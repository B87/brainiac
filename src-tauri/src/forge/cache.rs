//! `forge.db` (docs/architecture.md, Sync, cache, and request budget): pull
//! requests, files, and checks as last read, so the tabs open at once and a
//! refresh asks the provider only for what changed. Only
//! `PullRequestService` calls these, on the file's worker.

use rusqlite::{params, Connection, OptionalExtension};

use super::patch::FilePatch;
use crate::db::enum_name;
use crate::models::{
    AppResult, Conversation, PullRequest, PullRequestChecks, PullRequestFiles, PullRequestState,
};

/// A closed pull request is forgotten this long after it closed.
pub const KEEP_CLOSED_DAYS: i64 = 14;

/// A repository's list, as last read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListRead {
    pub etag: Option<String>,
    pub fetched_at: String,
}

pub fn list_read(conn: &Connection, forge: &str, closed: bool) -> AppResult<Option<ListRead>> {
    Ok(conn
        .query_row(
            "SELECT etag, fetched_at FROM list_reads WHERE forge_repository = ?1 AND closed = ?2",
            params![forge, closed],
            |r| {
                Ok(ListRead {
                    etag: r.get(0)?,
                    fetched_at: r.get(1)?,
                })
            },
        )
        .optional()?)
}

pub fn set_list_read(
    conn: &Connection,
    forge: &str,
    closed: bool,
    etag: Option<&str>,
    now: &str,
) -> AppResult<()> {
    conn.execute(
        "INSERT INTO list_reads (forge_repository, closed, etag, fetched_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (forge_repository, closed) DO UPDATE SET etag = excluded.etag, fetched_at = excluded.fetched_at",
        params![forge, closed, etag, now],
    )?;
    Ok(())
}

/// Store a pull request as read now.
pub fn put(conn: &Connection, forge: &str, pr: &PullRequest, now: &str) -> AppResult<()> {
    conn.execute(
        "INSERT INTO pull_requests (reference, forge_repository, number, state, updated_at, closed_at, version, json, fetched_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT (reference) DO UPDATE SET
             state = excluded.state, updated_at = excluded.updated_at, closed_at = excluded.closed_at,
             version = excluded.version, json = excluded.json, fetched_at = excluded.fetched_at",
        params![
            pr.reference,
            forge,
            pr.number as i64,
            enum_name(pr.state)?,
            pr.updated_at,
            pr.closed_at,
            pr.version,
            serde_json::to_string(pr)?,
            now,
        ],
    )?;
    Ok(())
}

pub fn get(conn: &Connection, reference: &str) -> AppResult<Option<(PullRequest, String)>> {
    let row: Option<(String, String)> = conn
        .query_row(
            "SELECT json, fetched_at FROM pull_requests WHERE reference = ?1",
            [reference],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    match row {
        Some((json, fetched_at)) => Ok(Some((serde_json::from_str(&json)?, fetched_at))),
        None => Ok(None),
    }
}

/// A repository's open pull requests, or its closed ones, newest update first.
pub fn list(conn: &Connection, forge: &str, closed: bool) -> AppResult<Vec<PullRequest>> {
    let states: &[PullRequestState] = if closed {
        &[PullRequestState::Merged, PullRequestState::Closed]
    } else {
        &[PullRequestState::Open, PullRequestState::Draft]
    };
    let names: Vec<String> = states
        .iter()
        .map(|s| enum_name(*s))
        .collect::<AppResult<_>>()?;
    let mut stmt = conn.prepare_cached(
        "SELECT json FROM pull_requests WHERE forge_repository = ?1 AND state IN (?2, ?3)
         ORDER BY updated_at DESC",
    )?;
    let rows = stmt.query_map(params![forge, names[0], names[1]], |r| {
        r.get::<_, String>(0)
    })?;
    let mut out = Vec::new();
    for json in rows {
        out.push(serde_json::from_str(&json?)?);
    }
    Ok(out)
}

/// Forget the open pull requests of a repository that a fresh list no longer
/// names: they were merged or closed elsewhere, and come back as such when
/// the closed list is read.
pub fn retain_open(conn: &Connection, forge: &str, references: &[String]) -> AppResult<()> {
    let keep = serde_json::to_string(references)?;
    conn.execute(
        "DELETE FROM pull_requests WHERE forge_repository = ?1 AND state IN ('open', 'draft')
         AND reference NOT IN (SELECT value FROM json_each(?2))",
        params![forge, keep],
    )?;
    Ok(())
}

pub fn files(conn: &Connection, reference: &str) -> AppResult<Option<PullRequestFiles>> {
    let json: Option<String> = conn
        .query_row(
            "SELECT json FROM pull_request_files WHERE reference = ?1",
            [reference],
            |r| r.get(0),
        )
        .optional()?;
    json.map(|j| Ok(serde_json::from_str(&j)?)).transpose()
}

pub fn put_files(conn: &Connection, files: &PullRequestFiles) -> AppResult<()> {
    conn.execute(
        "INSERT INTO pull_request_files (reference, head_sha, json, fetched_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (reference) DO UPDATE SET head_sha = excluded.head_sha, json = excluded.json, fetched_at = excluded.fetched_at",
        params![
            files.reference,
            files.head_sha,
            serde_json::to_string(files)?,
            files.fetched_at
        ],
    )?;
    Ok(())
}

pub fn checks(conn: &Connection, reference: &str) -> AppResult<Option<PullRequestChecks>> {
    let json: Option<String> = conn
        .query_row(
            "SELECT json FROM pull_request_checks WHERE reference = ?1",
            [reference],
            |r| r.get(0),
        )
        .optional()?;
    json.map(|j| Ok(serde_json::from_str(&j)?)).transpose()
}

pub fn put_checks(conn: &Connection, checks: &PullRequestChecks) -> AppResult<()> {
    conn.execute(
        "INSERT INTO pull_request_checks (reference, head_sha, json, fetched_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (reference) DO UPDATE SET head_sha = excluded.head_sha, json = excluded.json, fetched_at = excluded.fetched_at",
        params![
            checks.reference,
            checks.head_sha,
            serde_json::to_string(checks)?,
            checks.fetched_at
        ],
    )?;
    Ok(())
}

pub fn conversation(conn: &Connection, reference: &str) -> AppResult<Option<Conversation>> {
    let json: Option<String> = conn
        .query_row(
            "SELECT json FROM pull_request_conversations WHERE reference = ?1",
            [reference],
            |r| r.get(0),
        )
        .optional()?;
    json.map(|j| Ok(serde_json::from_str(&j)?)).transpose()
}

pub fn put_conversation(conn: &Connection, conversation: &Conversation) -> AppResult<()> {
    conn.execute(
        "INSERT INTO pull_request_conversations (reference, json, fetched_at) VALUES (?1, ?2, ?3)
         ON CONFLICT (reference) DO UPDATE SET json = excluded.json, fetched_at = excluded.fetched_at",
        params![
            conversation.reference,
            serde_json::to_string(conversation)?,
            conversation.fetched_at
        ],
    )?;
    Ok(())
}

/// The head commit whose provider diff is stored for a pull request.
pub fn patch_set(conn: &Connection, reference: &str) -> AppResult<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT head_sha FROM pull_request_patch_sets WHERE reference = ?1",
            [reference],
            |r| r.get(0),
        )
        .optional()?)
}

/// One file's part of the stored diff, by its path after or before the change.
pub fn patch(conn: &Connection, reference: &str, path: &str) -> AppResult<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT patch FROM pull_request_patches WHERE reference = ?1 AND path = ?2",
            params![reference, path],
            |r| r.get(0),
        )
        .optional()?)
}

/// Replace a pull request's stored diff with one read for `head_sha`. A
/// renamed file is stored under both its paths.
pub fn put_patch_set(
    conn: &Connection,
    reference: &str,
    head_sha: &str,
    files: &[FilePatch],
    now: &str,
) -> AppResult<()> {
    conn.execute(
        "DELETE FROM pull_request_patches WHERE reference = ?1",
        [reference],
    )?;
    let mut insert = conn.prepare_cached(
        "INSERT OR REPLACE INTO pull_request_patches (reference, path, patch) VALUES (?1, ?2, ?3)",
    )?;
    for f in files {
        insert.execute(params![reference, f.path, f.text])?;
        if let Some(old) = &f.old_path {
            insert.execute(params![reference, old, f.text])?;
        }
    }
    conn.execute(
        "INSERT INTO pull_request_patch_sets (reference, head_sha, fetched_at) VALUES (?1, ?2, ?3)
         ON CONFLICT (reference) DO UPDATE SET head_sha = excluded.head_sha, fetched_at = excluded.fetched_at",
        params![reference, head_sha, now],
    )?;
    Ok(())
}

/// Drop pull requests closed more than `KEEP_CLOSED_DAYS` ago, with their files and checks.
pub fn prune(conn: &Connection, now: &str) -> AppResult<usize> {
    let cutoff = chrono::DateTime::parse_from_rfc3339(now)
        .map(|t| t - chrono::Duration::days(KEEP_CLOSED_DAYS))
        .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        .unwrap_or_else(|_| now.to_string());
    let gone = conn.execute(
        "DELETE FROM pull_requests WHERE closed_at IS NOT NULL AND closed_at < ?1",
        [&cutoff],
    )?;
    conn.execute(
        "DELETE FROM pull_request_files WHERE reference NOT IN (SELECT reference FROM pull_requests)",
        [],
    )?;
    for table in [
        "pull_request_checks",
        "pull_request_conversations",
        "pull_request_patch_sets",
        "pull_request_patches",
    ] {
        conn.execute(
            &format!(
                "DELETE FROM {table} WHERE reference NOT IN (SELECT reference FROM pull_requests)"
            ),
            [],
        )?;
    }
    Ok(gone)
}
