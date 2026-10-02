//! SQLite persistence: a single connection owned by a dedicated worker thread,
//! ordered migrations, and consistent backups.
//!
//! Callers never touch the connection directly. They send a closure to the
//! worker (`Db::call`) and await its result, which keeps all database access
//! serialized and off the async executor threads (docs/architecture.md, Concurrency and lifecycle).

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;

use rusqlite::{params, Connection, OptionalExtension};

use crate::models::{
    ActivitySettings, AppError, AppResult, DiscoveryMode, MemberOrigin, Pin, PinEntityType,
    RepositoryTab, Settings, StatusSnapshot,
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

/// Snapshot an existing database before applying new migrations (docs/roadmap.md, Backup and restore).
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
