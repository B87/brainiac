//! Notes (SPEC.md, section 5): reading, the save contract, creating,
//! renaming, the trash, revisions and drafts, and the links between notes
//! and repositories. Scanning the vault and watching it are in `vault.rs`.
//!
//! `NoteService` is the one write path for notes: the UI, the vault watcher,
//! and later the MCP server all go through it, and it emits the committed
//! change events (docs/architecture.md, One write path). Like
//! `RepositoryService`, it does not depend on Tauri: events go through an
//! injected emitter, so it is testable with `cargo test`.

mod files;
pub(crate) mod history;
pub(crate) mod store;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

use crate::db::{self, Db};
use crate::index::{self, IndexDoc, EDIT_LIMIT};
use crate::models::{
    now_rfc3339, AppError, AppResult, Backlink, CreateNoteRequest, ErrorCode, FolderEntry,
    FolderEntryKind, FolderListing, IndexState, IndexStatus, LinkedRepository, NoteChangeOrigin,
    NoteChangedEvent, NoteContent, NoteContext, NoteLinkKind, NoteLists, NoteMissingEvent,
    NoteRevision, NoteSummary, NoteTextState, RenameNoteRequest, RenamePreview, RenameResult,
    RepositoryNotes, RepositorySuggestion, ResolvedLink, RevisionReason, SaveNoteRequest,
    SaveNoteResult, TaskChangedEvent, TrashedNote, UnresolvedLink, VaultInfo, VaultState,
};
pub(crate) use files::{mtime_ns, walk, Listings};
pub use files::{validate_relative, Found};
use store::{FileState, NoteRow};

/// A change committed by the notes or tasks services, for the frontend.
#[derive(Debug, Clone, PartialEq)]
pub enum KnowledgeEvent {
    NoteChanged(NoteChangedEvent),
    NoteMissing(NoteMissingEvent),
    TaskChanged(TaskChangedEvent),
    IndexStatus(IndexStatus),
}

/// Delivers committed changes; `lib.rs` turns them into Tauri events.
/// `Arc<dyn Fn…>` lets every service share one closure across threads.
pub type KnowledgeEmitter = Arc<dyn Fn(KnowledgeEvent) + Send + Sync>;

/// The v0.2 database files (docs/architecture.md, Storage layout).
#[derive(Clone)]
pub struct Stores {
    /// `brainiac.db`, shared with the repository service.
    pub core: Db,
    /// The one writer of `index.db`.
    pub index: Db,
    /// A read-only connection to `index.db` for searches and lists.
    pub reader: Db,
    pub history: Db,
}

impl Stores {
    /// Open `index.db` and `history.db` next to the core database.
    pub fn open(data_dir: &Path, core: Db) -> AppResult<Self> {
        let index = Db::open_index(&data_dir.join(db::INDEX_FILE))?;
        let reader = Db::open_read_only(&data_dir.join(db::INDEX_FILE))?;
        let history = Db::open_store(&data_dir.join(db::HISTORY_FILE), &db::HISTORY)?;
        Ok(Stores {
            core,
            index,
            reader,
            history,
        })
    }
}

/// The vault in use: its row and its folder, resolved through symbolic links
/// because file events name real paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveVault {
    pub id: String,
    pub name: String,
    pub root: PathBuf,
}

pub struct NoteService {
    pub(crate) stores: Stores,
    pub(crate) data_dir: PathBuf,
    pub(crate) vault: RwLock<Option<ActiveVault>>,
    status: Mutex<IndexStatus>,
    events: KnowledgeEmitter,
    /// One write at a time per note (docs/architecture.md, Concurrency).
    note_locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// Notes Brainiac is writing right now; a reconciliation leaves them to the write.
    pub(crate) in_flight: Mutex<HashSet<String>>,
    /// Writes hold it shared; an export holds it alone, so the copy is consistent.
    pub(crate) gate: tokio::sync::RwLock<()>,
    /// Saved notes whose search update failed and is retried.
    repairs: Mutex<HashSet<String>>,
    /// Write `brainiac_id` into a note when it first gets a task or repository link.
    write_note_ids: std::sync::atomic::AtomicBool,
    /// Vault watching and reconciliation state (`vault.rs`).
    pub(crate) watch: crate::vault::WatchState,
}

/// Removes a note from `in_flight` when the write ends, however it ends.
struct InFlight<'a> {
    service: &'a NoteService,
    id: String,
}

impl<'a> InFlight<'a> {
    fn new(service: &'a NoteService, id: &str) -> Self {
        service
            .in_flight
            .lock()
            .expect("in flight")
            .insert(id.to_string());
        InFlight {
            service,
            id: id.to_string(),
        }
    }
}

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        self.service
            .in_flight
            .lock()
            .expect("in flight")
            .remove(&self.id);
    }
}

fn conflict(message: impl Into<String>) -> AppError {
    AppError::new(ErrorCode::Conflict, message)
}

fn gone() -> AppError {
    AppError::not_found(
        "This note's file is gone. Restore it as a new file, or relink it to a file.",
    )
}

/// Run blocking file work off the async executor threads.
async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> AppResult<T> + Send + 'static,
) -> AppResult<T> {
    tokio::task::spawn_blocking(f).await.map_err(|e| {
        AppError::io("A file operation stopped unexpectedly.").with_details(e.to_string())
    })?
}

/// What a note's bytes mean to Brainiac.
pub(crate) struct ReadNote {
    pub hash: String,
    pub text_state: NoteTextState,
    /// The text, when it is editable.
    pub text: Option<String>,
}

pub(crate) fn interpret(bytes: Vec<u8>) -> ReadNote {
    let hash = index::content_hash(&bytes);
    if bytes.len() as u64 > EDIT_LIMIT {
        return ReadNote {
            hash,
            text_state: NoteTextState::TooLarge,
            text: None,
        };
    }
    match String::from_utf8(bytes) {
        Ok(text) => ReadNote {
            hash,
            text_state: NoteTextState::Text,
            text: Some(text),
        },
        Err(_) => ReadNote {
            hash,
            text_state: NoteTextState::NotUtf8,
            text: None,
        },
    }
}

/// The row and index entry for a note file read from disk.
pub(crate) fn describe(
    note_id: &str,
    relative_path: &str,
    read: &ReadNote,
    size: i64,
    mtime: i64,
) -> (FileState, IndexDoc) {
    let parsed = read
        .text
        .as_deref()
        .map(|t| index::parse_note(t, relative_path));
    let title = parsed
        .as_ref()
        .map(|p| p.title.clone())
        .unwrap_or_else(|| index::file_stem(relative_path).to_string());
    let state = FileState {
        relative_path: relative_path.to_string(),
        embedded_id: parsed.as_ref().and_then(|p| p.embedded_id.clone()),
        title: title.clone(),
        content_hash: read.hash.clone(),
        size,
        mtime,
        text_state: read.text_state,
    };
    let doc = IndexDoc {
        note_id: note_id.to_string(),
        content_hash: read.hash.clone(),
        title,
        relative_path: relative_path.to_string(),
        body: read.text.clone().unwrap_or_default(),
        links: parsed.map(|p| p.links).unwrap_or_default(),
    };
    (state, doc)
}

impl NoteService {
    pub fn new(stores: Stores, data_dir: PathBuf, events: KnowledgeEmitter) -> Arc<Self> {
        Arc::new(NoteService {
            stores,
            data_dir,
            vault: RwLock::new(None),
            status: Mutex::new(IndexStatus {
                state: IndexState::NoVault,
                done: 0,
                total: 0,
                pending_repairs: 0,
                message: None,
            }),
            events,
            note_locks: Mutex::new(HashMap::new()),
            in_flight: Mutex::new(HashSet::new()),
            gate: tokio::sync::RwLock::new(()),
            repairs: Mutex::new(HashSet::new()),
            write_note_ids: std::sync::atomic::AtomicBool::new(true),
            watch: crate::vault::WatchState::default(),
        })
    }

