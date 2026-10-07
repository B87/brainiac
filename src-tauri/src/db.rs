//! SQLite persistence: each database file has one connection owned by a
//! dedicated worker thread, ordered migrations, and consistent backups.
//!
//! Callers never touch a connection directly. They send a closure to the
//! worker (`Db::call`) and await its result, which keeps all access to a file
//! serialized and off the async executor threads (docs/architecture.md, Concurrency and lifecycle).
//!
//! v0.2 keeps three files (docs/architecture.md, Storage layout): the core
//! database (`CORE`, the file `brainiac.sqlite3`), the rebuildable index
//! (`INDEX`), and revision history (`HISTORY`). Each has its own
//! `application_id`, migration list, and backup policy.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;

use rusqlite::{params, Connection, OpenFlags, OptionalExtension};

use crate::forge::ForgeRepository;
use crate::models::{
    ActivitySettings, AppError, AppResult, DiscoveryMode, MemberOrigin, Pin, PinEntityType,
    RepositoryTab, Settings, StatusSnapshot,
};

/// Ordered migrations of the core database. Add new entries at the end; never edit a shipped one.
const MIGRATIONS: &[(&str, &str)] = &[
    ("0001_init", include_str!("../migrations/0001_init.sql")),
    (
        "0002_knowledge",
        include_str!("../migrations/0002_knowledge.sql"),
    ),
    (
        "0003_forge_accounts",
        include_str!("../migrations/0003_forge_accounts.sql"),
    ),
    ("0004_forges", include_str!("../migrations/0004_forges.sql")),
    (
        "0005_review_drafts",
        include_str!("../migrations/0005_review_drafts.sql"),
    ),
    (
        "0006_databases",
        include_str!("../migrations/0006_databases.sql"),
    ),
    (
        "0007_secret_sources",
        include_str!("../migrations/0007_secret_sources.sql"),
    ),
    (
        "0008_agent_runs",
        include_str!("../migrations/0008_agent_runs.sql"),
    ),
    (
        "0009_agent_test",
        include_str!("../migrations/0009_agent_test.sql"),
    ),
    (
        "0010_agent_model",
        include_str!("../migrations/0010_agent_model.sql"),
    ),
    (
        "0011_agent_hosts_remote",
        include_str!("../migrations/0011_agent_hosts_remote.sql"),
    ),
    (
        "0012_agent_host_controller",
        include_str!("../migrations/0012_agent_host_controller.sql"),
    ),
];

/// How many daily backups to keep.
const BACKUP_RETENTION: usize = 7;

/// Brainiac's `PRAGMA application_id` ("BRNC" in ASCII). It marks a database
/// file, and every backup copied from it, as Brainiac's. Files from before
/// 0.1.3 carry 0 and are adopted when opened.
pub const APPLICATION_ID: i32 = 0x4252_4E43;

/// The schema version this build migrates the core database to.
pub const SCHEMA_VERSION: u32 = MIGRATIONS.len() as u32;

/// When a file is snapshotted into the `backups` folder next to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backups {
    /// Never: the file is rebuilt instead (`index.db`).
    Never,
    /// Before migrations, and once per this many days, keeping `keep` copies.
    Every { days: u32, keep: usize },
}

/// One kind of database file: its name, its marker, its migrations, and how it is backed up.
#[derive(Debug, Clone, Copy)]
pub struct Store {
    /// Prefix of its backups (empty for the core database, whose backup names predate v0.2).
    pub backup_prefix: &'static str,
    pub application_id: i32,
    pub migrations: &'static [(&'static str, &'static str)],
    pub backups: Backups,
    /// Accept a file whose `application_id` is 0 (core databases from before 0.1.3).
    pub adopt_unmarked: bool,
    /// Create the file with incremental auto-vacuum, so space freed by a scan
    /// that rewrote many notes can be reclaimed.
    pub incremental_vacuum: bool,
}

/// `brainiac.db`: what cannot be rebuilt.
pub const CORE: Store = Store {
    backup_prefix: "",
    application_id: APPLICATION_ID,
    migrations: MIGRATIONS,
    backups: Backups::Every {
        days: 1,
        keep: BACKUP_RETENTION,
    },
    adopt_unmarked: true,
    incremental_vacuum: false,
};

/// `index.db`: note bodies, search, and links between notes, rebuilt from the vault ("BRNI").
pub const INDEX: Store = Store {
    backup_prefix: "index-",
    application_id: 0x4252_4E49,
    migrations: &[(
        "0001_index",
        include_str!("../migrations/index/0001_index.sql"),
    )],
    backups: Backups::Never,
    adopt_unmarked: false,
    incremental_vacuum: true,
};

/// `history.db`: revisions and drafts, snapshotted weekly ("BRNH").
pub const HISTORY: Store = Store {
    backup_prefix: "history-",
    application_id: 0x4252_4E48,
    migrations: &[
        (
            "0001_history",
            include_str!("../migrations/history/0001_history.sql"),
        ),
        (
            "0002_agent",
            include_str!("../migrations/history/0002_agent.sql"),
        ),
        (
            "0003_queries",
            include_str!("../migrations/history/0003_queries.sql"),
        ),
        (
            "0004_agent_runs",
            include_str!("../migrations/history/0004_agent_runs.sql"),
        ),
        (
            "0005_agent_run_model",
            include_str!("../migrations/history/0005_agent_run_model.sql"),
        ),
        (
            "0006_agent_run_host",
            include_str!("../migrations/history/0006_agent_run_host.sql"),
        ),
    ],
    backups: Backups::Every { days: 7, keep: 2 },
    adopt_unmarked: false,
    incremental_vacuum: false,
};

/// `forge.db`: pull requests as last read from the providers, rebuilt from them ("BRNF").
pub const FORGE: Store = Store {
    backup_prefix: "forge-",
    application_id: 0x4252_4E46,
    migrations: &[
        (
            "0001_forge",
            include_str!("../migrations/forge/0001_forge.sql"),
        ),
        (
            "0002_conversations",
            include_str!("../migrations/forge/0002_conversations.sql"),
        ),
    ],
    backups: Backups::Never,
    adopt_unmarked: false,
    incremental_vacuum: true,
};

