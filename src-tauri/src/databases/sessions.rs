//! Query sessions (docs/architecture.md, Databases — v0.4, Sessions): one
//! database session per tab, opened on the tab's first run, so nothing a tab
//! does reaches another. A session closes after 10 idle minutes, and at most
//! `MAX_SESSIONS` are open; the least recently used idle one closes first.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::connections::ConnectionService;
use super::driver::{plain_failure, Canceller, Ran, Session};
use super::export::FileSink;
use super::history::{QueryHistory, Recorded};
use super::statements::{self, byte_to_utf16, utf16_to_byte};
use crate::models::{
    now_rfc3339, AppError, AppResult, DbAccess, DbConnection, DbExportResult, DbFailureReason,
    DbSchema, ErrorCode, ExplainMode, ExportRequest, RunMode, RunStatementRequest, StatementResult,
    StatementRun,
};

/// Rows a result returns unless the tab asks for all of them.
pub const ROW_CAP: usize = 1_000;
/// The most rows Fetch All returns.
pub const FETCH_ALL_CAP: usize = 100_000;
/// Open sessions at once, across all tabs.
pub const MAX_SESSIONS: usize = 8;
/// A session unused this long is closed; the next run reconnects.
pub const IDLE_CLOSE: Duration = Duration::from_secs(10 * 60);

struct TabSession {
    connection_id: String,
    /// The connection's version when the session opened: an edited
    /// connection reconnects on the tab's next run.
    connection_version: i64,
    // `tokio::sync::Mutex` because the lock is held across a statement's
    // `await`; it also keeps a tab to one statement at a time.
    session: tokio::sync::Mutex<Option<Session>>,
    canceller: Mutex<Option<Canceller>>,
    /// Set when the tab had a session that was closed or lost, so the next
    /// run says session settings are gone.
    had_session: AtomicBool,
    /// Set by Cancel, so Run All stops before its next statement.
    stop: AtomicBool,
    /// Whether the session has a transaction open, readable while a statement
    /// holds the session's lock: quitting asks about it, and an edited
    /// connection keeps the session until it ends.
    in_transaction: AtomicBool,
    last_used: Mutex<Instant>,
}

pub struct QuerySessions {
    connections: Arc<ConnectionService>,
    tabs: Mutex<HashMap<String, Arc<TabSession>>>,
    schemas: Mutex<HashMap<String, (i64, DbSchema)>>,
    /// Where runs are recorded; `None` in tests that do not need it.
    history: Option<QueryHistory>,
    /// Settings → Databases: keep a history of runs.
    history_on: AtomicBool,
}

/// The rows a run returned or changed, for the history.
fn rows_of(result: &StatementResult) -> Option<i64> {
    match result {
        StatementResult::Rows { rows, .. } => Some(rows.len() as i64),
        StatementResult::Command { tag } => tag.rsplit(' ').next().and_then(|n| n.parse().ok()),
        _ => None,
    }
}

impl QuerySessions {
    pub fn new(connections: Arc<ConnectionService>) -> Self {
        QuerySessions {
            connections,
            tabs: Mutex::new(HashMap::new()),
            schemas: Mutex::new(HashMap::new()),
            history: None,
            history_on: AtomicBool::new(false),
        }
    }

    /// Record runs in `history` while `on`.
    pub fn with_history(mut self, history: QueryHistory, on: bool) -> Self {
        self.history = Some(history);
        self.history_on = AtomicBool::new(on);
        self
    }

    pub fn set_history(&self, on: bool) {
        self.history_on.store(on, Ordering::SeqCst);
    }

    pub fn history(&self) -> Option<&QueryHistory> {
        self.history.as_ref()
    }

    pub fn connections(&self) -> &Arc<ConnectionService> {
        &self.connections
    }