    pub fn set_write_note_ids(&self, on: bool) {
        self.write_note_ids
            .store(on, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn core(&self) -> &Db {
        &self.stores.core
    }

    pub fn vault(&self) -> Option<ActiveVault> {
        self.vault.read().expect("vault").clone()
    }

    pub(crate) fn require_vault(&self) -> AppResult<ActiveVault> {
        self.vault()
            .ok_or_else(|| AppError::new(ErrorCode::NotFound, "Choose a vault folder first."))
    }

    pub fn index_status(&self) -> IndexStatus {
        self.status.lock().expect("status").clone()
    }

    /// Change the index status and tell the frontend.
    pub(crate) fn set_status(&self, change: impl FnOnce(&mut IndexStatus)) {
        let status = {
            let mut s = self.status.lock().expect("status");
            change(&mut s);
            s.pending_repairs = self.repairs.lock().expect("repairs").len() as u64;
            s.clone()
        };
        self.emit(KnowledgeEvent::IndexStatus(status));
    }

    pub(crate) fn emit(&self, event: KnowledgeEvent) {
        (self.events)(event);
    }

    pub(crate) fn emit_changed(
        &self,
        id: &str,
        version: Option<String>,
        path: &str,
        origin: NoteChangeOrigin,
    ) {
        self.emit(KnowledgeEvent::NoteChanged(NoteChangedEvent {
            note_id: id.to_string(),
            version,
            relative_path: path.to_string(),
            origin,
        }));
    }

    fn note_lock(&self, id: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = self.note_locks.lock().expect("note locks");
        Arc::clone(locks.entry(id.to_string()).or_default())
    }

    pub async fn vault_state(&self) -> VaultState {
        VaultState {
            vault: self.vault().map(|v| VaultInfo {
                available: v.root.is_dir(),
                id: v.id,
                name: v.name,
                root_path: v.root.display().to_string(),
            }),
            index: self.index_status(),
        }
    }

    pub(crate) async fn row(&self, id: &str) -> AppResult<NoteRow> {
        let id2 = id.to_string();
        self.stores
            .core
            .call(move |conn| store::get_note(conn, &id2))
            .await?
            .ok_or_else(|| AppError::not_found("That note does not exist."))
    }

    pub async fn summary(&self, id: &str) -> AppResult<NoteSummary> {
        let id2 = id.to_string();
        self.stores
            .core
            .call(move |conn| store::get_summary(conn, &id2))
            .await?
            .ok_or_else(|| AppError::not_found("That note does not exist."))
    }

    /// The live note at a path: vault-relative (`Projects/Plan.md`) or an
    /// absolute path inside the vault, as agents often have.
    pub async fn note_id_at(&self, path: &str) -> AppResult<String> {
        let vault = self.require_vault()?;
        let absolute = std::path::Path::new(path);
        let relative = if absolute.is_absolute() {
            let canonical = absolute
                .canonicalize()
                .unwrap_or_else(|_| absolute.to_path_buf());
            let inside = canonical
                .strip_prefix(&vault.root)
                .map_err(|_| AppError::validation("That path is not inside the vault."))?;
            inside.to_string_lossy().into_owned()
        } else {
            path.trim_start_matches("./").to_string()
        };
        // Rejects `..` and anything else that would leave the vault.
        files::resolve(&vault.root, &relative)?;
        let vault_id = vault.id.clone();
        self.stores
            .core
            .call(move |conn| store::live_note_at(conn, &vault_id, &relative))
            .await?
            .map(|row| row.id)
            .ok_or_else(|| AppError::not_found("No note at that path."))
    }

    /// Absolute path of a vault-relative path, inside the vault.
    pub fn absolute(&self, relative: &str) -> AppResult<PathBuf> {
        let vault = self.require_vault()?;
        files::resolve(&vault.root, relative)
    }

    // -----------------------------------------------------------------------
    // Reading
    // -----------------------------------------------------------------------

    /// A note's text and version, and any unsaved draft. A note changed on
    /// disk since the last scan is read as it is now and reconciled.
    pub async fn read(self: &Arc<Self>, id: &str) -> AppResult<NoteContent> {
        let row = self.row(id).await?;
        let id2 = id.to_string();
        let draft = self
            .stores
            .history
            .call(move |conn| history::get_draft(conn, &id2))
            .await?;
        let id2 = id.to_string();
        let last_text = if row.is_live() {
            None
        } else {
            self.stores
                .history
                .call(move |conn| history::newest_text(conn, &id2))
                .await?
        };
        let missing = || NoteContent {
            note: store::summary_of(&row, false),
            text: last_text.clone(),
            version: row.content_hash.clone(),
            draft: draft.clone(),
        };
        let vault = self.require_vault()?;
        // The vault may have changed since the row was read.
        if !row.is_live() || row.vault_id != vault.id {
            return Ok(missing());
        }
        let path = files::resolve(&vault.root, &row.relative_path)?;
        let bytes = match blocking(move || Ok(std::fs::read(&path)?)).await {
            Ok(b) => b,
            Err(e) if e.code == ErrorCode::NotFound => {
                self.request_reconcile(vec![row.relative_path.clone()]);
                return Ok(missing());
            }
            Err(e) => return Err(e),
        };
        let read = interpret(bytes);
        if read.hash != row.content_hash {
            self.request_reconcile(vec![row.relative_path.clone()]);
        }
        let mut note = self.summary(id).await?;
        note.text_state = read.text_state;
        Ok(NoteContent {
            note,
            text: read.text,
            version: read.hash,
            draft,
        })
    }

    pub async fn mark_opened(&self, id: &str) -> AppResult<()> {
        let id2 = id.to_string();
        let now = now_rfc3339();
        self.stores
            .core
            .call(move |conn| store::mark_opened(conn, &id2, &now))
            .await
    }

    pub async fn lists(&self) -> AppResult<NoteLists> {
        let Some(vault) = self.vault() else {
            return Ok(NoteLists {
                pinned: Vec::new(),
                recent: Vec::new(),
            });
        };
        self.stores
            .core
            .call(move |conn| {
                Ok(NoteLists {
                    pinned: store::pinned(conn)?
                        .into_iter()
                        .filter(|n| !n.trashed)
                        .collect(),
                    recent: store::recent(conn, &vault.id, 10)?,
                })
            })
            .await
    }

    /// One folder of the vault as it is on disk: folders, notes, and other files.
    pub async fn list_folder(&self, folder: Option<String>) -> AppResult<FolderListing> {
        let vault = self.require_vault()?;
        let folder = folder.unwrap_or_default();
        let dir = if folder.is_empty() {
            vault.root.clone()
        } else {
            files::resolve(&vault.root, &folder)?
        };
        let entries = blocking(move || {
            let mut out = Vec::new();
            for entry in std::fs::read_dir(&dir)? {
                let Ok(entry) = entry else { continue };
                let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                    continue;
                };
                if name.starts_with('.') {
                    continue;
                }
                let Ok(kind) = entry.file_type() else {
                    continue;
                };
                let kind = if kind.is_dir() {
                    FolderEntryKind::Folder
                } else if kind.is_file() && index::is_note_path(&name) {
                    FolderEntryKind::Note
                } else if kind.is_file() {
                    FolderEntryKind::Other
                } else {
                    continue;
                };
                out.push((name, kind));
            }
            Ok(out)
        })
        .await?;
        let (vault_id, folder2) = (vault.id.clone(), folder.clone());
        let notes = self
            .stores
            .core
            .call(move |conn| store::summaries_in_folder(conn, &vault_id, &folder2))
            .await?;
        let mut listing: Vec<FolderEntry> = entries
            .into_iter()
            .map(|(name, kind)| {
                let relative_path = if folder.is_empty() {
                    name.clone()
                } else {
                    format!("{folder}/{name}")
                };
                FolderEntry {
                    note: notes.get(&relative_path).cloned(),
                    name,
                    relative_path,
                    kind,
                }
            })
            .collect();
        // Folders first, then by what the tree shows: a note's title, else the name.
        let label = |e: &FolderEntry| {
            e.note
                .as_ref()
                .map_or(e.name.as_str(), |n| n.title.as_str())
                .to_lowercase()
        };
        listing.sort_by(|a, b| {
            (a.kind != FolderEntryKind::Folder)
                .cmp(&(b.kind != FolderEntryKind::Folder))
                .then_with(|| label(a).cmp(&label(b)))
        });
        Ok(FolderListing {
            relative_path: folder,
            entries: listing,
        })
    }

    // -----------------------------------------------------------------------
    // Saving
    // -----------------------------------------------------------------------

    /// Save edits made in Brainiac (docs/architecture.md, Save contract). The
    /// draft is stored first and kept unless the save succeeds; a file that
    /// changed since `expected_version` is never overwritten.
    pub async fn save(self: &Arc<Self>, request: SaveNoteRequest) -> AppResult<SaveNoteResult> {
        if request.text.len() as u64 > EDIT_LIMIT {
            return Err(AppError::validation(
                "Notes over 5 MiB cannot be edited in Brainiac. Open it in another editor.",
            ));
        }
        let _gate = self.gate.read().await;
        let lock = self.note_lock(&request.note_id);
        let _guard = lock.lock().await;
        let vault = self.require_vault()?;
        let row = self.row(&request.note_id).await?;
        {
            let (id, base, text, now) = (
                request.note_id.clone(),
                request.expected_version.clone(),
                request.text.clone(),
                now_rfc3339(),
            );
            self.stores
                .history
                .call(move |conn| history::put_draft(conn, &id, &base, &text, &now))
                .await?;
        }
        if !row.is_live() {
            return Err(gone());
        }
        if row.text_state != NoteTextState::Text {
            return Err(AppError::validation(
                "This note is not editable text. Open it in another editor.",
            ));
        }
        let result = self
            .write_text(
                &vault,
                &row,
                &request.expected_version,
                &request.text,
                RevisionReason::AppSave,
            )
            .await?;
        let id = request.note_id.clone();
        self.stores
            .history
            .call(move |conn| history::delete_draft(conn, &id))
            .await?;
        Ok(result)
    }

    /// Replace a live note's text, provided the file still has version
    /// `expected`. The text it had is stored as a revision first (with
    /// `reason`), except within one typing session of Brainiac's own saves.
    /// Callers hold the gate and the note's lock.
    async fn write_text(
        self: &Arc<Self>,
        vault: &ActiveVault,
        row: &NoteRow,
        expected: &str,
        text: &str,
        reason: RevisionReason,
    ) -> AppResult<SaveNoteResult> {
        let path = files::resolve(&vault.root, &row.relative_path)?;
        let _flight = InFlight::new(self, &row.id);
        let read_path = path.clone();
        let current = match blocking(move || Ok(std::fs::read(&read_path)?)).await {
            Ok(bytes) => interpret(bytes),
            Err(e) if e.code == ErrorCode::NotFound => return Err(gone()),
            Err(e) => return Err(e),
        };
        if current.hash != expected {
            return Err(conflict(
                "The note changed on disk since it was opened. Your edits are kept as a draft.",
            ));
        }
        let new_hash = index::content_hash(text.as_bytes());
        if new_hash != current.hash {
            if let Some(previous) = current.text.clone() {
                let note_id = row.id.clone();
                let hash = current.hash.clone();
                self.stores
                    .history
                    .call(move |conn| {
                        let now = chrono::Utc::now();
                        if reason == RevisionReason::AppSave
                            && history::in_save_session(conn, &note_id, &now)?
                        {
                            return Ok(());
                        }
                        history::add_revisions(
                            conn,
                            &[history::NewRevision {
                                note_id,
                                content: previous,
                                content_hash: hash,
                                reason,
                            }],
                            &now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                        )
                    })
                    .await
                    .map_err(|e| {
                        AppError::new(
                            e.code,
                            "The note's previous version could not be kept, so it was not saved. Your edits are kept as a draft.",
                        )
                        .with_details(e.details.unwrap_or(e.message))
                    })?;
            }
            let (dest, bytes, expected) =
                (path.clone(), text.as_bytes().to_vec(), expected.to_string());
            blocking(move || {
                let check = dest.clone();
                files::write_atomic(&dest, &bytes, move || {
                    let now = index::content_hash(&std::fs::read(&check)?);
                    if now == expected {
                        Ok(())
                    } else {
                        Err(conflict(
                            "The note changed on disk while saving. Your edits are kept as a draft.",
                        ))
                    }
                })
            })
            .await?;
        }
        // The file is written: from here on nothing reports the save as
        // failed, or the next one would be refused as a conflict. What cannot
        // be recorded now is repaired by a reconciliation of the path.
        let meta_path = path.clone();
        let meta = blocking(move || Ok(std::fs::metadata(&meta_path)?)).await;
        let read = ReadNote {
            hash: new_hash.clone(),
            text_state: NoteTextState::Text,
            text: Some(text.to_string()),
        };
        let (size, mtime) = meta.as_ref().map_or((text.len() as i64, 0), |m| {
            (m.len() as i64, files::mtime_ns(m))
        });
        let (state, doc) = describe(&row.id, &row.relative_path, &read, size, mtime);
        let id = row.id.clone();
        let recorded = self
            .stores
            .core
            .call(move |conn| store::set_file(conn, &id, &state))
            .await;
        if let Err(e) = &recorded {
            tracing::warn!(error = %e, "could not record a save; reconciling the note");
            self.request_reconcile(vec![row.relative_path.clone()]);
        }
        let search_pending = match self.index_docs(vec![doc]).await {
            Ok(()) => false,
            Err(e) => {
                tracing::warn!(error = %e, "search update failed after a save; retrying");
                self.queue_repair(&row.id);
                true
            }
        };
        let origin = if reason == RevisionReason::Agent {
            NoteChangeOrigin::Agent
        } else {
            NoteChangeOrigin::App
        };
        self.emit_changed(&row.id, Some(new_hash.clone()), &row.relative_path, origin);
        let note = match self.summary(&row.id).await {
            Ok(n) => n,
            Err(_) => store::summary_of(row, false),
        };
        Ok(SaveNoteResult {
            note,
            version: new_hash,
            search_pending,
        })
    }

    pub(crate) async fn index_docs(&self, docs: Vec<IndexDoc>) -> AppResult<()> {
        self.stores
            .index
            .call(move |conn| index::upsert_docs(conn, &docs))
            .await
    }

    pub(crate) async fn unindex(&self, ids: Vec<String>) -> AppResult<()> {
        self.stores
            .index
            .call(move |conn| {
                index::remove_docs(conn, &ids)?;
                index::resolve_all_links(conn)?;
                Ok(())
            })
            .await
    }

    pub(crate) async fn resolve_links(&self) -> AppResult<()> {
        self.stores
            .index
            .call(|conn| index::resolve_all_links(conn).map(|_| ()))
            .await
    }

    /// Retry a failed search update in the background until it succeeds or
    /// the note changes again (a later save or scan indexes it anyway).
    fn queue_repair(self: &Arc<Self>, id: &str) {
        if !self.repairs.lock().expect("repairs").insert(id.to_string()) {
            return;
        }
        self.set_status(|_| {});
        let service = Arc::clone(self);
        let id = id.to_string();
        tokio::spawn(async move {
            let mut delay = std::time::Duration::from_secs(2);
            for _ in 0..12 {
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(std::time::Duration::from_secs(300));
                if service.repair(&id).await.is_ok() {
                    break;
                }
            }
            service.repairs.lock().expect("repairs").remove(&id);
            service.set_status(|_| {});
        });
    }

    async fn repair(&self, id: &str) -> AppResult<()> {
        let vault = self.require_vault()?;
        let row = self.row(id).await?;
        if !row.is_live() {
            return Ok(());
        }
        let path = files::resolve(&vault.root, &row.relative_path)?;
        let read = interpret(blocking(move || Ok(std::fs::read(&path)?)).await?);
        let (_, doc) = describe(&row.id, &row.relative_path, &read, row.size, row.mtime);
        self.index_docs(vec![doc]).await
    }

    // -----------------------------------------------------------------------
    // Creating, renaming, and the trash
    // -----------------------------------------------------------------------

    /// Create a note with a `brainiac_id` and a heading, named after its title.
    pub async fn create(self: &Arc<Self>, request: CreateNoteRequest) -> AppResult<NoteSummary> {
        self.create_with(request, NoteChangeOrigin::App, |id, title| {
            format!("---\nbrainiac_id: {id}\n---\n\n# {title}\n\n")
        })
        .await
    }

    /// Create a note with extra frontmatter and a body (Save as note,
    /// SPEC.md section 14). Values are written as JSON strings, which YAML
    /// reads as quoted strings.
    pub async fn create_with_body(
        self: &Arc<Self>,
        request: CreateNoteRequest,
        frontmatter: Vec<(String, String)>,
        body: String,
    ) -> AppResult<NoteSummary> {
        self.create_with(request, NoteChangeOrigin::App, move |id, _title| {
            let mut text = format!("---\nbrainiac_id: {id}\n");
            for (key, value) in &frontmatter {
                let value = serde_json::to_string(value).unwrap_or_else(|_| "\"\"".into());
                text.push_str(&format!("{key}: {value}\n"));
            }
            text.push_str("---\n\n");
            text.push_str(&body);
            text
        })
        .await
    }

    /// Create a note whose text `text_for(id, title)` gives.
    async fn create_with(
        self: &Arc<Self>,
        request: CreateNoteRequest,
        origin: NoteChangeOrigin,
        text_for: impl FnOnce(&str, &str) -> String,
    ) -> AppResult<NoteSummary> {
        let note_id = {
            let _gate = self.gate.read().await;
            let vault = self.require_vault()?;
            let folder = request.folder.unwrap_or_default();
            if !folder.is_empty() {
                files::validate_relative(&folder)?;
            }
            let title = request
                .title
                .as_deref()
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .unwrap_or("Untitled")
                .to_string();
            let id = uuid::Uuid::new_v4().to_string();
            // A scan that sees the file before it is recorded leaves it to this write.
            let _flight = InFlight::new(self, &id);
            let text = text_for(&id, &title);
            if text.len() as u64 > EDIT_LIMIT {
                return Err(AppError::validation(
                    "Notes over 5 MiB cannot be edited in Brainiac.",
                ));
            }
            let path = self
                .unique_path(
                    &vault,
                    &folder,
                    &files::file_name_for(&title),
                    text.as_bytes(),
                )
                .await?;
            self.adopt_new_file(&vault, &id, &path, text, origin)
                .await?;
            id
        };
        if let Some(repository_id) = request.repository_id {
            self.link_repository(&note_id, &repository_id).await?;
        }
        self.summary(&note_id).await
    }

    // -----------------------------------------------------------------------
    // Agent access (SPEC.md, section 9)
    // -----------------------------------------------------------------------

    /// Create a note with an agent's Markdown. It gets a `brainiac_id` like
    /// any new note, and a `# title` heading unless its text starts with one.
    pub async fn create_for_agent(
        self: &Arc<Self>,
        request: CreateNoteRequest,
        text: &str,
    ) -> AppResult<NoteSummary> {
        let text = text.to_string();
        self.create_with(request, NoteChangeOrigin::Agent, move |id, title| {
            let end = index::frontmatter(&text).end;
            let (front, body) = text.split_at(end);
            let body = if body.trim_start().starts_with('#') {
                body.to_string()
            } else {
                format!("# {title}\n\n{body}")
            };
            let text = format!("{front}{body}");
            // An agent's copy of another note must not take that note's identity.
            if index::frontmatter(&text).brainiac_id.is_some() {
                index::replace_embedded_id(&text, id)
            } else {
                index::with_embedded_id(&text, id)
            }
        })
        .await
    }

    /// Replace a note's text for an agent, provided the file still has
    /// `expected_version`. Unlike `save`, it never touches the note's draft,
    /// which holds the user's unsaved edits; its previous text is always kept
    /// as its own `agent` revision; and the note keeps its `brainiac_id` and
    /// never takes another note's.
    pub async fn save_for_agent(
        self: &Arc<Self>,
        note_id: &str,
        expected_version: &str,
        text: &str,
    ) -> AppResult<SaveNoteResult> {
        if text.len() as u64 > EDIT_LIMIT {
            return Err(AppError::validation(
                "Notes over 5 MiB cannot be edited in Brainiac.",
            ));
        }
        let _gate = self.gate.read().await;
        let lock = self.note_lock(note_id);
        let _guard = lock.lock().await;
        let vault = self.require_vault()?;
        let row = self.row(note_id).await?;
        if !row.is_live() || row.vault_id != vault.id {
            return Err(gone());
        }
        if row.text_state != NoteTextState::Text {
            return Err(AppError::validation("This note is not editable text."));
        }
        let text = match (&row.embedded_id, index::frontmatter(text).brainiac_id) {
            (Some(id), None) => index::with_embedded_id(text, id),
            (Some(id), Some(other)) if *id != other => index::replace_embedded_id(text, id),
            // An ID the note does not have belongs to another note.
            (None, Some(other)) if other != row.id => index::replace_embedded_id(text, &row.id),
            _ => text.to_string(),
        };
        self.write_text(&vault, &row, expected_version, &text, RevisionReason::Agent)
            .await
            .map_err(|e| {
                if e.code == ErrorCode::Conflict {
                    conflict("The note changed since you read it. Read it again and reapply your change.")
                } else {
                    e
                }
            })
    }

    /// Write `bytes` to a new file named `base.md` in `folder`, or `base 2.md`
    /// and so on when the name is taken. Returns the vault-relative path.
    async fn unique_path(
        &self,
        vault: &ActiveVault,
        folder: &str,
        base: &str,
        bytes: &[u8],
    ) -> AppResult<String> {
        for n in 1..1000 {
            let name = if n == 1 {
                format!("{base}.md")
            } else {
                format!("{base} {n}.md")
            };
            let rel = if folder.is_empty() {
                name
            } else {
                format!("{folder}/{name}")
            };
            let path = files::resolve(&vault.root, &rel)?;
            let data = bytes.to_vec();
            if blocking(move || files::create_new(&path, &data)).await? {
                return Ok(files::on_disk(&vault.root, &rel));
            }
        }
        Err(conflict("Too many notes with that name in this folder."))
    }

    /// Record a file Brainiac just wrote as the note `id` and index it.
    async fn adopt_new_file(
        &self,
        vault: &ActiveVault,
        id: &str,
        rel: &str,
        text: String,
        origin: NoteChangeOrigin,
    ) -> AppResult<()> {
        let path = files::resolve(&vault.root, rel)?;
        let meta = blocking(move || Ok(std::fs::metadata(&path)?)).await?;
        let read = ReadNote {
            hash: index::content_hash(text.as_bytes()),
            text_state: NoteTextState::Text,
            text: Some(text),
        };
        let (state, doc) = describe(id, rel, &read, meta.len() as i64, files::mtime_ns(&meta));
        let (id2, vault_id, now) = (id.to_string(), vault.id.clone(), now_rfc3339());
        self.stores
            .core
            .call(move |conn| match store::get_note(conn, &id2)? {
                Some(_) => store::set_file(conn, &id2, &state),
                None => store::insert_note(conn, &id2, &vault_id, &state, &now),
            })
            .await?;
        self.index_docs(vec![doc]).await?;
        self.resolve_links().await?;
        self.emit_changed(id, Some(read.hash), rel, origin);
        Ok(())
    }

    fn check_note_path(path: &str) -> AppResult<()> {
        files::validate_relative(path)?;
        if !index::is_note_path(path) {
            return Err(AppError::validation("A note's name must end in .md."));
        }
        Ok(())
    }

    /// The notes whose links a rename would rewrite.
    pub async fn preview_rename(&self, id: &str, new_path: &str) -> AppResult<RenamePreview> {
        Self::check_note_path(new_path)?;
        let sources = self.linking_notes(id).await?;
        let ids: Vec<String> = sources.into_keys().collect();
        let mut notes = self
            .stores
            .core
            .call(move |conn| store::summaries(conn, &ids))
            .await?;
        notes.retain(|n| !n.missing && n.text_state == NoteTextState::Text);
        notes.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
        Ok(RenamePreview {
            new_path: new_path.to_string(),
            linking_notes: notes,
        })
    }

    /// Links into a note, grouped by the note they are in.
    async fn linking_notes(&self, id: &str) -> AppResult<HashMap<String, Vec<index::StoredLink>>> {
        let id2 = id.to_string();
        let links = self
            .stores
            .reader
            .call(move |conn| index::links_into(conn, &id2))
            .await?;
        let mut by_source: HashMap<String, Vec<index::StoredLink>> = HashMap::new();
        for l in links {
            by_source
                .entry(l.source_note_id.clone())
                .or_default()
                .push(l);
        }
        Ok(by_source)
    }

    /// Rename or move a note inside the vault, optionally rewriting the links
    /// to it in other notes (off by default, because it edits them).
    pub async fn rename(self: &Arc<Self>, request: RenameNoteRequest) -> AppResult<RenameResult> {
        Self::check_note_path(&request.new_path)?;
        let linking = if request.update_links {
            self.linking_notes(&request.note_id).await?
        } else {
            HashMap::new()
        };
        let _gate = self.gate.read().await;
        let mut new_path = request.new_path.clone();
        let row = {
            let lock = self.note_lock(&request.note_id);
            let _guard = lock.lock().await;
            let vault = self.require_vault()?;
            let row = self.row(&request.note_id).await?;
            if !row.is_live() {
                return Err(gone());
            }
            if row.relative_path != request.new_path {
                let _flight = InFlight::new(self, &row.id);
                let from = files::resolve(&vault.root, &row.relative_path)?;
                let to = files::resolve(&vault.root, &request.new_path)?;
                blocking(move || files::move_file(&from, &to)).await?;
                // Folders that exist keep their own spelling of the name.
                new_path = files::on_disk(&vault.root, &request.new_path);
                let (id, path) = (row.id.clone(), new_path.clone());
                self.stores
                    .core
                    .call(move |conn| store::set_path(conn, &id, &path))
                    .await?;
                let (id, path) = (row.id.clone(), new_path.clone());
                self.stores
                    .index
                    .call(move |conn| {
                        index::move_doc(conn, &id, &path)?;
                        index::resolve_all_links(conn)?;
                        Ok(())
                    })
                    .await?;
                self.emit_changed(
                    &row.id,
                    Some(row.content_hash.clone()),
                    &new_path,
                    NoteChangeOrigin::App,
                );
            }
            row
        };
        let (mut updated, mut failed) = (Vec::new(), Vec::new());
        for (source, links) in linking {
            match self.rewrite_links_in(&source, &links, &new_path).await {
                Ok(Some(path)) => updated.push(path),
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!(error = %e, "could not update links in a note");
                    failed.push(source);
                }
            }
        }
        if !failed.is_empty() {
            let ids = std::mem::take(&mut failed);
            failed = self
                .stores
                .core
                .call(move |conn| store::summaries(conn, &ids))
                .await?
                .into_iter()
                .map(|n| n.relative_path)
                .collect();
        }
        Ok(RenameResult {
            note: self.summary(&row.id).await?,
            updated,
            failed,
        })
    }