/// File names in the data folder. The core keeps its v0.1 name.
pub const CORE_FILE: &str = "brainiac.sqlite3";
pub const INDEX_FILE: &str = "index.sqlite3";
pub const HISTORY_FILE: &str = "history.sqlite3";
pub const FORGE_FILE: &str = "forge.sqlite3";

type Job = Box<dyn FnOnce(&mut Connection) + Send + 'static>;

/// Handle to a database worker. Cheap to clone; all clones share one thread.
#[derive(Clone)]
pub struct Db {
    sender: mpsc::Sender<Job>,
    path: PathBuf,
}

impl Db {
    /// Open (or create) the core database at `path`, run pending migrations,
    /// and start the worker thread. Takes a daily backup before the first write.
    pub fn open(path: &Path) -> AppResult<Self> {
        Self::open_store(path, &CORE)
    }

    /// Open (or create) a database file of the given kind.
    pub fn open_store(path: &Path, store: &Store) -> AppResult<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut conn = Connection::open(path)?;
        check_compatible(&conn, path, store)?;
        if store.incremental_vacuum && schema_version(&conn)? == 0 {
            // Only takes effect before the first table is created.
            conn.execute_batch("PRAGMA auto_vacuum = INCREMENTAL;")?;
        }
        configure(&conn)?;
        backup_before_migration_if_needed(&conn, path, store)?;
        migrate(&mut conn, store)?;
        conn.pragma_update(None, "application_id", store.application_id)?;
        periodic_backup(&conn, path, store)?;
        Self::spawn(conn, path, "brainiac-db")
    }

    /// Open the index, which is rebuildable: a file written by a newer
    /// Brainiac, or one that cannot be read, is deleted and created again. A
    /// file another program owns is refused like any other store.
    pub fn open_index(path: &Path) -> AppResult<Self> {
        Self::open_rebuildable(path, &INDEX)
    }

    /// Open a store that can be rebuilt (`index.db`, `forge.db`): a file
    /// written by a newer Brainiac, or one that cannot be read, is deleted
    /// and created again. A file another program owns is refused like any
    /// other store.
    pub fn open_rebuildable(path: &Path, store: &Store) -> AppResult<Self> {
        match Self::open_store(path, store) {
            Ok(db) => Ok(db),
            Err(e) if is_foreign(path, store) => Err(e),
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, details = ?e.details, "recreating a rebuildable database");
                remove_database(path)?;
                Self::open_store(path, store)
            }
        }
    }

    /// A worker with a read-only connection to an existing file, for queries
    /// that must not wait behind its writer (docs/architecture.md, Storage layout).
    pub fn open_read_only(path: &Path) -> AppResult<Self> {
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.execute_batch("PRAGMA busy_timeout = 5000; PRAGMA query_only = ON;")?;
        Self::spawn(conn, path, "brainiac-db-read")
    }

    fn spawn(mut conn: Connection, path: &Path, name: &str) -> AppResult<Self> {
        let (sender, receiver) = mpsc::channel::<Job>();
        thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                for job in receiver {
                    job(&mut conn);
                }
                // The channel closed: every `Db` handle was dropped, so shut down cleanly.
                let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
            })
            .map_err(|e| {
                AppError::db("Could not start the database worker.").with_details(e.to_string())
            })?;
        Ok(Db {
            sender,
            path: path.to_path_buf(),
        })
    }

    /// Where the database file is.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Run `f` on the worker thread and await its result.
    pub async fn call<T, F>(&self, f: F) -> AppResult<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> AppResult<T> + Send + 'static,
    {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let job: Job = Box::new(move |conn| {
            let _ = tx.send(f(conn));
        });
        self.sender
            .send(job)
            .map_err(|_| AppError::db("The database worker is no longer running."))?;
        rx.await
            .map_err(|_| AppError::db("The database worker dropped the request."))?
    }

    /// Blocking variant for synchronous contexts (setup, tests).
    pub fn call_blocking<T, F>(&self, f: F) -> AppResult<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> AppResult<T> + Send + 'static,
    {
        let (tx, rx) = mpsc::channel();
        let job: Job = Box::new(move |conn| {
            let _ = tx.send(f(conn));
        });
        self.sender
            .send(job)
            .map_err(|_| AppError::db("The database worker is no longer running."))?;
        rx.recv()
            .map_err(|_| AppError::db("The database worker dropped the request."))?
    }

    /// Write a consistent snapshot of the live database to `dest`.
    pub async fn backup_to(&self, dest: PathBuf) -> AppResult<PathBuf> {
        self.call(move |conn| {
            write_backup(conn, &dest)?;
            Ok(dest)
        })
        .await
    }
}

/// Whether `path` holds a database another program marked as its own.
fn is_foreign(path: &Path, store: &Store) -> bool {
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .and_then(|c| c.query_row("PRAGMA application_id", [], |r| r.get::<_, i32>(0)))
        .is_ok_and(|id| id != 0 && id != store.application_id)
}

/// Delete a database file and its WAL and shared-memory files.
pub fn remove_database(path: &Path) -> AppResult<()> {
    for suffix in ["", "-wal", "-shm"] {
        let mut name = path.as_os_str().to_owned();
        name.push(suffix);
        match std::fs::remove_file(PathBuf::from(name)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

fn configure(conn: &Connection) -> AppResult<()> {
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA foreign_keys = ON;
         PRAGMA synchronous = NORMAL;
         PRAGMA busy_timeout = 5000;",
    )?;
    Ok(())
}

pub fn schema_version(conn: &Connection) -> AppResult<u32> {
    Ok(conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))? as u32)
}