    /// The tab's session holder, replaced when the tab now uses another
    /// connection or its connection was edited.
    fn tab(&self, tab_id: &str, connection: &DbConnection) -> Arc<TabSession> {
        let mut tabs = self.tabs.lock().expect("tabs lock");
        if let Some(tab) = tabs.get(tab_id) {
            if tab.connection_id == connection.id
                && (tab.connection_version == connection.version
                    // Replacing the session would roll the transaction back
                    // unseen; the edit applies once it is committed or rolled back.
                    || tab.in_transaction.load(Ordering::SeqCst))
            {
                return Arc::clone(tab);
            }
        }
        let tab = Arc::new(TabSession {
            connection_id: connection.id.clone(),
            connection_version: connection.version,
            session: tokio::sync::Mutex::new(None),
            canceller: Mutex::new(None),
            had_session: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            in_transaction: AtomicBool::new(false),
            last_used: Mutex::new(Instant::now()),
        });
        tabs.insert(tab_id.to_string(), Arc::clone(&tab));
        tab
    }

    /// Close idle sessions until there is room for one more.
    fn make_room(&self, keep: &Arc<TabSession>) {
        let tabs: Vec<Arc<TabSession>> = self
            .tabs
            .lock()
            .expect("tabs lock")
            .values()
            .cloned()
            .collect();
        let mut open: Vec<(Instant, Arc<TabSession>)> = tabs
            .into_iter()
            .filter(|t| !Arc::ptr_eq(t, keep))
            .filter_map(|t| {
                let is_open = t.session.try_lock().map(|s| s.is_some()).unwrap_or(true);
                let used = *t.last_used.lock().expect("last used lock");
                is_open.then_some((used, t))
            })
            .collect();
        open.sort_by_key(|(used, _)| *used);
        let mut count = open.len();
        for (_, tab) in open {
            if count < MAX_SESSIONS {
                break;
            }
            // A tab running a statement holds its lock and is skipped, and
            // an open transaction is never closed for room.
            if let Ok(mut session) = tab.session.try_lock() {
                if session.as_ref().is_some_and(|s| s.transaction().is_some()) {
                    continue;
                }
                if session.take().is_some() {
                    tab.had_session.store(true, Ordering::SeqCst);
                    count -= 1;
                }
            }
        }
    }

    /// The `:name` parameters of the statements a run would cover, each once.
    pub async fn parameters(&self, request: &RunStatementRequest) -> AppResult<Vec<String>> {
        let connection = self.connections.get(&request.connection_id).await?;
        let mut names: Vec<String> = Vec::new();
        for range in chosen(request, connection.kind) {
            for name in statements::parameters(&request.text[range], connection.kind) {
                if !names.contains(&name) {
                    names.push(name);
                }
            }
        }
        Ok(names)
    }