    /// Rename a note's file after its title, when the file was named after
    /// `from_title` (the title the editor last saw it match) and no other note
    /// links to it, taking the next free number when the name is used
    /// (SPEC.md, Note identity). Returns the note as it is now.
    pub async fn follow_title(
        self: &Arc<Self>,
        id: &str,
        from_title: &str,
    ) -> AppResult<NoteSummary> {
        let note = self.summary(id).await?;
        let Some(target) = note.title_file_name.as_deref() else {
            return Ok(note);
        };
        let stem = index::file_stem(&note.relative_path);
        if !files::name_matches(stem, &files::file_name_for(from_title)) {
            return Ok(note);
        }
        // Links name the file; renaming it would leave them unresolved.
        if self
            .linking_notes(id)
            .await?
            .keys()
            .any(|source| source != id)
        {
            return Ok(note);
        }
        let base = index::file_stem(target);
        let folder = note
            .relative_path
            .rsplit_once('/')
            .map_or(String::new(), |(f, _)| format!("{f}/"));
        for n in 1..100 {
            let new_path = if n == 1 {
                format!("{folder}{base}.md")
            } else {
                format!("{folder}{base} {n}.md")
            };
            let request = RenameNoteRequest {
                note_id: id.to_string(),
                new_path,
                update_links: false,
            };
            match self.rename(request).await {
                Ok(result) => return Ok(result.note),
                Err(e) if e.code == ErrorCode::Conflict => continue,
                Err(e) => return Err(e),
            }
        }
        Ok(note)
    }

