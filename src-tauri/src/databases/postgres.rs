//! The PostgreSQL driver: `tokio-postgres` over the extended protocol, with
//! rustls for TLS (docs/architecture.md, Databases — v0.4, The driver).
//!
//! Every statement runs alone in its own `BEGIN READ ONLY` … `COMMIT`, so a
//! statement cannot write: one statement cannot both make its transaction
//! read-write and write in it. Rows come through a portal with a row limit,
//! so a large result stops at the cap without rewriting the SQL, and the
//! transaction ends before the result is shown, so nothing on screen holds a
//! lock.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, SignatureScheme};
use tokio_postgres::config::SslMode;
use tokio_postgres::error::{ErrorPosition, SqlState};
use tokio_postgres::types::{to_sql_checked, Format, FromSql, IsNull, ToSql, Type};
use tokio_postgres::{CancelToken, Client, NoTls, Row, Statement};
use tokio_postgres_rustls::MakeRustlsConnect;

use super::driver::{plain_failure, Lost, Ran};
use super::export::{ExportValue, RowSink};
use super::statements;
use super::values::{pg_cell, pg_export_value, pg_kind};
use crate::credentials::Secret;
use crate::models::{
    now_rfc3339, AppError, AppResult, DbColumn, DbFailure, DbFailureReason, DbForeignKey, DbIndex,
    DbRelation, DbSchemaGroup, DbTls, ErrorCode, ExplainMode, ParamValue, PlanNode, RelationKind,
    ResultColumn, RunMode, StatementResult, TransactionInfo,
};

/// How long to wait for a server to answer a connection.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// The server ends a transaction left idle this long (SPEC.md, Databases: Safety).
const IDLE_IN_TRANSACTION: Duration = Duration::from_secs(15 * 60);
/// Rows fetched at a time when exporting.
const EXPORT_BATCH: usize = 1000;
/// How long past the statement timeout to wait before giving up on a server
/// that does not answer at all, such as one behind a dropped network.
const UNRESPONSIVE_GRACE: Duration = Duration::from_secs(10);

/// Where and how to connect.
#[derive(Debug, Clone)]
pub struct PgTarget {
    pub host: String,
    pub port: u16,
    pub database: String,
    pub user: String,
    pub password: Option<Secret>,
    pub tls: DbTls,
    pub ca_file: Option<PathBuf>,
    pub statement_timeout: Duration,
    /// Shown in `pg_stat_activity`: `Brainiac`, or `Brainiac health`.
    pub application_name: String,
}

impl PgTarget {
    fn place(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

/// One connection, owned by one tab.
pub struct PgSession {
    client: Client,
    cancel: PgCanceller,
    timeout: Duration,
    server_version: String,
    /// The connection stopped answering; the session must not be used again.
    broken: bool,
    /// The session's `default_transaction_read_only`.
    read_only_default: bool,
    /// The tab's open transaction, in Manual mode.
    transaction: Option<TransactionInfo>,
}

/// Cancels the statement running on a session, from any task.
#[derive(Clone)]
pub struct PgCanceller {
    token: CancelToken,
    tls: Option<MakeRustlsConnect>,
    /// Set by `cancel`, so a statement cancelled by the user is told apart
    /// from one stopped by the statement timeout (both are SQLSTATE 57014).
    cancelled: Arc<AtomicBool>,
}

impl PgCanceller {
    pub async fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        let sent = match &self.tls {
            Some(tls) => self.token.cancel_query(tls.clone()).await,
            None => self.token.cancel_query(NoTls).await,
        };
        if let Err(e) = sent {
            tracing::warn!(error = %e, "could not send the cancel request");
        }
    }
}

/// A raw value in PostgreSQL's binary format, whatever its type.
struct Raw<'a>(Option<&'a [u8]>);

impl<'a> FromSql<'a> for Raw<'a> {
    fn from_sql(_: &Type, raw: &'a [u8]) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        Ok(Raw(Some(raw)))
    }

    fn from_sql_null(_: &Type) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        Ok(Raw(None))
    }

    fn accepts(_: &Type) -> bool {
        true
    }
}

