//! The SQLite driver on files the tests create (docs/architecture.md,
//! Databases — v0.4): values, the row cap, read only that also refuses
//! `ATTACH` and `VACUUM INTO`, cancel, the time limit, and the schema.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use brainiac_lib::databases::sqlite::SqliteSession;
use brainiac_lib::models::{
    Cell, CellValue, ColumnKind, DbFailureReason, ErrorCode, RelationKind, RunMode, StatementResult,
};

fn fixture(dir: &Path) -> PathBuf {
    let path = dir.join("shop.db");
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch(
        "CREATE TABLE customers (id INTEGER PRIMARY KEY, email TEXT NOT NULL UNIQUE, joined DATE, meta JSON);
         CREATE TABLE orders (id INTEGER PRIMARY KEY, customer_id INTEGER REFERENCES customers (id), total NUMERIC(10, 2), note BLOB);
         CREATE INDEX orders_customer ON orders (customer_id);
         CREATE VIEW big_orders AS SELECT * FROM orders WHERE total > 100;
         INSERT INTO customers VALUES (1, 'a@example.com', '2026-10-04', '{\"vip\": true}');
         INSERT INTO orders VALUES (1, 1, 120.5, x'dead'), (2, 1, 9007199254740993, NULL);",
    )
    .unwrap();
    path
}

async fn open(path: &Path) -> SqliteSession {
    SqliteSession::open(path, Duration::from_secs(30), false)
        .await
        .unwrap()
}

fn rows(result: StatementResult) -> (Vec<(String, ColumnKind)>, Vec<Vec<Cell>>, bool) {
    match result {
        StatementResult::Rows {
            columns,
            rows,
            more,
        } => (
            columns.into_iter().map(|c| (c.type_name, c.kind)).collect(),
            rows,
            more,
        ),
        other => panic!("expected rows, got {other:?}"),
    }
}

fn failure(result: StatementResult) -> (DbFailureReason, String, Option<u32>) {
    match result {
        StatementResult::Failed { failure } => (failure.reason, failure.message, failure.position),
        other => panic!("expected a failure, got {other:?}"),
    }
}

#[tokio::test]
async fn values_and_column_kinds() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = open(&fixture(dir.path())).await;
    let (columns, rows, more) = rows(
        s.run("SELECT c.id, c.email, c.joined, c.meta, o.total, o.note, 1.5 AS ratio, NULL AS empty FROM customers c JOIN orders o ON o.customer_id = c.id ORDER BY o.id", &[], 100, RunMode::ReadOnly, None).await.map(|r| r.result)
            .unwrap(),
    );
    assert!(!more);
    let kinds: Vec<_> = columns.iter().map(|c| c.1).collect();
    assert_eq!(
        kinds,
        [
            ColumnKind::Number,
            ColumnKind::Text,
            ColumnKind::Temporal,
            ColumnKind::Json,
            ColumnKind::Numeric,
            ColumnKind::Bytes,
            ColumnKind::Number,
            ColumnKind::Text,
        ]
    );
    assert_eq!(rows[0][0], Cell::Number(1.0));
    assert_eq!(rows[0][1], Cell::Text("a@example.com".into()));
    assert_eq!(rows[0][4], Cell::Number(120.5));
    assert_eq!(
        rows[0][5],
        Cell::Value(CellValue::Bytes {
            size: 2,
            hex: "dead".into()
        })
    );
    assert_eq!(rows[0][7], Cell::Null);
    // Beyond 2^53 an integer is text, so JavaScript cannot round it.
    assert_eq!(rows[1][4], Cell::Text("9007199254740993".into()));
}

#[tokio::test]
async fn results_stop_at_the_cap() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = open(&fixture(dir.path())).await;
    let series = "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 5000) SELECT i FROM n";
    let (_, capped, more) = rows(
        s.run(series, &[], 1000, RunMode::ReadOnly, None)
            .await
            .map(|r| r.result)
            .unwrap(),
    );
    assert_eq!((capped.len(), more), (1000, true));
    let (_, all, more) = rows(
        s.run(series, &[], 5000, RunMode::ReadOnly, None)
            .await
            .map(|r| r.result)
            .unwrap(),
    );
    assert_eq!((all.len(), more), (5000, false));
}

