//! The driver: one `Session` type in front of PostgreSQL and SQLite, so the
//! sessions, commands, and window never see a PostgreSQL type or a SQLite
//! pragma (docs/architecture.md, Databases — v0.4, The driver).

use std::path::PathBuf;
use std::time::Duration;

use super::export::FileSink;
use super::postgres::{PgCanceller, PgSession, PgTarget};
use super::sqlite::{SqliteCanceller, SqliteSession};
use crate::models::{
    AppResult, DbFailure, DbFailureReason, DbSchemaGroup, ExplainMode, ParamValue, RunMode,
    StatementResult, TransactionInfo,
};

/// The statement ran into a connection that is gone; open a new session.
#[derive(Debug)]
pub struct Lost;

/// What a statement did.
pub struct Ran {
    pub result: StatementResult,
    /// It ran in a read-only transaction, so running it again cannot write.
    pub ran_read_only: bool,
}

/// What a session connects to.
#[derive(Debug, Clone)]
pub enum Target {
    Postgres(PgTarget),
    Sqlite {
        path: PathBuf,
        timeout: Duration,
        /// Opened read and write; otherwise SQLite opens it read only.
        writable: bool,
    },
}

/// An open connection, owned by one tab. An enum rather than a trait
/// object: there are two drivers, and `async fn` in a trait cannot be
/// called through `dyn`.
pub enum Session {
    // Boxed: a PostgreSQL session is several times the size of a SQLite one.
    Postgres(Box<PgSession>),
    Sqlite(SqliteSession),
}

/// Cancels the statement a session is running, from another task.
#[derive(Clone)]
pub enum Canceller {
    Postgres(PgCanceller),
    Sqlite(SqliteCanceller),
}

impl Canceller {
    pub async fn cancel(&self) {
        match self {
            Canceller::Postgres(c) => c.cancel().await,
            Canceller::Sqlite(c) => c.cancel(),
        }
    }
}

impl Session {
    pub async fn open(target: &Target) -> AppResult<Session> {
        Ok(match target {
            Target::Postgres(t) => Session::Postgres(Box::new(PgSession::connect(t).await?)),
            Target::Sqlite {
                path,
                timeout,
                writable,
            } => Session::Sqlite(SqliteSession::open(path, *timeout, *writable).await?),
        })
    }

    /// Run one statement in `mode`, returning at most `cap` rows. A
    /// failure's position is a byte offset into `sql`.
    pub async fn run(
        &mut self,
        sql: &str,
        params: &[ParamValue],
        cap: usize,
        mode: RunMode,
        explain: Option<ExplainMode>,
    ) -> Result<Ran, Lost> {
        match self {
            Session::Postgres(s) => s.run(sql, params, cap, mode, explain).await,
            Session::Sqlite(s) => s.run(sql, params, cap, mode, explain).await,
        }
    }

    /// The tab's open transaction, in Manual mode.
    pub fn transaction(&self) -> Option<TransactionInfo> {
        match self {
            Session::Postgres(s) => s.transaction(),
            Session::Sqlite(s) => s.transaction(),
        }
    }

    pub async fn end_transaction(&mut self, commit: bool) -> AppResult<()> {
        match self {
            Session::Postgres(s) => s.end_transaction(commit).await,
            Session::Sqlite(s) => s.end_transaction(commit).await,
        }
    }

    /// Export: the statement again, read only, every row to `sink`, which is
    /// completed on success and removed on failure.
    pub async fn export(
        &mut self,
        sql: &str,
        params: &[ParamValue],
        mut sink: FileSink,
    ) -> AppResult<u64> {
        use super::export::RowSink;
        match self {
            Session::Postgres(s) => match s.export(sql, params, &mut sink).await {
                Ok(count) => sink.finish().map(|()| count),
                Err(e) => {
                    sink.abandon();
                    Err(e)
                }
            },
            Session::Sqlite(s) => s.export(sql, params, sink).await,
        }
    }

    pub fn canceller(&self) -> Canceller {
        match self {
            Session::Postgres(s) => Canceller::Postgres(s.canceller()),
            Session::Sqlite(s) => Canceller::Sqlite(s.canceller()),
        }
    }

    pub fn is_closed(&self) -> bool {
        match self {
            Session::Postgres(s) => s.is_closed(),
            Session::Sqlite(_) => false,
        }
    }

    pub fn server_version(&self) -> &str {
        match self {
            Session::Postgres(s) => s.server_version(),
            Session::Sqlite(s) => s.server_version(),
        }
    }

    /// The schemas and their relations, and the schema unqualified names mean.
    pub async fn schema(&mut self) -> AppResult<(Vec<DbSchemaGroup>, Option<String>)> {
        match self {
            Session::Postgres(s) => s.schema().await,
            Session::Sqlite(s) => s.schema().await,
        }
    }
}

/// A failure with only a reason and a message.
pub fn plain_failure(reason: DbFailureReason, message: String) -> DbFailure {
    DbFailure {
        reason,
        message,
        code: None,
        detail: None,
        hint: None,
        position: None,
    }
}

/// A typed `BEGIN`, `COMMIT`, or `ROLLBACK` outside Manual mode. Refused:
/// each statement there runs in a transaction of its own, so a typed one
/// would seem to work while the statements after it commit regardless.
pub fn manual_only() -> DbFailure {
    plain_failure(
        DbFailureReason::Sql,
        "BEGIN, COMMIT, and ROLLBACK run only in Manual mode, where the transaction bar shows the open transaction. Switch this tab to Manual (on a connection that allows writes)."
            .into(),
    )
}

/// Rows a result keeps in memory at most, counting each value as shown (cut
/// at `CUT_AT`): past it, the result says more rows are available.
pub const RESULT_BYTES: usize = 128 * 1024 * 1024;
