//! The vault: choosing it, reconciling the index with its files, and
//! watching it (SPEC.md, The vault and Note identity; docs/architecture.md,
//! Vault watcher).
//!
//! A reconciliation compares files with the notes `brainiac.db` knows: a
//! changed file is re-read and re-indexed, keeping its previous text as a
//! revision; a new path is matched to a note that went missing (by
//! `brainiac_id`, then by identical content) before it becomes a new note;
//! a note whose file is gone becomes a tombstone that keeps its tasks and
//! links. The watcher only tells it which paths to look at.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use notify::{RecursiveMode, Watcher};
use tokio::sync::mpsc;

use crate::index::{self, IndexDoc, BATCH_BYTES, BATCH_NOTES};
use crate::models::{
    now_rfc3339, AppError, AppResult, IndexState, NoteChangeOrigin, NoteMissingEvent,
    NoteTextState, RevisionReason, VaultState,
};
use crate::notes::history::{self, NewRevision};
use crate::notes::store::{self, FileState, NoteRow};
use crate::notes::{
    describe, interpret, walk, ActiveVault, Found, KnowledgeEvent, Listings, NoteService, ReadNote,
};

/// Queued paths are processed once the vault has been quiet this long…
const QUIET: Duration = Duration::from_millis(300);
/// …and at least this often during a longer burst.
const MAX_WAIT: Duration = Duration::from_secs(2);
/// A burst ends after this long without events; until then a missing note
/// stays matchable by content (Decisions, 2 Oct 2026, spike S4).
const BURST_END: Duration = Duration::from_secs(2);
/// Per-note change events are skipped for scans that add more notes than this.
const EVENT_LIMIT: usize = 200;

enum WatchMsg {
    Paths(Vec<String>),
    /// The watcher lost events or failed: reconcile everything.
    Rescan,
}

/// A note that went missing in the current burst, matchable by content.
#[derive(Debug, Clone)]
struct Gone {
    id: String,
    hash: String,
    /// Its last indexed text, kept as a revision if it is still missing when the burst ends.
    body: Option<String>,
}

/// The watcher and the state it shares with reconciliations.
#[derive(Default)]
pub struct WatchState {
    burst: Mutex<Vec<Gone>>,
    sender: Mutex<Option<mpsc::UnboundedSender<WatchMsg>>>,
    watcher: Mutex<Option<notify::RecommendedWatcher>>,
    /// One reconciliation at a time.
    scan: tokio::sync::Mutex<()>,
    /// Bumped whenever another vault is chosen; scans of the old one stop.
    generation: AtomicU64,
    last_full: Mutex<Option<Instant>>,
    /// Off only for tests and tools that reconcile explicitly.
    disabled: std::sync::atomic::AtomicBool,
}

/// What to reconcile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    /// Walk the whole vault, and re-index notes whose index entry is stale.
    Full,
    /// Only these vault-relative paths (files or folders, existing or not).
    Paths(Vec<String>),
}

/// What one reconciliation did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanStats {
    pub read: usize,
    pub changed: usize,
    pub touched: usize,
    pub added: usize,
    pub moved: usize,
    pub missing: usize,
    pub unavailable: bool,
}

/// A file to commit: the note it is (known, moved, or new) and what was read.
struct Job {
    note_id: String,
    found: Found,
    read: ReadNote,
    /// The note's row before this change, when it existed.
    previous: Option<NoteRow>,
}

fn is_hidden(rel: &str) -> bool {
    rel.split('/').any(|c| c.starts_with('.'))
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> AppResult<T> + Send + 'static,
) -> AppResult<T> {
    tokio::task::spawn_blocking(f).await.map_err(|e| {
        AppError::io("A file operation stopped unexpectedly.").with_details(e.to_string())
    })?
}

impl NoteService {
    /// Load the active vault at startup, start watching it, and reconcile in the background.
    pub async fn start(self: &Arc<Self>) -> AppResult<()> {
        let row = self
            .stores
            .core
            .call(|conn| store::active_vault(conn))
            .await?;
        let Some(row) = row else {
            self.set_status(|s| s.state = IndexState::NoVault);
            return Ok(());
        };
        let root =
            std::fs::canonicalize(&row.root_path).unwrap_or_else(|_| PathBuf::from(&row.root_path));
        let vault = ActiveVault {
            id: row.id,
            name: row.name,
            root,
        };
        self.activate(vault);
        Ok(())
    }

