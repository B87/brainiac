//! The SQLite driver: `rusqlite` on a thread of its own per session, since
//! its calls block (docs/architecture.md, Databases — v0.4, Sessions). The
//! thread owns the connection and receives work through a channel, like the
//! app's own database worker, which it never shares.
//!
//! Read only is SQLite's own: a read-only connection opens the file read
//! only, so a write fails inside SQLite, and a read-and-write connection in a
//! read-only tab sets `query_only`. Two statements could still write a file
//! through a read-only connection, `ATTACH` and `VACUUM INTO`: the authorizer
//! refuses `ATTACH`, and in a read-only tab a statement that is not read only
//! is refused before it runs.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use rusqlite::types::{Value, ValueRef};
use rusqlite::{Connection, ErrorCode as SqliteCode, OpenFlags};

use super::driver::{manual_only, plain_failure, Lost, Ran, RESULT_BYTES};
use super::export::{ExportValue, FileSink, RowSink};
use super::statements::{self, Control};
use super::values::{bytes_cell, float_cell, integer_cell, text_cell, CUT_AT};
use crate::models::{
    now_rfc3339, AppError, AppResult, Cell, ColumnKind, DbColumn, DbFailure, DbFailureReason,
    DbForeignKey, DbIndex, DbRelation, DbSchemaGroup, ExplainMode, ParamValue, PlanNode,
    RelationKind, ResultColumn, RunMode, StatementResult, TransactionInfo,
};

type Job = Box<dyn FnOnce(&mut Connection) + Send + 'static>;

/// `PRAGMA application_id` of Brainiac's own files: the core, index,
/// history, and pull request databases. They open read only, always.
pub const BRAINIAC_FILES: [i32; 4] = [0x4252_4E43, 0x4252_4E49, 0x4252_4E48, 0x4252_4E46];

/// One connection to a file, owned by one tab.
pub struct SqliteSession {
    sender: mpsc::Sender<Job>,
    canceller: SqliteCanceller,
    deadline: Arc<Mutex<Option<Instant>>>,
    timeout: Duration,
    server_version: String,
    writable: bool,
    transaction: Option<TransactionInfo>,
}

/// Cancels the statement running on a session, from any thread.
#[derive(Clone)]
pub struct SqliteCanceller {
    interrupt: Arc<rusqlite::InterruptHandle>,
    cancelled: Arc<AtomicBool>,
}

impl SqliteCanceller {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        self.interrupt.interrupt();
    }
}

/// Whether the file is one of Brainiac's own databases.
pub fn is_brainiac_file(path: &Path) -> bool {
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .and_then(|c| c.query_row("PRAGMA application_id", [], |r| r.get::<_, i32>(0)))
        .is_ok_and(|id| BRAINIAC_FILES.contains(&id))
}