#[tokio::test]
async fn read_only_refuses_writes_attach_and_vacuum_into() {
    let dir = tempfile::tempdir().unwrap();
    let path = fixture(dir.path());
    let mut s = open(&path).await;
    let outside = dir.path().join("copy.db");
    let writes = [
        "INSERT INTO customers (email) VALUES ('b@example.com')".to_string(),
        "UPDATE orders SET total = 0".to_string(),
        "DELETE FROM orders".to_string(),
        "CREATE TABLE t (id)".to_string(),
        "DROP VIEW big_orders".to_string(),
        "PRAGMA user_version = 5".to_string(),
        "PRAGMA query_only = OFF".to_string(),
        format!("ATTACH '{}' AS other", outside.display()),
        format!("VACUUM INTO '{}'", outside.display()),
        "VACUUM".to_string(),
    ];
    for sql in &writes {
        let result = s
            .run(sql, &[], 10, RunMode::ReadOnly, None)
            .await
            .map(|r| r.result)
            .unwrap();
        // `PRAGMA query_only = OFF` is itself read only; the write after it must still fail.
        if sql.starts_with("PRAGMA query_only") {
            let (reason, _, _) = failure(
                s.run("DELETE FROM orders", &[], 10, RunMode::ReadOnly, None)
                    .await
                    .map(|r| r.result)
                    .unwrap(),
            );
            assert_eq!(reason, DbFailureReason::ReadOnly);
            continue;
        }
        let (reason, message, _) = failure(result);
        assert_eq!(reason, DbFailureReason::ReadOnly, "{sql}: {message}");
        assert!(
            message.starts_with("This connection is read only"),
            "{message}"
        );
    }
    assert!(
        !outside.exists(),
        "a file was written outside the connection"
    );
    let check = rusqlite::Connection::open(&path).unwrap();
    let count: i64 = check
        .query_row("SELECT count(*) FROM orders", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 2);
}

#[tokio::test]
async fn cancel_and_the_time_limit_interrupt_a_long_statement() {
    let dir = tempfile::tempdir().unwrap();
    let path = fixture(dir.path());
    let endless =
        "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n) SELECT count(*) FROM n";
    let mut s = open(&path).await;
    let canceller = s.canceller();
    let started = Instant::now();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        canceller.cancel();
    });
    let (reason, _, _) = failure(
        s.run(endless, &[], 10, RunMode::ReadOnly, None)
            .await
            .map(|r| r.result)
            .unwrap(),
    );
    assert_eq!(reason, DbFailureReason::Cancelled);
    assert!(started.elapsed() < Duration::from_secs(3));
    // The session is still usable, and the cancel does not stick.
    let (_, one, _) = rows(
        s.run("SELECT 1", &[], 10, RunMode::ReadOnly, None)
            .await
            .map(|r| r.result)
            .unwrap(),
    );
    assert_eq!(one[0][0], Cell::Number(1.0));

    let mut short = SqliteSession::open(&path, Duration::from_millis(300), false)
        .await
        .unwrap();
    let (reason, message, _) = failure(
        short
            .run(endless, &[], 10, RunMode::ReadOnly, None)
            .await
            .map(|r| r.result)
            .unwrap(),
    );
    assert_eq!(reason, DbFailureReason::Timeout, "{message}");
}

#[tokio::test]
async fn errors_carry_sqlites_message_and_position() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = open(&fixture(dir.path())).await;
    let (reason, message, position) = failure(
        s.run("SELECT * FROM nowhere", &[], 10, RunMode::ReadOnly, None)
            .await
            .map(|r| r.result)
            .unwrap(),
    );
    assert_eq!(reason, DbFailureReason::Sql);
    assert!(message.contains("nowhere"), "{message}");
    // SQLite reports a position for syntax errors, not for a missing table.
    assert!(matches!(position, None | Some(14)), "{position:?}");
    let (_, message, position) = failure(
        s.run(
            "SELECT 1 FROM orders WHERE AND",
            &[],
            10,
            RunMode::ReadOnly,
            None,
        )
        .await
        .map(|r| r.result)
        .unwrap(),
    );
    assert!(message.contains("syntax error"), "{message}");
    assert!(position.is_some(), "no position for a syntax error");
}

#[tokio::test]
async fn opening_reports_missing_and_foreign_files() {
    let dir = tempfile::tempdir().unwrap();
    let missing = SqliteSession::open(&dir.path().join("nope.db"), Duration::from_secs(1), false)
        .await
        .err()
        .unwrap();
    assert_eq!(missing.code, ErrorCode::NotFound);
    assert!(!dir.path().join("nope.db").exists(), "a file was created");
    let text = dir.path().join("notes.txt");
    std::fs::write(
        &text,
        "this is not a database, just some text that is long enough",
    )
    .unwrap();
    let foreign = SqliteSession::open(&text, Duration::from_secs(1), false)
        .await
        .err()
        .unwrap();
    assert!(
        foreign.message.contains("is not a SQLite database"),
        "{foreign:?}"
    );
}

#[tokio::test]
async fn the_schema_lists_tables_views_columns_indexes_and_keys() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = open(&fixture(dir.path())).await;
    let (groups, default_schema) = s.schema().await.unwrap();
    assert_eq!(default_schema.as_deref(), Some("main"));
    let main = &groups[0];
    let names: Vec<_> = main.relations.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["big_orders", "customers", "orders"]);
    assert_eq!(main.relations[0].kind, RelationKind::View);
    let customers = &main.relations[1];
    assert!(customers.columns[0].primary_key);
    assert!(!customers.columns[1].nullable);
    assert!(customers.indexes.iter().any(|i| i.unique));
    let orders = &main.relations[2];
    assert_eq!(orders.indexes[0].definition, "(customer_id)");
    assert_eq!(
        orders.foreign_keys[0].definition,
        "FOREIGN KEY (customer_id) REFERENCES customers(id)"
    );
}