    /// Choose (or create) the vault folder. Brainiac edits the notes where
    /// they are; nothing is moved or converted (SPEC.md, The vault).
    pub async fn select_vault(self: &Arc<Self>, path: &str, create: bool) -> AppResult<VaultState> {
        let requested = PathBuf::from(path);
        if create {
            if requested.exists()
                && std::fs::read_dir(&requested)?.any(|e| {
                    e.map(|e| !e.file_name().to_string_lossy().starts_with('.'))
                        .unwrap_or(false)
                })
            {
                return Err(AppError::validation(
                    "That folder is not empty. Choose it with Choose Folder… to use the notes in it.",
                ));
            }
            std::fs::create_dir_all(&requested)?;
        }
        let root = std::fs::canonicalize(&requested)?;
        if !root.is_dir() {
            return Err(AppError::validation("Choose a folder for the vault."));
        }
        if root.parent().is_none() {
            return Err(AppError::validation(
                "The top of the disk cannot be a vault.",
            ));
        }
        let data = std::fs::canonicalize(&self.data_dir).unwrap_or_else(|_| self.data_dir.clone());
        if data.starts_with(&root) || root.starts_with(&data) {
            return Err(AppError::validation(
                "The vault cannot contain Brainiac's own data folder, or be inside it.",
            ));
        }
        let name = root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Vault".into());
        let (root_text, name2, now) = (root.display().to_string(), name.clone(), now_rfc3339());
        // Every note write holds the gate for reading, so the switch waits
        // for writes in progress, and none starts against the old vault's
        // note in the new folder. The guard lets go when it goes out of scope.
        let _gate = self.gate.write().await;
        let row = self
            .stores
            .core
            .call(move |conn| store::activate_vault(conn, &root_text, &name2, &now))
            .await?;
        self.activate(ActiveVault {
            id: row.id,
            name,
            root,
        });
        Ok(self.vault_state().await)
    }

    /// Use `vault` from now on: watch it and reconcile it in the background.
    fn activate(self: &Arc<Self>, vault: ActiveVault) {
        self.watch.generation.fetch_add(1, Ordering::SeqCst);
        self.watch.burst.lock().expect("burst").clear();
        *self.vault.write().expect("vault") = Some(vault.clone());
        if self.watch.disabled.load(Ordering::SeqCst) {
            *self.watch.watcher.lock().expect("watcher") = None;
            *self.watch.sender.lock().expect("sender") = None;
        } else if let Err(e) = self.start_watching(&vault) {
            tracing::warn!(error = %e, "could not watch the vault; changes are found on the next scan");
        }
        self.set_status(|s| {
            s.state = IndexState::Indexing;
            s.done = 0;
            s.total = 0;
            s.message = None;
        });
        let service = Arc::clone(self);
        tokio::spawn(async move {
            if let Err(e) = service.reconcile(Scope::Full, false).await {
                tracing::warn!(error = %e, details = ?e.details, "vault scan failed");
                service.set_status(|s| {
                    s.state = IndexState::Unavailable;
                    s.message = Some(e.message.clone());
                });
            }
        });
    }

    /// Do not watch vaults chosen from now on; changes are found only by
    /// explicit reconciliation. For tests and tools.
    pub fn disable_watching(&self) {
        self.watch.disabled.store(true, Ordering::SeqCst);
    }

    /// Wait until the scan running now, if any, has finished.
    pub async fn settle(&self) {
        drop(self.watch.scan.lock().await);
    }

    /// Ask for some paths to be reconciled, through the watcher's batching when it runs.
    pub fn request_reconcile(self: &Arc<Self>, paths: Vec<String>) {
        let sender = self.watch.sender.lock().expect("sender").clone();
        if let Some(tx) = sender {
            if tx.send(WatchMsg::Paths(paths.clone())).is_ok() {
                return;
            }
        }
        let service = Arc::clone(self);
        tokio::spawn(async move {
            if let Err(e) = service.reconcile(Scope::Paths(paths), false).await {
                tracing::warn!(error = %e, "reconciling notes failed");
            }
        });
    }

    /// Reconcile the whole vault, as on wake and activation, unless one ran
    /// in the last `min_age`.
    pub fn request_full_reconcile(self: &Arc<Self>, min_age: Duration) {
        if self.vault().is_none() {
            return;
        }
        {
            let last = self.watch.last_full.lock().expect("last full");
            if last.is_some_and(|t| t.elapsed() < min_age) {
                return;
            }
        }
        let service = Arc::clone(self);
        tokio::spawn(async move {
            if let Err(e) = service.reconcile(Scope::Full, false).await {
                tracing::warn!(error = %e, "vault scan failed");
            }
        });
    }

    /// Empty the search index and build it again from the vault (Rebuild Index).
    pub async fn rebuild_search(self: &Arc<Self>) -> AppResult<()> {
        self.stores.index.call(index::clear).await?;
        self.stores
            .core
            .call(|conn| {
                conn.execute_batch(
                    "INSERT INTO task_search (task_search) VALUES ('rebuild');
                     INSERT INTO task_title_search (task_title_search) VALUES ('rebuild');",
                )?;
                Ok(())
            })
            .await?;
        if self.vault().is_none() {
            self.set_status(|s| {
                s.state = IndexState::NoVault;
                s.message = None;
            });
            return Ok(());
        }
        self.set_status(|s| {
            s.state = IndexState::Indexing;
            s.done = 0;
            s.total = 0;
            s.message = None;
        });
        self.reconcile(Scope::Full, false).await.map(|_| ())
    }

