//! SQLite persistence: a single connection owned by a dedicated worker thread,
//! ordered migrations, and consistent backups.
//!
//! Callers never touch the connection directly. They send a closure to the
//! worker (`Db::call`) and await its result, which keeps all database access
//! serialized and off the async executor threads (SPEC §5).

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;

use rusqlite::{params, Connection, OptionalExtension};

use crate::models::{
    AppError, AppResult, Pin, PinEntityType, RepositoryTab, Settings, StatusSnapshot,
};

/// Ordered migrations. Add new entries at the end; never edit a shipped one.
const MIGRATIONS: &[(&str, &str)] = &[("0001_init", include_str!("../migrations/0001_init.sql"))];

/// How many daily backups to keep.
const BACKUP_RETENTION: usize = 7;

type Job = Box<dyn FnOnce(&mut Connection) + Send + 'static>;

/// Handle to the database worker. Cheap to clone; all clones share one thread.
#[derive(Clone)]
pub struct Db {
    sender: mpsc::Sender<Job>,
    path: PathBuf,
}

impl Db {
    /// Open (or create) the database at `path`, run pending migrations, and
    /// start the worker thread. Takes a daily backup before the first write.
    pub fn open(path: &Path) -> AppResult<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut conn = Connection::open(path)?;
        configure(&conn)?;
        backup_before_migration_if_needed(&conn, path)?;
        migrate(&mut conn)?;
        daily_backup(&conn, path)?;

        let (sender, receiver) = mpsc::channel::<Job>();
        thread::Builder::new()
            .name("brainiac-db".into())
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

    /// Convenience for tests and tools: an in-memory-like database in a temp dir.
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

fn migrate(conn: &mut Connection) -> AppResult<()> {
    let current = schema_version(conn)? as usize;
    for (index, (name, sql)) in MIGRATIONS.iter().enumerate().skip(current) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql).map_err(|e| {
            AppError::db(format!("Migration {name} failed.")).with_details(e.to_string())
        })?;
        tx.pragma_update(None, "user_version", (index + 1) as i64)?;
        tx.commit()?;
        tracing::info!(migration = name, "applied database migration");
    }
    Ok(())
}

fn backups_dir(db_path: &Path) -> PathBuf {
    db_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("backups")
}

fn write_backup(conn: &Connection, dest: &Path) -> AppResult<()> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut dst = Connection::open(dest)?;
    let backup = rusqlite::backup::Backup::new(conn, &mut dst)?;
    backup.run_to_completion(256, std::time::Duration::from_millis(5), None)?;
    Ok(())
}

/// Snapshot an existing database before applying new migrations (SPEC §8).
fn backup_before_migration_if_needed(conn: &Connection, db_path: &Path) -> AppResult<()> {
    let current = schema_version(conn)? as usize;
    if current == 0 || current >= MIGRATIONS.len() {
        return Ok(());
    }
    let dest = backups_dir(db_path).join(format!(
        "pre-migration-v{}-{}.sqlite3",
        current,
        chrono::Utc::now().format("%Y%m%dT%H%M%SZ")
    ));
    write_backup(conn, &dest)?;
    tracing::info!(path = %dest.display(), "wrote pre-migration backup");
    Ok(())
}

/// One backup per calendar day, keeping the newest `BACKUP_RETENTION`.
fn daily_backup(conn: &Connection, db_path: &Path) -> AppResult<()> {
    let dir = backups_dir(db_path);
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let dest = dir.join(format!("daily-{today}.sqlite3"));
    if dest.exists() {
        return Ok(());
    }
    write_backup(conn, &dest)?;
    let mut daily: Vec<PathBuf> = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("daily-"))
        })
        .collect();
    daily.sort();
    while daily.len() > BACKUP_RETENTION {
        let old = daily.remove(0);
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
}

const REPO_COLUMNS: &str = "id, canonical_root, display_path, git_dir, common_git_dir, created_at, last_opened_at, last_checked_at, last_tab, status_json, error_json";

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

/// Store the outcome of a status observation: either a snapshot or an error.
pub fn store_observation(
    conn: &Connection,
    id: &str,
    checked_at: &str,
    status: Option<&StatusSnapshot>,
    error: Option<&AppError>,
) -> AppResult<()> {
    let status_json = status.map(serde_json::to_string).transpose()?;
    let error_json = error.map(serde_json::to_string).transpose()?;
    // Keep the last good snapshot when a refresh fails, so the UI can show stale data.
    conn.execute(
        "UPDATE repositories
         SET last_checked_at = ?2,
             status_json = COALESCE(?3, status_json),
             error_json = ?4
         WHERE id = ?1",
        params![id, checked_at, status_json, error_json],
    )?;
    Ok(())
}

pub fn delete_repository(conn: &Connection, id: &str) -> AppResult<bool> {
    conn.execute(
        "DELETE FROM pins WHERE entity_type = 'repository' AND entity_id = ?1",
        params![id],
    )?;
    Ok(conn.execute("DELETE FROM repositories WHERE id = ?1", params![id])? > 0)
}

pub fn recent_repository_ids(conn: &Connection, limit: usize) -> AppResult<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT id FROM repositories WHERE last_opened_at IS NOT NULL ORDER BY last_opened_at DESC LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit as i64], |r| r.get::<_, String>(0))?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
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
            entity_type: if kind == "workspace" {
                PinEntityType::Workspace
            } else {
                PinEntityType::Repository
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