impl PgSession {
    /// Connect, and make the session read only with the statement timeout.
    pub async fn connect(target: &PgTarget) -> AppResult<Self> {
        let mut config = tokio_postgres::Config::new();
        config
            .host(&target.host)
            .port(target.port)
            .dbname(&target.database)
            .user(&target.user)
            .application_name(&target.application_name)
            .connect_timeout(CONNECT_TIMEOUT);
        if let Some(password) = &target.password {
            config.password(password.expose());
        }
        let tls = match target.tls {
            DbTls::Off => {
                config.ssl_mode(SslMode::Disable);
                None
            }
            DbTls::Verify | DbTls::Require => {
                config.ssl_mode(SslMode::Require);
                Some(MakeRustlsConnect::new(tls_config(
                    target.tls,
                    target.ca_file.as_deref(),
                )?))
            }
        };
        let connecting = async {
            match &tls {
                Some(tls) => {
                    let (client, connection) = config.connect(tls.clone()).await?;
                    tokio::spawn(async move {
                        if let Err(e) = connection.await {
                            tracing::debug!(error = %e, "a database connection ended");
                        }
                    });
                    Ok::<_, tokio_postgres::Error>(client)
                }
                None => {
                    let (client, connection) = config.connect(NoTls).await?;
                    tokio::spawn(async move {
                        if let Err(e) = connection.await {
                            tracing::debug!(error = %e, "a database connection ended");
                        }
                    });
                    Ok(client)
                }
            }
        };
        let client = match tokio::time::timeout(CONNECT_TIMEOUT * 2, connecting).await {
            Ok(Ok(client)) => client,
            Ok(Err(e)) => return Err(connect_error(&e, target)),
            Err(_) => {
                return Err(AppError::dependency(format!(
                    "{} did not answer within {} seconds.",
                    target.place(),
                    (CONNECT_TIMEOUT * 2).as_secs()
                )))
            }
        };
        let timeout_ms = target.statement_timeout.as_millis().max(1);
        client
            .batch_execute(&format!(
                "SET default_transaction_read_only = on; SET statement_timeout = {timeout_ms}"
            ))
            .await
            .map_err(|e| connect_error(&e, target))?;
        let server_version = client
            .query_one("SHOW server_version", &[])
            .await
            .and_then(|row| row.try_get::<_, String>(0))
            .map_err(|e| connect_error(&e, target))?;
        let cancel = PgCanceller {
            token: client.cancel_token(),
            tls,
            cancelled: Arc::new(AtomicBool::new(false)),
        };
        Ok(PgSession {
            client,
            cancel,
            timeout: target.statement_timeout,
            server_version: format!("PostgreSQL {server_version}"),
            broken: false,
            read_only_default: true,
            transaction: None,
        })
    }

    pub fn server_version(&self) -> &str {
        &self.server_version
    }

    pub fn canceller(&self) -> PgCanceller {
        self.cancel.clone()
    }

    pub fn is_closed(&self) -> bool {
        self.broken || self.client.is_closed()
    }

    /// The connection itself, for Health's catalog queries.
    pub(crate) fn client(&self) -> &Client {
        &self.client
    }

    /// The open transaction, in Manual mode.
    pub fn transaction(&self) -> Option<TransactionInfo> {
        self.transaction.clone()
    }

    /// Run one statement in `mode` and return at most `cap` rows. `:name`
    /// parameters take their values from `params`; a failure carries its
    /// position as a byte offset into `sql`.
    pub async fn run(
        &mut self,
        sql: &str,
        params: &[ParamValue],
        cap: usize,
        mode: RunMode,
        explain: Option<ExplainMode>,
    ) -> Result<Ran, Lost> {
        self.cancel.cancelled.store(false, Ordering::SeqCst);
        let rewrite = statements::numbered(sql);
        let values = match bind_values(&rewrite.names, params) {
            Ok(v) => v,
            Err(failure) => {
                return Ok(Ran {
                    result: StatementResult::Failed { failure },
                    ran_read_only: true,
                })
            }
        };
        let prefix = match explain {
            None => "",
            Some(ExplainMode::Plan) => "EXPLAIN (FORMAT JSON) ",
            Some(ExplainMode::Analyze) => "EXPLAIN (ANALYZE, FORMAT JSON) ",
        };
        let text = format!("{prefix}{}", rewrite.sql);
        // A plain Explain never runs the statement, so it is read only whatever the mode.
        let mode = if explain == Some(ExplainMode::Plan) && self.transaction.is_none() {
            RunMode::ReadOnly
        } else {
            mode
        };
        let control = statements::transaction_control(sql);
        let limit = self.timeout + UNRESPONSIVE_GRACE;
        let ran =
            tokio::time::timeout(limit, self.dispatch(&text, &values, cap, mode, control)).await;
        match ran {
            Ok(Ok((fetched, ran_read_only))) => Ok(Ran {
                result: match explain {
                    Some(_) => plan_result(&fetched),
                    None => fetched.into_result(sql, cap),
                },
                ran_read_only,
            }),
            Ok(Err(e)) => {
                if e.is_closed() || self.client.is_closed() {
                    self.transaction = None;
                    return Err(Lost);
                }
                let position = |p: u32| {
                    let at = statements::char_position_to_byte(&text, p);
                    rewrite.original(at.saturating_sub(prefix.len())) as u32
                };
                Ok(Ran {
                    result: StatementResult::Failed {
                        failure: self.failure(&e, position),
                    },
                    ran_read_only: mode == RunMode::ReadOnly,
                })
            }
            Err(_) => {
                // The server stopped answering: this connection cannot be trusted again.
                self.broken = true;
                self.transaction = None;
                Ok(Ran {
                    result: StatementResult::Failed {
                        failure: plain_failure(
                            DbFailureReason::Timeout,
                            format!(
                                "The server did not answer within {} seconds. The session was closed; the next run reconnects.",
                                limit.as_secs()
                            ),
                        ),
                    },
                    ran_read_only: mode == RunMode::ReadOnly,
                })
            }
        }
    }

