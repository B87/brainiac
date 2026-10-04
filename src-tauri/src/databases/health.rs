//! Health (SPEC.md, Databases: Health): what a PostgreSQL server is doing
//! now and over the last hour, while its Health tab is visible.
//!
//! Each connection with Health open has a monitor that samples every 10
//! seconds on a read-only session of its own, named `Brainiac health`, with
//! a 2-second statement timeout, so a slow server cannot stall it. The hour
//! is a ring buffer in memory; nothing is written to disk. Memory, CPU, and
//! disk come from where the server runs: Docker's API on this Mac, or Cloud
//! Monitoring for Cloud SQL.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::connections::ConnectionService;
use super::driver::Target;
use super::postgres::{PgSession, PgTarget};
use crate::models::{
    now_rfc3339, AppError, AppResult, DbAccess, DbKind, DockerContainer, DockerContainers,
    ErrorCode, HealthEvent, HealthPoint, HealthSample, HealthSession, HealthSnapshot,
    HealthStatement, HealthTable, MachineSample, RunsOn,
};

/// How often a monitor samples.
pub const SAMPLE_EVERY: Duration = Duration::from_secs(10);
/// An hour of samples.
pub const POINTS: usize = 360;
/// The table list and Cloud Monitoring are read this often, not every sample.
const SLOW_EVERY: Duration = Duration::from_secs(60);
const STATEMENT_TIMEOUT: Duration = Duration::from_secs(2);

/// Receives every sample, to emit as `db_health_sample`.
pub type HealthEmitter = Arc<dyn Fn(HealthEvent) + Send + Sync>;

/// Where Google's token and Monitoring APIs are, and the credentials file:
/// Google's in the app, a local stand-in in tests.
#[derive(Debug, Clone)]
pub struct GoogleConfig {
    pub token_url: String,
    pub monitoring_url: String,
    /// `None`: `$GOOGLE_APPLICATION_CREDENTIALS`, else gcloud's own file.
    pub credentials: Option<PathBuf>,
}

impl GoogleConfig {
    pub fn production() -> Self {
        GoogleConfig {
            token_url: "https://oauth2.googleapis.com/token".into(),
            monitoring_url: "https://monitoring.googleapis.com".into(),
            credentials: None,
        }
    }
}

/// Docker sockets tried in order when a connection names none: Docker
/// Desktop's, the system one, then Colima's and OrbStack's.
pub fn docker_sockets() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    vec![
        home.join(".docker/run/docker.sock"),
        PathBuf::from("/var/run/docker.sock"),
        home.join(".colima/default/docker.sock"),
        home.join(".orbstack/run/docker.sock"),
    ]
}

/// Counters from the previous sample, to turn into rates.
#[derive(Clone)]
struct Counters {
    at: Instant,
    commits: i64,
    rollbacks: i64,
    hits: i64,
    reads: i64,
    deadlocks: i64,
}

struct DockerPrevious {
    at: Instant,
    cpu_total: u64,
    cpu_system: u64,
    read: u64,
    write: u64,
}

/// What a monitor keeps between samples.
#[derive(Default)]
struct State {
    session: Option<PgSession>,
    counters: Option<Counters>,
    /// `temp_files` counter readings, for the last hour's count.
    temp_files: VecDeque<(Instant, i64)>,
    tables: Vec<HealthTable>,
    tables_at: Option<Instant>,
    docker: Option<DockerPrevious>,
    cloud: Option<(Instant, MachineSample)>,
}

struct Monitor {
    connection_id: String,
    watchers: AtomicUsize,
    points: Mutex<VecDeque<HealthPoint>>,
    latest: Mutex<Option<HealthSample>>,
    // Held across the sample's `await`s, so a tokio mutex.
    state: tokio::sync::Mutex<State>,
}

pub struct HealthService {
    connections: Arc<ConnectionService>,
    emitter: HealthEmitter,
    monitors: Mutex<HashMap<String, Arc<Monitor>>>,
    google: GoogleConfig,
    google_token: tokio::sync::Mutex<Option<(String, Instant)>>,
    https: reqwest::Client,
}

impl HealthService {
    pub fn new(
        connections: Arc<ConnectionService>,
        emitter: HealthEmitter,
        google: GoogleConfig,
    ) -> Self {
        let https = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_default();
        HealthService {
            connections,
            emitter,
            monitors: Mutex::new(HashMap::new()),
            google,
            google_token: tokio::sync::Mutex::new(None),
            https,
        }
    }