    /// Rewrite one linking note's links to a renamed note. Returns its path when it changed.
    async fn rewrite_links_in(
        self: &Arc<Self>,
        source: &str,
        links: &[index::StoredLink],
        new_path: &str,
    ) -> AppResult<Option<String>> {
        let lock = self.note_lock(source);
        let _guard = lock.lock().await;
        let vault = self.require_vault()?;
        let row = self.row(source).await?;
        if !row.is_live() || row.text_state != NoteTextState::Text {
            return Ok(None);
        }
        let path = files::resolve(&vault.root, &row.relative_path)?;
        let current = interpret(blocking(move || Ok(std::fs::read(&path)?)).await?);
        let Some(text) = current.text else {
            return Ok(None);
        };
        if current.hash != row.content_hash {
            return Err(conflict("The linking note changed on disk."));
        }
        let Some(rewritten) = index::rewrite_links(&text, &row.relative_path, links, new_path)
        else {
            return Ok(None);
        };
        self.write_text(
            &vault,
            &row,
            &current.hash,
            &rewritten,
            RevisionReason::AppSave,
        )
        .await?;
        Ok(Some(row.relative_path))
    }

    fn trash_dir(&self, vault_id: &str, note_id: &str) -> PathBuf {
        self.data_dir.join("trash").join(vault_id).join(note_id)
    }