    // -----------------------------------------------------------------------
    // Watching
    // -----------------------------------------------------------------------

    fn start_watching(self: &Arc<Self>, vault: &ActiveVault) -> AppResult<()> {
        let (tx, rx) = mpsc::unbounded_channel::<WatchMsg>();
        let root = vault.root.clone();
        let callback_tx = tx.clone();
        let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            let event = match res {
                Ok(e) => e,
                Err(e) => {
                    tracing::warn!(error = %e, "vault watcher error; rescanning");
                    let _ = callback_tx.send(WatchMsg::Rescan);
                    return;
                }
            };
            if event.need_rescan() {
                let _ = callback_tx.send(WatchMsg::Rescan);
            }
            // Event kinds are not trusted: FSEvents merges flags per path and
            // never pairs the two sides of a rename. Every path named is
            // treated as possibly changed.
            let mut paths = Vec::new();
            for p in &event.paths {
                let Ok(rel) = p.strip_prefix(&root) else {
                    continue;
                };
                let Some(rel) = rel.to_str() else { continue };
                if rel.is_empty() {
                    let _ = callback_tx.send(WatchMsg::Rescan);
                } else if !is_hidden(rel) {
                    paths.push(rel.to_string());
                }
            }
            if !paths.is_empty() {
                let _ = callback_tx.send(WatchMsg::Paths(paths));
            }
        })
        .map_err(|e| {
            AppError::io("Could not start the vault watcher.").with_details(e.to_string())
        })?;
        watcher
            .watch(&vault.root, RecursiveMode::Recursive)
            .map_err(|e| {
                AppError::io(format!("Could not watch {}.", vault.root.display()))
                    .with_details(e.to_string())
            })?;
        // Replacing the previous watcher drops its callback and sender, which ends its loop.
        *self.watch.watcher.lock().expect("watcher") = Some(watcher);
        *self.watch.sender.lock().expect("sender") = Some(tx);
        tokio::spawn(Arc::clone(self).watch_loop(rx));
        Ok(())
    }

    /// Batch queued paths: process them together once the vault has been
    /// quiet for `QUIET`, at least every `MAX_WAIT`, and end the burst after
    /// `BURST_END` without events.
    async fn watch_loop(self: Arc<Self>, mut rx: mpsc::UnboundedReceiver<WatchMsg>) {
        let mut pending: BTreeSet<String> = BTreeSet::new();
        let mut first: Option<Instant> = None;
        let mut last: Option<Instant> = None;
        let mut full = false;
        let mut burst_open = false;
        loop {
            match tokio::time::timeout(Duration::from_millis(50), rx.recv()).await {
                Ok(Some(msg)) => {
                    let now = Instant::now();
                    match msg {
                        WatchMsg::Paths(paths) => {
                            pending.extend(paths);
                            first.get_or_insert(now);
                        }
                        WatchMsg::Rescan => full = true,
                    }
                    last = Some(now);
                    burst_open = true;
                    continue;
                }
                Ok(None) => break,
                Err(_) => {}
            }
            let now = Instant::now();
            let quiet = last.is_none_or(|l| now - l >= QUIET);
            if full && quiet {
                full = false;
                pending.clear();
                first = None;
                if let Err(e) = self.reconcile(Scope::Full, true).await {
                    tracing::warn!(error = %e, "vault scan failed");
                }
            } else if !pending.is_empty() && (quiet || first.is_some_and(|f| now - f >= MAX_WAIT)) {
                let paths: Vec<String> = std::mem::take(&mut pending).into_iter().collect();
                first = None;
                if let Err(e) = self.reconcile(Scope::Paths(paths), true).await {
                    tracing::warn!(error = %e, "reconciling changed notes failed");
                }
            }
            if burst_open
                && pending.is_empty()
                && !full
                && last.is_some_and(|l| now - l >= BURST_END)
            {
                burst_open = false;
                self.end_burst().await;
            }
        }
    }

    /// The burst is over: notes that went missing in it and were not found
    /// again keep their last text as a revision and are announced as missing.
    async fn end_burst(&self) {
        let gone: Vec<Gone> = std::mem::take(&mut *self.watch.burst.lock().expect("burst"));
        let mut revisions = Vec::new();
        let mut missing = Vec::new();
        for g in gone {
            if let Ok(row) = self.row(&g.id).await {
                if !row.is_live() {
                    if let Some(body) = g.body.filter(|b| !b.is_empty()) {
                        revisions.push(NewRevision {
                            note_id: g.id.clone(),
                            content: body,
                            content_hash: g.hash,
                            reason: RevisionReason::ExternalChange,
                        });
                    }
                    missing.push(g.id);
                }
            }
        }
        if let Err(e) = self.keep_revisions(revisions).await {
            tracing::warn!(error = %e, "could not keep the text of missing notes");
        }
        for id in missing {
            self.emit(KnowledgeEvent::NoteMissing(NoteMissingEvent {
                note_id: id,
            }));
        }
    }

    async fn keep_revisions(&self, revisions: Vec<NewRevision>) -> AppResult<()> {
        if revisions.is_empty() {
            return Ok(());
        }
        let now = now_rfc3339();
        self.stores
            .history
            .call(move |conn| history::add_revisions(conn, &revisions, &now))
            .await
    }

    // -----------------------------------------------------------------------
    // Reconciliation
    // -----------------------------------------------------------------------

    /// Bring the notes in `brainiac.db` and `index.db` in line with the
    /// vault's files for `scope`. With `hold_missing`, notes that disappear
    /// stay matchable by content until the current burst ends.
    pub async fn reconcile(
        self: &Arc<Self>,
        scope: Scope,
        hold_missing: bool,
    ) -> AppResult<ScanStats> {
        let _scan = self.watch.scan.lock().await;
        let generation = self.watch.generation.load(Ordering::SeqCst);
        let Some(vault) = self.vault() else {
            return Ok(ScanStats::default());
        };
        let full = scope == Scope::Full;
        let root = vault.root.clone();
        let readable =
            blocking(move || Ok(root.is_dir() && std::fs::read_dir(&root).is_ok())).await?;
        if !readable {
            // An unmounted disk is unavailable, never every note deleted.
            let shown = vault.root.display().to_string();
            self.set_status(|s| {
                s.state = IndexState::Unavailable;
                s.message = Some(format!(
                    "The vault folder “{shown}” can't be read. Connect its disk or choose the vault again."
                ));
            });
            return Ok(ScanStats {
                unavailable: true,
                ..Default::default()
            });
        }
        if full {
            *self.watch.last_full.lock().expect("last full") = Some(Instant::now());
        } else if self.index_status().state == IndexState::Unavailable {
            // The disk came back: look at everything.
            drop(_scan);
            return Box::pin(self.reconcile(Scope::Full, hold_missing)).await;
        }

        let vault_id = vault.id.clone();
        let known: Vec<NoteRow> = self
            .stores
            .core
            .call(move |conn| store::live_notes(conn, &vault_id))
            .await?;
        let indexed: Option<HashMap<String, String>> = if full {
            Some(
                self.stores
                    .index
                    .call(|conn| index::indexed_hashes(conn))
                    .await?,
            )
        } else {
            None
        };
        let in_flight: HashSet<String> = self.in_flight.lock().expect("in flight").clone();
        let by_path: HashMap<String, NoteRow> = known
            .into_iter()
            .map(|r| (r.relative_path.clone(), r))
            .collect();

        // Which files exist, and which known notes are gone.
        let root = vault.root.clone();
        let known_paths: Vec<String> = by_path.keys().cloned().collect();
        let (found, gone_paths) = blocking(move || survey(&root, &scope, &known_paths)).await?;

        let mut stats = ScanStats::default();
        let mut changed: Vec<(NoteRow, Found)> = Vec::new();
        let mut new_files: Vec<Found> = Vec::new();
        for f in found {
            match by_path.get(&f.relative_path) {
                Some(row) if in_flight.contains(&row.id) => {}
                Some(row) => {
                    let index_ok = indexed
                        .as_ref()
                        .is_none_or(|ix| ix.get(&row.id) == Some(&row.content_hash));
                    if row.size == f.size as i64 && row.mtime == f.mtime && index_ok {
                        continue;
                    }
                    changed.push((row.clone(), f));
                }
                None => new_files.push(f),
            }
        }
        let mut gone: Vec<NoteRow> = gone_paths
            .iter()
            .filter_map(|p| by_path.get(p))
            .filter(|r| !in_flight.contains(&r.id))
            .cloned()
            .collect();

        let total = changed.len() + new_files.len();
        if full {
            self.set_status(|s| {
                s.state = IndexState::Indexing;
                s.done = 0;
                s.total = total as u64;
                s.message = None;
            });
        }
        let mut progress = 0usize;
        let mut events: Vec<(String, String, String)> = Vec::new();
        // Notes re-indexed because their index entry was stale (a rebuilt
        // index): links into them resolve only once all are back.
        let mut reindexed = false;
        // Notes whose path now holds a file carrying another note's `brainiac_id`.
        let mut displaced: HashSet<String> = HashSet::new();

        // Known paths: changed content, or only a new size or time.
        for chunk in batches(changed, |(_, f)| f.size) {
            if self.watch.generation.load(Ordering::SeqCst) != generation {
                return Ok(stats);
            }
            let root = vault.root.clone();
            let jobs: Vec<Job> = blocking(move || {
                Ok(chunk
                    .into_iter()
                    .filter_map(|(row, found)| {
                        let bytes = std::fs::read(root.join(&found.relative_path)).ok()?;
                        Some(Job {
                            note_id: row.id.clone(),
                            read: interpret(bytes),
                            previous: Some(row),
                            found,
                        })
                    })
                    .collect())
            })
            .await?;
            stats.read += jobs.len();
            progress += jobs.len();
            let mut commit = Vec::new();
            let mut touches = Vec::new();
            for job in jobs {
                let previous = job.previous.as_ref().expect("known note");
                // `brainiac_id` comes before the path (SPEC.md, Note identity):
                // a file that now carries another ID is another note, and this
                // note went somewhere else.
                let file_id = job
                    .read
                    .text
                    .as_deref()
                    .and_then(|t| index::frontmatter(t).brainiac_id);
                if file_id.is_some() && file_id != previous.embedded_id {
                    displaced.insert(previous.id.clone());
                    gone.push(previous.clone());
                    new_files.push(job.found);
                    continue;
                }
                let index_ok = indexed
                    .as_ref()
                    .is_none_or(|ix| ix.get(&previous.id) == Some(&job.read.hash));
                if job.read.hash == previous.content_hash && index_ok {
                    touches.push((job.note_id, job.found.size as i64, job.found.mtime));
                    stats.touched += 1;
                } else {
                    reindexed |= !index_ok;
                    if job.read.hash != previous.content_hash {
                        stats.changed += 1;
                        events.push((
                            job.note_id.clone(),
                            job.read.hash.clone(),
                            job.found.relative_path.clone(),
                        ));
                    }
                    commit.push(job);
                }
            }
            if !touches.is_empty() {
                self.stores
                    .core
                    .call(move |conn| {
                        let tx = conn.transaction()?;
                        for (id, size, mtime) in &touches {
                            store::touch(&tx, id, *size, *mtime)?;
                        }
                        tx.commit()?;
                        Ok(())
                    })
                    .await?;
            }
            self.commit(&vault, commit).await?;
            if full {
                self.set_status(|s| s.done = progress as u64);
            }
        }

        // A displaced note gives up its path now, so the file there can be
        // recorded as the note it is.
        if !displaced.is_empty() {
            let (ids, now) = (displaced.iter().cloned().collect::<Vec<_>>(), now_rfc3339());
            self.stores
                .core
                .call(move |conn| {
                    let tx = conn.transaction()?;
                    for id in &ids {
                        store::mark_missing(&tx, id, &now, false)?;
                    }
                    tx.commit()?;
                    Ok(())
                })
                .await?;
        }

        // A file Brainiac is creating right now is recorded by that write.
        let creating: HashSet<String> = self.in_flight.lock().expect("in flight").clone();

        // New paths: a note that moved keeps its identity.
        let vault_id = vault.id.clone();
        let has_tombstones: bool = self
            .stores
            .core
            .call(move |conn| {
                Ok(conn.query_row(
                    "SELECT EXISTS (SELECT 1 FROM notes WHERE vault_id = ?1 AND missing_at IS NOT NULL
                       AND trashed_at IS NULL)",
                    [vault_id],
                    |r| r.get(0),
                )?)
            })
            .await?;
        let burst: Vec<Gone> = self.watch.burst.lock().expect("burst").clone();
        let matching = !gone.is_empty() || !burst.is_empty() || has_tombstones;
        let mut claimed: HashSet<String> = HashSet::new();
        let mut added: Vec<(String, String, String)> = Vec::new();
        if matching {
            // Read every new file first: identity needs all of them at once.
            let root = vault.root.clone();
            let reads: Vec<(Found, ReadNote)> = blocking(move || {
                Ok(new_files
                    .into_iter()
                    .filter_map(|f| {
                        let bytes = std::fs::read(root.join(&f.relative_path)).ok()?;
                        Some((f, interpret(bytes)))
                    })
                    .collect())
            })
            .await?;
            let reads: Vec<(Found, ReadNote)> = reads
                .into_iter()
                .filter(|(_, r)| {
                    let id = r
                        .text
                        .as_deref()
                        .and_then(|t| index::frontmatter(t).brainiac_id);
                    !id.is_some_and(|id| creating.contains(&id))
                })
                .collect();
            stats.read += reads.len();
            let embedded: Vec<Option<String>> = reads
                .iter()
                .map(|(f, r)| {
                    r.text
                        .as_deref()
                        .and_then(|t| index::frontmatter(t).brainiac_id)
                        .filter(|_| index::is_note_path(&f.relative_path))
                })
                .collect();
            let wanted: Vec<String> = embedded.iter().flatten().cloned().collect();
            let vault_id = vault.id.clone();
            let tombstones: Vec<NoteRow> = if wanted.is_empty() {
                Vec::new()
            } else {
                self.stores
                    .core
                    .call(move |conn| store::missing_by_embedded_id(conn, &vault_id, &wanted))
                    .await?
            };
            let (vault_id, paths) = (
                vault.id.clone(),
                reads
                    .iter()
                    .map(|(f, _)| f.relative_path.clone())
                    .collect::<Vec<_>>(),
            );
            let at_path: HashMap<String, NoteRow> = self
                .stores
                .core
                .call(move |conn| store::missing_at_paths(conn, &vault_id, &paths))
                .await?;
            let mut hash_count: HashMap<&str, usize> = HashMap::new();
            for (_, r) in &reads {
                *hash_count.entry(r.hash.as_str()).or_default() += 1;
            }
            let mut assigned: Vec<(String, Option<NoteRow>)> = Vec::with_capacity(reads.len());
            for (i, (_, read)) in reads.iter().enumerate() {
                // 1. brainiac_id, against any note that went missing.
                let by_id: Vec<&NoteRow> = match &embedded[i] {
                    Some(e) => {
                        let mut seen = HashSet::new();
                        gone.iter()
                            .chain(tombstones.iter())
                            .filter(|r| {
                                r.embedded_id.as_ref() == Some(e)
                                    && !claimed.contains(&r.id)
                                    && seen.insert(r.id.clone())
                            })
                            .collect()
                    }
                    None => Vec::new(),
                };
                if by_id.len() == 1 {
                    claimed.insert(by_id[0].id.clone());
                    assigned.push((by_id[0].id.clone(), Some(by_id[0].clone())));
                    continue;
                }
                // 2. The same path: a note that went missing and came back, as
                // after a `git checkout` away and back, unless it carries
                // another note's ID.
                let path = &reads[i].0.relative_path;
                if let Some(t) = at_path.get(path).filter(|t| {
                    !claimed.contains(&t.id)
                        && !displaced.contains(&t.id)
                        && (t.embedded_id.is_none() || t.embedded_id == embedded[i])
                }) {
                    claimed.insert(t.id.clone());
                    assigned.push((t.id.clone(), Some(t.clone())));
                    continue;
                }
                // 3. Identical content, against exactly one note that went
                // missing in this scan or burst; identical content alone
                // proves nothing when several files or notes share it.
                let by_content: Vec<(String, Option<NoteRow>)> = gone
                    .iter()
                    .filter(|r| r.content_hash == read.hash && !displaced.contains(&r.id))
                    .map(|r| (r.id.clone(), Some(r.clone())))
                    .chain(
                        burst
                            .iter()
                            .filter(|g| g.hash == read.hash)
                            .map(|g| (g.id.clone(), None)),
                    )
                    .filter(|(id, _)| !claimed.contains(id))
                    .collect();
                if by_content.len() == 1 && hash_count[read.hash.as_str()] == 1 {
                    claimed.insert(by_content[0].0.clone());
                    assigned.push(by_content[0].clone());
                    continue;
                }
                assigned.push((uuid::Uuid::new_v4().to_string(), None));
            }
            let mut jobs = Vec::with_capacity(reads.len());
            for ((found, read), (note_id, previous)) in reads.into_iter().zip(assigned) {
                if claimed.contains(&note_id) {
                    stats.moved += 1;
                    events.push((
                        note_id.clone(),
                        read.hash.clone(),
                        found.relative_path.clone(),
                    ));
                } else {
                    stats.added += 1;
                    added.push((
                        note_id.clone(),
                        read.hash.clone(),
                        found.relative_path.clone(),
                    ));
                }
                jobs.push(Job {
                    note_id,
                    found,
                    read,
                    previous,
                });
            }
            for chunk in batches(jobs, |j| j.found.size) {
                progress += chunk.len();
                self.commit(&vault, chunk).await?;
                if full {
                    self.set_status(|s| s.done = progress as u64);
                }
            }
        } else {
            // Nothing could match: stream the new files in batches.
            for chunk in batches(new_files, |f| f.size) {
                if self.watch.generation.load(Ordering::SeqCst) != generation {
                    return Ok(stats);
                }
                let root = vault.root.clone();
                let jobs: Vec<Job> = blocking(move || {
                    Ok(chunk
                        .into_iter()
                        .filter_map(|found| {
                            let bytes = std::fs::read(root.join(&found.relative_path)).ok()?;
                            Some(Job {
                                note_id: uuid::Uuid::new_v4().to_string(),
                                read: interpret(bytes),
                                previous: None,
                                found,
                            })
                        })
                        .collect())
                })
                .await?;
                let jobs: Vec<Job> = jobs
                    .into_iter()
                    .filter(|j| {
                        let id = j
                            .read
                            .text
                            .as_deref()
                            .and_then(|t| index::frontmatter(t).brainiac_id);
                        !id.is_some_and(|id| creating.contains(&id))
                    })
                    .collect();
                stats.read += jobs.len();
                stats.added += jobs.len();
                progress += jobs.len();
                for j in &jobs {
                    added.push((
                        j.note_id.clone(),
                        j.read.hash.clone(),
                        j.found.relative_path.clone(),
                    ));
                }
                self.commit(&vault, jobs).await?;
                if full {
                    self.set_status(|s| s.done = progress as u64);
                }
            }
        }

        // Notes whose files are gone become tombstones and leave search.
        let unclaimed: Vec<NoteRow> = gone
            .into_iter()
            .filter(|r| !claimed.contains(&r.id))
            .collect();
        stats.missing = unclaimed.len();
        // Matched notes from the burst list are found again.
        if !claimed.is_empty() {
            self.watch
                .burst
                .lock()
                .expect("burst")
                .retain(|g| !claimed.contains(&g.id));
        }
        if !unclaimed.is_empty() {
            let ids: Vec<String> = unclaimed.iter().map(|r| r.id.clone()).collect();
            let (ids2, now) = (ids.clone(), now_rfc3339());
            self.stores
                .core
                .call(move |conn| {
                    let tx = conn.transaction()?;
                    for id in &ids2 {
                        store::mark_missing(&tx, id, &now, false)?;
                    }
                    tx.commit()?;
                    Ok(())
                })
                .await?;
            // Their last text is kept, so a note deleted outside Brainiac can be restored.
            let ids2 = ids.clone();
            let mut texts = self
                .stores
                .index
                .call(move |conn| {
                    let texts = index::bodies(conn, &ids2)?;
                    index::remove_docs(conn, &ids2)?;
                    Ok(texts)
                })
                .await?;
            if hold_missing {
                self.watch
                    .burst
                    .lock()
                    .expect("burst")
                    .extend(unclaimed.iter().map(|r| Gone {
                        id: r.id.clone(),
                        hash: r.content_hash.clone(),
                        body: texts.remove(&r.id).map(|(_, body)| body),
                    }));
            } else {
                let revisions = texts
                    .into_iter()
                    .filter(|(_, (_, body))| !body.is_empty())
                    .map(|(note_id, (hash, content))| NewRevision {
                        note_id,
                        content,
                        content_hash: hash,
                        reason: RevisionReason::ExternalChange,
                    })
                    .collect();
                self.keep_revisions(revisions).await?;
            }
        }
        if stats.added + stats.moved + stats.missing > 0 || reindexed || !displaced.is_empty() {
            self.resolve_links().await?;
        }
        if full && stats.changed + stats.added > 1000 {
            let _ = self
                .stores
                .index
                .call(|conn| index::reclaim_space(conn))
                .await;
        }

        for (id, hash, path) in events {
            self.emit_changed(&id, Some(hash), &path, NoteChangeOrigin::External);
        }
        if added.len() <= EVENT_LIMIT {
            for (id, hash, path) in added {
                self.emit_changed(&id, Some(hash), &path, NoteChangeOrigin::External);
            }
        }
        for r in &unclaimed {
            self.emit_changed(&r.id, None, &r.relative_path, NoteChangeOrigin::External);
            if !hold_missing {
                self.emit(KnowledgeEvent::NoteMissing(NoteMissingEvent {
                    note_id: r.id.clone(),
                }));
            }
        }
        if full || self.index_status().state != IndexState::Ready {
            self.set_status(|s| {
                s.state = IndexState::Ready;
                s.done = s.total;
                s.message = None;
            });
        }
        if stats.read + stats.missing > 0 {
            tracing::debug!(?stats, full, "reconciled the vault");
        }
        Ok(stats)
    }

    /// Commit one batch of files: revisions of what outside changes replaced,
    /// then the notes in `brainiac.db`, then their index entries, each in
    /// its own transaction. An interrupted index update is found by its
    /// content hash on the next full scan and redone.
    async fn commit(&self, vault: &ActiveVault, jobs: Vec<Job>) -> AppResult<()> {
        if jobs.is_empty() {
            return Ok(());
        }
        // The previously indexed text of notes changed outside Brainiac.
        let replaced: Vec<String> = jobs
            .iter()
            .filter(|j| {
                j.previous
                    .as_ref()
                    .is_some_and(|p| p.content_hash != j.read.hash)
            })
            .map(|j| j.note_id.clone())
            .collect();
        if !replaced.is_empty() {
            let bodies = self
                .stores
                .index
                .call(move |conn| index::bodies(conn, &replaced))
                .await?;
            let revisions: Vec<NewRevision> = bodies
                .into_iter()
                .filter(|(_, (_, body))| !body.is_empty())
                .map(|(note_id, (hash, content))| NewRevision {
                    note_id,
                    content,
                    content_hash: hash,
                    reason: RevisionReason::ExternalChange,
                })
                .collect();
            if !revisions.is_empty() {
                let now = now_rfc3339();
                self.stores
                    .history
                    .call(move |conn| history::add_revisions(conn, &revisions, &now))
                    .await?;
            }
        }

        let mut rows: Vec<(String, bool, FileState)> = Vec::with_capacity(jobs.len());
        let mut docs: Vec<IndexDoc> = Vec::with_capacity(jobs.len());
        for job in jobs {
            let (state, doc) = describe(
                &job.note_id,
                &job.found.relative_path,
                &job.read,
                job.found.size as i64,
                job.found.mtime,
            );
            rows.push((job.note_id, job.previous.is_some(), state));
            // A note that is not text is found by name only.
            let doc = if job.read.text_state == NoteTextState::Text {
                doc
            } else {
                IndexDoc {
                    body: String::new(),
                    links: Vec::new(),
                    ..doc
                }
            };
            docs.push(doc);
        }
        let (vault_id, now) = (vault.id.clone(), now_rfc3339());
        // One row that cannot be recorded (such as a path another note holds
        // after a race with a rename) is left for the next scan; it does not
        // stop the rest of the batch.
        let failed: HashSet<String> = self
            .stores
            .core
            .call(move |conn| {
                let tx = conn.transaction()?;
                let mut failed = HashSet::new();
                for (id, existed, state) in &rows {
                    let result = if *existed || store::get_note(&tx, id)?.is_some() {
                        store::set_file(&tx, id, state)
                    } else {
                        store::insert_note(&tx, id, &vault_id, state, &now)
                    };
                    if let Err(e) = result {
                        tracing::warn!(error = %e, details = ?e.details, path = %state.relative_path, "could not record a note");
                        failed.insert(id.clone());
                    }
                }
                tx.commit()?;
                Ok(failed)
            })
            .await?;
        let docs: Vec<IndexDoc> = docs
            .into_iter()
            .filter(|d| !failed.contains(&d.note_id))
            .collect();
        self.index_docs(docs).await
    }
}

