//! The PostgreSQL driver against a throwaway server (docs/architecture.md,
//! Databases — v0.4; the v0.4 spike): values of every built-in type, the row
//! cap, read only that holds against statements that try to write, cancel,
//! timeouts, connection errors in words, TLS, and the schema.

mod postgres_server;

use std::time::{Duration, Instant};

use brainiac_lib::credentials::Secret;
use brainiac_lib::databases::postgres::{PgSession, PgTarget};
use brainiac_lib::models::{
    Cell, CellValue, ColumnKind, DbFailureReason, DbTls, ErrorCode, RelationKind, RunMode,
    StatementResult,
};
use postgres_server::PgServer;

async fn session(server: &PgServer) -> PgSession {
    PgSession::connect(&server.target()).await.unwrap()
}

async fn run(session: &mut PgSession, sql: &str) -> StatementResult {
    session
        .run(sql, &[], 1000, RunMode::ReadOnly, None)
        .await
        .map(|r| r.result)
        .expect("the connection was lost")
}

fn rows(result: StatementResult) -> (Vec<String>, Vec<Vec<Cell>>, bool) {
    match result {
        StatementResult::Rows {
            columns,
            rows,
            more,
        } => (
            columns.into_iter().map(|c| c.type_name).collect(),
            rows,
            more,
        ),
        other => panic!("expected rows, got {other:?}"),
    }
}

fn single(result: StatementResult) -> Cell {
    rows(result).1.remove(0).remove(0)
}

fn failure(result: StatementResult) -> (DbFailureReason, String) {
    match result {
        StatementResult::Failed { failure } => (failure.reason, failure.message),
        other => panic!("expected a failure, got {other:?}"),
    }
}

fn text(cell: &Cell) -> String {
    match cell {
        Cell::Null => "NULL".into(),
        Cell::Bool(b) => b.to_string(),
        Cell::Number(n) => n.to_string(),
        Cell::Text(t) => t.clone(),
        Cell::Value(v) => format!("{v:?}"),
    }
}