    /// Move a note to Brainiac's trash, in its data folder; its tasks and links stay.
    pub async fn trash(self: &Arc<Self>, id: &str) -> AppResult<()> {
        let _gate = self.gate.read().await;
        self.trash_inner(id).await
    }

    async fn trash_inner(self: &Arc<Self>, id: &str) -> AppResult<()> {
        let lock = self.note_lock(id);
        let _guard = lock.lock().await;
        let vault = self.require_vault()?;
        let row = self.row(id).await?;
        if row.trashed_at.is_some() {
            return Ok(());
        }
        let _flight = InFlight::new(self, id);
        if row.is_live() && row.vault_id == vault.id {
            let from = files::resolve(&vault.root, &row.relative_path)?;
            let dir = self.trash_dir(&row.vault_id, id);
            let name = row
                .relative_path
                .rsplit('/')
                .next()
                .unwrap_or("note.md")
                .to_string();
            // Each trashing gets its own folder, so a note trashed again
            // never replaces what was trashed before.
            let dir = dir.join(chrono::Utc::now().format("%Y%m%dT%H%M%S%.3fZ").to_string());
            blocking(move || {
                std::fs::create_dir_all(&dir)?;
                files::move_file(&from, &dir.join(name))
            })
            .await?;
        }
        let (id2, now) = (id.to_string(), now_rfc3339());
        self.stores
            .core
            .call(move |conn| store::mark_missing(conn, &id2, &now, true))
            .await?;
        self.unindex(vec![id.to_string()]).await?;
        self.emit_changed(id, None, &row.relative_path, NoteChangeOrigin::Trash);
        Ok(())
    }

