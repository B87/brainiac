//! Export and restore (SPEC.md, section 8; docs/architecture.md, Backups).
//!
//! An export is a folder: the vault's files, a consistent copy of
//! `brainiac.db`, the tasks as JSON, and a manifest. Restore checks that the
//! export is Brainiac's and readable by this version before replacing
//! anything, stages the database, and applies it at the next launch, when
//! nothing holds it open. The search index is never exported; it is rebuilt.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::db::{self, APPLICATION_ID, SCHEMA_VERSION};
use crate::index;
use crate::models::{
    now_rfc3339, AppError, AppResult, ExportResult, ExportedRepository, RestorePreview,
    RestoreRequest, RestoreResult, Task, TaskFilter,
};
use crate::notes::NoteService;

/// Version of the export folder's layout and manifest.
pub const EXPORT_FORMAT: u32 = 1;
const MANIFEST: &str = "manifest.json";
const DATABASE: &str = "brainiac.sqlite3";
const TASKS: &str = "tasks.json";
const VAULT: &str = "vault";
/// A restored database waiting for the next launch.
pub const PENDING_RESTORE: &str = "restore-pending.sqlite3";
/// How often a file that changes while it is copied is tried again.
const COPY_ATTEMPTS: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub format: u32,
    pub app: String,
    pub app_version: String,
    pub exported_at: String,
    pub schema_version: u32,
    pub vault: Option<ManifestVault>,
    pub notes: Vec<ManifestNote>,
    pub repositories: Vec<ManifestRepository>,
    pub tasks: u64,
    /// Files that changed during the export and were not copied consistently.
    pub problems: Vec<String>,
    pub complete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestVault {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestNote {
    pub id: String,
    pub relative_path: String,
    pub content_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestRepository {
    pub id: String,
    pub name: String,
    pub remote_url: Option<String>,
}

/// A task as `tasks.json` lists it: the task and its repository's identity.
#[derive(Debug, Clone, Serialize)]
struct ExportedTask {
    #[serde(flatten)]
    task: Task,
    repository: Option<ManifestRepository>,
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> AppResult<T> + Send + 'static,
) -> AppResult<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| AppError::io("The export stopped unexpectedly.").with_details(e.to_string()))?
}

fn folder_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

/// Write an export into a new folder inside `parent`. Brainiac's own writes
/// wait until it finishes; a file another program changes meanwhile is
/// copied again, and reported if it never holds still.
pub async fn export(notes: &Arc<NoteService>, parent: &Path) -> AppResult<ExportResult> {
    let _gate = notes.gate.write().await;
    // An export inside the vault would be indexed as a second copy of every
    // note, and copied again by the next export. The vault root is stored
    // canonical, so only `parent` needs resolving.
    if let Some(v) = notes.vault() {
        let parent = std::fs::canonicalize(parent).unwrap_or_else(|_| parent.to_path_buf());
        if parent.starts_with(&v.root) {
            return Err(AppError::validation(
                "Choose a folder outside the vault for the export.",
            ));
        }
    }
    let stamp = chrono::Local::now().format("%Y-%m-%d %H%M%S");
    let dest = parent.join(format!("Brainiac Export {stamp}"));
    if dest.exists() {
        return Err(AppError::new(
            crate::models::ErrorCode::Conflict,
            "An export with that name already exists. Try again in a second.",
        ));
    }
    std::fs::create_dir_all(&dest)?;
    let db_copy = dest.join(DATABASE);
    let target = db_copy.display().to_string();
    notes
        .core()
        .call(move |conn| {
            conn.execute("VACUUM INTO ?1", [target])?;
            Ok(())
        })
        .await?;

    let vault_root = notes.vault().map(|v| v.root);
    let result = blocking(move || write_rest(&dest, &db_copy, vault_root.as_deref())).await;
    result
}

