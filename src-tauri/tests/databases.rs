//! Saved connections and query sessions (SPEC.md, section 11): passwords
//! only in the Keychain (in memory here), the statement under the cursor,
//! Run All, positions of errors, a session per tab, reconnecting, and the
//! limit on open sessions.

mod postgres_server;

use std::sync::Arc;
use std::time::Duration;

use brainiac_lib::credentials::{CommandRunner, CredentialService, MemoryStore};
use brainiac_lib::databases::sessions::MAX_SESSIONS;
use brainiac_lib::databases::{ConnectionService, QuerySessions};
use brainiac_lib::db::Db;
use brainiac_lib::models::{
    Cell, DbAccess, DbEnvironment, DbFailureReason, DbKind, DbTls, ErrorCode, RunMode,
    RunStatementRequest, SaveDbConnectionRequest, SecretSource, StatementResult, StatementRun,
};
use postgres_server::PgServer;

struct Harness {
    _tmp: tempfile::TempDir,
    core: std::path::PathBuf,
    store: Arc<MemoryStore>,
    sessions: QuerySessions,
}

fn harness() -> Harness {
    let tmp = tempfile::tempdir().unwrap();
    let core = tmp.path().join("brainiac.sqlite3");
    let db = Db::open(&core).unwrap();
    let store = Arc::new(MemoryStore::default());
    let credentials = Arc::new(CredentialService::new(
        store.clone(),
        CommandRunner::new(tmp.path().join("commands")),
    ));
    let connections = Arc::new(ConnectionService::new(db, credentials));
    Harness {
        _tmp: tmp,
        core,
        store,
        sessions: QuerySessions::new(connections),
    }
}

fn postgres(
    server: &PgServer,
    password: SecretSource,
    typed: Option<&str>,
) -> SaveDbConnectionRequest {
    SaveDbConnectionRequest {
        id: None,
        expected_version: None,
        name: "Billing".into(),
        kind: DbKind::Postgres,
        environment: DbEnvironment::Local,
        access: DbAccess::ReadOnly,
        file_path: None,
        host: Some(" 127.0.0.1 ".into()),
        port: Some(server.port),
        database: Some("postgres".into()),
        user: Some("postgres".into()),
        tls: Some(DbTls::Off),
        ca_file: None,
        password_source: password,
        password: typed.map(str::to_string),
        statement_timeout_seconds: 30,
        runs_on: None,
    }
}

fn sqlite(path: &std::path::Path) -> SaveDbConnectionRequest {
    SaveDbConnectionRequest {
        id: None,
        expected_version: None,
        name: "Shop".into(),
        kind: DbKind::Sqlite,
        environment: DbEnvironment::Development,
        access: DbAccess::ReadOnly,
        file_path: Some(path.display().to_string()),
        host: Some("ignored".into()),
        port: None,
        database: None,
        user: None,
        tls: None,
        ca_file: None,
        password_source: SecretSource::Store,
        password: None,
        statement_timeout_seconds: 30,
        runs_on: None,
    }
}

fn run_request(
    tab: &str,
    connection: &str,
    text: &str,
    cursor: u32,
    all: bool,
) -> RunStatementRequest {
    RunStatementRequest {
        tab_id: tab.into(),
        connection_id: connection.into(),
        text: text.into(),
        from: cursor,
        to: cursor,
        all,
        fetch_all: false,
        mode: RunMode::ReadOnly,
        parameters: Vec::new(),
        explain: None,
    }
}

fn first_cell(run: &StatementRun) -> Cell {
    match &run.result {
        StatementResult::Rows { rows, .. } => rows[0][0].clone(),
        other => panic!("expected rows, got {other:?}"),
    }
}