    pub async fn list_trash(&self) -> AppResult<Vec<TrashedNote>> {
        let vault = self.require_vault()?;
        let rows = self
            .stores
            .core
            .call(move |conn| store::trashed(conn, &vault.id))
            .await?;
        Ok(rows
            .into_iter()
            .map(|(note, trashed_at)| TrashedNote { note, trashed_at })
            .collect())
    }

    /// Put a trashed note back at its path. A note already there is moved to
    /// the trash only when `overwrite` is set; otherwise this is a `CONFLICT`.
    pub async fn restore(self: &Arc<Self>, id: &str, overwrite: bool) -> AppResult<NoteSummary> {
        let _gate = self.gate.read().await;
        let vault = self.require_vault()?;
        let row = self.row(id).await?;
        if row.trashed_at.is_none() {
            return Err(AppError::validation("That note is not in the trash."));
        }
        let dest = files::resolve(&vault.root, &row.relative_path)?;
        if std::fs::symlink_metadata(&dest).is_ok() {
            if !overwrite {
                return Err(conflict(format!(
                    "A note already exists at “{}”.",
                    row.relative_path
                )));
            }
            let (vault_id, path) = (vault.id.clone(), row.relative_path.clone());
            let occupant = self
                .stores
                .core
                .call(move |conn| store::live_note_at(conn, &vault_id, &path))
                .await?;
            match occupant {
                Some(other) => self.trash_inner(&other.id).await?,
                None => {
                    return Err(conflict(format!(
                        "“{}” exists but is not a note Brainiac knows yet. Move it first.",
                        row.relative_path
                    )))
                }
            }
        }
        let lock = self.note_lock(id);
        let _guard = lock.lock().await;
        let _flight = InFlight::new(self, id);
        let dir = self.trash_dir(&row.vault_id, id);
        let target = dest.clone();
        blocking(move || {
            let gone = || AppError::not_found("The trashed file is gone from Brainiac's trash.");
            // The newest trashing of this note; older ones stay in the trash folder.
            let mut stamps: Vec<PathBuf> = std::fs::read_dir(&dir)
                .map_err(|_| gone())?
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect();
            stamps.sort();
            let newest = stamps.pop().ok_or_else(gone)?;
            let file = std::fs::read_dir(&newest)
                .map_err(|_| gone())?
                .flatten()
                .map(|e| e.path())
                .next()
                .ok_or_else(gone)?;
            files::move_file(&file, &target)?;
            let _ = std::fs::remove_dir(&newest);
            Ok(())
        })
        .await?;
        let text = blocking(move || Ok(std::fs::read(&dest)?)).await?;
        self.readopt(&vault, id, &row.relative_path, interpret(text))
            .await?;
        self.emit_changed(id, None, &row.relative_path, NoteChangeOrigin::Trash);
        self.summary(id).await
    }

    /// Record `read`, found at `rel`, as the existing note `id` and index it.
    async fn readopt(
        &self,
        vault: &ActiveVault,
        id: &str,
        rel: &str,
        read: ReadNote,
    ) -> AppResult<()> {
        let path = files::resolve(&vault.root, rel)?;
        let meta = blocking(move || Ok(std::fs::metadata(&path)?)).await?;
        let (state, doc) = describe(id, rel, &read, meta.len() as i64, files::mtime_ns(&meta));
        let (id2, vault_id) = (id.to_string(), vault.id.clone());
        self.stores
            .core
            .call(move |conn| {
                store::set_vault(conn, &id2, &vault_id)?;
                store::set_file(conn, &id2, &state)
            })
            .await?;
        self.index_docs(vec![doc]).await?;
        self.resolve_links().await
    }

    /// Write `text` to a missing note's last path (or a free variant of it)
    /// and make the note live again (Restore as New File).
    pub async fn recreate(self: &Arc<Self>, id: &str, text: String) -> AppResult<NoteSummary> {
        let _gate = self.gate.read().await;
        let lock = self.note_lock(id);
        let _guard = lock.lock().await;
        let vault = self.require_vault()?;
        let row = self.row(id).await?;
        if row.is_live() {
            return Err(AppError::validation("That note's file exists."));
        }
        let _flight = InFlight::new(self, id);
        let (folder, name) = match row.relative_path.rfind('/') {
            Some(i) => (&row.relative_path[..i], &row.relative_path[i + 1..]),
            None => ("", row.relative_path.as_str()),
        };
        let base = index::file_stem(name).to_string();
        let rel = self
            .unique_path(&vault, folder, &base, text.as_bytes())
            .await?;
        let read = ReadNote {
            hash: index::content_hash(text.as_bytes()),
            text_state: NoteTextState::Text,
            text: Some(text),
        };
        self.readopt(&vault, id, &rel, read).await?;
        let id2 = id.to_string();
        self.stores
            .history
            .call(move |conn| history::delete_draft(conn, &id2))
            .await?;
        self.emit_changed(id, None, &rel, NoteChangeOrigin::App);
        self.summary(id).await
    }

    /// Point a missing note at an existing file (Relink to a File…). A note
    /// Brainiac already knows at that path is merged into this one, unless it
    /// has tasks, links, or a pin of its own.
    pub async fn relink(self: &Arc<Self>, id: &str, path: &str) -> AppResult<NoteSummary> {
        Self::check_note_path(path)?;
        let _gate = self.gate.read().await;
        let lock = self.note_lock(id);
        let _guard = lock.lock().await;
        let vault = self.require_vault()?;
        let row = self.row(id).await?;
        if row.is_live() {
            return Err(AppError::validation("That note's file exists."));
        }
        // Read the file first: if that fails, nothing has changed yet.
        let abs = files::resolve(&vault.root, path)?;
        let bytes = blocking(move || Ok(std::fs::read(&abs)?)).await?;
        let path = &files::on_disk(&vault.root, path);
        let (vault_id, rel) = (vault.id.clone(), path.to_string());
        let occupant = self
            .stores
            .core
            .call(move |conn| {
                let other = store::live_note_at(conn, &vault_id, &rel)?;
                match other {
                    Some(o) if store::has_context(conn, &o.id)? => Err(conflict(
                        "That file is already a note with its own tasks, links, or pin.",
                    )),
                    Some(o) => {
                        store::delete_note(conn, &o.id)?;
                        Ok(Some(o.id))
                    }
                    None => Ok(None),
                }
            })
            .await?;
        if let Some(other) = &occupant {
            let ids = vec![other.clone()];
            self.stores
                .index
                .call(move |conn| index::remove_docs(conn, &ids))
                .await?;
        }
        self.readopt(&vault, id, path, interpret(bytes)).await?;
        self.emit_changed(id, None, path, NoteChangeOrigin::App);
        self.summary(id).await
    }

    // -----------------------------------------------------------------------
    // Revisions and drafts
    // -----------------------------------------------------------------------

    pub async fn revisions(&self, id: &str) -> AppResult<Vec<NoteRevision>> {
        let id = id.to_string();
        self.stores
            .history
            .call(move |conn| history::list(conn, &id))
            .await
    }

    pub async fn revision_text(&self, revision_id: &str) -> AppResult<String> {
        let rid = revision_id.to_string();
        self.stores
            .history
            .call(move |conn| history::get(conn, &rid))
            .await?
            .map(|(_, text)| text)
            .ok_or_else(|| AppError::not_found("That revision no longer exists."))
    }