fn write_rest(dest: &Path, db_copy: &Path, vault_root: Option<&Path>) -> AppResult<ExportResult> {
    let conn = Connection::open_with_flags(db_copy, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let vault: Option<(String, String)> = conn
        .query_row("SELECT id, name FROM vaults WHERE active = 1", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .optional()?;
    let known: HashMap<String, String> = match &vault {
        Some((id, _)) => {
            let mut stmt = conn.prepare(
                "SELECT relative_path, id FROM notes WHERE vault_id = ?1 AND missing_at IS NULL",
            )?;
            let rows = stmt.query_map([id], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect::<Result<_, _>>()?
        }
        None => HashMap::new(),
    };
    let repositories: Vec<ManifestRepository> = {
        let mut stmt = conn.prepare(
            "SELECT id, canonical_root, remote_url FROM repositories ORDER BY canonical_root",
        )?;
        let rows = stmt.query_map([], |r| {
            let root: String = r.get(1)?;
            Ok(ManifestRepository {
                id: r.get(0)?,
                name: folder_name(&root),
                remote_url: r.get(2)?,
            })
        })?;
        rows.collect::<Result<_, _>>()?
    };
    let tasks = crate::tasks::list(&conn, &TaskFilter::default())?;
    let by_id: HashMap<&str, &ManifestRepository> =
        repositories.iter().map(|r| (r.id.as_str(), r)).collect();
    let exported: Vec<ExportedTask> = tasks
        .iter()
        .map(|t| ExportedTask {
            task: t.clone(),
            repository: t
                .repository_id
                .as_deref()
                .and_then(|id| by_id.get(id).map(|r| (*r).clone())),
        })
        .collect();
    std::fs::write(dest.join(TASKS), serde_json::to_vec_pretty(&exported)?)?;

    let mut notes = Vec::new();
    let mut problems = Vec::new();
    if let Some(root) = vault_root {
        let vault_dest = dest.join(VAULT);
        std::fs::create_dir_all(&vault_dest)?;
        let mut files = Vec::new();
        collect_files(root, "", &mut files)?;
        for rel in files {
            match copy_stable(&root.join(&rel), &vault_dest.join(&rel)) {
                Ok(bytes) => {
                    if let Some(id) = known.get(&rel) {
                        notes.push(ManifestNote {
                            id: id.clone(),
                            relative_path: rel,
                            content_hash: index::content_hash(&bytes),
                        });
                    }
                }
                Err(e) => problems.push(format!("{rel}: {}", e.message)),
            }
        }
    }
    let complete = problems.is_empty();
    let manifest = Manifest {
        format: EXPORT_FORMAT,
        app: "Brainiac".into(),
        app_version: env!("CARGO_PKG_VERSION").into(),
        exported_at: now_rfc3339(),
        schema_version: db::schema_version(&conn)?,
        vault: vault.map(|(id, name)| ManifestVault { id, name }),
        notes: notes.clone(),
        repositories,
        tasks: tasks.len() as u64,
        problems: problems.clone(),
        complete,
    };
    std::fs::write(dest.join(MANIFEST), serde_json::to_vec_pretty(&manifest)?)?;
    Ok(ExportResult {
        path: dest.display().to_string(),
        notes: notes.len() as u64,
        tasks: tasks.len() as u64,
        problems,
        complete,
    })
}

/// Every file in the vault except hidden ones (`.git`, editor settings).
fn collect_files(root: &Path, sub: &str, out: &mut Vec<String>) -> AppResult<()> {
    let dir = if sub.is_empty() {
        root.to_path_buf()
    } else {
        root.join(sub)
    };
    for entry in std::fs::read_dir(&dir)? {
        let Ok(entry) = entry else { continue };
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if name.starts_with('.') {
            continue;
        }
        let rel = if sub.is_empty() {
            name
        } else {
            format!("{sub}/{name}")
        };
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            collect_files(root, &rel, out)?;
        } else if kind.is_file() {
            out.push(rel);
        }
    }
    Ok(())
}

/// Copy a file whose size and modification time hold still while it is read.
fn copy_stable(from: &Path, to: &Path) -> AppResult<Vec<u8>> {
    let stamp = |p: &Path| std::fs::metadata(p).map(|m| (m.len(), crate::notes::mtime_ns(&m)));
    for _ in 0..COPY_ATTEMPTS {
        let before = stamp(from)?;
        let bytes = std::fs::read(from)?;
        if stamp(from)? == before && bytes.len() as u64 == before.0 {
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(to, &bytes)?;
            return Ok(bytes);
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    Err(AppError::io("it kept changing while it was copied"))
}

fn read_manifest(export: &Path) -> AppResult<Manifest> {
    let not_export = || {
        AppError::validation(
            "That folder is not a Brainiac export: it has no readable manifest.json.",
        )
    };
    let text = std::fs::read(export.join(MANIFEST)).map_err(|_| not_export())?;
    let manifest: Manifest = serde_json::from_slice(&text).map_err(|_| not_export())?;
    if manifest.app != "Brainiac" {
        return Err(not_export());
    }
    if manifest.format > EXPORT_FORMAT || manifest.schema_version > SCHEMA_VERSION {
        return Err(AppError::validation(format!(
            "This export was made by a newer version of Brainiac ({}). Install it to restore this export.",
            manifest.app_version
        )));
    }
    Ok(manifest)
}

/// Check an export's database before trusting it.
fn check_database(path: &Path) -> AppResult<()> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|_| AppError::validation("The export's database is missing or unreadable."))?;
    let id: i32 = conn.query_row("PRAGMA application_id", [], |r| r.get(0))?;
    let version = db::schema_version(&conn)?;
    if id != APPLICATION_ID {
        return Err(AppError::validation(
            "The export's database is not Brainiac's.",
        ));
    }
    if version > SCHEMA_VERSION || version == 0 {
        return Err(AppError::validation(
            "The export's database is from a version of Brainiac this one cannot read.",
        ));
    }
    let ok: String = conn.query_row("PRAGMA quick_check", [], |r| r.get(0))?;
    if ok != "ok" {
        return Err(AppError::validation("The export's database is damaged.").with_details(ok));
    }
    Ok(())
}

/// What an export holds, after checking it is Brainiac's and readable.
pub fn preview(export: &Path) -> AppResult<RestorePreview> {
    let manifest = read_manifest(export)?;
    check_database(&export.join(DATABASE))?;
    Ok(RestorePreview {
        path: export.display().to_string(),
        exported_at: manifest.exported_at,
        app_version: manifest.app_version,
        vault_name: manifest.vault.as_ref().map(|v| v.name.clone()),
        notes: manifest.notes.len() as u64,
        tasks: manifest.tasks,
        repositories: manifest
            .repositories
            .iter()
            .map(|r| ExportedRepository {
                name: r.name.clone(),
                remote_url: r.remote_url.clone(),
            })
            .collect(),
    })
}

/// Stage a restore: put the vault in place, prepare the export's database
/// for this Mac (the vault's folder, repositories matched by remote URL),
/// and leave it for the next launch to swap in.
pub async fn restore(
    notes: &Arc<NoteService>,
    request: RestoreRequest,
) -> AppResult<RestoreResult> {
    let export = PathBuf::from(&request.export_path);
    let manifest = read_manifest(&export)?;
    check_database(&export.join(DATABASE))?;
    let current = notes
        .core()
        .call(|conn| db::list_repositories(conn))
        .await?;
    let data_dir = notes.data_dir.clone();
    blocking(move || stage(&export, &manifest, &request, &current, &data_dir)).await
}

fn stage(
    export: &Path,
    manifest: &Manifest,
    request: &RestoreRequest,
    current: &[db::RepositoryRow],
    data_dir: &Path,
) -> AppResult<RestoreResult> {
    let vault = PathBuf::from(&request.vault_path);
    if request.use_existing_vault {
        if !vault.is_dir() {
            return Err(AppError::validation(
                "Choose the folder that holds the vault's notes.",
            ));
        }
    } else {
        if vault.exists() && std::fs::read_dir(&vault)?.next().is_some() {
            return Err(AppError::validation(
                "Choose an empty or new folder for the restored notes, or use the existing vault.",
            ));
        }
        std::fs::create_dir_all(&vault)?;
        let source = export.join(VAULT);
        if source.is_dir() {
            let mut files = Vec::new();
            collect_files(&source, "", &mut files)?;
            for rel in files {
                let to = vault.join(&rel);
                if let Some(parent) = to.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::copy(source.join(&rel), to)?;
            }
        }
    }
    let vault = std::fs::canonicalize(&vault)?;
    let data = std::fs::canonicalize(data_dir).unwrap_or_else(|_| data_dir.to_path_buf());
    if data.starts_with(&vault) || vault.starts_with(&data) {
        return Err(AppError::validation(
            "The vault cannot contain Brainiac's own data folder, or be inside it.",
        ));
    }

    let staging = data_dir.join(".restore-staging.sqlite3");
    db::remove_database(&staging)?;
    std::fs::copy(export.join(DATABASE), &staging)?;
    let mut conn = Connection::open(&staging)?;
    // An older export is brought up to date first, so what follows sees
    // the current tables (its sources get the same defaults as an upgrade).
    db::migrate(&mut conn, &db::CORE)?;
    let tx = conn.transaction()?;
    // Every restored secret source waits for the user to allow it: a backup
    // must not connect a secret on this Mac to a destination it names
    // (SPEC.md, Secrets). Pending markers stay, for the user to act on.
    tx.execute("UPDATE forge_accounts SET source_approved = 0", [])?;
    tx.execute("UPDATE db_connections SET source_approved = 0", [])?;
    // Settings → Agents too, and an image ID names an image on an engine
    // the backup's Mac used: confirmation and a build on this one come
    // first (SPEC.md, Deleting and keeping).
    tx.execute("UPDATE agent_profiles SET source_approved = 0", [])?;
    tx.execute(
        "UPDATE agent_profiles SET image_id = NULL, image_recipe = NULL, image_built_at = NULL",
        [],
    )?;
    let vault_name = vault
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Vault".into());
    let vault_path = vault.display().to_string();
    match &manifest.vault {
        Some(v) => {
            tx.execute("UPDATE vaults SET active = 0", [])?;
            tx.execute(
                "UPDATE vaults SET root_path = ?2, name = ?3, active = 1 WHERE id = ?1",
                params![v.id, vault_path, vault_name],
            )?;
        }
        None => {
            tx.execute("UPDATE vaults SET active = 0", [])?;
            tx.execute(
                "INSERT INTO vaults (id, name, root_path, created_at, active) VALUES (?1, ?2, ?3, ?4, 1)",
                params![uuid::Uuid::new_v4().to_string(), vault_name, vault_path, now_rfc3339()],
            )?;
        }
    }

    // Repositories: a folder that is not on this Mac is matched to a
    // registered repository with the same remote URL; the rest offer Locate….
    let staged: Vec<(String, String, String, Option<String>)> = {
        let mut stmt =
            tx.prepare("SELECT id, canonical_root, display_path, remote_url FROM repositories")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
        rows.collect::<Result<_, _>>()?
    };
    let mut matched = 0u64;
    let mut unmatched = Vec::new();
    // Registered paths (`display_path`, which is unique) already taken by the export.
    let mut used: Vec<String> = staged
        .iter()
        .filter(|(_, root, _, _)| Path::new(root).is_dir())
        .map(|(_, _, display, _)| display.clone())
        .collect();
    for (id, root, _, url) in &staged {
        if Path::new(root).is_dir() {
            continue;
        }
        let candidate = url.as_ref().and_then(|u| {
            current
                .iter()
                .find(|c| c.remote_url.as_ref() == Some(u) && !used.contains(&c.display_path))
        });
        match candidate {
            Some(c) => {
                tx.execute(
                    "UPDATE repositories SET canonical_root = ?2, display_path = ?3, git_dir = ?4,
                       common_git_dir = ?5, status_json = NULL, error_json = NULL WHERE id = ?1",
                    params![
                        id,
                        c.canonical_root,
                        c.display_path,
                        c.git_dir,
                        c.common_git_dir
                    ],
                )?;
                used.push(c.display_path.clone());
                matched += 1;
            }
            None => unmatched.push(folder_name(root)),
        }
    }
    // Repositories registered here before the restore stay registered.
    for c in current {
        if used.contains(&c.display_path) {
            continue;
        }
        tx.execute(
            "INSERT OR IGNORE INTO repositories (id, canonical_root, display_path, git_dir, common_git_dir,
               created_at, remote_url) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                c.id,
                c.canonical_root,
                c.display_path,
                c.git_dir,
                c.common_git_dir,
                c.created_at,
                c.remote_url
            ],
        )?;
    }
    tx.commit()?;
    drop(conn);
    std::fs::rename(&staging, data_dir.join(PENDING_RESTORE))?;
    Ok(RestoreResult {
        matched_repositories: matched,
        unmatched_repositories: unmatched,
        needs_restart: true,
    })
}

/// At launch, before the core database opens: swap in a staged restore.
/// The data it replaces is snapshotted first, and the search index is
/// deleted so it is rebuilt from the restored vault. Returns whether a
/// restore was applied.
pub fn apply_pending_restore(data_dir: &Path) -> AppResult<bool> {
    let pending = data_dir.join(PENDING_RESTORE);
    if !pending.exists() {
        return Ok(false);
    }
    let core = data_dir.join(db::CORE_FILE);
    if core.exists() {
        let dest = db::backups_dir(&core).join(format!(
            "pre-restore-{}.sqlite3",
            chrono::Utc::now().format("%Y%m%dT%H%M%SZ")
        ));
        db::snapshot_file(&core, &dest)?;
    }
    db::remove_database(&core)?;
    std::fs::rename(&pending, &core)?;
    db::remove_database(&data_dir.join(db::INDEX_FILE))?;
    tracing::info!("applied a restored export");
    Ok(true)
}