#[tokio::test]
async fn sqlite_connections_are_saved_edited_and_deleted() {
    let h = harness();
    let file = h._tmp.path().join("shop.db");
    rusqlite::Connection::open(&file)
        .unwrap()
        .execute_batch("CREATE TABLE t (id INTEGER); INSERT INTO t VALUES (1), (2);")
        .unwrap();
    let connections = h.sessions.connections();
    let saved = connections.save(sqlite(&file)).await.unwrap();
    // SQLite keeps none of the server fields, and needs no password.
    assert_eq!(
        (saved.host.as_deref(), saved.password),
        (None, SecretSource::None)
    );
    assert!(saved.file_size.unwrap() > 0);
    assert_eq!(saved.version, 1);

    let mut edit = sqlite(&file);
    edit.id = Some(saved.id.clone());
    edit.expected_version = Some(1);
    edit.name = "Shop copy".into();
    let edited = connections.save(edit.clone()).await.unwrap();
    assert_eq!((edited.name.as_str(), edited.version), ("Shop copy", 2));
    // The same form saved again is refused: the connection changed since it opened.
    let stale = connections.save(edit).await.err().unwrap();
    assert_eq!(stale.code, ErrorCode::Conflict);

    let runs = h
        .sessions
        .run(run_request(
            "tab",
            &saved.id,
            "select count(*) from t",
            0,
            false,
        ))
        .await
        .unwrap();
    assert_eq!(first_cell(&runs[0]), Cell::Number(2.0));

    assert_eq!(
        connections.delete(&saved.id, 1).await.err().unwrap().code,
        ErrorCode::Conflict
    );
    connections.delete(&saved.id, 2).await.unwrap();
    assert!(connections.list().await.unwrap().is_empty());

    let missing = connections
        .save(sqlite(&h._tmp.path().join("nope.db")))
        .await
        .err()
        .unwrap();
    assert_eq!(missing.code, ErrorCode::NotFound);
}

#[tokio::test]
async fn passwords_go_to_the_keychain_and_never_to_the_database() {
    let Some(server) = PgServer::start() else {
        return;
    };
    let h = harness();
    let connections = h.sessions.connections();
    let saved = connections
        .save(postgres(
            &server,
            SecretSource::Store,
            Some(postgres_server::PASSWORD),
        ))
        .await
        .unwrap();
    assert_eq!(saved.host.as_deref(), Some("127.0.0.1"));
    let account = format!("db:{}", saved.id);
    assert_eq!(h.store.text(&account).unwrap(), postgres_server::PASSWORD);
    let bytes = std::fs::read(&h.core).unwrap();
    let wal = std::fs::read(h.core.with_extension("sqlite3-wal")).unwrap_or_default();
    let needle = postgres_server::PASSWORD.as_bytes();
    assert!(!bytes.windows(needle.len()).any(|w| w == needle));
    assert!(!wal.windows(needle.len()).any(|w| w == needle));

    let tested = connections
        .test(postgres(
            &server,
            SecretSource::Store,
            Some(postgres_server::PASSWORD),
        ))
        .await
        .unwrap();
    assert!(
        tested.server_version.starts_with("PostgreSQL "),
        "{tested:?}"
    );

    // Editing without typing a password keeps the one in the Keychain.
    let mut edit = postgres(&server, SecretSource::Store, None);
    edit.id = Some(saved.id.clone());
    edit.expected_version = Some(saved.version);
    connections.save(edit).await.unwrap();
    let runs = h
        .sessions
        .run(run_request("tab", &saved.id, "select 1", 0, false))
        .await
        .unwrap();
    assert_eq!(first_cell(&runs[0]), Cell::Number(1.0));

    // Switching to "ask" removes the Keychain item; deleting removes the row.
    let mut ask = postgres(&server, SecretSource::Ask, None);
    ask.id = Some(saved.id.clone());
    let asked = connections.save(ask).await.unwrap();
    assert!(h.store.text(&account).is_none());
    assert!(!asked.password_ready);
    connections.delete(&saved.id, asked.version).await.unwrap();
}

