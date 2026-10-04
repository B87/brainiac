//! Health (SPEC.md, Databases: Health) against a throwaway PostgreSQL
//! server, with local stand-ins for Docker's socket and Google's APIs.

mod postgres_server;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use brainiac_lib::credentials::MemorySecrets;
use brainiac_lib::databases::health::{GoogleConfig, HealthService};
use brainiac_lib::databases::ConnectionService;
use brainiac_lib::db::{self, Db};
use brainiac_lib::models::{
    DbAccess, DbEnvironment, DbKind, DbPassword, DbTls, ErrorCode, HealthEvent, RunsOn,
    SaveDbConnectionRequest,
};
use postgres_server::PgServer;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Harness {
    tmp: tempfile::TempDir,
    connections: Arc<ConnectionService>,
    health: Arc<HealthService>,
    events: Arc<Mutex<Vec<HealthEvent>>>,
}

fn harness(google: GoogleConfig) -> Harness {
    let tmp = tempfile::tempdir().unwrap();
    let core = Db::open(&tmp.path().join(db::CORE_FILE)).unwrap();
    let connections = Arc::new(ConnectionService::new(
        core,
        Arc::new(MemorySecrets::default()),
    ));
    let events = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&events);
    let health = Arc::new(HealthService::new(
        Arc::clone(&connections),
        Arc::new(move |e| seen.lock().unwrap().push(e)),
        google,
    ));
    Harness {
        tmp,
        connections,
        health,
        events,
    }
}

fn connection(
    server: &PgServer,
    access: DbAccess,
    runs_on: Option<RunsOn>,
) -> SaveDbConnectionRequest {
    SaveDbConnectionRequest {
        id: None,
        expected_version: None,
        name: "Billing".into(),
        kind: DbKind::Postgres,
        environment: DbEnvironment::Production,
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
        runs_on,
    }
}

async fn client(server: &PgServer, name: &str) -> tokio_postgres::Client {
    let (client, connection) = tokio_postgres::connect(
        &format!(
            "host=127.0.0.1 port={} user=postgres password='{}' dbname=postgres application_name={name}",
            server.port,
            postgres_server::PASSWORD
        ),
        tokio_postgres::NoTls,
    )
    .await
    .unwrap();
    tokio::spawn(connection);
    client
}

#[tokio::test]
async fn health_shows_connections_sessions_locks_and_tables() {
    let Some(server) = PgServer::start() else {
        return;
    };
    server.psql("CREATE TABLE invoices (id int); INSERT INTO invoices SELECT generate_series(1, 500); DELETE FROM invoices WHERE id <= 100; ANALYZE invoices;");
    let h = harness(GoogleConfig::production());
    let c = h
        .connections
        .save(connection(&server, DbAccess::ReadOnly, None))
        .await
        .unwrap();

    // One session holds a lock in an open transaction; another waits for it.
    let holder = client(&server, "billing-worker").await;
    holder
        .batch_execute("BEGIN; LOCK TABLE invoices IN ACCESS EXCLUSIVE MODE;")
        .await
        .unwrap();
    let waiter = client(&server, "report").await;
    let waiting =
        tokio::spawn(async move { waiter.batch_execute("SELECT count(*) FROM invoices").await });
    tokio::time::sleep(Duration::from_millis(300)).await;

    let snapshot = h.health.start(&c.id).await.unwrap();
    let sample = snapshot.latest.unwrap();
    assert_eq!(snapshot.points.len(), 1);
    assert_eq!(sample.problem, None);
    assert_eq!(sample.max_connections, 100);
    assert!(sample.total_connections >= 3, "{sample:?}");
    assert_eq!(sample.idle_in_transaction, 1);
    assert!(sample.sees_all);
    assert!(sample.longest_transaction_seconds.is_some());
    // The idle-in-transaction session comes first; the waiting one names it.
    assert_eq!(sample.sessions[0].application, "billing-worker");
    let holder_pid = sample.sessions[0].pid;
    let report = sample
        .sessions
        .iter()
        .find(|s| s.application == "report")
        .unwrap();
    assert_eq!(report.blocked_by, [holder_pid]);
    assert!(report.wait.as_deref().unwrap().starts_with("Lock"));
    assert!(report.query.as_deref().unwrap().contains("count(*)"));
    // Health's own session is not listed.
    assert!(sample
        .sessions
        .iter()
        .all(|s| s.application != "Brainiac health"));
    assert_eq!(sample.statements, None);
    let invoices = sample.tables.iter().find(|t| t.name == "invoices").unwrap();
    assert!(invoices.total_bytes > 0);
    assert_eq!(h.events.lock().unwrap().len(), 1);

    // Cancelling another session needs a read-and-write connection.
    let refused = h
        .health
        .signal_backend(&c.id, report.pid, false)
        .await
        .err()
        .unwrap();
    assert_eq!(refused.code, ErrorCode::PermissionDenied);
    let rw = h
        .connections
        .save(connection(&server, DbAccess::ReadWrite, None))
        .await
        .unwrap();
    assert!(h
        .health
        .signal_backend(&rw.id, report.pid, false)
        .await
        .unwrap());
    let cancelled = waiting.await.unwrap();
    assert!(
        cancelled.is_err(),
        "the waiting statement was not cancelled"
    );
    holder.batch_execute("ROLLBACK").await.unwrap();

    h.health.stop(&c.id);
    // A SQLite connection has no Health.
    let file = h.tmp.path().join("x.db");
    rusqlite::Connection::open(&file)
        .unwrap()
        .execute_batch("CREATE TABLE t (id)")
        .unwrap();
    let mut sqlite = connection(&server, DbAccess::ReadOnly, None);
    sqlite.kind = DbKind::Sqlite;
    sqlite.file_path = Some(file.display().to_string());
    let s = h.connections.save(sqlite).await.unwrap();
    assert_eq!(
        h.health.start(&s.id).await.err().unwrap().code,
        ErrorCode::Validation
    );
}