#[tokio::test]
async fn values_of_common_types_print_as_postgres_does() {
    let Some(server) = PgServer::start() else {
        return;
    };
    let mut s = session(&server).await;
    server.psql(
        "CREATE TYPE mood AS ENUM ('ok', 'happy');
         CREATE TYPE pair AS (a int, b text);
         CREATE DOMAIN positive AS int CHECK (VALUE > 0);",
    );
    let cases: &[(&str, &str)] = &[
        ("true", "true"),
        ("1::int2", "1"),
        ("2147483647::int4", "2147483647"),
        ("9007199254740993::int8", "9007199254740993"),
        ("1.5::float4", "1.5"),
        ("'NaN'::float8", "NaN"),
        ("123456789.000000001::numeric", "123456789.000000001"),
        ("0.00001::numeric", "0.00001"),
        ("(-12.340)::numeric(10,3)", "-12.340"),
        ("'NaN'::numeric", "NaN"),
        ("'Infinity'::numeric", "Infinity"),
        ("12.34::money", "12.34"),
        ("'héllo'::text", "héllo"),
        ("'a'::char(3)", "a  "),
        ("'x'::\"char\"", "x"),
        ("'n'::name", "n"),
        ("'{\"a\": [1, 2]}'::json", "{\"a\": [1, 2]}"),
        ("'{\"a\": [1, 2]}'::jsonb", "{\"a\": [1, 2]}"),
        (
            "'a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11'::uuid",
            "a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11",
        ),
        ("'2026-10-04'::date", "2026-10-04"),
        ("'infinity'::date", "infinity"),
        ("'0044-03-15 BC'::date", "0044-03-15 BC"),
        ("'14:02:03.25'::time", "14:02:03.25"),
        ("'14:02:03+05:30'::timetz", "14:02:03+05:30"),
        (
            "'2026-10-04 14:02:03.123456'::timestamp",
            "2026-10-04 14:02:03.123456",
        ),
        ("'-infinity'::timestamp", "-infinity"),
        (
            "'1 year 2 mons 3 days 04:05:06'::interval",
            "1 year 2 mons 3 days 04:05:06",
        ),
        ("'-1 day 2 hours'::interval", "-1 days +02:00:00"),
        ("'192.168.0.1/24'::inet", "192.168.0.1/24"),
        ("'10.0.0.1'::inet", "10.0.0.1"),
        ("'10.0.0.0/8'::cidr", "10.0.0.0/8"),
        ("'::1'::inet", "::1"),
        ("'08:00:2b:01:02:03'::macaddr", "08:00:2b:01:02:03"),
        ("B'10101'::bit(5)", "10101"),
        ("B'101'::varbit", "101"),
        ("'16/B374D848'::pg_lsn", "16/B374D848"),
        ("'(1,2.5)'::point", "(1,2.5)"),
        ("'<a/>'::xml", "<a/>"),
        ("'happy'::mood", "happy"),
        ("ROW(1, 'two words')::pair", "(1,\"two words\")"),
        ("5::positive", "5"),
        ("ARRAY[1, NULL, 3]", "{1,NULL,3}"),
        (
            "ARRAY[['a', 'b c'], ['', 'd\"e']]",
            "{{a,\"b c\"},{\"\",\"d\\\"e\"}}",
        ),
        ("'{}'::int[]", "{}"),
        ("ARRAY['2026-10-04'::date]", "{2026-10-04}"),
        ("int4range(1, 10)", "[1,10)"),
        ("'{[1,3), [5,7)}'::int4multirange", "{[1,3),[5,7)}"),
        ("'(0,1)'::tid", "(0,1)"),
        ("'42'::xid8", "42"),
        ("'empty'::int4range", "empty"),
        ("tstzrange(NULL, NULL)", "(,)"),
        ("NULL::text", "NULL"),
    ];
    for (expression, expected) in cases {
        let cell = single(run(&mut s, &format!("SELECT {expression}")).await);
        assert_eq!(text(&cell), *expected, "SELECT {expression}");
    }
    match single(run(&mut s, "SELECT '\\xdead'::bytea").await) {
        Cell::Value(CellValue::Bytes { size, hex }) => {
            assert_eq!((size, hex.as_str()), (2, "dead"))
        }
        other => panic!("bytea: {other:?}"),
    }
    // timestamptz is shown in the Mac's time zone, with its offset.
    let shown = text(&single(
        run(&mut s, "SELECT '2026-10-04 12:00:00+00'::timestamptz").await,
    ));
    assert!(shown.starts_with("2026-10-04 "), "{shown}");
    assert!(shown.contains('+') || shown.contains('-'), "{shown}");
}