#[tokio::test]
async fn a_connection_that_asks_needs_its_password_once_per_run() {
    let Some(server) = PgServer::start() else {
        return;
    };
    let h = harness();
    let connections = h.sessions.connections();
    let saved = connections
        .save(postgres(&server, SecretSource::Ask, None))
        .await
        .unwrap();
    assert!(!saved.password_ready);
    let locked = h
        .sessions
        .run(run_request("tab", &saved.id, "select 1", 0, false))
        .await
        .err()
        .unwrap();
    assert_eq!(locked.code, ErrorCode::PermissionDenied);

    // A wrong password is refused by the server and forgotten, so it is asked for again.
    connections
        .unlock(&saved.id, "wrong", saved.version)
        .await
        .unwrap();
    let refused = h
        .sessions
        .run(run_request("tab", &saved.id, "select 1", 0, false))
        .await
        .err()
        .unwrap();
    assert_eq!(refused.code, ErrorCode::Unauthenticated);
    assert!(!connections.get(&saved.id).await.unwrap().password_ready);

    let unlocked = connections
        .unlock(&saved.id, postgres_server::PASSWORD, saved.version)
        .await
        .unwrap();
    assert!(unlocked.password_ready);
    let runs = h
        .sessions
        .run(run_request("tab", &saved.id, "select 1", 0, false))
        .await
        .unwrap();
    assert_eq!(first_cell(&runs[0]), Cell::Number(1.0));
    assert!(h.store.text(&format!("db:{}", saved.id)).is_none());
}

#[tokio::test]
async fn runs_pick_statements_and_place_errors_in_the_text() {
    let Some(server) = PgServer::start() else {
        return;
    };
    let h = harness();
    let saved = h
        .sessions
        .connections()
        .save(postgres(
            &server,
            SecretSource::Store,
            Some(postgres_server::PASSWORD),
        ))
        .await
        .unwrap();
    let text = "select 'é😀' as a;\nselect 2;\nselect * from nowhere;\nselect 4;";
    // The cursor on the second line runs only that statement.
    let runs = h
        .sessions
        .run(run_request("tab", &saved.id, text, 20, false))
        .await
        .unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].sql, "select 2");
    assert_eq!(first_cell(&runs[0]), Cell::Number(2.0));
    // UTF-16 offsets: `é` is one unit, `😀` two.
    assert_eq!((runs[0].from, runs[0].to), (19, 28));

    // Run All stops at the first failure, which is placed in the whole text.
    let runs = h
        .sessions
        .run(run_request("tab", &saved.id, text, 0, true))
        .await
        .unwrap();
    assert_eq!(runs.len(), 3);
    match &runs[2].result {
        StatementResult::Failed { failure } => {
            assert_eq!(failure.reason, DbFailureReason::Sql);
            let at = failure.position.unwrap() as usize;
            let utf16: Vec<u16> = text.encode_utf16().collect();
            assert_eq!(String::from_utf16(&utf16[at..at + 7]).unwrap(), "nowhere");
        }
        other => panic!("{other:?}"),
    }

    // A selection runs each statement in it.
    let mut selection = run_request("tab", &saved.id, text, 0, false);
    selection.to = 27;
    let runs = h.sessions.run(selection).await.unwrap();
    assert_eq!(runs.len(), 2);

    let nothing = h
        .sessions
        .run(run_request(
            "tab",
            &saved.id,
            "  -- just a comment",
            0,
            false,
        ))
        .await
        .err()
        .unwrap();
    assert_eq!(nothing.code, ErrorCode::Validation);
}