    /// Run the statement under the cursor, each statement in the selection,
    /// or (with `all`) every statement, stopping at the first failure.
    pub async fn run(&self, request: RunStatementRequest) -> AppResult<Vec<StatementRun>> {
        let connection = self.connections.get(&request.connection_id).await?;
        if request.mode != RunMode::ReadOnly && connection.access != DbAccess::ReadWrite {
            return Err(AppError::validation(
                "This connection is read only. Its access can be changed in Edit Connection.",
            ));
        }
        let ranges = chosen(&request, connection.kind);
        if ranges.is_empty() {
            return Err(AppError::validation("There is no statement to run here."));
        }
        let cap = if request.fetch_all {
            FETCH_ALL_CAP
        } else {
            ROW_CAP
        };
        let tab = self.tab(&request.tab_id, &connection);
        tab.stop.store(false, Ordering::SeqCst);
        let mut guard = tab.session.try_lock().map_err(|_| {
            AppError::new(
                ErrorCode::Conflict,
                "A statement is already running in this tab.",
            )
        })?;
        if request.mode != RunMode::Manual
            && guard.as_ref().is_some_and(|s| s.transaction().is_some())
        {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "This tab has a transaction open. Commit or roll it back first.",
            ));
        }
        // The first Manual statement opens a transaction while it runs.
        if request.mode == RunMode::Manual {
            tab.in_transaction.store(true, Ordering::SeqCst);
        }
        let runs = self
            .run_statements(&request, &connection, ranges, cap, &tab, &mut guard)
            .await;
        tab.in_transaction.store(
            guard.as_ref().is_some_and(|s| s.transaction().is_some()),
            Ordering::SeqCst,
        );
        runs
    }

    /// The statements of a run, on the tab's locked session.
    async fn run_statements(
        &self,
        request: &RunStatementRequest,
        connection: &DbConnection,
        ranges: Vec<Range<usize>>,
        cap: usize,
        tab: &Arc<TabSession>,
        guard: &mut Option<Session>,
    ) -> AppResult<Vec<StatementRun>> {
        let text = request.text.as_str();
        let mut runs = Vec::new();
        for range in ranges {
            if tab.stop.load(Ordering::SeqCst) {
                break;
            }
            let sql = statements::without_semicolon(&text[range.clone()]);
            let started = Instant::now();
            let mut reconnected = false;
            let mut ran = None;
            for _attempt in 0..2 {
                if guard
                    .as_ref()
                    .is_some_and(|s| s.is_closed() && s.transaction().is_some())
                {
                    // The connection dropped with a transaction open: say so
                    // rather than carrying on in a new one.
                    *guard = None;
                    tab.had_session.store(true, Ordering::SeqCst);
                    ran = Some(Ran {
                        result: StatementResult::Failed {
                            failure: plain_failure(
                                DbFailureReason::Connection,
                                "The connection was lost. The server rolled the open transaction back.".into(),
                            ),
                        },
                        ran_read_only: false,
                    });
                    break;
                }
                if guard.as_ref().is_none_or(Session::is_closed) {
                    if guard.take().is_some() || tab.had_session.swap(false, Ordering::SeqCst) {
                        reconnected = true;
                    }
                    self.make_room(tab);
                    let target = self.connections.target(connection).await?;
                    let session = match Session::open(&target).await {
                        Ok(s) => s,
                        Err(e) => {
                            if e.code == ErrorCode::PermissionDenied {
                                // A wrong password is asked for or read again next time.
                                self.connections.forget(&connection.id);
                            }
                            return Err(e);
                        }
                    };
                    *tab.canceller.lock().expect("canceller lock") = Some(session.canceller());
                    *guard = Some(session);
                }
                let session = guard.as_mut().expect("a session was just opened");
                let had_transaction = session.transaction().is_some();
                match session
                    .run(sql, &request.parameters, cap, request.mode, request.explain)
                    .await
                {
                    Ok(r) => {
                        ran = Some(r);
                        break;
                    }
                    Err(_lost) => {
                        *guard = None;
                        reconnected = true;
                        // Only a read-only statement may run again on a new
                        // session: it cannot repeat a write.
                        if request.mode == RunMode::ReadOnly && !had_transaction {
                            continue;
                        }
                        let message = if had_transaction {
                            "The connection was lost. The server rolled the open transaction back."
                        } else {
                            "The connection was lost while the statement ran. It may or may not have been applied; check before running it again."
                        };
                        ran = Some(Ran {
                            result: StatementResult::Failed {
                                failure: plain_failure(DbFailureReason::Connection, message.into()),
                            },
                            ran_read_only: false,
                        });
                        break;
                    }
                }
            }
            let Ran {
                mut result,
                ran_read_only,
            } = ran.unwrap_or_else(|| Ran {
                result: StatementResult::Failed {
                    failure: plain_failure(
                        DbFailureReason::Connection,
                        "The connection was lost and could not be opened again.".into(),
                    ),
                },
                ran_read_only: true,
            });
            if let StatementResult::Failed { failure } = &mut result {
                // From a byte offset into the statement to a UTF-16 offset into the text.
                failure.position = failure
                    .position
                    .map(|p| byte_to_utf16(text, (range.start + p as usize).min(range.end)));
            }
            let elapsed = started.elapsed();
            let elapsed_ms = u32::try_from(elapsed.as_millis()).unwrap_or(u32::MAX);
            tracing::debug!(connection = %connection.id, elapsed_ms, "statement ran");
            // Explain Analyze runs the statement, so it is recorded; a plain Explain is not.
            if request.explain != Some(ExplainMode::Plan) && self.history_on.load(Ordering::SeqCst)
            {
                if let Some(history) = &self.history {
                    let recorded = Recorded {
                        connection_id: connection.id.clone(),
                        sql: sql.to_string(),
                        elapsed_ms,
                        rows: rows_of(&result),
                        error: match &result {
                            StatementResult::Failed { failure } => Some(failure.message.clone()),
                            _ => None,
                        },
                    };
                    if let Err(e) = history.record(recorded).await {
                        tracing::warn!(error = %e, "could not record a run in the history");
                    }
                }
            }
            *tab.last_used.lock().expect("last used lock") = Instant::now();
            let failed = matches!(result, StatementResult::Failed { .. });
            runs.push(StatementRun {
                from: byte_to_utf16(text, range.start),
                to: byte_to_utf16(text, range.end),
                sql: sql.to_string(),
                result,
                elapsed_ms,
                reconnected,
                ran_read_only,
                transaction: guard.as_ref().and_then(Session::transaction),
            });
            if failed {
                break;
            }
        }
        Ok(runs)
    }

    /// Commit or Roll Back a tab's open transaction.
    pub async fn end_transaction(&self, tab_id: &str, commit: bool) -> AppResult<()> {
        let tab = self.tabs.lock().expect("tabs lock").get(tab_id).cloned();
        let Some(tab) = tab else { return Ok(()) };
        let mut guard = tab.session.try_lock().map_err(|_| {
            AppError::new(
                ErrorCode::Conflict,
                "A statement is still running in this tab. Cancel it first.",
            )
        })?;
        match guard.as_mut() {
            Some(session) => {
                let ended = session.end_transaction(commit).await;
                tab.in_transaction
                    .store(session.transaction().is_some(), Ordering::SeqCst);
                *tab.last_used.lock().expect("last used lock") = Instant::now();
                ended
            }
            None => Ok(()),
        }
    }

    /// Tabs with a transaction open, asked about before quitting.
    pub fn open_transactions(&self) -> Vec<String> {
        self.tabs
            .lock()
            .expect("tabs lock")
            .iter()
            .filter(|(_, t)| t.in_transaction.load(Ordering::SeqCst))
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Export…: the statement again, read only, every row to a file.
    pub async fn export(&self, request: ExportRequest) -> AppResult<DbExportResult> {
        let connection = self.connections.get(&request.connection_id).await?;
        let sql = statements::without_semicolon(request.sql.trim()).to_string();
        if sql.is_empty() {
            return Err(AppError::validation("There is no statement to export."));
        }
        let tab = self.tab(&request.tab_id, &connection);
        let mut guard = tab.session.try_lock().map_err(|_| {
            AppError::new(
                ErrorCode::Conflict,
                "A statement is already running in this tab.",
            )
        })?;
        if guard.as_ref().is_some_and(|s| s.transaction().is_some()) {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "This tab has a transaction open. Commit or roll it back before exporting.",
            ));
        }
        if guard.as_ref().is_none_or(Session::is_closed) {
            self.make_room(&tab);
            let target = self.connections.target(&connection).await?;
            let session = Session::open(&target).await?;
            *tab.canceller.lock().expect("canceller lock") = Some(session.canceller());
            *guard = Some(session);
        }
        let path = std::path::PathBuf::from(&request.path);
        let sink = FileSink::create(&path, request.format)?;
        let session = guard.as_mut().expect("a session was just opened");
        let rows = session.export(&sql, &request.parameters, sink).await?;
        *tab.last_used.lock().expect("last used lock") = Instant::now();
        tracing::info!(connection = %connection.id, rows, "rows exported");
        Ok(DbExportResult {
            rows,
            path: request.path,
        })
    }

    /// Cancel the statement a tab is running, on the server; Run All stops too.
    pub async fn cancel(&self, tab_id: &str) {
        let tab = self.tabs.lock().expect("tabs lock").get(tab_id).cloned();
        let Some(tab) = tab else { return };
        tab.stop.store(true, Ordering::SeqCst);
        let canceller = tab.canceller.lock().expect("canceller lock").clone();
        if let Some(canceller) = canceller {
            canceller.cancel().await;
        }
    }

    /// Close a tab's session, when the tab closes.
    pub fn close(&self, tab_id: &str) {
        let tab = self.tabs.lock().expect("tabs lock").remove(tab_id);
        if let Some(tab) = tab {
            // A running statement keeps its session until it ends; then it is dropped with the tab.
            if let Ok(mut session) = tab.session.try_lock() {
                session.take();
            }
        }
    }

    /// Close every session of a connection, and forget its schema, when it is deleted.
    pub fn close_connection(&self, connection_id: &str) {
        let ids: Vec<String> = self
            .tabs
            .lock()
            .expect("tabs lock")
            .iter()
            .filter(|(_, t)| t.connection_id == connection_id)
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            self.close(&id);
        }
        self.schemas
            .lock()
            .expect("schemas lock")
            .remove(connection_id);
    }

    /// Close sessions unused for `idle`; called every minute.
    pub fn close_idle(&self, idle: Duration) {
        let tabs: Vec<Arc<TabSession>> = self
            .tabs
            .lock()
            .expect("tabs lock")
            .values()
            .cloned()
            .collect();
        for tab in tabs {
            if tab.last_used.lock().expect("last used lock").elapsed() < idle {
                continue;
            }
            if let Ok(mut session) = tab.session.try_lock() {
                // The server ends an idle transaction itself, after 15 minutes.
                if session.as_ref().is_some_and(|s| s.transaction().is_some()) {
                    continue;
                }
                if session.take().is_some() {
                    tab.had_session.store(true, Ordering::SeqCst);
                    tracing::debug!(connection = %tab.connection_id, "idle database session closed");
                }
            }
        }
    }

    /// How many sessions are open, for tests.
    pub fn open_sessions(&self) -> usize {
        self.tabs
            .lock()
            .expect("tabs lock")
            .values()
            .filter(|t| t.session.try_lock().map(|s| s.is_some()).unwrap_or(true))
            .count()
    }

    /// The connection's schema, read on first use and on Refresh Schema with
    /// a session of its own, never a tab's.
    pub async fn schema(&self, connection_id: &str, refresh: bool) -> AppResult<DbSchema> {
        let connection = self.connections.get(connection_id).await?;
        if !refresh {
            if let Some((version, schema)) = self
                .schemas
                .lock()
                .expect("schemas lock")
                .get(connection_id)
            {
                if *version == connection.version {
                    return Ok(schema.clone());
                }
            }
        }
        let target = self.connections.target(&connection).await?;
        let mut session = match Session::open(&target).await {
            Ok(s) => s,
            Err(e) => {
                if e.code == ErrorCode::PermissionDenied {
                    self.connections.forget(&connection.id);
                }
                return Err(e);
            }
        };
        let (schemas, default_schema) = session.schema().await?;
        let schema = DbSchema {
            connection_id: connection.id.clone(),
            schemas,
            default_schema,
            read_at: now_rfc3339(),
        };
        self.schemas
            .lock()
            .expect("schemas lock")
            .insert(connection.id.clone(), (connection.version, schema.clone()));
        Ok(schema)
    }
}