/// Refuse a file another program wrote, or one a newer Brainiac migrated.
/// SQLite enforces neither header field, and running older code on a newer
/// schema would skip tables it does not know about.
fn check_compatible(conn: &Connection, path: &Path, store: &Store) -> AppResult<()> {
    let id: i32 = conn.query_row("PRAGMA application_id", [], |r| r.get(0))?;
    let fresh = schema_version(conn)? == 0;
    let accepted = id == store.application_id || (id == 0 && (store.adopt_unmarked || fresh));
    if !accepted {
        return Err(AppError::db(format!(
            "{} is not a Brainiac database.",
            path.display()
        )));
    }
    let version = schema_version(conn)?;
    let known = store.migrations.len() as u32;
    if version > known {
        return Err(AppError::db(format!(
            "This data was saved by a newer version of Brainiac. Install the newer \
             version, or restore a snapshot from {}.",
            backups_dir(path).display()
        ))
        .with_details(format!(
            "Data version {version}; this version of Brainiac reads up to {known}."
        )));
    }
    Ok(())
}

/// Apply the migrations `conn` does not have yet. Also used by restore, which
/// brings an older export's database up to date before preparing it.
pub fn migrate(conn: &mut Connection, store: &Store) -> AppResult<()> {
    let current = schema_version(conn)? as usize;
    for (index, (name, sql)) in store.migrations.iter().enumerate().skip(current) {
        // Rebuilding `agent_hosts` drops a table other rows reference.
        // SQLite ignores `foreign_keys` inside a transaction, so it is
        // set around this one migration and the new table is checked after.
        let rebuilds_parent = *name == "0011_agent_hosts_remote";
        if rebuilds_parent {
            conn.pragma_update(None, "foreign_keys", false)?;
        }
        let tx = conn.transaction()?;
        tx.execute_batch(sql).map_err(|e| {
            AppError::db(format!("Migration {name} failed.")).with_details(e.to_string())
        })?;
        tx.pragma_update(None, "user_version", (index + 1) as i64)?;
        tx.commit()?;
        if rebuilds_parent {
            conn.pragma_update(None, "foreign_keys", true)?;
            let broken: i64 =
                conn.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |r| {
                    r.get(0)
                })?;
            if broken != 0 {
                return Err(AppError::db(
                    "Migration 0011_agent_hosts_remote left a broken reference.",
                ));
            }
        }
        tracing::info!(migration = name, "applied database migration");
    }
    Ok(())
}

/// The folder snapshots of the database at `db_path` are written to.
pub fn backups_dir(db_path: &Path) -> PathBuf {
    db_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("backups")
}

/// Copy the database to `dest` through a hidden temporary file that is renamed
/// into place only when complete, so a crash never leaves a partial file that
/// looks like a finished backup.
fn write_backup(conn: &Connection, dest: &Path) -> AppResult<()> {
    let parent = dest.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let name = dest
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("backup");
    let partial = parent.join(format!(".{name}{PARTIAL_SUFFIX}"));
    let _ = std::fs::remove_file(&partial);
    let mut dst = Connection::open(&partial)?;
    let backup = rusqlite::backup::Backup::new(conn, &mut dst)?;
    backup.run_to_completion(256, std::time::Duration::from_millis(5), None)?;
    // `backup` borrows `dst`; dropping it ends the borrow so `dst` can be closed.
    drop(backup);
    // `close` hands the connection back with the error on failure; keep only the error.
    dst.close().map_err(|(_, e)| e)?;
    std::fs::rename(&partial, dest)?;
    Ok(())
}