/// Serves HTTP over a Unix socket the way Docker does, from canned stats.
async fn fake_docker(dir: &std::path::Path) -> (String, Arc<Mutex<Vec<String>>>) {
    let path = dir.join("docker.sock");
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&requests);
    tokio::spawn(async move {
        let mut call = 0u64;
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let mut head = Vec::new();
            let mut buf = [0u8; 2048];
            while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = socket.read(&mut buf).await.unwrap();
                if n == 0 {
                    break;
                }
                head.extend_from_slice(&buf[..n]);
            }
            let line = String::from_utf8_lossy(&head)
                .lines()
                .next()
                .unwrap_or_default()
                .to_string();
            seen.lock().unwrap().push(line.clone());
            call += 1;
            let body = if line.contains("/containers/json") {
                r#"[{"Id":"abcdef1234567890","Names":["/billing-db"],"Image":"postgres:16","Ports":[{"PrivatePort":5432,"PublicPort":5433,"Type":"tcp"}]}]"#.to_string()
            } else {
                format!(
                    r#"{{"memory_stats":{{"usage":{},"limit":1073741824,"stats":{{"inactive_file":104857600}}}},"cpu_stats":{{"cpu_usage":{{"total_usage":{}}},"system_cpu_usage":{},"online_cpus":4}},"blkio_stats":{{"io_service_bytes_recursive":[{{"op":"read","value":{}}},{{"op":"write","value":100}}]}}}}"#,
                    629145600,
                    1_000_000 * call,
                    4_000_000 * call,
                    1000 * call
                )
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(response.as_bytes()).await;
        }
    });
    (path.display().to_string(), requests)
}

#[tokio::test]
async fn docker_containers_report_memory_cpu_and_io_with_get_only() {
    let Some(server) = PgServer::start() else {
        return;
    };
    let h = harness(GoogleConfig::production());
    let (socket, requests) = fake_docker(h.tmp.path()).await;
    let listed = brainiac_lib::databases::health::list_containers(Some(&socket))
        .await
        .unwrap();
    assert_eq!(listed.containers[0].name, "billing-db");
    assert_eq!(listed.containers[0].ports, ["5433→5432"]);

    let runs_on = RunsOn::Docker {
        container: "billing-db".into(),
        socket: Some(socket),
    };
    let c = h
        .connections
        .save(connection(&server, DbAccess::ReadOnly, Some(runs_on)))
        .await
        .unwrap();
    let first = h.health.start(&c.id).await.unwrap().latest.unwrap();
    let machine = first.machine.unwrap();
    assert_eq!(machine.problem, None);
    assert_eq!(machine.source, "Docker");
    // Docker's usage minus the reclaimable page cache: 600 MiB - 100 MiB.
    assert_eq!(machine.memory_used, Some(524_288_000));
    assert_eq!(machine.memory_limit, Some(1_073_741_824));
    assert!(
        machine.cpu_ratio.is_none(),
        "the first sample has nothing to compare with"
    );
    h.health.stop(&c.id);
    // A second look compares with the first.
    let second = h.health.start(&c.id).await.unwrap();
    let machine = second.latest.unwrap().machine.unwrap();
    assert_eq!(machine.cpu_ratio, Some(0.25));
    assert!(machine.disk_read_rate.unwrap() > 0.0);
    assert_eq!(second.points.len(), 2);
    h.health.stop(&c.id);
    assert!(requests
        .lock()
        .unwrap()
        .iter()
        .all(|r| r.starts_with("GET ")));
}