    /// Make new transactions read only or not, as the tab's mode needs.
    async fn read_only_default(&mut self, on: bool) -> Result<(), tokio_postgres::Error> {
        if self.read_only_default == on {
            return Ok(());
        }
        let sql = if on {
            "SET default_transaction_read_only = on".to_string()
        } else {
            // An open transaction holds locks others wait on; the server ends
            // one left idle this long, as a last resort.
            format!(
                "SET default_transaction_read_only = off; SET idle_in_transaction_session_timeout = {}",
                IDLE_IN_TRANSACTION.as_millis()
            )
        };
        self.client.batch_execute(&sql).await?;
        self.read_only_default = on;
        Ok(())
    }

    async fn dispatch(
        &mut self,
        text: &str,
        values: &[TextParam],
        cap: usize,
        mode: RunMode,
        control: statements::Control,
    ) -> Result<(Fetched, bool), tokio_postgres::Error> {
        use statements::Control;
        match mode {
            RunMode::ReadOnly => {
                self.read_only_default(true).await?;
                Ok((
                    fetch_read_only(&mut self.client, text, values, cap).await?,
                    true,
                ))
            }
            RunMode::AutoCommit => {
                self.read_only_default(false).await?;
                // Read only first: a statement that reads is then known to be
                // safe to run again (Fetch All, Export). One that writes is
                // refused before it changes anything, and runs again writable.
                match fetch_read_only(&mut self.client, text, values, cap).await {
                    Ok(fetched) => Ok((fetched, true)),
                    Err(e) if needs_writable(&e) => {
                        Ok((fetch_plain(&self.client, text, values, cap).await?, false))
                    }
                    Err(e) => Err(e),
                }
            }
            RunMode::Manual => {
                self.read_only_default(false).await?;
                if self.transaction.is_none() && control != Control::Begin {
                    self.client.batch_execute("BEGIN").await?;
                    self.transaction = Some(TransactionInfo {
                        statements: 0,
                        since: now_rfc3339(),
                        failed: false,
                    });
                }
                let fetched = fetch_plain(&self.client, text, values, cap).await;
                match (&fetched, control) {
                    (Ok(_), Control::Begin) => {
                        if self.transaction.is_none() {
                            self.transaction = Some(TransactionInfo {
                                statements: 0,
                                since: now_rfc3339(),
                                failed: false,
                            });
                        }
                    }
                    (Ok(_), Control::End) => self.transaction = None,
                    (Ok(_), Control::RollbackTo) => {
                        if let Some(tx) = &mut self.transaction {
                            tx.failed = false;
                            tx.statements += 1;
                        }
                    }
                    (Ok(_), Control::None) => {
                        if let Some(tx) = &mut self.transaction {
                            tx.statements += 1;
                        }
                    }
                    (Err(_), _) => {
                        if let Some(tx) = &mut self.transaction {
                            tx.failed = true;
                        }
                    }
                }
                Ok((fetched?, false))
            }
        }
    }

    /// Commit or Roll Back the tab's transaction. A failed transaction
    /// rolls back whichever is asked.
    pub async fn end_transaction(&mut self, commit: bool) -> AppResult<()> {
        if self.transaction.is_none() {
            return Ok(());
        }
        let sql = if commit { "COMMIT" } else { "ROLLBACK" };
        let ended = self.client.batch_execute(sql).await;
        self.transaction = None;
        ended.map_err(|e| match e.as_db_error() {
            Some(db) => AppError::db(db.message().to_string()),
            None => AppError::dependency(
                "The connection was lost; the server rolled the transaction back.",
            )
            .with_details(e.to_string()),
        })
    }

    /// Run a statement again read only and hand every row to `sink`, in
    /// batches, without the cap: Export.
    pub async fn export(
        &mut self,
        sql: &str,
        params: &[ParamValue],
        sink: &mut (dyn RowSink + Send),
    ) -> AppResult<u64> {
        let rewrite = statements::numbered(sql);
        let values =
            bind_values(&rewrite.names, params).map_err(|f| AppError::validation(f.message))?;
        self.read_only_default(true)
            .await
            .map_err(|e| export_error(&e))?;
        let refs: Vec<&(dyn ToSql + Sync)> =
            values.iter().map(|v| v as &(dyn ToSql + Sync)).collect();
        let tx = self
            .client
            .build_transaction()
            .read_only(true)
            .start()
            .await
            .map_err(|e| export_error(&e))?;
        let statement = tx
            .prepare(&rewrite.sql)
            .await
            .map_err(|e| export_error(&e))?;
        let columns: Vec<(String, Type)> = statement
            .columns()
            .iter()
            .map(|c| (c.name().to_string(), c.type_().clone()))
            .collect();
        if columns.is_empty() {
            return Err(AppError::validation(
                "The statement returns no rows to export.",
            ));
        }
        sink.columns(&columns.iter().map(|c| c.0.clone()).collect::<Vec<_>>())?;
        let portal = tx
            .bind(&statement, &refs)
            .await
            .map_err(|e| export_error(&e))?;
        let mut count = 0u64;
        loop {
            let batch = tx
                .query_portal(&portal, EXPORT_BATCH as i32)
                .await
                .map_err(|e| export_error(&e))?;
            for row in &batch {
                let values: Vec<ExportValue> = columns
                    .iter()
                    .enumerate()
                    .map(|(i, (_, ty))| match row.try_get::<_, Raw>(i) {
                        Ok(Raw(raw)) => pg_export_value(ty, raw),
                        Err(_) => ExportValue::Null,
                    })
                    .collect();
                sink.row(&values)?;
                count += 1;
            }
            if batch.len() < EXPORT_BATCH {
                break;
            }
        }
        drop(portal);
        tx.commit().await.map_err(|e| export_error(&e))?;
        Ok(count)
    }