/// Snapshot the database file at `src` (not opened by a worker) to `dest`.
pub fn snapshot_file(src: &Path, dest: &Path) -> AppResult<()> {
    let conn = Connection::open_with_flags(src, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    write_backup(&conn, dest)
}

/// Suffix of a backup still being written; see `write_backup`.
const PARTIAL_SUFFIX: &str = ".partial";

/// Snapshot an existing database before applying new migrations (SPEC.md, Backup and restore).
fn backup_before_migration_if_needed(
    conn: &Connection,
    db_path: &Path,
    store: &Store,
) -> AppResult<()> {
    let current = schema_version(conn)? as usize;
    if current == 0 || current >= store.migrations.len() || store.backups == Backups::Never {
        return Ok(());
    }
    let dest = backups_dir(db_path).join(format!(
        "{}pre-migration-v{}-{}.sqlite3",
        store.backup_prefix,
        current,
        chrono::Utc::now().format("%Y%m%dT%H%M%SZ")
    ));
    write_backup(conn, &dest)?;
    tracing::info!(path = %dest.display(), "wrote pre-migration backup");
    Ok(())
}

/// One backup per period, keeping the newest `keep`. The core's are named
/// `daily-<date>`, the history's `history-every-<date>`.
fn periodic_backup(conn: &Connection, db_path: &Path, store: &Store) -> AppResult<()> {
    let Backups::Every { days, keep } = store.backups else {
        return Ok(());
    };
    let dir = backups_dir(db_path);
    let prefix = if store.backup_prefix.is_empty() {
        "daily-".to_string()
    } else {
        format!("{}every-", store.backup_prefix)
    };
    let today = chrono::Local::now().date_naive();
    let name_of = |p: &PathBuf| {
        p.file_name()
            .and_then(|n| n.to_str())
            .map(str::to_owned)
            .unwrap_or_default()
    };
    let names: Vec<PathBuf> = match std::fs::read_dir(&dir) {
        Ok(rd) => rd.filter_map(|e| e.ok().map(|e| e.path())).collect(),
        Err(_) => Vec::new(),
    };
    let mut existing: Vec<PathBuf> = names
        .iter()
        .filter(|p| name_of(p).starts_with(&prefix) && name_of(p).ends_with(".sqlite3"))
        .cloned()
        .collect();
    existing.sort();
    let newest = existing.last().and_then(|p| {
        let name = name_of(p);
        let date = name.strip_prefix(&prefix)?.strip_suffix(".sqlite3")?;
        chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()
    });
    if newest.is_some_and(|d| (today - d).num_days() < i64::from(days)) {
        return Ok(());
    }
    let dest = dir.join(format!("{prefix}{}.sqlite3", today.format("%Y-%m-%d")));
    write_backup(conn, &dest)?;
    existing.push(dest);
    // Leftovers from a backup interrupted by a crash.
    for stale in names
        .iter()
        .filter(|p| name_of(p).ends_with(PARTIAL_SUFFIX))
    {
        let _ = std::fs::remove_file(stale);
    }
    while existing.len() > keep {
        let old = existing.remove(0);
        let _ = std::fs::remove_file(old);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

pub fn load_settings(conn: &Connection) -> AppResult<Settings> {
    let mut stmt = conn.prepare("SELECT key, value_json FROM settings")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    let mut map = serde_json::to_value(Settings::default())?;
    for row in rows {
        let (key, value) = row?;
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&value) {
            map[key] = v;
        }
    }
    Ok(serde_json::from_value(map).unwrap_or_default())
}

pub fn save_settings(conn: &mut Connection, settings: &Settings) -> AppResult<()> {
    let value = serde_json::to_value(settings)?;
    let tx = conn.transaction()?;
    if let serde_json::Value::Object(fields) = value {
        for (key, v) in fields {
            tx.execute(
                "INSERT INTO settings (key, value_json, version) VALUES (?1, ?2, 1)
                 ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, version = version + 1",
                params![key, v.to_string()],
            )?;
        }
    }
    tx.commit()?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Repositories
// ---------------------------------------------------------------------------

/// A repository row as stored. Converted to `RepositorySummary` by `workspaces.rs`.
#[derive(Debug, Clone, PartialEq)]
pub struct RepositoryRow {
    pub id: String,
    pub canonical_root: String,
    pub display_path: String,
    pub git_dir: String,
    pub common_git_dir: String,
    pub created_at: String,
    pub last_opened_at: Option<String>,
    pub last_checked_at: Option<String>,
    pub last_tab: Option<RepositoryTab>,
    pub status: Option<StatusSnapshot>,
    pub error: Option<AppError>,
    /// When Brainiac's own last fetch succeeded.
    pub last_fetch_at: Option<String>,
    /// Why Brainiac's last fetch failed; cleared by a successful one.
    pub last_fetch_error: Option<AppError>,
    /// The `origin` fetch URL as last observed.
    pub remote_url: Option<String>,
    /// Where its pull requests live when not where `origin` points (v0.3).
    pub forge_override: Option<ForgeRepository>,
}

impl From<&RepositoryRow> for crate::git::Checkout {
    fn from(row: &RepositoryRow) -> Self {
        let root = std::path::PathBuf::from(&row.canonical_root);
        crate::git::Checkout {
            repository_id: row.id.clone(),
            name: root
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| row.canonical_root.clone()),
            root,
            git_dir: row.git_dir.clone().into(),
            common_git_dir: row.common_git_dir.clone().into(),
        }
    }
}

const REPO_COLUMNS: &str = "id, canonical_root, display_path, git_dir, common_git_dir, created_at, last_opened_at, last_checked_at, last_tab, status_json, error_json, last_fetch_at, last_fetch_error_json, remote_url,
    (SELECT f.kind FROM repository_forges f WHERE f.repository_id = repositories.id),
    (SELECT f.owner FROM repository_forges f WHERE f.repository_id = repositories.id),
    (SELECT f.name FROM repository_forges f WHERE f.repository_id = repositories.id)";

fn row_to_repository(r: &rusqlite::Row<'_>) -> rusqlite::Result<RepositoryRow> {
    let last_tab: Option<String> = r.get(8)?;
    let status_json: Option<String> = r.get(9)?;
    let error_json: Option<String> = r.get(10)?;
    Ok(RepositoryRow {
        id: r.get(0)?,
        canonical_root: r.get(1)?,
        display_path: r.get(2)?,
        git_dir: r.get(3)?,
        common_git_dir: r.get(4)?,
        created_at: r.get(5)?,
        last_opened_at: r.get(6)?,
        last_checked_at: r.get(7)?,
        last_tab: last_tab.and_then(|t| serde_json::from_value(serde_json::Value::String(t)).ok()),
        status: status_json.and_then(|s| serde_json::from_str(&s).ok()),
        error: error_json.and_then(|s| serde_json::from_str(&s).ok()),
        last_fetch_at: r.get(11)?,
        last_fetch_error: r
            .get::<_, Option<String>>(12)?
            .and_then(|s| serde_json::from_str(&s).ok()),
        remote_url: r.get(13)?,
        forge_override: match (
            r.get::<_, Option<String>>(14)?,
            r.get::<_, Option<String>>(15)?,
            r.get::<_, Option<String>>(16)?,
        ) {
            (Some(kind), Some(owner), Some(name)) => {
                ForgeRepository::new(parse_enum(kind)?, &owner, &name)
            }
            _ => None,
        },
    })
}

pub fn list_repositories(conn: &Connection) -> AppResult<Vec<RepositoryRow>> {
    let sql = format!("SELECT {REPO_COLUMNS} FROM repositories ORDER BY display_path");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], row_to_repository)?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

pub fn get_repository(conn: &Connection, id: &str) -> AppResult<Option<RepositoryRow>> {
    let sql = format!("SELECT {REPO_COLUMNS} FROM repositories WHERE id = ?1");
    Ok(conn
        .query_row(&sql, params![id], row_to_repository)
        .optional()?)
}

pub fn find_repository_by_display_path(
    conn: &Connection,
    display_path: &str,
) -> AppResult<Option<RepositoryRow>> {
    let sql = format!("SELECT {REPO_COLUMNS} FROM repositories WHERE display_path = ?1");
    Ok(conn
        .query_row(&sql, params![display_path], row_to_repository)
        .optional()?)
}

pub fn insert_repository(conn: &Connection, row: &RepositoryRow) -> AppResult<()> {
    conn.execute(
        "INSERT INTO repositories (id, canonical_root, display_path, git_dir, common_git_dir, created_at, last_opened_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            row.id,
            row.canonical_root,
            row.display_path,
            row.git_dir,
            row.common_git_dir,
            row.created_at,
            row.last_opened_at
        ],
    )?;
    Ok(())
}