    fn monitor(&self, connection_id: &str) -> Arc<Monitor> {
        let mut monitors = self.monitors.lock().expect("monitors lock");
        Arc::clone(
            monitors
                .entry(connection_id.to_string())
                .or_insert_with(|| {
                    Arc::new(Monitor {
                        connection_id: connection_id.to_string(),
                        watchers: AtomicUsize::new(0),
                        points: Mutex::new(VecDeque::with_capacity(POINTS)),
                        latest: Mutex::new(None),
                        state: tokio::sync::Mutex::new(State::default()),
                    })
                }),
        )
    }

    /// Health became visible: sample now and every 10 seconds until `stop`.
    /// Returns the hour so far, with this sample.
    pub async fn start(self: &Arc<Self>, connection_id: &str) -> AppResult<HealthSnapshot> {
        let connection = self.connections.get(connection_id).await?;
        if connection.kind != DbKind::Postgres {
            return Err(AppError::validation(
                "Health is for PostgreSQL servers. A SQLite file has no server to watch.",
            ));
        }
        let monitor = self.monitor(connection_id);
        let first = monitor.watchers.fetch_add(1, Ordering::SeqCst) == 0;
        self.sample_once(&monitor).await;
        if first {
            let service = Arc::clone(self);
            let watched = Arc::clone(&monitor);
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(SAMPLE_EVERY).await;
                    if watched.watchers.load(Ordering::SeqCst) == 0 {
                        // Hidden: drop the session; the hour so far stays in memory.
                        watched.state.lock().await.session = None;
                        break;
                    }
                    service.sample_once(&watched).await;
                }
            });
        }
        let points = monitor
            .points
            .lock()
            .expect("points lock")
            .iter()
            .cloned()
            .collect();
        let latest = monitor.latest.lock().expect("latest lock").clone();
        Ok(HealthSnapshot { points, latest })
    }

    /// Health was hidden or closed.
    pub fn stop(&self, connection_id: &str) {
        if let Some(monitor) = self
            .monitors
            .lock()
            .expect("monitors lock")
            .get(connection_id)
        {
            let mut current = monitor.watchers.load(Ordering::SeqCst);
            while current > 0 {
                match monitor.watchers.compare_exchange(
                    current,
                    current - 1,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                ) {
                    Ok(_) => break,
                    Err(now) => current = now,
                }
            }
        }
    }

    /// Forget a deleted connection's hour.
    pub fn forget(&self, connection_id: &str) {
        self.monitors
            .lock()
            .expect("monitors lock")
            .remove(connection_id);
    }

    async fn sample_once(&self, monitor: &Monitor) {
        let sample = self.sample(monitor).await;
        let point = HealthPoint {
            at: sample.at.clone(),
            connections: sample.total_connections,
            cache_hit_ratio: sample.cache_hit_ratio,
            transactions_per_second: sample.transactions_per_second,
            memory_ratio: sample.machine.as_ref().and_then(|m| m.memory_ratio),
            cpu_ratio: sample.machine.as_ref().and_then(|m| m.cpu_ratio),
        };
        {
            let mut points = monitor.points.lock().expect("points lock");
            if points.len() == POINTS {
                points.pop_front();
            }
            points.push_back(point.clone());
        }
        *monitor.latest.lock().expect("latest lock") = Some(sample.clone());
        (self.emitter)(HealthEvent { sample, point });
    }

    async fn health_target(&self, connection_id: &str) -> AppResult<(PgTarget, Option<RunsOn>)> {
        let connection = self.connections.get(connection_id).await?;
        let runs_on = connection.runs_on.clone();
        match self.connections.target(&connection).await? {
            Target::Postgres(mut target) => {
                target.application_name = "Brainiac health".into();
                target.statement_timeout = STATEMENT_TIMEOUT;
                Ok((target, runs_on))
            }
            Target::Sqlite { .. } => Err(AppError::validation("Health is for PostgreSQL servers.")),
        }
    }

    async fn sample(&self, monitor: &Monitor) -> HealthSample {
        let mut state = monitor.state.lock().await;
        let mut sample = empty_sample(&monitor.connection_id);
        let runs_on = match self.health_target(&monitor.connection_id).await {
            Ok((target, runs_on)) => {
                if state.session.as_ref().is_none_or(PgSession::is_closed) {
                    match PgSession::connect(&target).await {
                        Ok(s) => state.session = Some(s),
                        Err(e) => sample.problem = Some(e.message),
                    }
                }
                runs_on
            }
            Err(e) => {
                sample.problem = Some(e.message);
                None
            }
        };
        if state.session.is_some() {
            if let Err(e) = read_server(&mut state, &mut sample).await {
                sample.problem = Some(e);
                state.session = None;
            }
        }
        sample.machine = match runs_on {
            Some(RunsOn::Docker { container, socket }) => {
                Some(docker_sample(&mut state, &container, socket.as_deref()).await)
            }
            Some(RunsOn::CloudSql { project, instance }) => {
                let fresh = state
                    .cloud
                    .as_ref()
                    .is_some_and(|(at, _)| at.elapsed() < SLOW_EVERY);
                if !fresh {
                    let read = self.cloud_sample(&project, &instance).await;
                    state.cloud = Some((Instant::now(), read));
                }
                state.cloud.as_ref().map(|(_, s)| s.clone())
            }
            None => None,
        };
        sample
    }

    /// Cancel a session's statement, or end the session, on the server.
    /// Only for connections whose access is read and write.
    pub async fn signal_backend(
        &self,
        connection_id: &str,
        pid: i32,
        terminate: bool,
    ) -> AppResult<bool> {
        let connection = self.connections.get(connection_id).await?;
        if connection.access != DbAccess::ReadWrite {
            return Err(AppError::new(
                ErrorCode::PermissionDenied,
                "Cancelling another session needs a connection whose access is read and write.",
            ));
        }
        let (target, _) = self.health_target(connection_id).await?;
        let session = PgSession::connect(&target).await?;
        let function = if terminate {
            "pg_terminate_backend"
        } else {
            "pg_cancel_backend"
        };
        let row = session
            .client()
            .query_one(&format!("SELECT {function}($1)"), &[&pid])
            .await
            .map_err(|e| match e.as_db_error() {
                Some(db) => AppError::new(ErrorCode::PermissionDenied, db.message().to_string()),
                None => {
                    AppError::dependency("The server did not answer.").with_details(e.to_string())
                }
            })?;
        let done: bool = row.get(0);
        tracing::info!(connection = %connection_id, pid, terminate, done, "signalled a server session");
        Ok(done)
    }

    async fn google_token(&self) -> Result<String, String> {
        let mut cached = self.google_token.lock().await;
        if let Some((token, expires)) = cached.as_ref() {
            if Instant::now() + Duration::from_secs(60) < *expires {
                return Ok(token.clone());
            }
        }
        let path = self
            .google
            .credentials
            .clone()
            .or_else(|| std::env::var_os("GOOGLE_APPLICATION_CREDENTIALS").map(PathBuf::from))
            .unwrap_or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .unwrap_or_default()
                    .join(".config/gcloud/application_default_credentials.json")
            });
        let file = std::fs::read_to_string(&path).map_err(|_| {
            "No Google credentials on this Mac. Run `gcloud auth application-default login`."
                .to_string()
        })?;
        let credentials: serde_json::Value = serde_json::from_str(&file)
            .map_err(|_| format!("{} is not a credentials file.", path.display()))?;
        if credentials["type"] != "authorized_user" {
            return Err(
                "Only gcloud's own credentials are supported. Run `gcloud auth application-default login`."
                    .into(),
            );
        }
        let field = |k: &str| credentials[k].as_str().unwrap_or_default().to_string();
        let response = self
            .https
            .post(&self.google.token_url)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(format!(
                "grant_type=refresh_token&client_id={}&client_secret={}&refresh_token={}",
                percent(&field("client_id")),
                percent(&field("client_secret")),
                percent(&field("refresh_token"))
            ))
            .send()
            .await
            .map_err(|e| format!("Google could not be reached: {e}"))?;
        if !response.status().is_success() {
            return Err(
                "Google refused the credentials. Run `gcloud auth application-default login` again.".into(),
            );
        }
        let body: serde_json::Value = response
            .json()
            .await
            .map_err(|_| "Google's answer could not be read.".to_string())?;
        let token = body["access_token"]
            .as_str()
            .ok_or("Google's answer had no access token.")?
            .to_string();
        let lifetime = body["expires_in"].as_u64().unwrap_or(3600);
        *cached = Some((
            token.clone(),
            Instant::now() + Duration::from_secs(lifetime),
        ));
        Ok(token)
    }

    /// The latest value of a Cloud SQL metric over the last 10 minutes.
    async fn cloud_metric(
        &self,
        token: &str,
        project: &str,
        instance: &str,
        metric: &str,
    ) -> Result<Option<f64>, String> {
        let end = chrono::Utc::now();
        let start = end - chrono::Duration::minutes(10);
        let filter = format!(
            "metric.type=\"cloudsql.googleapis.com/database/{metric}\" AND resource.labels.database_id=\"{project}:{instance}\""
        );
        let url = format!(
            "{}/v3/projects/{}/timeSeries?filter={}&interval.startTime={}&interval.endTime={}",
            self.google.monitoring_url.trim_end_matches('/'),
            percent(project),
            percent(&filter),
            percent(&start.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
            percent(&end.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        );
        let response = self
            .https
            .get(&url)
            .bearer_auth(token)
            .send()
            .await
            .map_err(|e| format!("Cloud Monitoring could not be reached: {e}"))?;
        match response.status().as_u16() {
            200 => {}
            403 => return Err("Cloud Monitoring refused: the account needs the Monitoring Viewer role on the project.".into()),
            404 => return Err(format!("Cloud Monitoring has no project {project}.")),
            status => return Err(format!("Cloud Monitoring answered {status}.")),
        }
        let body: serde_json::Value = response
            .json()
            .await
            .map_err(|_| "Cloud Monitoring's answer could not be read.".to_string())?;
        let value = &body["timeSeries"][0]["points"][0]["value"];
        Ok(value["doubleValue"]
            .as_f64()
            .or_else(|| value["int64Value"].as_str().and_then(|v| v.parse().ok()))
            .or_else(|| value["int64Value"].as_f64()))
    }

    async fn cloud_sample(&self, project: &str, instance: &str) -> MachineSample {
        let mut sample = MachineSample {
            source: "Cloud Monitoring · 1 min behind".into(),
            ..empty_machine()
        };
        let token = match self.google_token().await {
            Ok(t) => t,
            Err(problem) => {
                sample.problem = Some(problem);
                return sample;
            }
        };
        let read = |metric: &'static str| self.cloud_metric(&token, project, instance, metric);
        let results = (
            read("memory/utilization").await,
            read("memory/quota").await,
            read("cpu/utilization").await,
            read("disk/bytes_used").await,
            read("disk/quota").await,
        );
        match results {
            (Ok(memory), Ok(memory_quota), Ok(cpu), Ok(disk), Ok(disk_quota)) => {
                sample.memory_ratio = memory;
                sample.memory_limit = memory_quota.map(|q| q as u64);
                sample.memory_used = memory.zip(memory_quota).map(|(r, q)| (r * q) as u64);
                sample.cpu_ratio = cpu;
                sample.disk_used = disk.map(|d| d as u64);
                sample.disk_quota = disk_quota.map(|d| d as u64);
                if memory.is_none() && cpu.is_none() {
                    sample.problem = Some(format!(
                        "Cloud Monitoring has no recent data for {project}:{instance}. Check the instance's name."
                    ));
                }
            }
            (a, b, c, d, e) => {
                sample.problem = [a, b, c, d, e].into_iter().find_map(Result::err);
            }
        }
        sample
    }
}

fn percent(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn empty_machine() -> MachineSample {
    MachineSample {
        source: String::new(),
        memory_used: None,
        memory_limit: None,
        memory_ratio: None,
        cpu_ratio: None,
        disk_used: None,
        disk_quota: None,
        disk_read_rate: None,
        disk_write_rate: None,
        problem: None,
    }
}

fn empty_sample(connection_id: &str) -> HealthSample {
    HealthSample {
        connection_id: connection_id.to_string(),
        at: now_rfc3339(),
        max_connections: 0,
        active: 0,
        idle: 0,
        idle_in_transaction: 0,
        total_connections: 0,
        cache_hit_ratio: None,
        transactions_per_second: None,
        rollbacks_per_second: None,
        deadlocks: 0,
        temp_files_hour: 0,
        longest_transaction_seconds: None,
        sessions: Vec::new(),
        statements: None,
        tables: Vec::new(),
        sees_all: false,
        machine: None,
        problem: None,
    }
}

const ACTIVITY: &str = "
SELECT current_setting('max_connections')::int,
       count(*) FILTER (WHERE state = 'active')::int,
       count(*) FILTER (WHERE state = 'idle')::int,
       count(*) FILTER (WHERE state LIKE 'idle in transaction%')::int,
       count(*)::int,
       (extract(epoch FROM max(now() - xact_start)
            FILTER (WHERE datname = current_database() AND pid <> pg_backend_pid())))::float8,
       pg_has_role(current_user, 'pg_monitor', 'MEMBER')
         OR (SELECT rolsuper FROM pg_roles WHERE rolname = current_user)
  FROM pg_stat_activity
 WHERE backend_type = 'client backend'";

const DATABASE: &str = "
SELECT xact_commit, xact_rollback, blks_hit, blks_read, deadlocks, temp_files
  FROM pg_stat_database WHERE datname = current_database()";

/// Idle-in-transaction and waiting sessions first, then the longest in their state.
const SESSIONS: &str = "
SELECT pid, usename::text, coalesce(application_name, ''), client_addr::text, state,
       extract(epoch FROM now() - state_change)::float8,
       extract(epoch FROM now() - xact_start)::float8,
       CASE WHEN wait_event IS NOT NULL THEN wait_event_type || ': ' || wait_event END,
       pg_blocking_pids(pid), query, application_name LIKE 'Brainiac%'
  FROM pg_stat_activity
 WHERE backend_type = 'client backend' AND datname = current_database() AND pid <> pg_backend_pid()
 ORDER BY (state LIKE 'idle in transaction%') DESC, cardinality(pg_blocking_pids(pid)) > 0 DESC,
          state = 'active' DESC, state_change
 LIMIT 200";

const STATEMENTS: &str = "
SELECT query, calls, total_exec_time, mean_exec_time, rows
  FROM pg_stat_statements
 WHERE dbid = (SELECT oid FROM pg_database WHERE datname = current_database())
 ORDER BY total_exec_time DESC LIMIT 10";

/// Sizes from `pg_class`'s page counts (as of the last vacuum or analyze),
/// with TOAST and indexes: `pg_total_relation_size` would wait for a table
/// someone holds locked, which is when Health is needed most.
const TABLES: &str = "
SELECT s.schemaname::text, s.relname::text,
       ((c.relpages + coalesce(t.relpages, 0)
         + coalesce((SELECT sum(i.relpages) FROM pg_index x JOIN pg_class i ON i.oid = x.indexrelid
                      WHERE x.indrelid = c.oid), 0))::int8
        * current_setting('block_size')::int8),
       s.n_live_tup, s.n_dead_tup,
       to_char(s.last_autovacuum AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')
  FROM pg_stat_user_tables s
  JOIN pg_class c ON c.oid = s.relid
  LEFT JOIN pg_class t ON t.oid = c.reltoastrelid
 ORDER BY 3 DESC LIMIT 10";

/// PostgreSQL's own statistics, into `sample`.
async fn read_server(state: &mut State, sample: &mut HealthSample) -> Result<(), String> {
    let client = state.session.as_ref().ok_or("No session.")?.client();
    let text = |e: tokio_postgres::Error| match e.as_db_error() {
        Some(db) => db.message().to_string(),
        None => format!("The server stopped answering: {e}"),
    };
    let row = client.query_one(ACTIVITY, &[]).await.map_err(text)?;
    sample.max_connections = row.get::<_, i32>(0).max(0) as u32;
    sample.active = row.get::<_, i32>(1) as u32;
    sample.idle = row.get::<_, i32>(2) as u32;
    sample.idle_in_transaction = row.get::<_, i32>(3) as u32;
    sample.total_connections = row.get::<_, i32>(4) as u32;
    sample.longest_transaction_seconds = row.get(5);
    sample.sees_all = row.get::<_, Option<bool>>(6).unwrap_or(false);

    let row = client.query_one(DATABASE, &[]).await.map_err(text)?;
    let now = Counters {
        at: Instant::now(),
        commits: row.get(0),
        rollbacks: row.get(1),
        hits: row.get(2),
        reads: row.get(3),
        deadlocks: row.get(4),
    };
    let temp_files: i64 = row.get(5);
    if let Some(before) = &state.counters {
        let seconds = now.at.duration_since(before.at).as_secs_f64().max(0.001);
        let transactions = (now.commits - before.commits) + (now.rollbacks - before.rollbacks);
        sample.transactions_per_second = Some(transactions.max(0) as f64 / seconds);
        sample.rollbacks_per_second =
            Some((now.rollbacks - before.rollbacks).max(0) as f64 / seconds);
        let (hits, reads) = (now.hits - before.hits, now.reads - before.reads);
        sample.cache_hit_ratio = (hits + reads > 0).then(|| hits as f64 / (hits + reads) as f64);
        sample.deadlocks = (now.deadlocks - before.deadlocks).max(0);
    } else if now.hits + now.reads > 0 {
        // The first sample: the ratio since the statistics were reset.
        sample.cache_hit_ratio = Some(now.hits as f64 / (now.hits + now.reads) as f64);
    }
    state.counters = Some(now.clone());
    state.temp_files.push_back((now.at, temp_files));
    while state
        .temp_files
        .front()
        .is_some_and(|(at, _)| now.at.duration_since(*at) > Duration::from_secs(3600))
    {
        state.temp_files.pop_front();
    }
    sample.temp_files_hour = state
        .temp_files
        .front()
        .map(|(_, first)| temp_files - first)
        .unwrap_or(0)
        .max(0);

    let rows = client.query(SESSIONS, &[]).await.map_err(text)?;
    sample.sessions = rows
        .iter()
        .map(|r| {
            let query: Option<String> = r.get(9);
            HealthSession {
                pid: r.get(0),
                user: r.get(1),
                application: r.get(2),
                client: r.get(3),
                state: r.get(4),
                state_seconds: r.get(5),
                transaction_seconds: r.get(6),
                wait: r.get(7),
                blocked_by: r.get::<_, Option<Vec<i32>>>(8).unwrap_or_default(),
                query: query.filter(|q| q != "<insufficient privilege>"),
                own: r.get::<_, Option<bool>>(10).unwrap_or(false),
            }
        })
        .collect();

    let installed: bool = client
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM pg_extension WHERE extname = 'pg_stat_statements')",
            &[],
        )
        .await
        .map_err(text)?
        .get(0);
    sample.statements = if installed {
        // Before PostgreSQL 13 the columns had other names; such servers show none.
        client.query(STATEMENTS, &[]).await.ok().map(|rows| {
            rows.iter()
                .map(|r| HealthStatement {
                    query: r.get(0),
                    calls: r.get(1),
                    total_ms: r.get(2),
                    mean_ms: r.get(3),
                    rows: r.get(4),
                })
                .collect()
        })
    } else {
        None
    };

    if state.tables_at.is_none_or(|at| at.elapsed() >= SLOW_EVERY) {
        // Best effort: the previous list stays when this fails.
        let rows = client.query(TABLES, &[]).await.unwrap_or_default();
        state.tables = rows
            .iter()
            .map(|r| {
                let (live, dead): (i64, i64) = (r.get(3), r.get(4));
                HealthTable {
                    schema: r.get(0),
                    name: r.get(1),
                    total_bytes: r.get(2),
                    live_rows: live,
                    dead_ratio: (live + dead > 0).then(|| dead as f64 / (live + dead) as f64),
                    last_autovacuum: r.get(5),
                }
            })
            .collect();
        state.tables_at = Some(Instant::now());
    }
    sample.tables = state.tables.clone();
    Ok(())
}

fn docker_client(socket: &Path) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .unix_socket(socket)
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| format!("Docker cannot be reached: {e}"))
}

