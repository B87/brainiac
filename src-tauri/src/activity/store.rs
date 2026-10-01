//! The activity tables (`ref_baselines`, `ref_tips`, `activity_events`).
//! Only `ActivityTracker` uses this module, so the tracker's in-memory state
//! (fingerprints, caches) can never disagree with rows changed behind its back.
//!
//! Workspace filters are applied in SQL: a pattern's `*` is SQLite's `GLOB`
//! `*`, and patterns cannot contain the other `GLOB` metacharacters
//! (`clean_patterns` rejects `?`, `[` and `]`).

use std::collections::HashMap;

use rusqlite::{params, params_from_iter, Connection, OptionalExtension};

use crate::db::{enum_name, parse_enum};
use crate::models::{ActivityDetail, ActivityKind, AppResult};

/// A stored activity event.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct EventRow {
    pub id: String,
    pub git_store: String,
    pub kind: ActivityKind,
    /// Full ref name.
    pub ref_name: String,
    /// What workspace patterns match: the branch without its remote, or the tag.
    pub match_name: String,
    pub old_id: Option<String>,
    pub new_id: String,
    pub observed_at: String,
    pub seen_at: Option<String>,
    pub detail: ActivityDetail,
}

/// Which events a workspace sees: in its Git directories, matching one of its
/// patterns, and observed after it started watching that pattern.
#[derive(Debug, Clone, Default)]
pub(super) struct Filter {
    pub stores: Vec<String>,
    /// `(is_tag, pattern, since)`.
    pub patterns: Vec<(bool, String, String)>,
}

impl Filter {
    /// A SQL condition and its parameters, numbered from `?{first}`.
    fn sql(&self, first: usize) -> (String, Vec<String>) {
        let mut values = vec![serde_json::to_string(&self.stores).unwrap_or_default()];
        let mut n = first + 1;
        let mut clauses = Vec::new();
        for (tag, pattern, since) in &self.patterns {
            let kind = if *tag {
                "substr(ref_name, 1, 10) = 'refs/tags/'"
            } else {
                "substr(ref_name, 1, 10) <> 'refs/tags/'"
            };
            clauses.push(format!(
                "({kind} AND match_name GLOB ?{n} AND observed_at >= ?{})",
                n + 1
            ));
            values.push(pattern.clone());
            values.push(since.clone());
            n += 2;
        }
        let patterns = if clauses.is_empty() {
            "0".to_string()
        } else {
            clauses.join(" OR ")
        };
        (
            format!("git_store IN (SELECT value FROM json_each(?{first})) AND ({patterns})"),
            values,
        )
    }
}

/// The patterns (JSON) a Git directory's baseline covers, and its tips.
pub(super) fn baseline(
    conn: &Connection,
    git_store: &str,
) -> AppResult<Option<(String, HashMap<String, String>)>> {
    let watched: Option<String> = conn
        .query_row(
            "SELECT watched_json FROM ref_baselines WHERE git_store = ?1",
            params![git_store],
            |r| r.get(0),
        )
        .optional()?;
    let Some(watched) = watched else {
        return Ok(None);
    };
    let mut stmt = conn.prepare("SELECT ref_name, target_id FROM ref_tips WHERE git_store = ?1")?;
    let rows = stmt.query_map(params![git_store], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(Some((watched, rows.collect::<Result<_, _>>()?)))
}

/// Write one tracking pass atomically: the new baseline (patterns and tips)
/// and its events, and prune events older than `prune_before`.
pub(super) fn save_pass(
    conn: &mut Connection,
    git_store: &str,
    watched_json: &str,
    tips: &[(String, String)],
    events: &[EventRow],
    at: &str,
    prune_before: &str,
) -> AppResult<()> {
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO ref_baselines (git_store, watched_json, taken_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(git_store) DO UPDATE SET watched_json = excluded.watched_json, taken_at = excluded.taken_at",
        params![git_store, watched_json, at],
    )?;
    tx.execute(
        "DELETE FROM ref_tips WHERE git_store = ?1",
        params![git_store],
    )?;
    for (name, target) in tips {
        tx.execute(
            "INSERT INTO ref_tips (git_store, ref_name, target_id) VALUES (?1, ?2, ?3)",
            params![git_store, name, target],
        )?;
    }
    for e in events {
        tx.execute(
            "INSERT INTO activity_events (id, git_store, kind, ref_name, match_name, old_id, new_id, observed_at, seen_at, detail_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                e.id,
                e.git_store,
                enum_name(e.kind)?,
                e.ref_name,
                e.match_name,
                e.old_id,
                e.new_id,
                e.observed_at,
                e.seen_at,
                serde_json::to_string(&e.detail)?
            ],
        )?;
    }
    tx.execute(
        "DELETE FROM activity_events WHERE observed_at < ?1",
        params![prune_before],
    )?;
    tx.commit()?;
    Ok(())
}

/// Drop a Git directory's baseline and tips, so the next pass starts over
/// silently; `events` also drops its feed.
pub(super) fn forget(conn: &Connection, git_store: &str, events: bool) -> AppResult<()> {
    conn.execute(
        "DELETE FROM ref_baselines WHERE git_store = ?1",
        params![git_store],
    )?;
    conn.execute(
        "DELETE FROM ref_tips WHERE git_store = ?1",
        params![git_store],
    )?;
    if events {
        conn.execute(
            "DELETE FROM activity_events WHERE git_store = ?1",
            params![git_store],
        )?;
    }
    Ok(())
}

const EVENT_COLUMNS: &str =
    "id, git_store, kind, ref_name, match_name, old_id, new_id, observed_at, seen_at, detail_json";