/// Record the `origin` URL a repository was last seen with.
pub fn set_remote_url(conn: &Connection, id: &str, url: Option<&str>) -> AppResult<()> {
    conn.execute(
        "UPDATE repositories SET remote_url = ?2 WHERE id = ?1 AND remote_url IS NOT ?2",
        params![id, url],
    )?;
    Ok(())
}

/// Point a repository's pull requests at `forge`, or back at its `origin` with `None`.
pub fn set_repository_forge(
    conn: &Connection,
    id: &str,
    forge: Option<&ForgeRepository>,
    now: &str,
) -> AppResult<()> {
    match forge {
        Some(f) => conn.execute(
            "INSERT INTO repository_forges (repository_id, kind, owner, name, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (repository_id) DO UPDATE SET
                 kind = excluded.kind, owner = excluded.owner, name = excluded.name",
            params![id, enum_name(f.kind)?, f.owner, f.name, now],
        )?,
        None => conn.execute(
            "DELETE FROM repository_forges WHERE repository_id = ?1",
            params![id],
        )?,
    };
    Ok(())
}

pub fn touch_repository_opened(conn: &Connection, id: &str, at: &str) -> AppResult<()> {
    conn.execute(
        "UPDATE repositories SET last_opened_at = ?2 WHERE id = ?1",
        params![id, at],
    )?;
    Ok(())
}

pub fn set_repository_tab(conn: &Connection, id: &str, tab: RepositoryTab) -> AppResult<()> {
    let tab = serde_json::to_value(tab)?
        .as_str()
        .unwrap_or("changes")
        .to_string();
    conn.execute(
        "UPDATE repositories SET last_tab = ?2 WHERE id = ?1",
        params![id, tab],
    )?;
    Ok(())
}

/// Store the outcome of a status observation of the working tree at `root`:
/// either a snapshot or an error. Returns false, storing nothing, when the
/// registration no longer points at `root` (it was relocated meanwhile).
pub fn store_observation(
    conn: &Connection,
    id: &str,
    root: &str,
    checked_at: &str,
    status: Option<&StatusSnapshot>,
    error: Option<&AppError>,
) -> AppResult<bool> {
    let status_json = status.map(serde_json::to_string).transpose()?;
    let error_json = error.map(serde_json::to_string).transpose()?;
    // Keep the last good snapshot when a refresh fails, so the UI can show stale data.
    Ok(conn.execute(
        "UPDATE repositories
         SET last_checked_at = ?2,
             status_json = COALESCE(?3, status_json),
             error_json = ?4
         WHERE id = ?1 AND canonical_root = ?5",
        params![id, checked_at, status_json, error_json, root],
    )? > 0)
}

/// Where a registration's working tree and Git directories are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    pub root: String,
    pub git_dir: String,
    pub common_git_dir: String,
}

/// Point a registration at a new location (SPEC.md, Relocating a repository).
/// The last error and fetch error described the old location and are cleared;
/// for a `different_repository`, so are its cached status and last fetch time.
pub fn set_repository_location(
    conn: &Connection,
    id: &str,
    location: &Location,
    different_repository: bool,
) -> AppResult<()> {
    conn.execute(
        "UPDATE repositories
         SET canonical_root = ?2, display_path = ?2, git_dir = ?3, common_git_dir = ?4,
             error_json = NULL, last_fetch_error_json = NULL,
             status_json = CASE WHEN ?5 THEN NULL ELSE status_json END,
             last_checked_at = CASE WHEN ?5 THEN NULL ELSE last_checked_at END,
             last_fetch_at = CASE WHEN ?5 THEN NULL ELSE last_fetch_at END
         WHERE id = ?1",
        params![
            id,
            location.root,
            location.git_dir,
            location.common_git_dir,
            different_repository
        ],
    )?;
    Ok(())
}

/// Remove a registration, its pin, and its workspace memberships. The schema's
/// `ON DELETE SET NULL` would otherwise leave those members behind as
/// non-Git rows; removing the registration is an explicit "stop tracking".
/// A workspace whose root this was keeps its other members (`root_repository_id`
/// becomes NULL through the foreign key).
pub fn delete_repository(conn: &Connection, id: &str) -> AppResult<Deleted> {
    let store: Option<String> = conn
        .query_row(
            "SELECT common_git_dir FROM repositories WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )
        .optional()?;
    conn.execute(
        "DELETE FROM pins WHERE entity_type = 'repository' AND entity_id = ?1",
        params![id],
    )?;
    conn.execute(
        "DELETE FROM workspace_members WHERE repository_id = ?1",
        params![id],
    )?;
    let removed = conn.execute("DELETE FROM repositories WHERE id = ?1", params![id])? > 0;
    let mut orphaned_store = None;
    if let Some(store) = store {
        let others: i64 = conn.query_row(
            "SELECT COUNT(*) FROM repositories WHERE common_git_dir = ?1",
            params![store],
            |r| r.get(0),
        )?;
        if others == 0 {
            orphaned_store = Some(store);
        }
    }
    Ok(Deleted {
        removed,
        orphaned_store,
    })
}

/// What `delete_repository` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Deleted {
    pub removed: bool,
    /// The shared Git directory that has no checkout left; the activity
    /// tracker, which owns its ref tracking, forgets it.
    pub orphaned_store: Option<String>,
}

/// Return the ID of the repository registered at `row.display_path`, inserting
/// `row` when there is none. The boolean is true when a row was inserted.
/// Running inside one database job keeps find-then-insert free of races.
pub fn ensure_repository(conn: &Connection, row: &RepositoryRow) -> AppResult<(String, bool)> {
    match find_repository_by_display_path(conn, &row.display_path)? {
        Some(existing) => Ok((existing.id, false)),
        None => {
            insert_repository(conn, row)?;
            Ok((row.id.clone(), true))
        }
    }
}

