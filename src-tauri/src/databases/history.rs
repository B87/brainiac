//! Run history and open tabs, in `history.db` (SPEC.md, Databases: History):
//! the SQL, the connection, when, how long, and the row count or error of
//! each run, never its rows. History keeps 90 days or 10,000 runs per
//! connection, whichever is smaller.

use chrono::{Duration, Utc};
use rusqlite::{params, Connection};

use crate::db::Db;
use crate::models::{AppResult, HistoryEntry, QueryTab, RunMode};

/// Runs kept per connection.
pub const MAX_RUNS: i64 = 10_000;
/// Days a run is kept.
pub const KEEP_DAYS: i64 = 90;
/// Statement text kept per run; longer statements are cut.
const MAX_SQL: usize = 100 * 1024;

#[derive(Clone)]
pub struct QueryHistory {
    db: Db,
}

/// A run to record.
pub struct Recorded {
    pub connection_id: String,
    pub sql: String,
    pub elapsed_ms: u32,
    pub rows: Option<i64>,
    pub error: Option<String>,
}

fn mode_word(mode: RunMode) -> &'static str {
    match mode {
        RunMode::ReadOnly => "read_only",
        RunMode::AutoCommit => "auto_commit",
        RunMode::Manual => "manual",
    }
}

fn mode_of(word: &str) -> RunMode {
    match word {
        "auto_commit" => RunMode::AutoCommit,
        "manual" => RunMode::Manual,
        _ => RunMode::ReadOnly,
    }
}

impl QueryHistory {
    pub fn new(db: Db) -> Self {
        QueryHistory { db }
    }

    pub async fn record(&self, run: Recorded) -> AppResult<()> {
        let mut sql = run.sql;
        if sql.len() > MAX_SQL {
            let mut end = MAX_SQL;
            while !sql.is_char_boundary(end) {
                end -= 1;
            }
            sql.truncate(end);
        }
        self.db
            .call(move |conn| {
                let now = Utc::now();
                conn.execute(
                    "INSERT INTO query_runs (id, connection_id, sql, ran_at, elapsed_ms, row_count, error)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        uuid::Uuid::new_v4().to_string(),
                        run.connection_id,
                        sql,
                        now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                        run.elapsed_ms,
                        run.rows,
                        run.error
                    ],
                )?;
                prune(conn, &run.connection_id, now)?;
                Ok(())
            })
            .await
    }

    /// The connection's runs, newest first, whose SQL contains `search`.
    pub async fn list(
        &self,
        connection_id: &str,
        search: &str,
        offset: u32,
        limit: u32,
    ) -> AppResult<Vec<HistoryEntry>> {
        let connection_id = connection_id.to_string();
        let pattern = format!(
            "%{}%",
            search
                .trim()
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        );
        self.db
            .call(move |conn| {
                let mut statement = conn.prepare(
                    "SELECT id, connection_id, sql, ran_at, elapsed_ms, row_count, error FROM query_runs
                      WHERE connection_id = ?1 AND sql LIKE ?2 ESCAPE '\\'
                      ORDER BY ran_at DESC LIMIT ?3 OFFSET ?4",
                )?;
                let rows = statement.query_map(
                    params![connection_id, pattern, limit.min(500), offset],
                    |r| {
                        Ok(HistoryEntry {
                            id: r.get(0)?,
                            connection_id: r.get(1)?,
                            sql: r.get(2)?,
                            ran_at: r.get(3)?,
                            elapsed_ms: r.get(4)?,
                            rows: r.get(5)?,
                            error: r.get(6)?,
                        })
                    },
                )?;
                Ok(rows.collect::<Result<_, _>>()?)
            })
            .await
    }

    pub async fn clear(&self, connection_id: &str) -> AppResult<()> {
        let connection_id = connection_id.to_string();
        self.db
            .call(move |conn| {
                conn.execute(
                    "DELETE FROM query_runs WHERE connection_id = ?1",
                    [connection_id],
                )?;
                Ok(())
            })
            .await
    }

    pub async fn tabs(&self) -> AppResult<Vec<QueryTab>> {
        self.db
            .call(|conn| {
                let mut statement = conn.prepare(
                    "SELECT id, connection_id, saved_query_id, title, text, saved_version, dirty, mode
                       FROM query_tabs ORDER BY position",
                )?;
                let rows = statement.query_map([], |r| {
                    Ok(QueryTab {
                        id: r.get(0)?,
                        connection_id: r.get(1)?,
                        saved_query_id: r.get(2)?,
                        title: r.get(3)?,
                        text: r.get(4)?,
                        saved_version: r.get(5)?,
                        dirty: r.get(6)?,
                        mode: mode_of(&r.get::<_, String>(7)?),
                    })
                })?;
                Ok(rows.collect::<Result<_, _>>()?)
            })
            .await
    }

    /// Replace the open tabs, in their order.
    pub async fn save_tabs(&self, tabs: Vec<QueryTab>) -> AppResult<()> {
        self.db
            .call(move |conn| {
                let tx = conn.transaction()?;
                tx.execute("DELETE FROM query_tabs", [])?;
                for (position, tab) in tabs.iter().enumerate() {
                    tx.execute(
                        "INSERT INTO query_tabs (id, position, connection_id, saved_query_id, title, text,
                            saved_version, dirty, mode)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                        params![
                            tab.id,
                            position as i64,
                            tab.connection_id,
                            tab.saved_query_id,
                            tab.title,
                            tab.text,
                            tab.saved_version,
                            tab.dirty,
                            mode_word(tab.mode)
                        ],
                    )?;
                }
                tx.commit()?;
                Ok(())
            })
            .await
    }
}

/// Drop runs older than `KEEP_DAYS` and beyond `MAX_RUNS` for the connection.
fn prune(conn: &Connection, connection_id: &str, now: chrono::DateTime<Utc>) -> AppResult<()> {
    let cutoff =
        (now - Duration::days(KEEP_DAYS)).to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    conn.execute(
        "DELETE FROM query_runs WHERE connection_id = ?1 AND ran_at < ?2",
        params![connection_id, cutoff],
    )?;
    conn.execute(
        "DELETE FROM query_runs WHERE connection_id = ?1 AND id NOT IN (
            SELECT id FROM query_runs WHERE connection_id = ?1 ORDER BY ran_at DESC LIMIT ?2)",
        params![connection_id, MAX_RUNS],
    )?;
    Ok(())
}