impl SqliteSession {
    /// Open the file, read only unless `writable`. A missing file is an
    /// error: Brainiac never creates one.
    pub async fn open(path: &Path, timeout: Duration, writable: bool) -> AppResult<Self> {
        if !path.is_file() {
            return Err(AppError::not_found(format!(
                "There is no file at {}.",
                path.display()
            )));
        }
        let writable = writable && !is_brainiac_file(path);
        let path: PathBuf = path.to_path_buf();
        let deadline: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));
        let cancelled = Arc::new(AtomicBool::new(false));
        let (opened_tx, opened_rx) = tokio::sync::oneshot::channel();
        let (sender, receiver) = mpsc::channel::<Job>();
        let watch_deadline = Arc::clone(&deadline);
        let watch_cancelled = Arc::clone(&cancelled);
        thread::Builder::new()
            .name("brainiac-sqlite-session".into())
            .spawn(move || {
                let mut conn = match open_file(&path, writable, watch_deadline, watch_cancelled) {
                    Ok(conn) => conn,
                    Err(e) => {
                        let _ = opened_tx.send(Err(e));
                        return;
                    }
                };
                let _ = opened_tx.send(Ok(conn.get_interrupt_handle()));
                for job in receiver {
                    job(&mut conn);
                }
                // Closing with a transaction open rolls it back.
            })
            .map_err(|e| {
                AppError::io("Could not start the SQLite session.").with_details(e.to_string())
            })?;
        let interrupt = opened_rx
            .await
            .map_err(|_| AppError::io("The SQLite session stopped while opening."))??;
        Ok(SqliteSession {
            sender,
            canceller: SqliteCanceller {
                interrupt: Arc::new(interrupt),
                cancelled,
            },
            deadline,
            timeout,
            server_version: format!("SQLite {}", rusqlite::version()),
            writable,
            transaction: None,
        })
    }

    pub fn server_version(&self) -> &str {
        &self.server_version
    }

    pub fn canceller(&self) -> SqliteCanceller {
        self.canceller.clone()
    }

    pub fn transaction(&self) -> Option<TransactionInfo> {
        self.transaction.clone()
    }

    async fn call<T, F>(&self, f: F) -> Result<T, Lost>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> T + Send + 'static,
    {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let job: Job = Box::new(move |conn| {
            let _ = tx.send(f(conn));
        });
        self.sender.send(job).map_err(|_| Lost)?;
        rx.await.map_err(|_| Lost)
    }

    /// Run one statement in `mode` and return at most `cap` rows. A failure
    /// carries its position as a byte offset into `sql`.
    pub async fn run(
        &mut self,
        sql: &str,
        params: &[ParamValue],
        cap: usize,
        mode: RunMode,
        explain: Option<ExplainMode>,
    ) -> Result<Ran, Lost> {
        self.canceller.cancelled.store(false, Ordering::SeqCst);
        *self.deadline.lock().expect("deadline lock") = Some(Instant::now() + self.timeout);
        let job = RunJob {
            sql: sql.to_string(),
            params: params.to_vec(),
            cap,
            mode: if self.writable {
                mode
            } else {
                RunMode::ReadOnly
            },
            explain,
            timeout: self.timeout,
            cancelled: Arc::clone(&self.canceller.cancelled),
            writable: self.writable,
        };
        let outcome = self.call(move |conn| job.run(conn)).await;
        *self.deadline.lock().expect("deadline lock") = None;
        let (ran, in_transaction) = outcome?;
        let succeeded = !matches!(ran.result, StatementResult::Failed { .. });
        self.transaction = match (in_transaction, self.transaction.take()) {
            (false, _) => None,
            (true, Some(mut tx)) => {
                if succeeded && statements::transaction_control(sql) == Control::None {
                    tx.statements += 1;
                }
                Some(tx)
            }
            (true, None) => Some(TransactionInfo {
                statements: u32::from(
                    succeeded && statements::transaction_control(sql) == Control::None,
                ),
                since: now_rfc3339(),
                failed: false,
            }),
        };
        Ok(ran)
    }

    /// Commit or Roll Back the tab's transaction.
    pub async fn end_transaction(&mut self, commit: bool) -> AppResult<()> {
        if self.transaction.is_none() {
            return Ok(());
        }
        let sql = if commit { "COMMIT" } else { "ROLLBACK" };
        let ended = self
            .call(move |conn| {
                if conn.is_autocommit() {
                    Ok(())
                } else {
                    conn.execute_batch(sql)
                }
            })
            .await
            .map_err(|_| AppError::io("The SQLite session stopped."))?;
        match ended {
            Ok(()) => {
                self.transaction = None;
                Ok(())
            }
            Err(e) => Err(AppError::db(e.to_string())),
        }
    }

    /// Run a statement read only and write every row to `sink`, which is
    /// completed or removed: Export.
    pub async fn export(
        &mut self,
        sql: &str,
        params: &[ParamValue],
        mut sink: FileSink,
    ) -> AppResult<u64> {
        let sql = sql.to_string();
        let params = params.to_vec();
        let partial = sink.partial_path();
        *self.deadline.lock().expect("deadline lock") = None;
        // A Cancel of an earlier statement must not stop this export.
        self.canceller.cancelled.store(false, Ordering::SeqCst);
        let done = self
            .call(
                move |conn| match export_rows(conn, &sql, &params, &mut sink) {
                    Ok(count) => sink.finish().map(|()| count),
                    Err(e) => {
                        sink.abandon();
                        Err(e)
                    }
                },
            )
            .await;
        match done {
            Ok(result) => result,
            Err(Lost) => {
                let _ = std::fs::remove_file(partial);
                Err(AppError::io(
                    "The SQLite session stopped during the export.",
                ))
            }
        }
    }

    pub async fn schema(&mut self) -> AppResult<(Vec<DbSchemaGroup>, Option<String>)> {
        let schema = self
            .call(read_schema)
            .await
            .map_err(|_| AppError::io("The SQLite session stopped."))??;
        Ok((schema, Some("main".to_string())))
    }
}