    fn failure(&self, e: &tokio_postgres::Error, position: impl Fn(u32) -> u32) -> DbFailure {
        let Some(db) = e.as_db_error() else {
            return plain_failure(DbFailureReason::Sql, e.to_string());
        };
        let reason = match db.code() {
            c if *c == SqlState::READ_ONLY_SQL_TRANSACTION => DbFailureReason::ReadOnly,
            c if *c == SqlState::QUERY_CANCELED => {
                if self.cancel.cancelled.load(Ordering::SeqCst) {
                    DbFailureReason::Cancelled
                } else {
                    DbFailureReason::Timeout
                }
            }
            _ => DbFailureReason::Sql,
        };
        let message = match reason {
            DbFailureReason::ReadOnly => format!(
                "This connection is read only: {}.",
                db.message().trim_end_matches('.')
            ),
            DbFailureReason::Timeout => format!(
                "The statement ran longer than the {} second limit and was stopped.",
                self.timeout.as_secs()
            ),
            DbFailureReason::Cancelled => "Cancelled.".to_string(),
            _ => db.message().to_string(),
        };
        let position = match db.position() {
            Some(ErrorPosition::Original(p)) => Some(position(*p)),
            _ => None,
        };
        DbFailure {
            reason,
            message,
            code: Some(db.code().code().to_string()),
            detail: db.detail().map(str::to_string),
            hint: db.hint().map(str::to_string),
            position,
        }
    }

    /// The schemas, relations, columns, indexes, and foreign keys outside
    /// PostgreSQL's own schemas, and the schema unqualified names mean.
    pub async fn schema(&mut self) -> AppResult<(Vec<DbSchemaGroup>, Option<String>)> {
        read_schema(&mut self.client)
            .await
            .map_err(|e| AppError::db("Could not read the schema.").with_details(e.to_string()))
    }
}

/// Rows as PostgreSQL sent them, before they become cells.
enum Fetched {
    Rows {
        statement: Statement,
        rows: Vec<Row>,
        more: bool,
    },
    Command {
        count: u64,
    },
    /// `$1`-style parameters with no values for them.
    Parameters(usize),
}

impl Fetched {
    fn into_result(self, sql: &str, cap: usize) -> StatementResult {
        match self {
            Fetched::Parameters(n) => StatementResult::Failed {
                failure: plain_failure(
                    DbFailureReason::Parameters,
                    format!(
                        "This statement has {n} numbered parameter{} ($1…). Name them, as in :name, to be asked for their values.",
                        if n == 1 { "" } else { "s" }
                    ),
                ),
            },
            Fetched::Command { count } => StatementResult::Command {
                tag: command_tag(sql, count),
            },
            Fetched::Rows {
                statement,
                rows,
                more,
            } => {
                let columns = statement
                    .columns()
                    .iter()
                    .map(|c| ResultColumn {
                        name: c.name().to_string(),
                        type_name: c.type_().name().to_string(),
                        kind: pg_kind(c.type_()),
                    })
                    .collect();
                let types: Vec<&Type> = statement.columns().iter().map(|c| c.type_()).collect();
                let rows = rows
                    .iter()
                    .take(cap)
                    .map(|row| {
                        types
                            .iter()
                            .enumerate()
                            .map(|(i, ty)| match row.try_get::<_, Raw>(i) {
                                Ok(Raw(raw)) => pg_cell(ty, raw),
                                Err(_) => pg_cell(ty, None),
                            })
                            .collect()
                    })
                    .collect();
                StatementResult::Rows {
                    columns,
                    rows,
                    more,
                }
            }
        }
    }
}

/// A parameter value sent as text, for the server to parse with the type it
/// inferred for the parameter: `42` for an `int`, `2026-10-04` for a `date`,
/// and the server's own error for `abc` as an `int`.
#[derive(Debug)]
pub struct TextParam(Option<String>);

impl ToSql for TextParam {
    fn to_sql(
        &self,
        _ty: &Type,
        out: &mut bytes::BytesMut,
    ) -> Result<IsNull, Box<dyn std::error::Error + Sync + Send>> {
        match &self.0 {
            None => Ok(IsNull::Yes),
            Some(v) => {
                out.extend_from_slice(v.as_bytes());
                Ok(IsNull::No)
            }
        }
    }

