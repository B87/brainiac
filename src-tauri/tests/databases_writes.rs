//! Writes, transactions, parameters, Explain, Export, saved queries, and
//! history (SPEC.md, section 11), against a throwaway PostgreSQL server and
//! SQLite files the tests create.

mod postgres_server;

use std::sync::Arc;
use std::time::Duration;

use brainiac_lib::credentials::MemorySecrets;
use brainiac_lib::databases::history::QueryHistory;
use brainiac_lib::databases::{ConnectionService, QuerySessions, SavedQueryService};
use brainiac_lib::db::{self, Db};
use brainiac_lib::models::{
    Cell, DbAccess, DbEnvironment, DbFailureReason, DbKind, DbPassword, DbTls, ErrorCode,
    ExplainMode, ExportFormat, ExportRequest, ParamValue, QueryTab, RunMode, RunStatementRequest,
    SaveDbConnectionRequest, SaveQueryRequest, StatementResult, StatementRun,
};
use postgres_server::PgServer;

struct Harness {
    tmp: tempfile::TempDir,
    core: Db,
    sessions: QuerySessions,
}

fn harness() -> Harness {
    let tmp = tempfile::tempdir().unwrap();
    let core = Db::open(&tmp.path().join(db::CORE_FILE)).unwrap();
    let history = Db::open_store(&tmp.path().join(db::HISTORY_FILE), &db::HISTORY).unwrap();
    let connections = Arc::new(ConnectionService::new(
        core.clone(),
        Arc::new(MemorySecrets::default()),
    ));
    Harness {
        tmp,
        core,
        sessions: QuerySessions::new(connections).with_history(QueryHistory::new(history), true),
    }
}

fn postgres(server: &PgServer, access: DbAccess) -> SaveDbConnectionRequest {
    SaveDbConnectionRequest {
        id: None,
        expected_version: None,
        name: "Billing".into(),
        kind: DbKind::Postgres,
        environment: DbEnvironment::Local,
        access,
        file_path: None,
        host: Some("127.0.0.1".into()),
        port: Some(server.port),
        database: Some("postgres".into()),
        user: Some("postgres".into()),
        tls: Some(DbTls::Off),
        ca_file: None,
        password_storage: DbPassword::Keychain,
        password: Some(postgres_server::PASSWORD.into()),
        statement_timeout_seconds: 30,
        runs_on: None,
    }
}

fn sqlite(path: &std::path::Path, access: DbAccess) -> SaveDbConnectionRequest {
    SaveDbConnectionRequest {
        id: None,
        expected_version: None,
        name: "Shop".into(),
        kind: DbKind::Sqlite,
        environment: DbEnvironment::Local,
        access,
        file_path: Some(path.display().to_string()),
        host: None,
        port: None,
        database: None,
        user: None,
        tls: None,
        ca_file: None,
        password_storage: DbPassword::None,
        password: None,
        statement_timeout_seconds: 30,
        runs_on: None,
    }
}

fn request(tab: &str, connection: &str, sql: &str, mode: RunMode) -> RunStatementRequest {
    RunStatementRequest {
        tab_id: tab.into(),
        connection_id: connection.into(),
        text: sql.into(),
        from: 0,
        to: 0,
        all: true,
        fetch_all: false,
        mode,
        parameters: Vec::new(),
        explain: None,
    }
}

async fn run(
    h: &Harness,
    tab: &str,
    connection: &str,
    sql: &str,
    mode: RunMode,
) -> Vec<StatementRun> {
    h.sessions
        .run(request(tab, connection, sql, mode))
        .await
        .unwrap()
}

fn cell(run: &StatementRun) -> Cell {
    match &run.result {
        StatementResult::Rows { rows, .. } => rows[0][0].clone(),
        other => panic!("expected rows, got {other:?}"),
    }
}

fn failed(run: &StatementRun) -> (DbFailureReason, String) {
    match &run.result {
        StatementResult::Failed { failure } => (failure.reason, failure.message.clone()),
        other => panic!("expected a failure, got {other:?}"),
    }
}

async fn count(h: &Harness, connection: &str) -> Cell {
    cell(
        &run(
            h,
            "counter",
            connection,
            "select count(*) from t",
            RunMode::ReadOnly,
        )
        .await[0],
    )
}