pub fn recent_repository_ids(conn: &Connection, limit: usize) -> AppResult<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT id FROM repositories WHERE last_opened_at IS NOT NULL ORDER BY last_opened_at DESC LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit as i64], |r| r.get::<_, String>(0))?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

// ---------------------------------------------------------------------------
// Workspaces
// ---------------------------------------------------------------------------

/// A workspace row as stored, without its members.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceRow {
    pub id: String,
    pub name: String,
    pub discovery_mode: DiscoveryMode,
    pub root_repository_id: Option<String>,
    pub discovery_root: Option<String>,
    pub discovery_path: Option<String>,
    pub created_at: String,
    /// Whether the workspace tracks its repositories' pull requests (v0.3).
    pub pull_requests: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MemberRow {
    pub id: String,
    pub workspace_id: String,
    pub display_name: String,
    pub canonical_path: String,
    pub origin: MemberOrigin,
    pub repository_id: Option<String>,
}

/// The `snake_case` name serde gives a unit enum variant, which is also the
/// value stored in the lookup tables.
pub(crate) fn enum_name<T: serde::Serialize>(value: T) -> AppResult<String> {
    match serde_json::to_value(value)? {
        serde_json::Value::String(s) => Ok(s),
        other => Err(AppError::db("Unexpected enum encoding.").with_details(other.to_string())),
    }
}

/// Parse a stored enum name back; `T: DeserializeOwned` means any serde type
/// that can be built from owned data (no borrowed strings).
pub(crate) fn parse_enum<T: serde::de::DeserializeOwned>(name: String) -> rusqlite::Result<T> {
    serde_json::from_value(serde_json::Value::String(name)).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })
}

const WORKSPACE_COLUMNS: &str =
    "id, name, discovery_mode, root_repository_id, discovery_root, discovery_path, created_at, pull_requests";

fn row_to_workspace(r: &rusqlite::Row<'_>) -> rusqlite::Result<WorkspaceRow> {
    Ok(WorkspaceRow {
        id: r.get(0)?,
        name: r.get(1)?,
        discovery_mode: parse_enum(r.get(2)?)?,
        root_repository_id: r.get(3)?,
        discovery_root: r.get(4)?,
        discovery_path: r.get(5)?,
        created_at: r.get(6)?,
        pull_requests: bool_col(r, 7)?,
    })
}

const MEMBER_COLUMNS: &str =
    "id, workspace_id, display_name, canonical_path, origin, repository_id";

fn row_to_member(r: &rusqlite::Row<'_>) -> rusqlite::Result<MemberRow> {
    Ok(MemberRow {
        id: r.get(0)?,
        workspace_id: r.get(1)?,
        display_name: r.get(2)?,
        canonical_path: r.get(3)?,
        origin: parse_enum(r.get(4)?)?,
        repository_id: r.get(5)?,
    })
}

pub fn insert_workspace(conn: &Connection, row: &WorkspaceRow) -> AppResult<()> {
    conn.execute(
        "INSERT INTO workspaces (id, name, discovery_mode, root_repository_id, discovery_root, discovery_path, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            row.id,
            row.name,
            enum_name(row.discovery_mode)?,
            row.root_repository_id,
            row.discovery_root,
            row.discovery_path,
            row.created_at
        ],
    )?;
    Ok(())
}

pub fn get_workspace(conn: &Connection, id: &str) -> AppResult<Option<WorkspaceRow>> {
    let sql = format!("SELECT {WORKSPACE_COLUMNS} FROM workspaces WHERE id = ?1");
    Ok(conn
        .query_row(&sql, params![id], row_to_workspace)
        .optional()?)
}

/// All workspaces, ordered by name ignoring case.
pub fn list_workspaces(conn: &Connection) -> AppResult<Vec<WorkspaceRow>> {
    let sql = format!(
        "SELECT {WORKSPACE_COLUMNS} FROM workspaces ORDER BY name COLLATE NOCASE, name, id"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], row_to_workspace)?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// Members of one workspace, or of every workspace when `workspace_id` is `None`.