fn row_to_event(r: &rusqlite::Row<'_>) -> rusqlite::Result<EventRow> {
    Ok(EventRow {
        id: r.get(0)?,
        git_store: r.get(1)?,
        kind: parse_enum(r.get(2)?)?,
        ref_name: r.get(3)?,
        match_name: r.get(4)?,
        old_id: r.get(5)?,
        new_id: r.get(6)?,
        observed_at: r.get(7)?,
        seen_at: r.get(8)?,
        detail: serde_json::from_str(&r.get::<_, String>(9)?).unwrap_or_default(),
    })
}

/// A workspace's events, unread first, newest first, at most `limit`.
pub(super) fn list_events(
    conn: &Connection,
    filter: &Filter,
    limit: usize,
) -> AppResult<Vec<EventRow>> {
    let (cond, mut values) = filter.sql(1);
    values.push(limit.to_string());
    let sql = format!(
        "SELECT {EVENT_COLUMNS} FROM activity_events WHERE {cond}
         ORDER BY seen_at IS NOT NULL, observed_at DESC, id
         LIMIT CAST(?{} AS INTEGER)",
        values.len()
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(values.iter()), row_to_event)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// How many of a workspace's events are unread.
pub(super) fn count_unseen(conn: &Connection, filter: &Filter) -> AppResult<u32> {
    let (cond, values) = filter.sql(1);
    let sql = format!("SELECT COUNT(*) FROM activity_events WHERE seen_at IS NULL AND {cond}");
    let n: i64 = conn.query_row(&sql, params_from_iter(values.iter()), |r| r.get(0))?;
    Ok(n as u32)
}

/// Mark a workspace's unread events as seen: the listed ones, or all of them.
pub(super) fn mark_seen(
    conn: &Connection,
    filter: &Filter,
    event_ids: Option<&[String]>,
    at: &str,
) -> AppResult<usize> {
    // `?1` is the time; the filter's parameters follow; the IDs come last.
    let (cond, values) = filter.sql(2);
    let mut all = vec![at.to_string()];
    all.extend(values);
    let ids = match event_ids {
        Some(ids) => {
            all.push(serde_json::to_string(ids)?);
            format!("AND id IN (SELECT value FROM json_each(?{}))", all.len())
        }
        None => String::new(),
    };
    let sql =
        format!("UPDATE activity_events SET seen_at = ?1 WHERE seen_at IS NULL AND {cond} {ids}");
    Ok(conn.execute(&sql, params_from_iter(all.iter()))?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!("../../migrations/0001_init.sql"))
            .unwrap();
        conn
    }

    fn event(id: &str, store: &str, full: &str, name: &str, at: &str) -> EventRow {
        EventRow {
            id: id.into(),
            git_store: store.into(),
            kind: ActivityKind::Advanced,
            ref_name: full.into(),
            match_name: name.into(),
            old_id: None,
            new_id: "c".into(),
            observed_at: at.into(),
            seen_at: None,
            detail: ActivityDetail::default(),
        }
    }

    #[test]
    fn filters_by_store_pattern_kind_and_start_time() {
        let mut conn = db();
        let events = [
            event(
                "1",
                "s",
                "refs/remotes/origin/main",
                "main",
                "2026-10-01T10:00:00Z",
            ),
            event(
                "2",
                "s",
                "refs/remotes/origin/release/1",
                "release/1",
                "2026-10-01T11:00:00Z",
            ),
            event("3", "s", "refs/tags/v1", "v1", "2026-10-01T12:00:00Z"),
            event(
                "4",
                "other",
                "refs/remotes/origin/main",
                "main",
                "2026-10-01T13:00:00Z",
            ),
            event(
                "5",
                "s",
                "refs/remotes/origin/main",
                "main",
                "2026-09-01T00:00:00Z",
            ),
        ];
        save_pass(
            &mut conn,
            "s",
            "{}",
            &[],
            &events,
            "2026-10-01T00:00:00Z",
            "2000-01-01T00:00:00Z",
        )
        .unwrap();
        let filter = Filter {
            stores: vec!["s".into()],
            patterns: vec![
                (false, "main".into(), "2026-10-01T00:00:00Z".into()),
                (false, "release/*".into(), "2026-10-01T00:00:00Z".into()),
                // A tag pattern named like a branch must not match branches.
                (true, "main".into(), "2000-01-01T00:00:00Z".into()),
            ],
        };
        let ids: Vec<String> = list_events(&conn, &filter, 10)
            .unwrap()
            .into_iter()
            .map(|e| e.id)
            .collect();
        assert_eq!(ids, vec!["2", "1"]);
        assert_eq!(count_unseen(&conn, &filter).unwrap(), 2);
        assert_eq!(
            mark_seen(&conn, &filter, Some(&["1".to_string()]), "now").unwrap(),
            1
        );
        assert_eq!(count_unseen(&conn, &filter).unwrap(), 1);
        assert_eq!(mark_seen(&conn, &filter, None, "now").unwrap(), 1);
        assert_eq!(count_unseen(&conn, &filter).unwrap(), 0);
        // Other workspaces' events are untouched.
        let all = Filter {
            stores: vec!["s".into(), "other".into()],
            patterns: vec![
                (false, "*".into(), String::new()),
                (true, "*".into(), String::new()),
            ],
        };
        assert_eq!(count_unseen(&conn, &all).unwrap(), 3);
        // The newest unread come first even past the filtered ones.
        assert_eq!(list_events(&conn, &all, 1).unwrap()[0].id, "4");
    }
}