/// Split work into batches of at most `BATCH_NOTES` items or `BATCH_BYTES`;
/// a larger item goes alone.
fn batches<T>(items: Vec<T>, size: impl Fn(&T) -> u64) -> Vec<Vec<T>> {
    let mut out: Vec<Vec<T>> = Vec::new();
    let mut current: Vec<T> = Vec::new();
    let mut bytes = 0u64;
    for item in items {
        let s = size(&item);
        if !current.is_empty() && (current.len() >= BATCH_NOTES || bytes + s > BATCH_BYTES as u64) {
            out.push(std::mem::take(&mut current));
            bytes = 0;
        }
        bytes += s;
        current.push(item);
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// The Markdown files in scope, and the known paths in scope that no longer exist.
fn survey(root: &Path, scope: &Scope, known: &[String]) -> AppResult<(Vec<Found>, Vec<String>)> {
    let mut listings = Listings::default();
    let mut found: Vec<Found> = Vec::new();
    let mut gone: Vec<String> = Vec::new();
    let mut unreadable: Vec<String> = Vec::new();
    match scope {
        Scope::Full => {
            walk(root, "", &mut found, &mut unreadable)?;
            let present: HashSet<&str> = found.iter().map(|f| f.relative_path.as_str()).collect();
            gone = known
                .iter()
                .filter(|k| !present.contains(k.as_str()))
                .cloned()
                .collect();
        }
        Scope::Paths(paths) => {
            let mut seen: HashSet<String> = HashSet::new();
            for rel in paths {
                if is_hidden(rel) || crate::notes::validate_relative(rel).is_err() {
                    continue;
                }
                if listings.exists(root, rel) {
                    let abs = root.join(rel);
                    match std::fs::symlink_metadata(&abs) {
                        Ok(m) if m.is_dir() => {
                            // A folder that appeared or was renamed: everything in it.
                            if let Err(e) = walk(root, rel, &mut found, &mut unreadable) {
                                if e.kind() != std::io::ErrorKind::NotFound {
                                    unreadable.push(rel.clone());
                                }
                            }
                        }
                        Ok(m) if m.is_file() && index::is_note_path(rel) => found.push(Found {
                            relative_path: rel.clone(),
                            size: m.len(),
                            mtime: crate::notes::mtime_ns(&m),
                        }),
                        _ => {}
                    }
                }
                // A vanished path stands for every note indexed under it,
                // because a renamed folder is reported only as itself.
                let prefix = format!("{rel}/");
                for k in known.iter().filter(|k| *k == rel || k.starts_with(&prefix)) {
                    if seen.contains(k) {
                        continue;
                    }
                    if !listings.exists(root, k) {
                        gone.push(k.clone());
                        seen.insert(k.clone());
                    }
                }
            }
            let mut dedupe: HashSet<String> = HashSet::new();
            found.retain(|f| dedupe.insert(f.relative_path.clone()));
            gone.sort();
            gone.dedup();
        }
    }
    // Notes in a folder that cannot be read (permissions) are not deleted.
    gone.retain(|p| !unreadable.iter().any(|u| p.starts_with(&format!("{u}/"))));
    Ok((found, gone))
}