fn open_file(
    path: &Path,
    writable: bool,
    deadline: Arc<Mutex<Option<Instant>>>,
    cancelled: Arc<AtomicBool>,
) -> AppResult<Connection> {
    let access = if writable {
        OpenFlags::SQLITE_OPEN_READ_WRITE
    } else {
        OpenFlags::SQLITE_OPEN_READ_ONLY
    };
    let conn = Connection::open_with_flags(path, access | OpenFlags::SQLITE_OPEN_NO_MUTEX)
        .map_err(|e| open_error(path, e))?;
    conn.busy_timeout(Duration::from_secs(5))?;
    conn.execute_batch("PRAGMA query_only = ON;")?;
    conn.authorizer(Some(|context: AuthContext<'_>| match context.action {
        AuthAction::Attach { .. } => Authorization::Deny,
        _ => Authorization::Allow,
    }))?;
    // Every 1,000 steps: stop a statement past its time limit or cancelled.
    conn.progress_handler(
        1000,
        Some(move || {
            cancelled.load(Ordering::SeqCst)
                || deadline
                    .lock()
                    .map(|d| d.is_some_and(|d| Instant::now() > d))
                    .unwrap_or(false)
        }),
    )?;
    // Reading the schema here turns a file that is not a database into an error now.
    conn.query_row("SELECT count(*) FROM sqlite_schema", [], |r| {
        r.get::<_, i64>(0)
    })
    .map_err(|e| open_error(path, e))?;
    Ok(conn)
}

fn open_error(path: &Path, e: rusqlite::Error) -> AppError {
    let message = match e.sqlite_error_code() {
        Some(SqliteCode::NotADatabase) => format!("{} is not a SQLite database.", path.display()),
        Some(SqliteCode::CannotOpen) | Some(SqliteCode::PermissionDenied) => {
            format!("{} could not be opened.", path.display())
        }
        Some(SqliteCode::DatabaseBusy) | Some(SqliteCode::DatabaseLocked) => {
            format!("{} is locked by another program.", path.display())
        }
        _ => format!("{} could not be read.", path.display()),
    };
    AppError::io(message).with_details(e.to_string())
}

/// A statement to run on the session's thread.
struct RunJob {
    sql: String,
    params: Vec<ParamValue>,
    cap: usize,
    mode: RunMode,
    explain: Option<ExplainMode>,
    timeout: Duration,
    cancelled: Arc<AtomicBool>,
    writable: bool,
}

const PLAN_PREFIX: &str = "EXPLAIN QUERY PLAN ";

impl RunJob {
    /// What the statement did, and whether a transaction is open after it.
    fn run(self, conn: &mut Connection) -> (Ran, bool) {
        let ran = self.run_inner(conn);
        if self.mode != RunMode::Manual && !conn.is_autocommit() {
            // Outside Manual nothing may stay open: a `SAVEPOINT` starts a
            // transaction whose lock would block other programs' writers.
            let _ = conn.execute_batch("ROLLBACK");
        }
        (ran, !conn.is_autocommit())
    }

    fn failed(&self, e: rusqlite::Error) -> Ran {
        Ran {
            result: StatementResult::Failed {
                failure: failure(e, self.timeout, &self.cancelled),
            },
            ran_read_only: self.mode == RunMode::ReadOnly,
        }
    }