    fn accepts(_: &Type) -> bool {
        true
    }

    fn encode_format(&self, _ty: &Type) -> Format {
        Format::Text
    }

    to_sql_checked!();
}

fn bind_values(names: &[String], params: &[ParamValue]) -> Result<Vec<TextParam>, DbFailure> {
    let mut values = Vec::with_capacity(names.len());
    let mut missing = Vec::new();
    for name in names {
        match params.iter().find(|p| &p.name == name) {
            Some(p) => values.push(TextParam(p.value.clone())),
            None => missing.push(format!(":{name}")),
        }
    }
    if missing.is_empty() {
        Ok(values)
    } else {
        Err(plain_failure(
            DbFailureReason::Parameters,
            format!("Enter a value for {}.", missing.join(", ")),
        ))
    }
}

/// Whether a statement refused in a read-only transaction should run again
/// writable: it writes, it cannot run in a transaction block (`VACUUM`), or
/// it ends the transaction itself (a procedure that commits).
fn needs_writable(e: &tokio_postgres::Error) -> bool {
    e.code().is_some_and(|c| {
        *c == SqlState::READ_ONLY_SQL_TRANSACTION
            || *c == SqlState::ACTIVE_SQL_TRANSACTION
            || *c == SqlState::INVALID_TRANSACTION_TERMINATION
    })
}

fn refs(values: &[TextParam]) -> Vec<&(dyn ToSql + Sync)> {
    values.iter().map(|v| v as &(dyn ToSql + Sync)).collect()
}

/// One statement alone in a read-only transaction, its rows through a
/// portal with a row limit, so a large result stops at the cap and the
/// transaction ends before the result is shown.
async fn fetch_read_only(
    client: &mut Client,
    sql: &str,
    values: &[TextParam],
    cap: usize,
) -> Result<Fetched, tokio_postgres::Error> {
    let tx = client.build_transaction().read_only(true).start().await?;
    let statement = tx.prepare(sql).await?;
    if statement.params().len() != values.len() {
        return Ok(Fetched::Parameters(statement.params().len()));
    }
    let params = refs(values);
    if statement.columns().is_empty() {
        let count = tx.execute(&statement, &params).await?;
        tx.commit().await?;
        return Ok(Fetched::Command { count });
    }
    let portal = tx.bind(&statement, &params).await?;
    let limit = i32::try_from(cap.saturating_add(1)).unwrap_or(i32::MAX);
    let rows = tx.query_portal(&portal, limit).await?;
    drop(portal);
    tx.commit().await?;
    let more = rows.len() > cap;
    Ok(Fetched::Rows {
        statement,
        rows,
        more,
    })
}

/// One statement in the session's own transaction state: an implicit
/// transaction that commits, or the tab's open transaction. Rows past the
/// cap are read and dropped.
async fn fetch_plain(
    client: &Client,
    sql: &str,
    values: &[TextParam],
    cap: usize,
) -> Result<Fetched, tokio_postgres::Error> {
    use futures_util::StreamExt;
    let statement = client.prepare(sql).await?;
    if statement.params().len() != values.len() {
        return Ok(Fetched::Parameters(statement.params().len()));
    }
    if statement.columns().is_empty() {
        let count = client.execute_raw(&statement, refs(values)).await?;
        return Ok(Fetched::Command { count });
    }
    let stream = client.query_raw(&statement, refs(values)).await?;
    let mut stream = std::pin::pin!(stream);
    let mut rows = Vec::new();
    let mut more = false;
    while let Some(row) = stream.next().await {
        let row = row?;
        if rows.len() == cap {
            more = true;
            continue;
        }
        rows.push(row);
    }
    Ok(Fetched::Rows {
        statement,
        rows,
        more,
    })
}

fn export_error(e: &tokio_postgres::Error) -> AppError {
    match e.as_db_error() {
        Some(db) if *db.code() == SqlState::READ_ONLY_SQL_TRANSACTION => AppError::validation(
            "Export runs the statement again read only, and this statement writes.",
        ),
        Some(db) => AppError::db(db.message().to_string()),
        None => AppError::dependency("The export stopped: the connection was lost.")
            .with_details(e.to_string()),
    }
}

/// Explain's JSON plan as a tree.
fn plan_result(fetched: &Fetched) -> StatementResult {
    let text = match fetched {
        Fetched::Rows { rows, .. } => rows
            .first()
            .and_then(|r| r.try_get::<_, Raw>(0).ok())
            .and_then(|Raw(raw)| raw)
            .and_then(|raw| std::str::from_utf8(raw).ok()),
        _ => None,
    };
    let parsed = text
        .and_then(|t| serde_json::from_str::<serde_json::Value>(t).ok())
        .and_then(|v| v.get(0).cloned());
    match parsed {
        Some(top) => StatementResult::Plan {
            plan: plan_node(top.get("Plan").unwrap_or(&serde_json::Value::Null)),
            planning_ms: top.get("Planning Time").and_then(serde_json::Value::as_f64),
            execution_ms: top
                .get("Execution Time")
                .and_then(serde_json::Value::as_f64),
        },
        None => StatementResult::Failed {
            failure: plain_failure(DbFailureReason::Sql, "PostgreSQL returned no plan.".into()),
        },
    }
}