fn find_socket(socket: Option<&str>) -> Result<PathBuf, String> {
    match socket {
        Some(s) => Ok(PathBuf::from(s)),
        None => docker_sockets()
            .into_iter()
            .find(|p| p.exists())
            .ok_or_else(|| {
                "Docker is not running on this Mac: no Docker socket was found.".to_string()
            }),
    }
}

/// Running containers, for choosing what a connection runs on. Only `GET`
/// requests are ever sent to Docker.
pub async fn list_containers(socket: Option<&str>) -> AppResult<DockerContainers> {
    let path = find_socket(socket).map_err(AppError::dependency)?;
    let client = docker_client(&path).map_err(AppError::dependency)?;
    let body: serde_json::Value = client
        .get("http://docker/containers/json")
        .send()
        .await
        .map_err(|e| AppError::dependency("Docker did not answer.").with_details(e.to_string()))?
        .json()
        .await
        .map_err(|e| {
            AppError::dependency("Docker's answer could not be read.").with_details(e.to_string())
        })?;
    let containers = body
        .as_array()
        .map(|list| {
            list.iter()
                .map(|c| DockerContainer {
                    id: c["Id"]
                        .as_str()
                        .unwrap_or_default()
                        .chars()
                        .take(12)
                        .collect(),
                    name: c["Names"][0]
                        .as_str()
                        .unwrap_or_default()
                        .trim_start_matches('/')
                        .to_string(),
                    image: c["Image"].as_str().unwrap_or_default().to_string(),
                    ports: c["Ports"]
                        .as_array()
                        .map(|ports| {
                            ports
                                .iter()
                                .filter_map(|p| {
                                    let private = p["PrivatePort"].as_u64()?;
                                    Some(match p["PublicPort"].as_u64() {
                                        Some(public) => format!("{public}→{private}"),
                                        None => private.to_string(),
                                    })
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(DockerContainers {
        socket: path.display().to_string(),
        containers,
    })
}

/// Memory as Docker's CLI shows it (without the reclaimable page cache), the
/// share of the CPUs used since the previous sample, and disk I/O rates.
async fn docker_sample(state: &mut State, container: &str, socket: Option<&str>) -> MachineSample {
    let mut sample = MachineSample {
        source: "Docker".into(),
        ..empty_machine()
    };
    let read = async {
        let path = find_socket(socket)?;
        let client = docker_client(&path)?;
        let response = client
            .get(format!(
                "http://docker/containers/{}/stats?stream=false&one-shot=true",
                percent(container)
            ))
            .send()
            .await
            .map_err(|e| format!("Docker did not answer: {e}"))?;
        if response.status().as_u16() == 404 {
            return Err(format!("Docker has no container named {container}."));
        }
        response
            .json::<serde_json::Value>()
            .await
            .map_err(|_| "Docker's answer could not be read.".to_string())
    };
    let stats = match read.await {
        Ok(s) => s,
        Err(problem) => {
            sample.problem = Some(problem);
            return sample;
        }
    };
    let memory = &stats["memory_stats"];
    let usage = memory["usage"].as_u64();
    let cache = memory["stats"]["inactive_file"]
        .as_u64()
        .or_else(|| memory["stats"]["total_inactive_file"].as_u64())
        .or_else(|| memory["stats"]["cache"].as_u64())
        .unwrap_or(0);
    sample.memory_used = usage.map(|u| u.saturating_sub(cache));
    sample.memory_limit = memory["limit"].as_u64();
    sample.memory_ratio = sample
        .memory_used
        .zip(sample.memory_limit)
        .filter(|(_, l)| *l > 0)
        .map(|(u, l)| u as f64 / l as f64);
    let cpu_total = stats["cpu_stats"]["cpu_usage"]["total_usage"]
        .as_u64()
        .unwrap_or(0);
    let cpu_system = stats["cpu_stats"]["system_cpu_usage"].as_u64().unwrap_or(0);
    let (mut read_bytes, mut write_bytes) = (0u64, 0u64);
    if let Some(entries) = stats["blkio_stats"]["io_service_bytes_recursive"].as_array() {
        for entry in entries {
            let value = entry["value"].as_u64().unwrap_or(0);
            match entry["op"].as_str().map(str::to_ascii_lowercase).as_deref() {
                Some("read") => read_bytes += value,
                Some("write") => write_bytes += value,
                _ => {}
            }
        }
    }
    let now = DockerPrevious {
        at: Instant::now(),
        cpu_total,
        cpu_system,
        read: read_bytes,
        write: write_bytes,
    };
    if let Some(before) = &state.docker {
        let system = now.cpu_system.saturating_sub(before.cpu_system);
        if system > 0 {
            sample.cpu_ratio =
                Some(now.cpu_total.saturating_sub(before.cpu_total) as f64 / system as f64);
        }
        let seconds = now.at.duration_since(before.at).as_secs_f64().max(0.001);
        sample.disk_read_rate = Some(now.read.saturating_sub(before.read) as f64 / seconds);
        sample.disk_write_rate = Some(now.write.saturating_sub(before.write) as f64 / seconds);
    }
    state.docker = Some(now);
    sample
}