pub fn list_members(conn: &Connection, workspace_id: Option<&str>) -> AppResult<Vec<MemberRow>> {
    let sql = format!(
        "SELECT {MEMBER_COLUMNS} FROM workspace_members
         WHERE ?1 IS NULL OR workspace_id = ?1
         ORDER BY workspace_id, display_name COLLATE NOCASE, canonical_path"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![workspace_id], row_to_member)?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// Insert a member unless the workspace already has one at that path.
/// Returns true when a row was inserted.
pub fn insert_member(conn: &Connection, row: &MemberRow) -> AppResult<bool> {
    Ok(conn.execute(
        "INSERT OR IGNORE INTO workspace_members (id, workspace_id, display_name, canonical_path, origin, repository_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            row.id,
            row.workspace_id,
            row.display_name,
            row.canonical_path,
            enum_name(row.origin)?,
            row.repository_id
        ],
    )? > 0)
}

/// Delete one member by path and return it, or `None` when there was no such member.
pub fn delete_member(
    conn: &Connection,
    workspace_id: &str,
    canonical_path: &str,
) -> AppResult<Option<MemberRow>> {
    let sql = format!(
        "SELECT {MEMBER_COLUMNS} FROM workspace_members WHERE workspace_id = ?1 AND canonical_path = ?2"
    );
    let member = conn
        .query_row(&sql, params![workspace_id, canonical_path], row_to_member)
        .optional()?;
    if let Some(m) = &member {
        conn.execute("DELETE FROM workspace_members WHERE id = ?1", params![m.id])?;
    }
    Ok(member)
}

/// Move a member to another path. A different member of the same workspace
/// already at that path (a non-Git folder, say) is removed first, since a
/// workspace has one member per path.
pub fn move_member(
    conn: &Connection,
    member_id: &str,
    canonical_path: &str,
    display_name: &str,
    origin: MemberOrigin,
) -> AppResult<()> {
    conn.execute(
        "DELETE FROM workspace_members
         WHERE canonical_path = ?2 AND id <> ?1
           AND workspace_id = (SELECT workspace_id FROM workspace_members WHERE id = ?1)",
        params![member_id, canonical_path],
    )?;
    conn.execute(
        "UPDATE workspace_members SET canonical_path = ?2, display_name = ?3, origin = ?4 WHERE id = ?1",
        params![member_id, canonical_path, display_name, enum_name(origin)?],
    )?;
    Ok(())
}

pub fn set_discovery_root(conn: &Connection, workspace_id: &str, root: &str) -> AppResult<()> {
    conn.execute(
        "UPDATE workspaces SET discovery_root = ?2 WHERE id = ?1",
        params![workspace_id, root],
    )?;
    Ok(())
}

pub fn set_workspace_root(
    conn: &Connection,
    workspace_id: &str,
    root_repository_id: Option<&str>,
) -> AppResult<()> {
    conn.execute(
        "UPDATE workspaces SET root_repository_id = ?2 WHERE id = ?1",
        params![workspace_id, root_repository_id],
    )?;
    Ok(())
}

/// Turn a workspace's pull request tracking on or off; `false` when it does not exist.
pub fn set_workspace_pull_requests(conn: &Connection, id: &str, enabled: bool) -> AppResult<bool> {
    let n = conn.execute(
        "UPDATE workspaces SET pull_requests = ?2 WHERE id = ?1",
        params![id, enabled],
    )?;
    Ok(n > 0)
}

/// Every repository whose pull requests are tracked: a member of a workspace
/// with pull requests on, with its `origin` URL and its override, if any.
pub fn tracked_repositories(conn: &Connection) -> AppResult<Vec<RepositoryRow>> {
    let sql = format!(
        "SELECT {REPO_COLUMNS} FROM repositories WHERE id IN (
             SELECT m.repository_id FROM workspace_members m
             JOIN workspaces w ON w.id = m.workspace_id
             WHERE w.pull_requests = 1 AND m.repository_id IS NOT NULL)
         ORDER BY display_path"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], row_to_repository)?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

pub fn rename_workspace(conn: &Connection, id: &str, name: &str) -> AppResult<bool> {
    Ok(conn.execute(
        "UPDATE workspaces SET name = ?2 WHERE id = ?1",
        params![id, name],
    )? > 0)
}

/// Delete a workspace, its pin, and (through `ON DELETE CASCADE`) its members.
/// Repository registrations are untouched.
pub fn delete_workspace(conn: &Connection, id: &str) -> AppResult<bool> {
    conn.execute(
        "DELETE FROM pins WHERE entity_type = 'workspace' AND entity_id = ?1",
        params![id],
    )?;
    Ok(conn.execute("DELETE FROM workspaces WHERE id = ?1", params![id])? > 0)
}

// ---------------------------------------------------------------------------
// Pins
// ---------------------------------------------------------------------------

pub fn list_pins(conn: &Connection) -> AppResult<Vec<Pin>> {
    let mut stmt =
        conn.prepare("SELECT entity_type, entity_id, position FROM pins ORDER BY position")?;
    let rows = stmt.query_map([], |r| {
        let kind: String = r.get(0)?;
        Ok(Pin {
            entity_type: match kind.as_str() {
                "workspace" => PinEntityType::Workspace,
                "note" => PinEntityType::Note,
                _ => PinEntityType::Repository,
            },
            entity_id: r.get(1)?,
            position: r.get(2)?,
        })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

pub fn set_pinned(
    conn: &Connection,
    entity_type: PinEntityType,
    entity_id: &str,
    pinned: bool,
) -> AppResult<()> {
    let kind = match entity_type {
        PinEntityType::Repository => "repository",
        PinEntityType::Workspace => "workspace",
        PinEntityType::Note => "note",
    };
    if pinned {
        conn.execute(
            "INSERT OR IGNORE INTO pins (entity_type, entity_id, position)
             VALUES (?1, ?2, (SELECT COALESCE(MAX(position), 0) + 1 FROM pins))",
            params![kind, entity_id],
        )?;
    } else {
        conn.execute(
            "DELETE FROM pins WHERE entity_type = ?1 AND entity_id = ?2",
            params![kind, entity_id],
        )?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Fetching and activity
// ---------------------------------------------------------------------------

/// Record the outcome of one of Brainiac's fetches for every checkout of a
/// shared Git directory. A success clears the error; a failure keeps the time
/// of the last success.
pub fn store_fetch_outcome(
    conn: &Connection,
    git_store: &str,
    at: &str,
    error: Option<&AppError>,
) -> AppResult<()> {
    match error {
        None => conn.execute(
            "UPDATE repositories SET last_fetch_at = ?2, last_fetch_error_json = NULL WHERE common_git_dir = ?1",
            params![git_store, at],
        )?,
        Some(e) => conn.execute(
            "UPDATE repositories SET last_fetch_error_json = ?2 WHERE common_git_dir = ?1",
            params![git_store, serde_json::to_string(e)?],
        )?,
    };
    Ok(())
}

fn bool_col(r: &rusqlite::Row<'_>, i: usize) -> rusqlite::Result<bool> {
    Ok(r.get::<_, i64>(i)? != 0)
}

fn json_list(text: String) -> Vec<String> {
    serde_json::from_str(&text).unwrap_or_default()
}

const ACTIVITY_COLUMNS: &str = "w.id, w.watched_branches_json, w.watched_tags_json, w.auto_fetch, w.notify_moves, w.morning_digest, w.warn_conflicts, w.watched_since_json, w.created_at";

/// A workspace's activity settings and, for each watched pattern, since when
/// it is watched. Events observed before a pattern was watched are not that
/// workspace's news (SPEC.md, Workspace activity).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchConfig {
    pub settings: ActivitySettings,
    /// `"b:<pattern>"` or `"t:<pattern>"` → RFC 3339 time it started being
    /// watched; patterns without an entry date from the workspace's creation.
    pub since: std::collections::HashMap<String, String>,
}

impl WatchConfig {
    /// `(is_tag, pattern, since)` for every watched pattern.
    pub fn patterns(&self) -> Vec<(bool, String, String)> {
        let entry = |tag: bool, p: &String| {
            let key = format!("{}:{p}", if tag { "t" } else { "b" });
            (
                tag,
                p.clone(),
                self.since.get(&key).cloned().unwrap_or_default(),
            )
        };
        let s = &self.settings;
        s.watched_branches
            .iter()
            .map(|p| entry(false, p))
            .chain(s.watched_tags.iter().map(|p| entry(true, p)))
            .collect()
    }
}

fn since_map(
    json: &str,
    created_at: &str,
    settings: &ActivitySettings,
) -> std::collections::HashMap<String, String> {
    let stored: std::collections::HashMap<String, String> =
        serde_json::from_str(json).unwrap_or_default();
    let mut since = std::collections::HashMap::new();
    for (tag, list) in [
        (false, &settings.watched_branches),
        (true, &settings.watched_tags),
    ] {
        for p in list {
            let key = format!("{}:{p}", if tag { "t" } else { "b" });
            let at = stored
                .get(&key)
                .cloned()
                .unwrap_or_else(|| created_at.to_string());
            since.insert(key, at);
        }
    }
    since
}

fn row_to_activity(r: &rusqlite::Row<'_>) -> rusqlite::Result<(String, WatchConfig)> {
    let settings = ActivitySettings {
        watched_branches: json_list(r.get(1)?),
        watched_tags: json_list(r.get(2)?),
        auto_fetch: bool_col(r, 3)?,
        notify_moves: bool_col(r, 4)?,
        morning_digest: bool_col(r, 5)?,
        warn_conflicts: bool_col(r, 6)?,
    };
    let since = since_map(&r.get::<_, String>(7)?, &r.get::<_, String>(8)?, &settings);
    Ok((r.get(0)?, WatchConfig { settings, since }))
}

/// Activity configuration of every workspace, keyed by workspace ID.
pub fn activity_settings(
    conn: &Connection,
) -> AppResult<std::collections::HashMap<String, WatchConfig>> {
    let sql = format!("SELECT {ACTIVITY_COLUMNS} FROM workspaces w");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], row_to_activity)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// A workspace that watches a shared Git directory through one of its members.
#[derive(Debug, Clone, PartialEq)]
pub struct Watcher {
    pub workspace_id: String,
    pub workspace_name: String,
    pub config: WatchConfig,
}

/// Workspaces with at least one member checkout of `git_store`.
pub fn watchers_of_store(conn: &Connection, git_store: &str) -> AppResult<Vec<Watcher>> {
    let sql = format!(
        "SELECT {ACTIVITY_COLUMNS}, w.name FROM workspaces w
         WHERE EXISTS (
           SELECT 1 FROM workspace_members m JOIN repositories r ON r.id = m.repository_id
           WHERE m.workspace_id = w.id AND r.common_git_dir = ?1)
         ORDER BY w.name COLLATE NOCASE, w.id"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![git_store], |r| {
        let (workspace_id, config) = row_to_activity(r)?;
        Ok(Watcher {
            workspace_id,
            workspace_name: r.get(9)?,
            config,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Save a workspace's activity settings. Patterns it already watched keep
/// their start time; new ones start now. Returns the previous settings, or
/// `None` when there is no such workspace.
pub fn set_activity_settings(
    conn: &mut Connection,
    workspace_id: &str,
    settings: &ActivitySettings,
    now: &str,
) -> AppResult<Option<ActivitySettings>> {
    let tx = conn.transaction()?;
    let sql = format!("SELECT {ACTIVITY_COLUMNS} FROM workspaces w WHERE w.id = ?1");
    let Some((_, previous)) = tx
        .query_row(&sql, params![workspace_id], row_to_activity)
        .optional()?
    else {
        return Ok(None);
    };
    let mut since = std::collections::HashMap::new();
    for (tag, list) in [
        (false, &settings.watched_branches),
        (true, &settings.watched_tags),
    ] {
        for p in list {
            let key = format!("{}:{p}", if tag { "t" } else { "b" });
            let at = previous
                .since
                .get(&key)
                .cloned()
                .unwrap_or_else(|| now.to_string());
            since.insert(key, at);
        }
    }
    tx.execute(
        "UPDATE workspaces SET watched_branches_json = ?2, watched_tags_json = ?3, auto_fetch = ?4,
             notify_moves = ?5, morning_digest = ?6, warn_conflicts = ?7, watched_since_json = ?8
         WHERE id = ?1",
        params![
            workspace_id,
            serde_json::to_string(&settings.watched_branches)?,
            serde_json::to_string(&settings.watched_tags)?,
            settings.auto_fetch,
            settings.notify_moves,
            settings.morning_digest,
            settings.warn_conflicts,
            serde_json::to_string(&since)?
        ],
    )?;
    tx.commit()?;
    Ok(Some(previous.settings))
}

/// The local date of each workspace's last digest check, keyed by workspace ID.
pub fn last_digest_dates(
    conn: &Connection,
) -> AppResult<std::collections::HashMap<String, Option<String>>> {
    let mut stmt = conn.prepare("SELECT id, last_digest_on FROM workspaces")?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn set_last_digest_on(conn: &Connection, workspace_id: &str, date: &str) -> AppResult<()> {
    conn.execute(
        "UPDATE workspaces SET last_digest_on = ?2 WHERE id = ?1",
        params![workspace_id, date],
    )?;
    Ok(())
}

/// The shared Git directory of every registered repository, by repository ID.
pub fn repository_stores(
    conn: &Connection,
) -> AppResult<std::collections::HashMap<String, String>> {
    let mut stmt = conn.prepare("SELECT id, common_git_dir FROM repositories")?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}