/// Plan details worth showing, in the order PostgreSQL's text format shows them.
const PLAN_DETAILS: &[&str] = &[
    "Index Cond",
    "Recheck Cond",
    "Hash Cond",
    "Merge Cond",
    "Join Filter",
    "Filter",
    "Rows Removed by Filter",
    "Sort Key",
    "Sort Method",
    "Group Key",
    "Heap Fetches",
];

fn plan_node(node: &serde_json::Value) -> PlanNode {
    let text = |key: &str| node.get(key).and_then(serde_json::Value::as_str);
    let number = |key: &str| node.get(key).and_then(serde_json::Value::as_f64);
    let mut label = text("Node Type").unwrap_or("?").to_string();
    if let Some(strategy) = text("Strategy").filter(|s| *s != "Plain") {
        label = format!("{strategy} {label}");
    }
    if let Some(join) = text("Join Type").filter(|j| *j != "Inner") {
        label = format!("{label} ({join})");
    }
    if let Some(index) = text("Index Name") {
        label.push_str(&format!(" using {index}"));
    }
    if let Some(relation) = text("Relation Name") {
        label.push_str(&format!(" on {relation}"));
        if let Some(alias) = text("Alias").filter(|a| *a != relation) {
            label.push_str(&format!(" {alias}"));
        }
    }
    let details = PLAN_DETAILS
        .iter()
        .filter_map(|key| {
            let value = node.get(*key)?;
            let shown = match value {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Array(items) => items
                    .iter()
                    .map(|i| {
                        i.as_str()
                            .map(str::to_string)
                            .unwrap_or_else(|| i.to_string())
                    })
                    .collect::<Vec<_>>()
                    .join(", "),
                other => other.to_string(),
            };
            Some(format!("{key}: {shown}"))
        })
        .collect();
    PlanNode {
        label,
        startup_cost: number("Startup Cost"),
        total_cost: number("Total Cost"),
        rows: number("Plan Rows"),
        actual_rows: number("Actual Rows"),
        actual_ms: number("Actual Total Time"),
        loops: number("Actual Loops"),
        details,
        children: node
            .get("Plans")
            .and_then(serde_json::Value::as_array)
            .map(|plans| plans.iter().map(plan_node).collect())
            .unwrap_or_default(),
    }
}

/// The command tag shown for a statement without rows, such as `UPDATE 42`.
/// `tokio-postgres` gives the row count but not the tag, so the tag is the
/// statement's first keyword.
pub fn command_tag(sql: &str, count: u64) -> String {
    let keyword = statements::first_keyword(sql);
    match keyword.as_str() {
        "INSERT" => format!("INSERT 0 {count}"),
        "UPDATE" | "DELETE" | "MERGE" | "SELECT" | "COPY" | "FETCH" | "MOVE" => {
            format!("{keyword} {count}")
        }
        "" => "OK".to_string(),
        _ => keyword,
    }
}

const USER_SCHEMAS: &str = "n.nspname <> 'information_schema' AND n.nspname NOT LIKE 'pg\\_%'";