#[tokio::test]
async fn auto_commit_writes_and_says_which_statements_only_read() {
    let Some(server) = PgServer::start() else {
        return;
    };
    server.psql(
        "CREATE TABLE t (id int);
         CREATE PROCEDURE p() LANGUAGE plpgsql AS $$ BEGIN INSERT INTO t VALUES (100); COMMIT; END $$;",
    );
    let h = harness();
    let c = h
        .sessions
        .connections()
        .save(postgres(&server, DbAccess::ReadWrite))
        .await
        .unwrap();
    let runs = run(
        &h,
        "a",
        &c.id,
        "insert into t values (1), (2) returning id; select count(*) from t; vacuum t; call p()",
        RunMode::AutoCommit,
    )
    .await;
    assert_eq!(runs.len(), 4, "{runs:?}");
    assert!(!runs[0].ran_read_only);
    assert!(matches!(runs[0].result, StatementResult::Rows { ref rows, .. } if rows.len() == 2));
    assert!(runs[1].ran_read_only);
    assert_eq!(cell(&runs[1]), Cell::Number(2.0));
    assert!(
        matches!(runs[2].result, StatementResult::Command { .. }),
        "{:?}",
        runs[2].result
    );
    assert!(
        matches!(runs[3].result, StatementResult::Command { .. }),
        "{:?}",
        runs[3].result
    );
    assert_eq!(count(&h, &c.id).await, Cell::Number(3.0));
    assert!(runs.iter().all(|r| r.transaction.is_none()));

    // The same connection in a read-only tab still refuses writes.
    let refused = run(&h, "b", &c.id, "delete from t", RunMode::ReadOnly).await;
    assert_eq!(failed(&refused[0]).0, DbFailureReason::ReadOnly);

    // A read-only connection refuses a writable mode outright.
    let ro = h
        .sessions
        .connections()
        .save(postgres(&server, DbAccess::ReadOnly))
        .await
        .unwrap();
    let e = h
        .sessions
        .run(request("c", &ro.id, "select 1", RunMode::AutoCommit))
        .await
        .err()
        .unwrap();
    assert_eq!(e.code, ErrorCode::Validation);
}

#[tokio::test]
async fn manual_transactions_stay_open_until_commit_or_roll_back() {
    let Some(server) = PgServer::start() else {
        return;
    };
    server.psql("CREATE TABLE t (id int)");
    let h = harness();
    let c = h
        .sessions
        .connections()
        .save(postgres(&server, DbAccess::ReadWrite))
        .await
        .unwrap();
    let runs = run(
        &h,
        "m",
        &c.id,
        "insert into t values (1); insert into t values (2)",
        RunMode::Manual,
    )
    .await;
    let tx = runs[1].transaction.clone().unwrap();
    assert_eq!((tx.statements, tx.failed), (2, false));
    // Another tab has its own session and does not see the uncommitted rows.
    assert_eq!(count(&h, &c.id).await, Cell::Number(0.0));
    // Nothing closes a session holding a transaction for being idle.
    h.sessions.close_idle(Duration::ZERO);
    assert_eq!(h.sessions.open_transactions(), ["m"]);
    // The tab cannot leave Manual with a transaction open.
    let e = h
        .sessions
        .run(request("m", &c.id, "select 1", RunMode::AutoCommit))
        .await
        .err()
        .unwrap();
    assert_eq!(e.code, ErrorCode::Conflict);

    h.sessions.end_transaction("m", false).await.unwrap();
    assert_eq!(count(&h, &c.id).await, Cell::Number(0.0));
    assert!(h.sessions.open_transactions().is_empty());

    run(&h, "m", &c.id, "insert into t values (3)", RunMode::Manual).await;
    h.sessions.end_transaction("m", true).await.unwrap();
    assert_eq!(count(&h, &c.id).await, Cell::Number(1.0));

    // A failed statement marks the transaction failed; a typed ROLLBACK ends it.
    let runs = run(
        &h,
        "m",
        &c.id,
        "insert into t values ('x')",
        RunMode::Manual,
    )
    .await;
    assert!(runs[0].transaction.as_ref().unwrap().failed);
    let runs = run(&h, "m", &c.id, "select 1", RunMode::Manual).await;
    assert!(
        failed(&runs[0]).1.contains("aborted"),
        "{:?}",
        runs[0].result
    );
    let runs = run(&h, "m", &c.id, "rollback", RunMode::Manual).await;
    assert!(runs[0].transaction.is_none());

    // A lost connection says the server rolled the transaction back.
    run(&h, "m", &c.id, "insert into t values (4)", RunMode::Manual).await;
    server.psql("SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE application_name = 'Brainiac' AND state = 'idle in transaction'");
    tokio::time::sleep(Duration::from_millis(200)).await;
    let runs = run(&h, "m", &c.id, "insert into t values (5)", RunMode::Manual).await;
    let (reason, message) = failed(&runs[0]);
    assert_eq!(reason, DbFailureReason::Connection);
    assert!(
        message.contains("rolled the open transaction back"),
        "{message}"
    );
    assert_eq!(count(&h, &c.id).await, Cell::Number(1.0));
}