/// The statements a run covers, as byte ranges into the text.
fn chosen(request: &RunStatementRequest, dialect: crate::models::DbKind) -> Vec<Range<usize>> {
    let text = request.text.as_str();
    if request.all {
        return statements::split(text, dialect);
    }
    let from = utf16_to_byte(text, request.from.min(request.to));
    let to = utf16_to_byte(text, request.from.max(request.to));
    if from < to {
        // The selection's statements, placed in the whole text.
        return statements::split(&text[from..to], dialect)
            .into_iter()
            .map(|r| from + r.start..from + r.end)
            .collect();
    }
    let all = statements::split(text, dialect);
    statements::statement_at(&all, from).into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::DbKind;

    fn request(text: &str, from: u32, to: u32, all: bool) -> RunStatementRequest {
        RunStatementRequest {
            tab_id: "t".into(),
            connection_id: "c".into(),
            text: text.into(),
            from,
            to,
            all,
            fetch_all: false,
            mode: RunMode::ReadOnly,
            parameters: Vec::new(),
            explain: None,
        }
    }

    #[test]
    fn runs_cover_the_cursor_the_selection_or_everything() {
        let text = "select 1;\nselect 'é';\nselect 3;";
        let pick = |r: &RunStatementRequest| -> Vec<&str> {
            chosen(r, DbKind::Postgres)
                .into_iter()
                .map(|range| &text[range])
                .collect()
        };
        assert_eq!(pick(&request(text, 12, 12, false)), ["select 'é';"]);
        assert_eq!(
            pick(&request(text, 0, 20, false)),
            ["select 1;", "select 'é'"]
        );
        assert_eq!(pick(&request(text, 5, 5, true)).len(), 3);
        assert!(pick(&request("  -- nothing", 0, 0, false)).is_empty());
    }
}