    fn run_inner(&self, conn: &mut Connection) -> Ran {
        let query_only = self.mode == RunMode::ReadOnly || !self.writable;
        if let Err(e) = conn.pragma_update(None, "query_only", query_only) {
            return self.failed(e);
        }
        let control = statements::transaction_control(&self.sql);
        if self.mode != RunMode::Manual && control != Control::None {
            return Ran {
                result: StatementResult::Failed {
                    failure: manual_only(),
                },
                ran_read_only: true,
            };
        }
        // A plain Explain never runs the statement, so it opens no transaction.
        if self.mode == RunMode::Manual
            && conn.is_autocommit()
            && control != Control::Begin
            && self.explain != Some(ExplainMode::Plan)
        {
            if let Err(e) = conn.execute_batch("BEGIN") {
                return self.failed(e);
            }
        }
        if self.explain.is_some() {
            return self.explain(conn);
        }
        let mut statement = match conn.prepare(&self.sql) {
            Ok(s) => s,
            Err(e) => return self.failed(e),
        };
        let ran_read_only = statement.readonly();
        if query_only && !ran_read_only {
            return Ran {
                result: StatementResult::Failed {
                    failure: DbFailure {
                        code: Some("SQLITE_READONLY".into()),
                        ..plain_failure(
                            DbFailureReason::ReadOnly,
                            "This connection is read only: the statement would write.".into(),
                        )
                    },
                },
                ran_read_only: true,
            };
        }
        if let Err(failure) = bind(&mut statement, &self.params) {
            return Ran {
                result: StatementResult::Failed { failure },
                ran_read_only,
            };
        }
        let result = rows_of(&mut statement, &self.sql, self.cap).unwrap_or_else(|e| {
            StatementResult::Failed {
                failure: failure(e, self.timeout, &self.cancelled),
            }
        });
        Ran {
            result,
            ran_read_only,
        }
    }

    /// `EXPLAIN QUERY PLAN` as a tree; Analyze also runs the statement and times it.
    fn explain(&self, conn: &mut Connection) -> Ran {
        let plan_sql = format!("{PLAN_PREFIX}{}", self.sql);
        let steps: Result<Vec<(i64, i64, String)>, rusqlite::Error> = (|| {
            let mut statement = conn.prepare(&plan_sql)?;
            bind(&mut statement, &self.params).map_err(|f| {
                rusqlite::Error::SqliteFailure(
                    rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_RANGE),
                    Some(f.message),
                )
            })?;
            let mut rows = statement.raw_query();
            let mut steps = Vec::new();
            while let Some(row) = rows.next()? {
                steps.push((row.get(0)?, row.get(1)?, row.get(3)?));
            }
            Ok(steps)
        })();
        let steps = match steps {
            Ok(s) => s,
            Err(e) => {
                let mut ran = self.failed(e);
                if let StatementResult::Failed { failure } = &mut ran.result {
                    failure.position = failure
                        .position
                        .map(|p| p.saturating_sub(PLAN_PREFIX.len() as u32));
                }
                return ran;
            }
        };
        let plan = plan_tree(&steps);
        let mut execution_ms = None;
        let mut ran_read_only = true;
        if self.explain == Some(ExplainMode::Analyze) {
            let started = Instant::now();
            let measured = RunJob {
                explain: None,
                sql: self.sql.clone(),
                params: self.params.clone(),
                cap: usize::MAX,
                mode: self.mode,
                timeout: self.timeout,
                cancelled: Arc::clone(&self.cancelled),
                writable: self.writable,
            }
            .run_inner(conn);
            if matches!(measured.result, StatementResult::Failed { .. }) {
                return measured;
            }
            ran_read_only = measured.ran_read_only;
            execution_ms = Some(started.elapsed().as_secs_f64() * 1000.0);
        }
        Ran {
            result: StatementResult::Plan {
                plan,
                planning_ms: None,
                execution_ms,
            },
            ran_read_only,
        }
    }
}

/// SQLite's plan rows (id, parent, detail) as a tree under one root.
fn plan_tree(steps: &[(i64, i64, String)]) -> PlanNode {
    fn children(steps: &[(i64, i64, String)], parent: i64) -> Vec<PlanNode> {
        steps
            .iter()
            .filter(|(_, p, _)| *p == parent)
            .map(|(id, _, detail)| PlanNode {
                label: detail.clone(),
                startup_cost: None,
                total_cost: None,
                rows: None,
                actual_rows: None,
                actual_ms: None,
                loops: None,
                details: Vec::new(),
                children: children(steps, *id),
            })
            .collect()
    }
    PlanNode {
        label: "QUERY PLAN".into(),
        startup_cost: None,
        total_cost: None,
        rows: None,
        actual_rows: None,
        actual_ms: None,
        loops: None,
        details: Vec::new(),
        children: children(steps, 0),
    }
}