/// The spike's sweep: every built-in base type, from a value of it built
/// from the first sample its input function accepts. Each one is either shown
/// as text or labelled as a type to cast, and none breaks the session.
#[tokio::test]
async fn every_builtin_type_is_shown_or_labelled() {
    let Some(server) = PgServer::start() else {
        return;
    };
    let mut s = session(&server).await;
    let (_, types, _) = rows(
        run(
            &mut s,
            "SELECT typname::text FROM pg_type
              WHERE typtype IN ('b', 'r', 'm') AND oid < 10000 AND typisdefined AND typname NOT LIKE '\\_%'
                AND typname NOT IN ('any', 'unknown') ORDER BY 1",
        )
        .await,
    );
    let samples = [
        "0",
        "1",
        "t",
        "a",
        "{}",
        "2026-10-04",
        "12:00",
        "(0,0)",
        "((0,0),(1,1))",
        "<(0,0),1>",
        "[(0,0),(1,1)]",
        "{1,2,3}",
        "0/0",
        "08:00:2b:01:02:03",
        "08:00:2b:01:02:03:04:05",
        "127.0.0.1",
        "[1,2]",
        "{[1,2]}",
        "a & b",
        "'a'",
        "<a/>",
        "$.a",
        "{\"a\":1}",
        "00000000-0000-0000-0000-000000000000",
        "1 day",
        "12:00+02",
        "2026-10-04 12:00+00",
        "1:1:0",
        "pg_class",
        "int4",
        "=",
        "english",
        "simple",
        "0",
        "10:10:",
    ];
    let mut labelled = Vec::new();
    let mut reached = 0;
    for row in types {
        let Cell::Text(name) = &row[0] else { continue };
        // A type no sample parses for is internal, such as `cstring`.
        for sample in samples {
            let sql = format!("SELECT '{sample}'::{name}");
            match run(&mut s, &sql).await {
                StatementResult::Rows { rows, .. } => {
                    if let Cell::Value(CellValue::Other { .. }) = &rows[0][0] {
                        labelled.push(name.clone());
                    }
                    reached += 1;
                    break;
                }
                StatementResult::Failed { .. } => continue,
                other => panic!("{sql} gave {other:?}"),
            }
        }
    }
    assert!(reached > 60, "only {reached} types had a value to check");
    // Types shown with "cast it to text" instead of their value. Anything
    // else falling here is a regression in values.rs.
    let allowed = [
        "line",
        "path",
        "pg_snapshot",
        "polygon",
        "regcollation",
        "regconfig",
        "regdictionary",
        "regnamespace",
        "regoper",
        "regoperator",
        "regprocedure",
        "regrole",
        "tsquery",
        "tsvector",
        "txid_snapshot",
    ];
    let unexpected: Vec<_> = labelled
        .iter()
        .filter(|t| !allowed.contains(&t.as_str()))
        .collect();
    eprintln!("types shown as 'cast to text': {labelled:?}");
    assert!(unexpected.is_empty(), "not decoded: {unexpected:?}");
    // The session is still usable after all of that.
    assert_eq!(text(&single(run(&mut s, "SELECT 1").await)), "1");
}

#[tokio::test]
async fn results_stop_at_the_cap_and_say_more_exist() {
    let Some(server) = PgServer::start() else {
        return;
    };
    let mut s = session(&server).await;
    let (_, capped, more) = rows(
        s.run(
            "SELECT generate_series(1, 5000)",
            &[],
            1000,
            RunMode::ReadOnly,
            None,
        )
        .await
        .map(|r| r.result)
        .unwrap(),
    );
    assert_eq!((capped.len(), more), (1000, true));
    let (_, all, more) = rows(
        s.run(
            "SELECT generate_series(1, 1000)",
            &[],
            1000,
            RunMode::ReadOnly,
            None,
        )
        .await
        .map(|r| r.result)
        .unwrap(),
    );
    assert_eq!((all.len(), more), (1000, false));
    let (types, _, _) = rows(
        s.run(
            "SELECT 1 AS a, 'x'::text AS b",
            &[],
            10,
            RunMode::ReadOnly,
            None,
        )
        .await
        .map(|r| r.result)
        .unwrap(),
    );
    assert_eq!(types, ["int4", "text"]);
}