/// Answers Google's token endpoint and Cloud Monitoring's timeSeries.
async fn fake_google() -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&requests);
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let mut head = Vec::new();
            let mut buf = [0u8; 4096];
            loop {
                let n = socket.read(&mut buf).await.unwrap_or(0);
                if n == 0 {
                    break;
                }
                head.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&head);
                if let Some(end) = text.find("\r\n\r\n") {
                    let length = text
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length: ")
                                .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                        })
                        .unwrap_or(0);
                    if head.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            let text = String::from_utf8_lossy(&head).to_string();
            seen.lock().unwrap().push(text.clone());
            let body = if text.starts_with("POST /token") {
                r#"{"access_token":"ya29.test","expires_in":3600}"#.to_string()
            } else {
                let value = if text.contains("memory%2Futilization") {
                    r#"{"doubleValue":0.62}"#
                } else if text.contains("memory%2Fquota") {
                    r#"{"int64Value":"4294967296"}"#
                } else if text.contains("cpu%2Futilization") {
                    r#"{"doubleValue":0.31}"#
                } else if text.contains("disk%2Fbytes_used") {
                    r#"{"int64Value":"1073741824"}"#
                } else {
                    r#"{"int64Value":"10737418240"}"#
                };
                format!(r#"{{"timeSeries":[{{"points":[{{"value":{value}}}]}}]}}"#)
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(response.as_bytes()).await;
        }
    });
    (base, requests)
}

#[tokio::test]
async fn cloud_sql_reads_cloud_monitoring_with_gcloud_credentials() {
    let Some(server) = PgServer::start() else {
        return;
    };
    let (base, requests) = fake_google().await;
    let dir = tempfile::tempdir().unwrap();
    let credentials = dir.path().join("adc.json");
    std::fs::write(
        &credentials,
        r#"{"type":"authorized_user","client_id":"id.apps","client_secret":"s3cret","refresh_token":"1//refresh"}"#,
    )
    .unwrap();
    let h = harness(GoogleConfig {
        token_url: format!("{base}/token"),
        monitoring_url: base,
        credentials: Some(credentials.clone()),
    });
    let runs_on = RunsOn::CloudSql {
        project: "acme-prod".into(),
        instance: "billing".into(),
    };
    let c = h
        .connections
        .save(connection(&server, DbAccess::ReadOnly, Some(runs_on)))
        .await
        .unwrap();
    let machine = h
        .health
        .start(&c.id)
        .await
        .unwrap()
        .latest
        .unwrap()
        .machine
        .unwrap();
    assert_eq!(machine.problem, None);
    assert!(machine.source.starts_with("Cloud Monitoring"));
    assert_eq!(machine.memory_ratio, Some(0.62));
    assert_eq!(machine.memory_limit, Some(4_294_967_296));
    assert_eq!(machine.cpu_ratio, Some(0.31));
    assert_eq!(machine.disk_used, Some(1_073_741_824));
    assert_eq!(machine.disk_quota, Some(10_737_418_240));
    h.health.stop(&c.id);
    {
        let seen = requests.lock().unwrap();
        assert!(seen[0].contains("refresh_token=1%2F%2Frefresh"));
        assert!(
            seen[1].contains("authorization: Bearer ya29.test")
                || seen[1].contains("Authorization: Bearer ya29.test")
        );
        assert!(seen[1].contains("database_id%3D%22acme-prod%3Abilling%22"));
    }

    // Credentials of another kind are refused with what to do instead.
    std::fs::write(&credentials, r#"{"type":"service_account"}"#).unwrap();
    let h2 = harness(GoogleConfig {
        token_url: "http://127.0.0.1:1/token".into(),
        monitoring_url: "http://127.0.0.1:1".into(),
        credentials: Some(credentials),
    });
    let c2 = h2
        .connections
        .save(connection(
            &server,
            DbAccess::ReadOnly,
            Some(RunsOn::CloudSql {
                project: "p".into(),
                instance: "i".into(),
            }),
        ))
        .await
        .unwrap();
    let machine = h2
        .health
        .start(&c2.id)
        .await
        .unwrap()
        .latest
        .unwrap()
        .machine
        .unwrap();
    assert!(machine
        .problem
        .unwrap()
        .contains("gcloud auth application-default login"));
    h2.health.stop(&c2.id);
}

#[tokio::test]
async fn the_statements_that_took_longest_come_from_pg_stat_statements() {
    let Some(server) = PgServer::start_with_statements() else {
        return;
    };
    server.psql("CREATE EXTENSION pg_stat_statements; CREATE TABLE t (id int); INSERT INTO t SELECT generate_series(1, 20000);");
    for _ in 0..3 {
        server.psql("SELECT count(*) FROM t a, (SELECT 1 FROM t LIMIT 50) b");
    }
    let h = harness(GoogleConfig::production());
    let c = h
        .connections
        .save(connection(&server, DbAccess::ReadOnly, None))
        .await
        .unwrap();
    let sample = h.health.start(&c.id).await.unwrap().latest.unwrap();
    h.health.stop(&c.id);
    let statements = sample.statements.expect("pg_stat_statements is installed");
    let top = statements
        .iter()
        .find(|s| s.query.contains("FROM t a"))
        .expect("the slow statement is listed");
    assert_eq!(top.calls, 3);
    assert!(top.total_ms > 0.0);
}