#[tokio::test]
async fn named_parameters_are_bound_as_values_never_pasted() {
    let Some(server) = PgServer::start() else {
        return;
    };
    server.psql("CREATE TABLE people (email text, age int); INSERT INTO people VALUES ('o''brien@example.com', 40)");
    let h = harness();
    let c = h
        .sessions
        .connections()
        .save(postgres(&server, DbAccess::ReadOnly))
        .await
        .unwrap();
    let sql = "select age + :years from people where email = :email and :email <> ''";
    let mut r = request("p", &c.id, sql, RunMode::ReadOnly);
    assert_eq!(h.sessions.parameters(&r).await.unwrap(), ["years", "email"]);
    let missing = h.sessions.run(r.clone()).await.unwrap();
    let (reason, message) = failed(&missing[0]);
    assert_eq!(reason, DbFailureReason::Parameters);
    assert!(
        message.contains(":years") && message.contains(":email"),
        "{message}"
    );

    r.parameters = vec![
        ParamValue {
            name: "years".into(),
            value: Some("2".into()),
        },
        ParamValue {
            name: "email".into(),
            value: Some("o'brien@example.com".into()),
        },
    ];
    let runs = h.sessions.run(r.clone()).await.unwrap();
    assert_eq!(cell(&runs[0]), Cell::Number(42.0));

    // The server parses the text with the parameter's type, and says so when it cannot.
    r.parameters[0].value = Some("abc".into());
    let runs = h.sessions.run(r.clone()).await.unwrap();
    let (reason, message) = failed(&runs[0]);
    assert_eq!(reason, DbFailureReason::Sql);
    assert!(message.contains("abc"), "{message}");

    // Error positions point into the text as written, with its :names.
    let mut r = request("p", &c.id, "select :a, nope from people", RunMode::ReadOnly);
    r.parameters = vec![ParamValue {
        name: "a".into(),
        value: Some("1".into()),
    }];
    let runs = h.sessions.run(r).await.unwrap();
    match &runs[0].result {
        StatementResult::Failed { failure } => assert_eq!(failure.position, Some(11)),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn explain_shows_the_plan_and_analyze_measures_it() {
    let Some(server) = PgServer::start() else {
        return;
    };
    server.psql("CREATE TABLE t (id int PRIMARY KEY, n int); INSERT INTO t SELECT g, g FROM generate_series(1, 1000) g; ANALYZE t;");
    let h = harness();
    let c = h
        .sessions
        .connections()
        .save(postgres(&server, DbAccess::ReadWrite))
        .await
        .unwrap();
    let mut r = request(
        "e",
        &c.id,
        "select * from t where id = 5",
        RunMode::ReadOnly,
    );
    r.explain = Some(ExplainMode::Plan);
    let runs = h.sessions.run(r.clone()).await.unwrap();
    match &runs[0].result {
        StatementResult::Plan {
            plan, execution_ms, ..
        } => {
            assert!(plan.label.contains("Scan"), "{}", plan.label);
            assert!(plan.label.contains(" on t"), "{}", plan.label);
            assert!(plan.total_cost.is_some());
            assert!(execution_ms.is_none());
        }
        other => panic!("{other:?}"),
    }
    r.explain = Some(ExplainMode::Analyze);
    let runs = h.sessions.run(r).await.unwrap();
    match &runs[0].result {
        StatementResult::Plan {
            plan, execution_ms, ..
        } => {
            assert!(execution_ms.is_some());
            assert_eq!(plan.actual_rows, Some(1.0));
        }
        other => panic!("{other:?}"),
    }
    // Explain of a write in an auto-commit tab does not run it.
    let mut w = request("e", &c.id, "delete from t", RunMode::AutoCommit);
    w.explain = Some(ExplainMode::Plan);
    h.sessions.run(w).await.unwrap();
    assert_eq!(
        cell(&run(&h, "x", &c.id, "select count(*) from t", RunMode::ReadOnly).await[0]),
        Cell::Number(1000.0)
    );
}

#[tokio::test]
async fn export_writes_every_row_and_refuses_writes() {
    let Some(server) = PgServer::start() else {
        return;
    };
    let h = harness();
    let c = h
        .sessions
        .connections()
        .save(postgres(&server, DbAccess::ReadWrite))
        .await
        .unwrap();
    let path = h.tmp.path().join("series.csv");
    let exported = h
        .sessions
        .export(ExportRequest {
            tab_id: "x".into(),
            connection_id: c.id.clone(),
            sql: "select g as n, 'row ' || g as label, 0.10::numeric(5,2) as amount from generate_series(1, 2500) g;".into(),
            parameters: Vec::new(),
            format: ExportFormat::Csv,
            path: path.display().to_string(),
        })
        .await
        .unwrap();
    assert_eq!(exported.rows, 2500);
    let text = std::fs::read_to_string(&path).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2501);
    assert_eq!(lines[0], "n,label,amount");
    assert_eq!(lines[2500], "2500,row 2500,0.10");

    let json = h.tmp.path().join("bad.json");
    let e = h
        .sessions
        .export(ExportRequest {
            tab_id: "x".into(),
            connection_id: c.id.clone(),
            sql: "create table nope (id int)".into(),
            parameters: Vec::new(),
            format: ExportFormat::Json,
            path: json.display().to_string(),
        })
        .await
        .err()
        .unwrap();
    assert!(!json.exists());
    assert!(!e.message.is_empty());
}

#[tokio::test]
async fn sqlite_writes_in_transactions_with_parameters() {
    let h = harness();
    let file = h.tmp.path().join("shop.db");
    rusqlite::Connection::open(&file)
        .unwrap()
        .execute_batch("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)")
        .unwrap();
    let c = h
        .sessions
        .connections()
        .save(sqlite(&file, DbAccess::ReadWrite))
        .await
        .unwrap();
    let mut r = request(
        "s",
        &c.id,
        "insert into t (name) values (:name)",
        RunMode::Manual,
    );
    r.parameters = vec![ParamValue {
        name: "name".into(),
        value: Some("O'Brien".into()),
    }];
    let runs = h.sessions.run(r).await.unwrap();
    assert_eq!(
        runs[0].transaction.as_ref().unwrap().statements,
        1,
        "{runs:?}"
    );
    h.sessions.end_transaction("s", false).await.unwrap();
    assert_eq!(count(&h, &c.id).await, Cell::Number(0.0));

    let mut r = request(
        "s",
        &c.id,
        "insert into t (name) values (:name)",
        RunMode::AutoCommit,
    );
    r.parameters = vec![ParamValue {
        name: "name".into(),
        value: Some("O'Brien".into()),
    }];
    let runs = h.sessions.run(r).await.unwrap();
    assert!(!runs[0].ran_read_only);
    let mut r = request(
        "s",
        &c.id,
        "select name from t where id = :id limit :n",
        RunMode::AutoCommit,
    );
    r.parameters = vec![
        ParamValue {
            name: "id".into(),
            value: Some("1".into()),
        },
        ParamValue {
            name: "n".into(),
            value: Some("5".into()),
        },
    ];
    let runs = h.sessions.run(r.clone()).await.unwrap();
    assert_eq!(cell(&runs[0]), Cell::Text("O'Brien".into()));
    assert!(runs[0].ran_read_only);

    r.explain = Some(ExplainMode::Plan);
    let runs = h.sessions.run(r).await.unwrap();
    match &runs[0].result {
        StatementResult::Plan { plan, .. } => assert!(!plan.children.is_empty()),
        other => panic!("{other:?}"),
    }

    let out = h.tmp.path().join("t.json");
    h.sessions
        .export(ExportRequest {
            tab_id: "s".into(),
            connection_id: c.id.clone(),
            sql: "select * from t".into(),
            parameters: Vec::new(),
            format: ExportFormat::Json,
            path: out.display().to_string(),
        })
        .await
        .unwrap();
    let parsed: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
    assert_eq!(parsed[0]["name"], "O'Brien");

    // Brainiac's own databases open read only, whatever is asked.
    let own = h.tmp.path().join(db::CORE_FILE);
    let e = h
        .sessions
        .connections()
        .save(sqlite(&own, DbAccess::ReadWrite))
        .await
        .err()
        .unwrap();
    assert!(e.message.contains("Brainiac's own databases"), "{e:?}");
}

#[tokio::test]
async fn saved_queries_keep_versions_parameters_and_lose_deleted_connections() {
    let h = harness();
    let queries = SavedQueryService::new(h.core.clone());
    let file = h.tmp.path().join("shop.db");
    rusqlite::Connection::open(&file)
        .unwrap()
        .execute_batch("CREATE TABLE t (id)")
        .unwrap();
    let c = h
        .sessions
        .connections()
        .save(sqlite(&file, DbAccess::ReadOnly))
        .await
        .unwrap();
    let save = |id: Option<String>, version: Option<i64>, name: &str| SaveQueryRequest {
        id,
        expected_version: version,
        name: name.into(),
        folder: " /Billing//Monthly/ ".into(),
        description: "Late invoices".into(),
        connection_id: Some(c.id.clone()),
        sql: "select * from t where id = :id".into(),
    };
    let saved = queries.save(save(None, None, "Late")).await.unwrap();
    assert_eq!(saved.folder, "Billing/Monthly");
    let updated = queries
        .save(save(Some(saved.id.clone()), Some(1), "Late v2"))
        .await
        .unwrap();
    assert_eq!(updated.version, 2);
    let stale = queries
        .save(save(Some(saved.id.clone()), Some(1), "Late v3"))
        .await
        .err()
        .unwrap();
    assert_eq!(stale.code, ErrorCode::Conflict);
    let remembered = queries
        .remember_parameters(
            &saved.id,
            vec![ParamValue {
                name: "id".into(),
                value: Some("7".into()),
            }],
        )
        .await
        .unwrap();
    assert_eq!(remembered.version, 2);
    assert_eq!(remembered.parameters[0].value.as_deref(), Some("7"));

    h.sessions
        .connections()
        .delete(&c.id, c.version)
        .await
        .unwrap();
    assert_eq!(queries.get(&saved.id).await.unwrap().connection_id, None);
    assert_eq!(
        queries.delete(&saved.id, 1).await.err().unwrap().code,
        ErrorCode::Conflict
    );
    queries.delete(&saved.id, 2).await.unwrap();
    assert!(queries.list().await.unwrap().is_empty());
}

#[tokio::test]
async fn history_records_runs_without_rows_and_tabs_survive() {
    let h = harness();
    let file = h.tmp.path().join("shop.db");
    rusqlite::Connection::open(&file)
        .unwrap()
        .execute_batch("CREATE TABLE t (secret TEXT); INSERT INTO t VALUES ('row value')")
        .unwrap();
    let c = h
        .sessions
        .connections()
        .save(sqlite(&file, DbAccess::ReadOnly))
        .await
        .unwrap();
    run(
        &h,
        "h",
        &c.id,
        "select secret from t; select * from nowhere",
        RunMode::ReadOnly,
    )
    .await;
    let history = h.sessions.history().unwrap();
    let entries = history.list(&c.id, "", 0, 50).await.unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[1].rows, Some(1));
    assert!(entries[0].error.as_deref().unwrap().contains("nowhere"));
    assert!(entries.iter().all(|e| !e.sql.contains("row value")));
    assert_eq!(history.list(&c.id, "secret", 0, 50).await.unwrap().len(), 1);
    history.clear(&c.id).await.unwrap();
    assert!(history.list(&c.id, "", 0, 50).await.unwrap().is_empty());

    h.sessions.set_history(false);
    run(&h, "h", &c.id, "select 1", RunMode::ReadOnly).await;
    assert!(history.list(&c.id, "", 0, 50).await.unwrap().is_empty());

    let tabs = vec![
        QueryTab {
            id: "one".into(),
            connection_id: Some(c.id.clone()),
            saved_query_id: None,
            title: "Untitled 1".into(),
            text: "select 1".into(),
            saved_version: None,
            dirty: true,
            mode: RunMode::Manual,
        },
        QueryTab {
            id: "two".into(),
            connection_id: None,
            saved_query_id: None,
            title: "Untitled 2".into(),
            text: String::new(),
            saved_version: None,
            dirty: false,
            mode: RunMode::ReadOnly,
        },
    ];
    history.save_tabs(tabs.clone()).await.unwrap();
    assert_eq!(history.tabs().await.unwrap(), tabs);
}