/// A typed value for a parameter typed as text: integers and reals as
/// numbers, so `LIMIT :n` and comparisons with numeric columns work.
fn parameter_value(text: &Option<String>) -> Value {
    match text {
        None => Value::Null,
        Some(t) => {
            if let Ok(i) = t.trim().parse::<i64>() {
                Value::Integer(i)
            } else if let Ok(f) = t.trim().parse::<f64>().map_err(|_| ()).and_then(|f| {
                if f.is_finite() && t.trim().contains(['.', 'e', 'E']) {
                    Ok(f)
                } else {
                    Err(())
                }
            }) {
                Value::Real(f)
            } else {
                Value::Text(t.clone())
            }
        }
    }
}

fn bind(statement: &mut rusqlite::Statement<'_>, params: &[ParamValue]) -> Result<(), DbFailure> {
    let mut missing = Vec::new();
    for index in 1..=statement.parameter_count() {
        let name = statement.parameter_name(index).map(str::to_string);
        let Some(name) = name.as_deref().and_then(|n| n.strip_prefix(':')) else {
            return Err(plain_failure(
                DbFailureReason::Parameters,
                "Name each parameter, as in :name, to be asked for its value.".into(),
            ));
        };
        match params.iter().find(|p| p.name == name) {
            Some(p) => statement
                .raw_bind_parameter(index, parameter_value(&p.value))
                .map_err(|e| plain_failure(DbFailureReason::Parameters, e.to_string()))?,
            None => missing.push(format!(":{name}")),
        }
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(plain_failure(
            DbFailureReason::Parameters,
            format!("Enter a value for {}.", missing.join(", ")),
        ))
    }
}

/// Step a bound statement, keeping at most `cap` rows.
fn rows_of(
    statement: &mut rusqlite::Statement<'_>,
    sql: &str,
    cap: usize,
) -> Result<StatementResult, rusqlite::Error> {
    let count = statement.column_count();
    if count == 0 {
        let changed = statement.raw_execute()?;
        return Ok(StatementResult::Command {
            tag: super::postgres::command_tag(sql, changed as u64),
        });
    }
    let mut columns: Vec<ResultColumn> = statement
        .columns()
        .iter()
        .map(|c| {
            let declared = c.decl_type().unwrap_or("").to_string();
            ResultColumn {
                name: c.name().to_string(),
                kind: declared_kind(&declared),
                type_name: declared,
            }
        })
        .collect();
    let mut rows: Vec<Vec<Cell>> = Vec::new();
    let mut bytes = 0;
    let mut more = false;
    let mut raw = statement.raw_query();
    while let Some(row) = raw.next()? {
        if rows.len() == cap || bytes > RESULT_BYTES {
            more = true;
            break;
        }
        let mut cells = Vec::with_capacity(count);
        for (i, column) in columns.iter_mut().enumerate() {
            let value = row.get_ref(i)?;
            bytes += 16
                + match value {
                    ValueRef::Text(b) | ValueRef::Blob(b) => b.len().min(CUT_AT),
                    _ => 0,
                };
            if column.type_name.is_empty() && column.kind == ColumnKind::Other {
                // An expression has no declared type: the first value decides.
                if let Some(kind) = value_kind(&value) {
                    column.kind = kind;
                }
            }
            cells.push(cell(value));
        }
        rows.push(cells);
    }
    for column in &mut columns {
        if column.kind == ColumnKind::Other && column.type_name.is_empty() {
            column.kind = ColumnKind::Text;
        }
    }
    Ok(StatementResult::Rows {
        columns,
        rows,
        more,
    })
}