async fn read_schema(
    client: &mut Client,
) -> Result<(Vec<DbSchemaGroup>, Option<String>), tokio_postgres::Error> {
    let tx = client.build_transaction().read_only(true).start().await?;
    let default_schema: Option<String> = tx
        .query_one("SELECT current_schema()::text", &[])
        .await?
        .get(0);
    let names = tx
        .query(
            &format!("SELECT n.nspname::text FROM pg_catalog.pg_namespace n WHERE {USER_SCHEMAS} ORDER BY 1"),
            &[],
        )
        .await?;
    let mut groups: Vec<DbSchemaGroup> = names
        .iter()
        .map(|r| DbSchemaGroup {
            name: r.get(0),
            relations: Vec::new(),
        })
        .collect();
    let relations = tx
        .query(
            &format!(
                "SELECT n.nspname::text, c.relname::text, c.relkind::text,
                        CASE WHEN c.reltuples < 0 THEN NULL ELSE c.reltuples::int8 END
                   FROM pg_catalog.pg_class c
                   JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
                  WHERE c.relkind IN ('r', 'p', 'v', 'm', 'f') AND NOT c.relispartition AND {USER_SCHEMAS}
                  ORDER BY 1, 2"
            ),
            &[],
        )
        .await?;
    for r in &relations {
        let schema: String = r.get(0);
        let kind = match r.get::<_, String>(2).as_str() {
            "v" => RelationKind::View,
            "m" => RelationKind::MaterializedView,
            "f" => RelationKind::ForeignTable,
            _ => RelationKind::Table,
        };
        if let Some(group) = groups.iter_mut().find(|g| g.name == schema) {
            group.relations.push(DbRelation {
                name: r.get(1),
                kind,
                estimated_rows: r.get(3),
                columns: Vec::new(),
                indexes: Vec::new(),
                foreign_keys: Vec::new(),
            });
        }
    }
    let columns = tx
        .query(
            &format!(
                "SELECT n.nspname::text, c.relname::text, a.attname::text,
                        pg_catalog.format_type(a.atttypid, a.atttypmod), NOT a.attnotnull,
                        pg_catalog.pg_get_expr(d.adbin, d.adrelid),
                        COALESCE(a.attnum = ANY(pk.conkey), false)
                   FROM pg_catalog.pg_attribute a
                   JOIN pg_catalog.pg_class c ON c.oid = a.attrelid
                   JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
                   LEFT JOIN pg_catalog.pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum
                   LEFT JOIN pg_catalog.pg_constraint pk ON pk.conrelid = c.oid AND pk.contype = 'p'
                  WHERE a.attnum > 0 AND NOT a.attisdropped
                    AND c.relkind IN ('r', 'p', 'v', 'm', 'f') AND NOT c.relispartition AND {USER_SCHEMAS}
                  ORDER BY n.nspname, c.relname, a.attnum"
            ),
            &[],
        )
        .await?;
    for r in &columns {
        if let Some(relation) = find_relation(&mut groups, r.get(0), r.get(1)) {
            relation.columns.push(DbColumn {
                name: r.get(2),
                type_name: r.get(3),
                nullable: r.get(4),
                default: r.get(5),
                primary_key: r.get(6),
            });
        }
    }
    let indexes = tx
        .query(
            &format!(
                "SELECT n.nspname::text, c.relname::text, i.relname::text,
                        pg_catalog.pg_get_indexdef(x.indexrelid), x.indisunique, x.indisprimary
                   FROM pg_catalog.pg_index x
                   JOIN pg_catalog.pg_class i ON i.oid = x.indexrelid
                   JOIN pg_catalog.pg_class c ON c.oid = x.indrelid
                   JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
                  WHERE {USER_SCHEMAS}
                  ORDER BY 1, 2, 3"
            ),
            &[],
        )
        .await?;
    for r in &indexes {
        if let Some(relation) = find_relation(&mut groups, r.get(0), r.get(1)) {
            relation.indexes.push(DbIndex {
                name: r.get(2),
                definition: r.get(3),
                unique: r.get(4),
                primary: r.get(5),
            });
        }
    }
    let keys = tx
        .query(
            &format!(
                "SELECT n.nspname::text, c.relname::text, k.conname::text, pg_catalog.pg_get_constraintdef(k.oid)
                   FROM pg_catalog.pg_constraint k
                   JOIN pg_catalog.pg_class c ON c.oid = k.conrelid
                   JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
                  WHERE k.contype = 'f' AND {USER_SCHEMAS}
                  ORDER BY 1, 2, 3"
            ),
            &[],
        )
        .await?;
    for r in &keys {
        if let Some(relation) = find_relation(&mut groups, r.get(0), r.get(1)) {
            relation.foreign_keys.push(DbForeignKey {
                name: r.get(2),
                definition: r.get(3),
            });
        }
    }
    tx.commit().await?;
    Ok((groups, default_schema))
}

fn find_relation(
    groups: &mut [DbSchemaGroup],
    schema: String,
    name: String,
) -> Option<&mut DbRelation> {
    groups
        .iter_mut()
        .find(|g| g.name == schema)?
        .relations
        .iter_mut()
        .find(|r| r.name == name)
}

/// TLS for `Verify` (the Mac's trust store plus the CA file) or `Require`
/// (encrypted, any certificate).
fn tls_config(tls: DbTls, ca_file: Option<&std::path::Path>) -> AppResult<ClientConfig> {
    // Explicit, because rustls cannot pick a provider when two are compiled in.
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let builder = ClientConfig::builder_with_provider(Arc::clone(&provider))
        .with_safe_default_protocol_versions()
        .map_err(|e| AppError::io("TLS could not be set up.").with_details(e.to_string()))?;
    let verifier: Arc<dyn ServerCertVerifier> = match tls {
        DbTls::Require => Arc::new(AnyCertificate(provider)),
        _ => {
            let extra = match ca_file {
                Some(path) => read_certificates(path)?,
                None => Vec::new(),
            };
            let verifier = if extra.is_empty() {
                rustls_platform_verifier::Verifier::new(provider)
            } else {
                rustls_platform_verifier::Verifier::new_with_extra_roots(extra, provider)
            }
            .map_err(|e| {
                AppError::validation("The CA file could not be used.").with_details(e.to_string())
            })?;
            Arc::new(verifier)
        }
    };
    Ok(builder
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth())
}

