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
    ActivityDetail, ActivityKind, ActivitySettings, AppError, AppResult, DiscoveryMode,
    MemberOrigin, Pin, PinEntityType, RepositoryTab, Settings, StatusSnapshot,
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
    /// When Brainiac's own last fetch succeeded.
    pub last_fetch_at: Option<String>,
    /// Why Brainiac's last fetch failed; cleared by a successful one.
    pub last_fetch_error: Option<AppError>,
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

const REPO_COLUMNS: &str = "id, canonical_root, display_path, git_dir, common_git_dir, created_at, last_opened_at, last_checked_at, last_tab, status_json, error_json, last_fetch_at, last_fetch_error_json";

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

/// Remove a registration, its pin, and its workspace memberships. The schema's
/// `ON DELETE SET NULL` would otherwise leave those members behind as
/// non-Git rows; removing the registration is an explicit "stop tracking".
/// A workspace whose root this was keeps its other members (`root_repository_id`
/// becomes NULL through the foreign key).
pub fn delete_repository(conn: &Connection, id: &str) -> AppResult<bool> {
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
    // Ref tracking belongs to the shared Git directory; drop it with its last checkout.
    if let Some(store) = store {
        let others: i64 = conn.query_row(
            "SELECT COUNT(*) FROM repositories WHERE common_git_dir = ?1",
            params![store],
            |r| r.get(0),
        )?;
        if others == 0 {
            forget_store(conn, &store, true)?;
        }
    }
    Ok(removed)
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
fn enum_name<T: serde::Serialize>(value: T) -> AppResult<String> {
    match serde_json::to_value(value)? {
        serde_json::Value::String(s) => Ok(s),
        other => Err(AppError::db("Unexpected enum encoding.").with_details(other.to_string())),
    }
}

/// Parse a stored enum name back; `T: DeserializeOwned` means any serde type
/// that can be built from owned data (no borrowed strings).
fn parse_enum<T: serde::de::DeserializeOwned>(name: String) -> rusqlite::Result<T> {
    serde_json::from_value(serde_json::Value::String(name)).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })
}

const WORKSPACE_COLUMNS: &str =
    "id, name, discovery_mode, root_repository_id, discovery_root, discovery_path, created_at";