#[tokio::test]
async fn each_tab_has_its_own_session_and_reconnects_when_it_is_lost() {
    let Some(server) = PgServer::start() else {
        return;
    };
    server.psql("CREATE SCHEMA other");
    let h = harness();
    let saved = h
        .sessions
        .connections()
        .save(postgres(
            &server,
            SecretSource::Store,
            Some(postgres_server::PASSWORD),
        ))
        .await
        .unwrap();
    let run = |tab: &'static str, sql: &'static str| {
        h.sessions.run(run_request(tab, &saved.id, sql, 0, false))
    };
    run("a", "set search_path = other").await.unwrap();
    let a = run("a", "show search_path").await.unwrap();
    let b = run("b", "show search_path").await.unwrap();
    assert_eq!(first_cell(&a[0]), Cell::Text("other".into()));
    assert_ne!(first_cell(&b[0]), Cell::Text("other".into()));
    assert!(!a[0].reconnected);

    server.psql("SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE application_name = 'Brainiac'");
    tokio::time::sleep(Duration::from_millis(200)).await;
    let again = run("a", "show search_path").await.unwrap();
    // The statement ran on a new session, which lost the tab's `SET`.
    assert!(again[0].reconnected);
    assert_ne!(first_cell(&again[0]), Cell::Text("other".into()));

    h.sessions.close("a");
    h.sessions.close("b");
    assert_eq!(h.sessions.open_sessions(), 0);
}

#[tokio::test]
async fn at_most_eight_sessions_are_open_and_cancel_reaches_the_server() {
    let Some(server) = PgServer::start() else {
        return;
    };
    let h = harness();
    let saved = h
        .sessions
        .connections()
        .save(postgres(
            &server,
            SecretSource::Store,
            Some(postgres_server::PASSWORD),
        ))
        .await
        .unwrap();
    for i in 0..MAX_SESSIONS + 3 {
        let tab = format!("tab{i}");
        h.sessions
            .run(run_request(&tab, &saved.id, "select 1", 0, false))
            .await
            .unwrap();
        assert!(h.sessions.open_sessions() <= MAX_SESSIONS);
    }
    // The first tab's session was closed for room, so its next run reconnects.
    let runs = h
        .sessions
        .run(run_request("tab0", &saved.id, "select 1", 0, false))
        .await
        .unwrap();
    assert!(runs[0].reconnected);

    let sessions = Arc::new(h.sessions);
    let running = {
        let sessions = Arc::clone(&sessions);
        let id = saved.id.clone();
        tokio::spawn(async move {
            sessions
                .run(run_request(
                    "slow",
                    &id,
                    "select pg_sleep(30); select 2;",
                    0,
                    true,
                ))
                .await
        })
    };
    tokio::time::sleep(Duration::from_millis(500)).await;
    // A second run in the same tab is refused while the first is running.
    let busy = sessions
        .run(run_request("slow", &saved.id, "select 3", 0, false))
        .await
        .err()
        .unwrap();
    assert_eq!(busy.code, ErrorCode::Conflict);
    sessions.cancel("slow").await;
    let runs = running.await.unwrap().unwrap();
    assert_eq!(runs.len(), 1, "Run All went on after the cancel");
    match &runs[0].result {
        StatementResult::Failed { failure } => {
            assert_eq!(failure.reason, DbFailureReason::Cancelled)
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn the_schema_is_kept_until_refreshed() {
    let Some(server) = PgServer::start() else {
        return;
    };
    server.psql("CREATE TABLE first_table (id int)");
    let h = harness();
    let saved = h
        .sessions
        .connections()
        .save(postgres(
            &server,
            SecretSource::Store,
            Some(postgres_server::PASSWORD),
        ))
        .await
        .unwrap();
    let names = |schema: &brainiac_lib::models::DbSchema| -> Vec<String> {
        schema
            .schemas
            .iter()
            .flat_map(|g| g.relations.iter().map(|r| r.name.clone()))
            .collect()
    };
    let first = h.sessions.schema(&saved.id, false).await.unwrap();
    assert_eq!(names(&first), ["first_table"]);
    server.psql("CREATE TABLE second_table (id int)");
    let cached = h.sessions.schema(&saved.id, false).await.unwrap();
    assert_eq!(names(&cached), ["first_table"]);
    let refreshed = h.sessions.schema(&saved.id, true).await.unwrap();
    assert_eq!(names(&refreshed), ["first_table", "second_table"]);
}