    /// Make a revision the note's text again. The text it replaces becomes a revision too.
    pub async fn restore_revision(
        self: &Arc<Self>,
        id: &str,
        revision_id: &str,
        expected_version: &str,
    ) -> AppResult<SaveNoteResult> {
        let rid = revision_id.to_string();
        let (note_id, text) = self
            .stores
            .history
            .call(move |conn| history::get(conn, &rid))
            .await?
            .ok_or_else(|| AppError::not_found("That revision no longer exists."))?;
        if note_id != id {
            return Err(AppError::validation(
                "That revision belongs to another note.",
            ));
        }
        let _gate = self.gate.read().await;
        let lock = self.note_lock(id);
        let _guard = lock.lock().await;
        let vault = self.require_vault()?;
        let row = self.row(id).await?;
        if !row.is_live() {
            return Err(gone());
        }
        self.write_text(
            &vault,
            &row,
            expected_version,
            &text,
            RevisionReason::Restore,
        )
        .await
    }

    /// Keep unsaved edits that cannot be saved now (the file changed on disk,
    /// or is gone) as the note's draft, against the version they started from.
    pub async fn keep_draft(&self, id: &str, base_version: &str, text: String) -> AppResult<()> {
        if text.len() as u64 > EDIT_LIMIT {
            return Err(AppError::validation(
                "Notes over 5 MiB cannot be edited in Brainiac.",
            ));
        }
        let (id, base, now) = (id.to_string(), base_version.to_string(), now_rfc3339());
        self.stores
            .history
            .call(move |conn| history::put_draft(conn, &id, &base, &text, &now))
            .await
    }

    pub async fn discard_draft(&self, id: &str) -> AppResult<()> {
        let id = id.to_string();
        self.stores
            .history
            .call(move |conn| history::delete_draft(conn, &id))
            .await
    }

    /// Save text as a new note next to `id` (Save Draft as Copy). The copy
    /// gets its own `brainiac_id`, so it is not taken for the original.
    pub async fn save_copy(self: &Arc<Self>, id: &str, text: String) -> AppResult<NoteSummary> {
        let _gate = self.gate.read().await;
        let vault = self.require_vault()?;
        let row = self.row(id).await?;
        let new_id = uuid::Uuid::new_v4().to_string();
        let _flight = InFlight::new(self, &new_id);
        let text = index::replace_embedded_id(&text, &new_id);
        let (folder, name) = match row.relative_path.rfind('/') {
            Some(i) => (
                row.relative_path[..i].to_string(),
                &row.relative_path[i + 1..],
            ),
            None => (String::new(), row.relative_path.as_str()),
        };
        let base = format!("{} (copy)", index::file_stem(name));
        let rel = self
            .unique_path(&vault, &folder, &base, text.as_bytes())
            .await?;
        self.adopt_new_file(&vault, &new_id, &rel, text, NoteChangeOrigin::App)
            .await?;
        self.summary(&new_id).await
    }

    /// Write `brainiac_id` into a note that is getting a task or repository
    /// link, so a rename outside Brainiac cannot lose that context (SPEC.md,
    /// Note identity). Skipped when the setting is off, the note already has
    /// one, or the file changed since the last scan.
    pub async fn ensure_embedded_id(self: &Arc<Self>, id: &str) -> AppResult<()> {
        if !self
            .write_note_ids
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            return Ok(());
        }
        let _gate = self.gate.read().await;
        let lock = self.note_lock(id);
        let _guard = lock.lock().await;
        let vault = self.require_vault()?;
        let row = self.row(id).await?;
        if row.embedded_id.is_some() || !row.is_live() || row.text_state != NoteTextState::Text {
            return Ok(());
        }
        let path = files::resolve(&vault.root, &row.relative_path)?;
        let current = interpret(blocking(move || Ok(std::fs::read(&path)?)).await?);
        let (Some(text), true) = (current.text, current.hash == row.content_hash) else {
            return Ok(());
        };
        let with_id = index::with_embedded_id(&text, &row.id);
        self.write_text(
            &vault,
            &row,
            &current.hash,
            &with_id,
            RevisionReason::AppSave,
        )
        .await?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Context: repositories, tasks, backlinks
    // -----------------------------------------------------------------------

    /// What the context panel shows for a note.
    pub async fn context(&self, id: &str) -> AppResult<NoteContext> {
        let id2 = id.to_string();
        let (links, dismissed, repositories, tasks) = self
            .stores
            .core
            .call(move |conn| {
                Ok((
                    store::repository_links(conn, &id2)?,
                    store::dismissed(conn, &id2)?,
                    store::repositories(conn)?,
                    crate::tasks::for_note(conn, &id2)?,
                ))
            })
            .await?;
        let id2 = id.to_string();
        let (body, into, out) = self
            .stores
            .reader
            .call(move |conn| {
                Ok((
                    index::body(conn, &id2)?.unwrap_or_default(),
                    index::links_into(conn, &id2)?,
                    index::links_from(conn, &id2)?,
                ))
            })
            .await?;

        // Backlinks: one per linking note, at its first link.
        let mut first_link: Vec<index::StoredLink> = Vec::new();
        for l in into {
            if !first_link
                .iter()
                .any(|f| f.source_note_id == l.source_note_id)
            {
                first_link.push(l);
            }
        }
        first_link.truncate(200);
        let source_ids: Vec<String> = first_link
            .iter()
            .map(|l| l.source_note_id.clone())
            .collect();
        let ids = source_ids.clone();
        let sources = self
            .stores
            .core
            .call(move |conn| store::summaries(conn, &ids))
            .await?;
        let lines: Vec<(String, u32)> = first_link
            .iter()
            .map(|l| (l.source_note_id.clone(), l.line))
            .collect();
        let excerpts: HashMap<String, String> = self
            .stores
            .reader
            .call(move |conn| {
                let mut out = HashMap::new();
                for (source, line) in lines {
                    if let Some(text) = index::body(conn, &source)? {
                        out.insert(source, index::line_excerpt(&text, line));
                    }
                }
                Ok(out)
            })
            .await?;
        let mut backlinks: Vec<Backlink> = sources
            .into_iter()
            .filter(|n| !n.missing)
            .map(|note| {
                let line = first_link
                    .iter()
                    .find(|l| l.source_note_id == note.id)
                    .map_or(0, |l| l.line);
                Backlink {
                    excerpt: excerpts.get(&note.id).cloned().unwrap_or_default(),
                    note,
                    line,
                }
            })
            .collect();
        backlinks.sort_by(|a, b| {
            a.note
                .title
                .to_lowercase()
                .cmp(&b.note.title.to_lowercase())
        });

        let row = self.row(id).await?;
        let mut unresolved: Vec<UnresolvedLink> = Vec::new();
        for l in out.into_iter().filter(|l| l.target_note_id.is_none()) {
            if unresolved.iter().any(|u| u.raw_target == l.raw_target) {
                continue;
            }
            unresolved.push(UnresolvedLink {
                suggested_path: index::suggested_path(&row.relative_path, &l.raw_target, l.kind),
                raw_target: l.raw_target,
                kind: l.kind,
                line: l.line,
            });
        }

        let registered: HashMap<String, (String, Option<String>)> = repositories
            .iter()
            .map(|(id, name, url)| (id.clone(), (name.clone(), url.clone())))
            .collect();
        let linked: Vec<LinkedRepository> = links
            .iter()
            .map(|l| LinkedRepository {
                repository_id: l.repository_id.clone(),
                name: registered
                    .get(&l.repository_id)
                    .map_or(l.repository_name.clone(), |r| r.0.clone()),
                remote_url: l.remote_url.clone(),
                registered: l.registered,
                reconnect_to: if l.registered {
                    None
                } else {
                    l.remote_url.as_ref().and_then(|url| {
                        repositories
                            .iter()
                            .find(|(_, _, u)| u.as_ref() == Some(url))
                            .map(|(id, _, _)| id.clone())
                    })
                },
            })
            .collect();
        let suggestions = mentioned_repositories(&body, &repositories)
            .into_iter()
            .filter(|(rid, _)| {
                !links.iter().any(|l| &l.repository_id == rid) && !dismissed.contains(rid)
            })
            .map(|(repository_id, name)| RepositorySuggestion {
                repository_id,
                name,
            })
            .collect();
        Ok(NoteContext {
            note_id: id.to_string(),
            repositories: linked,
            tasks,
            backlinks,
            unresolved,
            suggestions,
        })
    }