fn export_rows(
    conn: &mut Connection,
    sql: &str,
    params: &[ParamValue],
    sink: &mut FileSink,
) -> AppResult<u64> {
    conn.pragma_update(None, "query_only", true)?;
    let mut statement = conn.prepare(sql).map_err(|e| AppError::db(e.to_string()))?;
    if !statement.readonly() {
        return Err(AppError::validation(
            "Export runs the statement again read only, and this statement writes.",
        ));
    }
    bind(&mut statement, params).map_err(|f| AppError::validation(f.message))?;
    let names: Vec<String> = statement
        .column_names()
        .into_iter()
        .map(str::to_string)
        .collect();
    if names.is_empty() {
        return Err(AppError::validation(
            "The statement returns no rows to export.",
        ));
    }
    sink.columns(&names)?;
    let mut rows = statement.raw_query();
    let mut count = 0;
    while let Some(row) = rows.next().map_err(|e| AppError::db(e.to_string()))? {
        let values: Vec<ExportValue> = (0..names.len())
            .map(|i| match row.get_ref(i) {
                Ok(ValueRef::Null) | Err(_) => ExportValue::Null,
                Ok(ValueRef::Integer(v)) => ExportValue::Number(v.to_string()),
                Ok(ValueRef::Real(v)) => ExportValue::Number(v.to_string()),
                Ok(ValueRef::Text(t)) => ExportValue::Text(String::from_utf8_lossy(t).into_owned()),
                Ok(ValueRef::Blob(b)) => ExportValue::Text(format!(
                    "\\x{}",
                    b.iter().map(|x| format!("{x:02x}")).collect::<String>()
                )),
            })
            .collect();
        sink.row(&values)?;
        count += 1;
    }
    Ok(count)
}

fn cell(value: ValueRef<'_>) -> Cell {
    match value {
        ValueRef::Null => Cell::Null,
        ValueRef::Integer(v) => integer_cell(v),
        ValueRef::Real(v) => float_cell(v),
        ValueRef::Text(t) => text_cell(String::from_utf8_lossy(t).into_owned()),
        ValueRef::Blob(b) => bytes_cell(b),
    }
}

fn value_kind(value: &ValueRef<'_>) -> Option<ColumnKind> {
    match value {
        ValueRef::Null => None,
        ValueRef::Integer(_) | ValueRef::Real(_) => Some(ColumnKind::Number),
        ValueRef::Text(_) => Some(ColumnKind::Text),
        ValueRef::Blob(_) => Some(ColumnKind::Bytes),
    }
}

/// A column's kind from its declared type, by SQLite's affinity rules plus
/// the names people use for dates and JSON. No declared type is `Other`
/// until a value says what it is.
pub fn declared_kind(declared: &str) -> ColumnKind {
    let t = declared.to_ascii_uppercase();
    if t.is_empty() {
        ColumnKind::Other
    } else if t.contains("BOOL") {
        ColumnKind::Bool
    } else if t.contains("INT") {
        ColumnKind::Number
    } else if t.contains("JSON") {
        ColumnKind::Json
    } else if t.contains("DATE") || t.contains("TIME") {
        ColumnKind::Temporal
    } else if t.contains("UUID") {
        ColumnKind::Uuid
    } else if t.contains("CHAR") || t.contains("CLOB") || t.contains("TEXT") {
        ColumnKind::Text
    } else if t.contains("BLOB") {
        ColumnKind::Bytes
    } else if t.contains("REAL") || t.contains("FLOA") || t.contains("DOUB") {
        ColumnKind::Number
    } else if t.contains("NUMERIC") || t.contains("DECIMAL") {
        ColumnKind::Numeric
    } else {
        ColumnKind::Text
    }
}