/// The row cap uses a portal's row limit. A data-modifying statement with
/// `RETURNING` runs to completion on the first fetch, so capping a write never
/// cuts it short (checked with `tokio-postgres` directly, since the driver's
/// sessions are read only).
#[tokio::test]
async fn a_capped_portal_runs_a_returning_write_to_completion() {
    let Some(server) = PgServer::start() else {
        return;
    };
    server.psql("CREATE TABLE t (id int)");
    let (mut client, connection) = tokio_postgres::connect(
        &format!(
            "host=127.0.0.1 port={} user=postgres password='{}' dbname=postgres",
            server.port,
            postgres_server::PASSWORD
        ),
        tokio_postgres::NoTls,
    )
    .await
    .unwrap();
    tokio::spawn(connection);
    let tx = client.transaction().await.unwrap();
    let statement = tx
        .prepare("INSERT INTO t SELECT generate_series(1, 10) RETURNING id")
        .await
        .unwrap();
    let portal = tx.bind(&statement, &[]).await.unwrap();
    let first = tx.query_portal(&portal, 3).await.unwrap();
    assert_eq!(first.len(), 3);
    drop(portal);
    tx.commit().await.unwrap();
    let count: i64 = client
        .query_one("SELECT count(*) FROM t", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(count, 10);
}

#[tokio::test]
async fn read_only_holds_against_statements_that_try_to_write() {
    let Some(server) = PgServer::start() else {
        return;
    };
    server.psql(
        "CREATE TABLE t (id int);
         INSERT INTO t VALUES (1);
         CREATE SEQUENCE s;
         CREATE PROCEDURE p() LANGUAGE plpgsql AS $$ BEGIN INSERT INTO t VALUES (99); COMMIT; END $$;
         CREATE PROCEDURE q() LANGUAGE plpgsql AS $$ BEGIN COMMIT; INSERT INTO t VALUES (98); END $$;",
    );
    let mut s = session(&server).await;
    let writes = [
        "INSERT INTO t VALUES (2)",
        "UPDATE t SET id = 3",
        "DELETE FROM t",
        "CREATE TABLE u (id int)",
        "DROP TABLE t",
        "TRUNCATE t",
        "SELECT * FROM t FOR UPDATE",
        "SELECT nextval('s')",
        "DO $$ BEGIN INSERT INTO t VALUES (4); END $$",
        "CALL p()",
        "CALL q()",
        "COPY t FROM '/dev/null'",
        "WITH x AS (INSERT INTO t VALUES (5) RETURNING id) SELECT * FROM x",
        "SELECT set_config('transaction_read_only', 'off', true)",
    ];
    for sql in writes {
        let result = run(&mut s, sql).await;
        assert!(
            matches!(result, StatementResult::Failed { .. }),
            "{sql} was allowed: {result:?}"
        );
    }
    // Statements that change the session's or a transaction's mode run, and
    // the next statement is still read only, in its own read-only transaction.
    let switches = [
        "SET TRANSACTION READ WRITE",
        "START TRANSACTION READ WRITE",
        "BEGIN",
        "COMMIT",
        "SET SESSION CHARACTERISTICS AS TRANSACTION READ WRITE",
        "SET default_transaction_read_only = off",
        "SELECT set_config('default_transaction_read_only', 'off', false)",
    ];
    for sql in switches {
        let _ = run(&mut s, sql).await;
        let (reason, message) = failure(run(&mut s, "INSERT INTO t VALUES (6)").await);
        assert_eq!(reason, DbFailureReason::ReadOnly, "after {sql}: {message}");
        assert!(
            message.starts_with("This connection is read only"),
            "{message}"
        );
    }
    let count = single(run(&mut s, "SELECT count(*) FROM t").await);
    assert_eq!(count, Cell::Number(1.0));
    // A `COPY … TO STDOUT` is refused cleanly and leaves the session usable.
    let copy = run(&mut s, "COPY t TO STDOUT").await;
    assert!(matches!(copy, StatementResult::Failed { .. }), "{copy:?}");
    assert_eq!(single(run(&mut s, "SELECT 7").await), Cell::Number(7.0));
}

#[tokio::test]
async fn cancel_and_timeout_stop_the_statement_on_the_server() {
    let Some(server) = PgServer::start() else {
        return;
    };
    let mut s = session(&server).await;
    let canceller = s.canceller();
    let started = Instant::now();
    let cancelling = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        let sent = Instant::now();
        canceller.cancel().await;
        sent
    });
    let result = run(&mut s, "SELECT pg_sleep(30)").await;
    let sent = cancelling.await.unwrap();
    let latency = sent.elapsed();
    eprintln!("cancel latency: {latency:?}");
    assert_eq!(failure(result).0, DbFailureReason::Cancelled);
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(latency < Duration::from_secs(1), "{latency:?}");
    // The session is still usable.
    assert_eq!(single(run(&mut s, "SELECT 1").await), Cell::Number(1.0));

    let mut target = server.target();
    target.statement_timeout = Duration::from_secs(1);
    let mut short = PgSession::connect(&target).await.unwrap();
    let (reason, message) = failure(
        short
            .run("SELECT pg_sleep(5)", &[], 10, RunMode::ReadOnly, None)
            .await
            .map(|r| r.result)
            .unwrap(),
    );
    assert_eq!(reason, DbFailureReason::Timeout, "{message}");
}