fn row_to_workspace(r: &rusqlite::Row<'_>) -> rusqlite::Result<WorkspaceRow> {
    Ok(WorkspaceRow {
        id: r.get(0)?,
        name: r.get(1)?,
        discovery_mode: parse_enum(r.get(2)?)?,
        root_repository_id: r.get(3)?,
        discovery_root: r.get(4)?,
        discovery_path: r.get(5)?,
        created_at: r.get(6)?,
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

const ACTIVITY_COLUMNS: &str = "w.id, w.watched_branches_json, w.watched_tags_json, w.auto_fetch, w.notify_moves, w.morning_digest, w.warn_conflicts";

fn row_to_activity(r: &rusqlite::Row<'_>) -> rusqlite::Result<(String, ActivitySettings)> {
    Ok((
        r.get(0)?,
        ActivitySettings {
            watched_branches: json_list(r.get(1)?),
            watched_tags: json_list(r.get(2)?),
            auto_fetch: bool_col(r, 3)?,
            notify_moves: bool_col(r, 4)?,
            morning_digest: bool_col(r, 5)?,
            warn_conflicts: bool_col(r, 6)?,
        },
    ))
}

/// Activity settings of every workspace, keyed by workspace ID.
pub fn activity_settings(
    conn: &Connection,
) -> AppResult<std::collections::HashMap<String, ActivitySettings>> {
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
    pub settings: ActivitySettings,
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
        let (workspace_id, settings) = row_to_activity(r)?;
        Ok(Watcher {
            workspace_id,
            workspace_name: r.get(7)?,
            settings,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn set_activity_settings(
    conn: &Connection,
    workspace_id: &str,
    settings: &ActivitySettings,
) -> AppResult<bool> {
    Ok(conn.execute(
        "UPDATE workspaces SET watched_branches_json = ?2, watched_tags_json = ?3, auto_fetch = ?4,
             notify_moves = ?5, morning_digest = ?6, warn_conflicts = ?7
         WHERE id = ?1",
        params![
            workspace_id,
            serde_json::to_string(&settings.watched_branches)?,
            serde_json::to_string(&settings.watched_tags)?,
            settings.auto_fetch,
            settings.notify_moves,
            settings.morning_digest,
            settings.warn_conflicts
        ],
    )? > 0)
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

/// The baseline of a shared Git directory: the watched patterns it was taken
/// with (JSON) and the stored tips, full ref name to commit ID.
pub fn baseline(
    conn: &Connection,
    git_store: &str,
) -> AppResult<Option<(String, std::collections::HashMap<String, String>)>> {
    let watched: Option<String> = conn
        .query_row(
            "SELECT watched_json FROM ref_baselines WHERE git_store = ?1",
            params![git_store],
            |r| r.get(0),
        )
        .optional()?;
    let Some(watched) = watched else {
        return Ok(None);
    };
    let mut stmt = conn.prepare("SELECT ref_name, target_id FROM ref_tips WHERE git_store = ?1")?;
    let rows = stmt.query_map(params![git_store], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(Some((watched, rows.collect::<Result<_, _>>()?)))
}

/// A stored activity event.
#[derive(Debug, Clone, PartialEq)]
pub struct EventRow {
    pub id: String,
    pub git_store: String,
    pub kind: ActivityKind,
    /// Full ref name.
    pub ref_name: String,
    /// What workspace patterns match: the branch without its remote, or the tag.
    pub match_name: String,
    pub old_id: Option<String>,
    pub new_id: String,
    pub observed_at: String,
    pub seen_at: Option<String>,
    pub detail: ActivityDetail,
}

/// Write one tracking pass atomically: the new baseline (patterns and tips)
/// and its events, and prune events older than `prune_before`.
pub fn save_pass(
    conn: &mut Connection,
    git_store: &str,
    watched_json: &str,
    tips: &[(String, String)],
    events: &[EventRow],
    at: &str,
    prune_before: &str,
) -> AppResult<()> {
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO ref_baselines (git_store, watched_json, taken_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(git_store) DO UPDATE SET watched_json = excluded.watched_json, taken_at = excluded.taken_at",
        params![git_store, watched_json, at],
    )?;
    tx.execute(
        "DELETE FROM ref_tips WHERE git_store = ?1",
        params![git_store],
    )?;
    for (name, target) in tips {
        tx.execute(
            "INSERT INTO ref_tips (git_store, ref_name, target_id) VALUES (?1, ?2, ?3)",
            params![git_store, name, target],
        )?;
    }
    for e in events {
        tx.execute(
            "INSERT INTO activity_events (id, git_store, kind, ref_name, match_name, old_id, new_id, observed_at, seen_at, detail_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                e.id,
                e.git_store,
                enum_name(e.kind)?,
                e.ref_name,
                e.match_name,
                e.old_id,
                e.new_id,
                e.observed_at,
                e.seen_at,
                serde_json::to_string(&e.detail)?
            ],
        )?;
    }
    tx.execute(
        "DELETE FROM activity_events WHERE observed_at < ?1",
        params![prune_before],
    )?;
    tx.commit()?;
    Ok(())
}

/// Drop a shared Git directory's baseline and tips, so the next pass starts
/// over silently; `events` also drops its feed.
pub fn forget_store(conn: &Connection, git_store: &str, events: bool) -> AppResult<()> {
    conn.execute(
        "DELETE FROM ref_baselines WHERE git_store = ?1",
        params![git_store],
    )?;
    conn.execute(
        "DELETE FROM ref_tips WHERE git_store = ?1",
        params![git_store],
    )?;
    if events {
        conn.execute(
            "DELETE FROM activity_events WHERE git_store = ?1",
            params![git_store],
        )?;
    }
    Ok(())
}

fn row_to_event(r: &rusqlite::Row<'_>) -> rusqlite::Result<EventRow> {
    Ok(EventRow {
        id: r.get(0)?,
        git_store: r.get(1)?,
        kind: parse_enum(r.get(2)?)?,
        ref_name: r.get(3)?,
        match_name: r.get(4)?,
        old_id: r.get(5)?,
        new_id: r.get(6)?,
        observed_at: r.get(7)?,
        seen_at: r.get(8)?,
        detail: serde_json::from_str(&r.get::<_, String>(9)?).unwrap_or_default(),
    })
}

const EVENT_COLUMNS: &str =
    "id, git_store, kind, ref_name, match_name, old_id, new_id, observed_at, seen_at, detail_json";

/// Events of the given shared Git directories, unread first, newest first, at
/// most `limit`. Callers filter by a workspace's patterns.
pub fn list_events(
    conn: &Connection,
    git_stores: &[String],
    limit: usize,
) -> AppResult<Vec<EventRow>> {
    if git_stores.is_empty() {
        return Ok(Vec::new());
    }
    // A JSON array keeps the store list to one bound parameter.
    let sql = format!(
        "SELECT {EVENT_COLUMNS} FROM activity_events
         WHERE git_store IN (SELECT value FROM json_each(?1))
         ORDER BY seen_at IS NOT NULL, observed_at DESC, id
         LIMIT ?2"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(
        params![serde_json::to_string(git_stores)?, limit as i64],
        row_to_event,
    )?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Every unread event, for per-workspace unread counts.
pub fn unseen_events(conn: &Connection) -> AppResult<Vec<EventRow>> {
    let sql = format!("SELECT {EVENT_COLUMNS} FROM activity_events WHERE seen_at IS NULL");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], row_to_event)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Mark the given events as seen.
pub fn mark_events_seen(conn: &Connection, event_ids: &[String], at: &str) -> AppResult<usize> {
    Ok(conn.execute(
        "UPDATE activity_events SET seen_at = ?2
         WHERE seen_at IS NULL AND id IN (SELECT value FROM json_each(?1))",
        params![serde_json::to_string(event_ids)?, at],
    )?)
}

/// The shared Git directory of every registered repository, by repository ID.
pub fn repository_stores(
    conn: &Connection,
) -> AppResult<std::collections::HashMap<String, String>> {
    let mut stmt = conn.prepare("SELECT id, common_git_dir FROM repositories")?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}