fn read_certificates(path: &std::path::Path) -> AppResult<Vec<CertificateDer<'static>>> {
    let certificates = CertificateDer::pem_file_iter(path)
        .and_then(|certs| certs.collect::<Result<Vec<_>, _>>())
        .map_err(|e| {
            AppError::validation(format!(
                "{} is not a PEM file of certificates.",
                path.display()
            ))
            .with_details(e.to_string())
        })?;
    if certificates.is_empty() {
        return Err(AppError::validation(format!(
            "{} has no certificates in it.",
            path.display()
        )));
    }
    Ok(certificates)
}

/// `Require without verifying`: encrypts, but accepts any certificate. The
/// signatures are still checked, so the handshake is with the certificate's key.
#[derive(Debug)]
struct AnyCertificate(Arc<CryptoProvider>);

impl ServerCertVerifier for AnyCertificate {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

/// Why a connection failed, in words (SPEC.md, Databases: Test Connection).
fn connect_error(e: &tokio_postgres::Error, target: &PgTarget) -> AppError {
    let details = error_chain(e);
    if let Some(db) = e.as_db_error() {
        let code = db.code();
        if *code == SqlState::INVALID_PASSWORD
            || *code == SqlState::INVALID_AUTHORIZATION_SPECIFICATION
        {
            return AppError::new(
                ErrorCode::PermissionDenied,
                format!(
                    "{} refused the password for user {}.",
                    target.place(),
                    target.user
                ),
            )
            .with_details(db.message().to_string());
        }
        if *code == SqlState::INVALID_CATALOG_NAME {
            return AppError::not_found(format!(
                "The database {} does not exist on {}.",
                target.database,
                target.place()
            ));
        }
        return AppError::new(ErrorCode::DependencyUnavailable, db.message().to_string())
            .with_details(details);
    }
    if let Some(tls) = rustls_error(e) {
        use rustls::CertificateError as C;
        let message = match tls {
            rustls::Error::InvalidCertificate(C::UnknownIssuer) | rustls::Error::InvalidCertificate(C::BadSignature) => format!(
                "This Mac does not trust the certificate of {}. Add its certificate authority as a CA file, or choose Require without verifying.",
                target.host
            ),
            rustls::Error::InvalidCertificate(C::NotValidForName)
            | rustls::Error::InvalidCertificate(C::NotValidForNameContext { .. }) => format!(
                "The certificate of {} is for another host name.",
                target.host
            ),
            rustls::Error::InvalidCertificate(C::Expired) | rustls::Error::InvalidCertificate(C::ExpiredContext { .. }) => format!(
                "The certificate of {} has expired.",
                target.host
            ),
            rustls::Error::InvalidCertificate(_) => format!(
                "The certificate of {} could not be verified. Add its certificate authority as a CA file, or choose Require without verifying.",
                target.host
            ),
            _ => format!("TLS with {} failed.", target.place()),
        };
        return AppError::dependency(message).with_details(details);
    }
    let lower = details.to_ascii_lowercase();
    let message = if lower.contains("server does not support tls") {
        format!(
            "{} does not offer TLS. Set TLS to Off only if the network to it is trusted.",
            target.place()
        )
    } else if lower.contains("connection refused") {
        format!(
            "Nothing is listening at {}. Is the server running?",
            target.place()
        )
    } else if lower.contains("lookup") || lower.contains("nodename nor servname") {
        format!("The host {} was not found.", target.host)
    } else if lower.contains("timeout") || lower.contains("timed out") {
        format!(
            "{} did not answer within {} seconds.",
            target.place(),
            CONNECT_TIMEOUT.as_secs()
        )
    } else {
        format!("Could not connect to {}.", target.place())
    };
    AppError::dependency(message).with_details(details)
}

/// Every message in an error's chain, including errors wrapped in `io::Error`.
fn error_chain(e: &(dyn std::error::Error + 'static)) -> String {
    let mut parts = vec![e.to_string()];
    let mut current = e.source();
    while let Some(err) = current {
        let text = err.to_string();
        if parts.last() != Some(&text) {
            parts.push(text);
        }
        current = err.source();
    }
    parts.join(": ")
}

/// The rustls error behind a failed connection, if TLS failed.
fn rustls_error(e: &tokio_postgres::Error) -> Option<rustls::Error> {
    let mut current: Option<&(dyn std::error::Error + 'static)> = Some(e);
    while let Some(err) = current {
        if let Some(tls) = err.downcast_ref::<rustls::Error>() {
            return Some(tls.clone());
        }
        // `io::Error::source` skips the error it wraps, so look inside it.
        if let Some(io) = err.downcast_ref::<std::io::Error>() {
            if let Some(tls) = io
                .get_ref()
                .and_then(|inner| inner.downcast_ref::<rustls::Error>())
            {
                return Some(tls.clone());
            }
        }
        current = err.source();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_tags_name_the_statement() {
        assert_eq!(command_tag("update t set a = 1", 42), "UPDATE 42");
        assert_eq!(command_tag("insert into t values (1)", 1), "INSERT 0 1");
        assert_eq!(command_tag("  set search_path = x", 0), "SET");
        assert_eq!(command_tag("-- c\ncreate table t ()", 0), "CREATE");
    }
}