#[tokio::test]
async fn errors_carry_the_database_message_and_position() {
    let Some(server) = PgServer::start() else {
        return;
    };
    let mut s = session(&server).await;
    match run(&mut s, "SELECT * FROM nowhere").await {
        StatementResult::Failed { failure } => {
            assert_eq!(failure.reason, DbFailureReason::Sql);
            assert_eq!(failure.code.as_deref(), Some("42P01"));
            assert!(failure.message.contains("nowhere"), "{}", failure.message);
            // A byte offset into the statement: where `nowhere` starts.
            assert_eq!(failure.position, Some(14));
        }
        other => panic!("{other:?}"),
    }
    let (reason, message) = failure(run(&mut s, "SELECT $1").await);
    assert_eq!(reason, DbFailureReason::Parameters, "{message}");
    match run(&mut s, "SET search_path = public").await {
        StatementResult::Command { tag } => assert_eq!(tag, "SET"),
        other => panic!("{other:?}"),
    }
    // A failure does not leave the session in an aborted transaction.
    assert_eq!(single(run(&mut s, "SELECT 2").await), Cell::Number(2.0));
}

#[tokio::test]
async fn a_lost_connection_is_reported_as_lost() {
    let Some(server) = PgServer::start() else {
        return;
    };
    let mut s = session(&server).await;
    server.psql("SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE application_name = 'Brainiac'");
    // The next statement finds the connection gone.
    let mut lost = false;
    for _ in 0..3 {
        if s.run("SELECT 1", &[], 10, RunMode::ReadOnly, None)
            .await
            .map(|r| r.result)
            .is_err()
        {
            lost = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(lost && s.is_closed());
}

#[tokio::test]
async fn connection_errors_say_what_went_wrong() {
    let Some(server) = PgServer::start() else {
        return;
    };
    let mut wrong = server.target();
    wrong.password = Some(Secret::new("not it").unwrap());
    let e = PgSession::connect(&wrong).await.err().unwrap();
    assert_eq!(e.code, ErrorCode::Unauthenticated, "{e:?}");
    assert!(
        e.message.contains("refused the password for user postgres"),
        "{}",
        e.message
    );

    let mut missing = server.target();
    missing.database = "nope".into();
    let e = PgSession::connect(&missing).await.err().unwrap();
    assert_eq!(e.code, ErrorCode::NotFound);
    assert!(
        e.message.contains("The database nope does not exist"),
        "{}",
        e.message
    );

    let mut closed = server.target();
    closed.port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let e = PgSession::connect(&closed).await.err().unwrap();
    assert!(e.message.contains("Nothing is listening"), "{}", e.message);

    let mut tls = server.target();
    tls.tls = DbTls::Require;
    let e = PgSession::connect(&tls).await.err().unwrap();
    assert!(e.message.contains("does not offer TLS"), "{}", e.message);
}

#[tokio::test]
async fn tls_verifies_with_the_ca_file_or_encrypts_without_verifying() {
    let Some(server) = PgServer::start_tls() else {
        return;
    };
    let ca = server.ca_file.clone();
    let target = |host: &str, tls: DbTls, ca_file: bool| PgTarget {
        host: host.into(),
        tls,
        ca_file: if ca_file { ca.clone() } else { None },
        ..server.target()
    };

    let verified = PgSession::connect(&target("localhost", DbTls::Verify, true)).await;
    assert!(verified.is_ok(), "{:?}", verified.err());

    let untrusted = PgSession::connect(&target("localhost", DbTls::Verify, false))
        .await
        .err()
        .unwrap();
    assert!(
        untrusted.message.contains("does not trust the certificate"),
        "{untrusted:?}"
    );

    let wrong_name = PgSession::connect(&target("127.0.0.1", DbTls::Verify, true))
        .await
        .err()
        .unwrap();
    assert!(
        wrong_name.message.contains("another host name")
            || wrong_name.message.contains("could not be verified"),
        "{wrong_name:?}"
    );

    let encrypted = PgSession::connect(&target("127.0.0.1", DbTls::Require, false)).await;
    assert!(encrypted.is_ok(), "{:?}", encrypted.err());
    let mut s = encrypted.unwrap();
    let ssl = single(
        s.run(
            "SELECT ssl FROM pg_stat_ssl WHERE pid = pg_backend_pid()",
            &[],
            1,
            RunMode::ReadOnly,
            None,
        )
        .await
        .unwrap()
        .result,
    );
    assert_eq!(ssl, Cell::Bool(true));

    let plain = PgSession::connect(&target("127.0.0.1", DbTls::Off, false)).await;
    assert!(plain.is_ok(), "{:?}", plain.err());
}

#[tokio::test]
async fn the_schema_lists_relations_columns_indexes_and_keys() {
    let Some(server) = PgServer::start() else {
        return;
    };
    server.psql(
        "CREATE SCHEMA billing;
         CREATE TABLE billing.customers (id serial PRIMARY KEY, email text NOT NULL UNIQUE);
         CREATE TABLE billing.invoices (id int PRIMARY KEY, customer_id int REFERENCES billing.customers (id), due_on date DEFAULT now());
         CREATE VIEW billing.open_invoices AS SELECT * FROM billing.invoices;
         CREATE SCHEMA empty;
         ANALYZE;",
    );
    let mut s = session(&server).await;
    let (groups, default_schema) = s.schema().await.unwrap();
    assert_eq!(default_schema.as_deref(), Some("public"));
    let names: Vec<_> = groups.iter().map(|g| g.name.as_str()).collect();
    assert_eq!(names, ["billing", "empty", "public"]);
    let billing = &groups[0];
    let invoices = billing
        .relations
        .iter()
        .find(|r| r.name == "invoices")
        .unwrap();
    assert_eq!(invoices.kind, RelationKind::Table);
    let columns: Vec<_> = invoices
        .columns
        .iter()
        .map(|c| {
            (
                c.name.as_str(),
                c.type_name.as_str(),
                c.nullable,
                c.primary_key,
            )
        })
        .collect();
    assert_eq!(
        columns,
        [
            ("id", "integer", false, true),
            ("customer_id", "integer", true, false),
            ("due_on", "date", true, false),
        ]
    );
    assert_eq!(invoices.columns[2].default.as_deref(), Some("now()"));
    assert!(invoices.indexes.iter().any(|i| i.primary));
    assert_eq!(invoices.foreign_keys.len(), 1);
    assert!(invoices.foreign_keys[0]
        .definition
        .contains("REFERENCES billing.customers(id)"));
    let view = billing
        .relations
        .iter()
        .find(|r| r.name == "open_invoices")
        .unwrap();
    assert_eq!(view.kind, RelationKind::View);
    let customers = billing
        .relations
        .iter()
        .find(|r| r.name == "customers")
        .unwrap();
    assert!(customers.indexes.iter().any(|i| i.unique && !i.primary));
    let _ = ColumnKind::Text;
}