fn failure(e: rusqlite::Error, timeout: Duration, cancelled: &AtomicBool) -> DbFailure {
    let (message, position) = match &e {
        rusqlite::Error::SqlInputError { msg, offset, .. } => {
            (msg.clone(), (*offset >= 0).then_some(*offset as u32))
        }
        rusqlite::Error::SqliteFailure(_, Some(msg)) => (msg.clone(), None),
        other => (other.to_string(), None),
    };
    let code = e.sqlite_error_code();
    let reason = match code {
        Some(SqliteCode::ReadOnly) => DbFailureReason::ReadOnly,
        Some(SqliteCode::AuthorizationForStatementDenied) => DbFailureReason::ReadOnly,
        Some(SqliteCode::OperationInterrupted) => {
            if cancelled.load(Ordering::SeqCst) {
                DbFailureReason::Cancelled
            } else {
                DbFailureReason::Timeout
            }
        }
        _ => DbFailureReason::Sql,
    };
    let message = match reason {
        DbFailureReason::ReadOnly if code == Some(SqliteCode::AuthorizationForStatementDenied) => {
            "This connection is read only: ATTACH could write another file, so Brainiac does not run it.".to_string()
        }
        DbFailureReason::ReadOnly => format!("This connection is read only: {message}."),
        DbFailureReason::Timeout => format!(
            "The statement ran longer than the {} second limit and was stopped.",
            timeout.as_secs()
        ),
        DbFailureReason::Cancelled => "Cancelled.".to_string(),
        _ => message,
    };
    DbFailure {
        reason,
        message,
        code: code.map(|c| format!("{c:?}")),
        detail: None,
        hint: None,
        position,
    }
}

fn read_schema(conn: &mut Connection) -> AppResult<Vec<DbSchemaGroup>> {
    let mut relations = Vec::new();
    let names: Vec<(String, String)> = conn
        .prepare(
            "SELECT name, type FROM sqlite_schema
              WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\'
              ORDER BY name",
        )?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    for (name, kind) in names {
        let columns = conn
            .prepare("SELECT name, type, \"notnull\", dflt_value, pk FROM pragma_table_info(?1)")?
            .query_map([&name], |r| {
                Ok(DbColumn {
                    name: r.get(0)?,
                    type_name: r.get(1)?,
                    nullable: r.get::<_, i64>(2)? == 0,
                    default: r.get(3)?,
                    primary_key: r.get::<_, i64>(4)? > 0,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let index_rows: Vec<(String, bool, String)> = conn
            .prepare("SELECT name, \"unique\", origin FROM pragma_index_list(?1) ORDER BY name")?
            .query_map([&name], |r| {
                Ok((r.get(0)?, r.get::<_, i64>(1)? != 0, r.get(2)?))
            })?
            .collect::<Result<_, _>>()?;
        let mut indexes = Vec::new();
        for (index, unique, origin) in index_rows {
            let columns: Vec<String> = conn
                .prepare("SELECT coalesce(name, '<expression>') FROM pragma_index_info(?1) ORDER BY seqno")?
                .query_map([&index], |r| r.get(0))?
                .collect::<Result<_, _>>()?;
            indexes.push(DbIndex {
                definition: format!("({})", columns.join(", ")),
                name: index,
                unique,
                primary: origin == "pk",
            });
        }
        let key_rows: Vec<(i64, String, String, Option<String>)> = conn
            .prepare("SELECT id, \"table\", \"from\", \"to\" FROM pragma_foreign_key_list(?1) ORDER BY id, seq")?
            .query_map([&name], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
            .collect::<Result<_, _>>()?;
        let mut foreign_keys: Vec<DbForeignKey> = Vec::new();
        let mut current: Option<(i64, String, Vec<String>, Vec<String>)> = None;
        let mut flush = |c: Option<(i64, String, Vec<String>, Vec<String>)>| {
            if let Some((id, table, from, to)) = c {
                foreign_keys.push(DbForeignKey {
                    name: format!("fk_{id}"),
                    definition: format!(
                        "FOREIGN KEY ({}) REFERENCES {table}({})",
                        from.join(", "),
                        to.join(", ")
                    ),
                });
            }
        };
        for (id, table, from, to) in key_rows {
            match &mut current {
                Some((cid, _, f, t)) if *cid == id => {
                    f.push(from);
                    t.push(to.unwrap_or_default());
                }
                _ => {
                    flush(current.take());
                    current = Some((id, table, vec![from], vec![to.unwrap_or_default()]));
                }
            }
        }
        flush(current.take());
        relations.push(DbRelation {
            name,
            kind: if kind == "view" {
                RelationKind::View
            } else {
                RelationKind::Table
            },
            estimated_rows: None,
            columns,
            indexes,
            foreign_keys,
        });
    }
    Ok(vec![DbSchemaGroup {
        name: "main".to_string(),
        relations,
    }])
}