    /// The note a link in `from_id` leads to, or where Create would put it.
    pub async fn resolve_link(
        &self,
        from_id: &str,
        target: &str,
        wikilink: bool,
    ) -> AppResult<ResolvedLink> {
        let row = self.row(from_id).await?;
        let kind = if wikilink {
            NoteLinkKind::Wikilink
        } else {
            NoteLinkKind::Markdown
        };
        let (from, raw) = (row.relative_path.clone(), target.to_string());
        let found = self
            .stores
            .reader
            .call(move |conn| index::resolve_one(conn, &from, &raw, kind))
            .await?;
        let note = match found {
            Some(id) => Some(self.summary(&id).await?),
            None => None,
        };
        Ok(ResolvedLink {
            note,
            suggested_path: index::suggested_path(&row.relative_path, target, kind),
        })
    }

    /// A repository's Notes tab: linked notes, open tasks, and notes that mention it.
    pub async fn repository_notes(&self, repository_id: &str) -> AppResult<RepositoryNotes> {
        let rid = repository_id.to_string();
        let (linked, tasks, name) = self
            .stores
            .core
            .call(move |conn| {
                let linked = store::notes_linked_to(conn, &rid)?;
                let tasks = crate::tasks::open_for_repository(conn, &rid)?;
                let name = store::repositories(conn)?
                    .into_iter()
                    .find(|(id, _, _)| id == &rid)
                    .map(|(_, name, _)| name);
                Ok((store::summaries(conn, &linked)?, tasks, name))
            })
            .await?;
        let mut suggested = Vec::new();
        if let (Some(name), Some(vault)) = (name, self.vault()) {
            let phrase = format!("\"{}\"", name.replace('"', ""));
            let ids: Vec<String> = self
                .stores
                .reader
                .call(move |conn| {
                    let mut stmt = conn.prepare(
                        "SELECT b.note_id FROM note_search JOIN note_bodies b ON b.seq = note_search.rowid
                         WHERE note_search MATCH ?1 ORDER BY bm25(note_search, 10.0, 1.0) LIMIT 30",
                    )?;
                    let rows = stmt.query_map([phrase], |r| r.get::<_, String>(0))?;
                    Ok(rows.filter_map(Result::ok).collect())
                })
                .await
                .unwrap_or_default();
            let ids: Vec<String> = ids
                .into_iter()
                .filter(|i| !linked.iter().any(|n| &n.id == i))
                .collect();
            suggested = self
                .stores
                .core
                .call(move |conn| store::summaries(conn, &ids))
                .await?
                .into_iter()
                .filter(|n| !n.missing && !n.trashed)
                .collect();
            let _ = vault;
        }
        Ok(RepositoryNotes {
            repository_id: repository_id.to_string(),
            notes: linked.into_iter().filter(|n| !n.trashed).collect(),
            tasks,
            suggested,
        })
    }

    /// Link a note to a registered repository (Link…, a suggestion, or New
    /// Note for repository), then write the note's `brainiac_id`.
    pub async fn link_repository(
        self: &Arc<Self>,
        note_id: &str,
        repository_id: &str,
    ) -> AppResult<()> {
        {
            let _gate = self.gate.read().await;
            let (nid, rid, now) = (
                note_id.to_string(),
                repository_id.to_string(),
                now_rfc3339(),
            );
            self.stores
                .core
                .call(move |conn| {
                    store::get_note(conn, &nid)?
                        .ok_or_else(|| AppError::not_found("That note does not exist."))?;
                    let repo = db::get_repository(conn, &rid)?
                        .ok_or_else(|| AppError::not_found("That repository is not registered."))?;
                    let name = Path::new(&repo.canonical_root)
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or(repo.canonical_root.clone());
                    store::link_repository(
                        conn,
                        &nid,
                        &rid,
                        &name,
                        repo.remote_url.as_deref(),
                        &now,
                    )
                })
                .await?;
        }
        if let Err(e) = self.ensure_embedded_id(note_id).await {
            tracing::warn!(error = %e, "could not write brainiac_id into the note");
        }
        self.announce_context(note_id).await;
        Ok(())
    }

    pub async fn unlink_repository(&self, note_id: &str, repository_id: &str) -> AppResult<()> {
        let _gate = self.gate.read().await;
        let (nid, rid) = (note_id.to_string(), repository_id.to_string());
        self.stores
            .core
            .call(move |conn| store::unlink_repository(conn, &nid, &rid))
            .await?;
        self.announce_context(note_id).await;
        Ok(())
    }

    pub async fn dismiss_suggestion(&self, note_id: &str, repository_id: &str) -> AppResult<()> {
        let (nid, rid) = (note_id.to_string(), repository_id.to_string());
        self.stores
            .core
            .call(move |conn| store::dismiss_suggestion(conn, &nid, &rid))
            .await
    }

    /// Move the links of a removed repository to a registered one with the same remote.
    pub async fn reconnect_repository(&self, from: &str, to: &str) -> AppResult<()> {
        let _gate = self.gate.read().await;
        let (from, to) = (from.to_string(), to.to_string());
        let changed = self
            .stores
            .core
            .call(move |conn| {
                let repo = db::get_repository(conn, &to)?
                    .ok_or_else(|| AppError::not_found("That repository is not registered."))?;
                let name = Path::new(&repo.canonical_root)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or(repo.canonical_root.clone());
                store::reconnect(conn, &from, &to, &name, repo.remote_url.as_deref())
            })
            .await?;
        // The tasks' versions went up: open views must take the new ones,
        // or their next edit would be refused as a conflict.
        for (task_id, version) in changed.tasks {
            self.emit(KnowledgeEvent::TaskChanged(TaskChangedEvent {
                task_id,
                version: Some(version),
            }));
        }
        for note_id in &changed.notes {
            self.announce_context(note_id).await;
        }
        Ok(())
    }

    /// Tell open views that a note's context changed; its text did not.
    pub(crate) async fn announce_context(&self, note_id: &str) {
        if let Ok(row) = self.row(note_id).await {
            let version = row.is_live().then(|| row.content_hash.clone());
            self.emit_changed(note_id, version, &row.relative_path, NoteChangeOrigin::App);
        }
    }
}

/// Registered repositories a note mentions by name as a whole word, ignoring
/// case. Names under three characters are too ambiguous to suggest.
fn mentioned_repositories(
    body: &str,
    repositories: &[(String, String, Option<String>)],
) -> Vec<(String, String)> {
    let lower = body.to_lowercase();
    let mut out = Vec::new();
    for (id, name, _) in repositories {
        let needle = name.to_lowercase();
        if needle.chars().count() < 3 {
            continue;
        }
        let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '-');
        let found = lower.match_indices(&needle).any(|(i, _)| {
            !word(lower[..i].chars().next_back()) && !word(lower[i + needle.len()..].chars().next())
        });
        if found && !out.iter().any(|(_, n): &(String, String)| n == name) {
            out.push((id.clone(), name.clone()));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repositories_are_suggested_when_mentioned_as_words() {
        let repos = vec![
            ("1".to_string(), "billing".to_string(), None),
            ("2".to_string(), "api".to_string(), None),
            ("3".to_string(), "web".to_string(), None),
        ];
        let found =
            mentioned_repositories("Deploy Billing first, then the website and api.", &repos);
        assert_eq!(
            found,
            vec![
                ("1".to_string(), "billing".to_string()),
                ("2".to_string(), "api".to_string())
            ]
        );
        assert!(mentioned_repositories("billing-service", &repos).is_empty());
    }
}
