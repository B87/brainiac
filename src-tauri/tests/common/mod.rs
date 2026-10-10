//! A temporary vault and Brainiac data folder for the notes, tasks, and
//! backup tests. Each test binary uses some of these helpers, not all.
#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use brainiac_lib::db::{self, Db, RepositoryRow};
use brainiac_lib::models::{IndexState, SearchKind, SearchRequest};
use brainiac_lib::notes::{KnowledgeEvent, NoteService, Stores};
use brainiac_lib::tasks::TaskService;
use brainiac_lib::vault::Scope;

pub struct Harness {
    pub tmp: tempfile::TempDir,
    pub vault: PathBuf,
    pub data: PathBuf,
    pub core: Db,
    pub notes: Arc<NoteService>,
    pub tasks: TaskService,
    pub events: Arc<Mutex<Vec<KnowledgeEvent>>>,
}

impl Harness {
    /// A vault with Brainiac's data next to it. With `watch`, the vault
    /// watcher runs; otherwise only explicit reconciliation sees changes.
    pub async fn new(watch: bool) -> Harness {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path().join("vault");
        fs::create_dir_all(&vault).unwrap();
        Self::with_vault(tmp, vault, watch).await
    }

    /// Like `new`, opening the vault through `vault_path`, which may be a
    /// symbolic link to the real folder.
    pub async fn with_vault(tmp: tempfile::TempDir, vault_path: PathBuf, watch: bool) -> Harness {
        let data = tmp.path().join("data");
        let core = Db::open(&data.join(db::CORE_FILE)).unwrap();
        let (notes, events) = Self::service(&data, core.clone());
        if !watch {
            notes.disable_watching();
        }
        notes
            .select_vault(vault_path.to_str().unwrap(), false)
            .await
            .unwrap();
        let vault = notes.vault().unwrap().root;
        let harness = Harness {
            tasks: TaskService::new(Arc::clone(&notes)),
            tmp,
            vault,
            data,
            core,
            notes,
            events,
        };
        harness.wait_ready().await;
        harness
    }

    pub fn service(data: &Path, core: Db) -> (Arc<NoteService>, Arc<Mutex<Vec<KnowledgeEvent>>>) {
        let events: Arc<Mutex<Vec<KnowledgeEvent>>> = Arc::default();
        let sink = Arc::clone(&events);
        let stores = Stores::open(data, core).unwrap();
        let notes = NoteService::new(
            stores,
            data.to_path_buf(),
            Arc::new(move |e| sink.lock().unwrap().push(e.clone())),
        );
        (notes, events)
    }

    pub async fn wait_ready(&self) {
        wait_for("the first scan", || {
            let s = self.notes.index_status();
            async move { s.state == IndexState::Ready }
        })
        .await;
    }

    /// Reconcile everything now.
    pub async fn scan(&self) {
        self.notes.reconcile(Scope::Full, false).await.unwrap();
    }

    pub fn write(&self, rel: &str, text: &str) {
        let path = self.vault.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    pub fn read(&self, rel: &str) -> String {
        fs::read_to_string(self.vault.join(rel)).unwrap()
    }

    /// Hidden or temporary files Brainiac might have left in the vault.
    pub fn leftover_files(&self) -> Vec<String> {
        fn walk(dir: &Path, out: &mut Vec<String>) {
            for e in fs::read_dir(dir).unwrap().flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if e.file_type().unwrap().is_dir() {
                    walk(&e.path(), out);
                } else if name.starts_with('.') || name.contains("brainiac-") {
                    out.push(name);
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.vault, &mut out);
        out
    }

    pub fn saw(&self, pred: impl Fn(&KnowledgeEvent) -> bool) -> bool {
        self.events.lock().unwrap().iter().any(pred)
    }

    /// Paths of the notes a keyword search finds, best first.
    pub async fn search_notes(&self, query: &str) -> Vec<String> {
        let results = brainiac_lib::index::search(
            &self.notes,
            SearchRequest {
                query: query.into(),
                kinds: Some(vec![SearchKind::Note]),
                limit: Some(50),
            },
        )
        .await
        .unwrap();
        results.notes.hits.into_iter().map(|h| h.detail).collect()
    }

    /// Register a folder as a repository directly in the database; the notes
    /// service needs only its row.
    pub async fn register_repository(&self, name: &str) -> String {
        let root = self.tmp.path().join("repos").join(name);
        fs::create_dir_all(&root).unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let row = RepositoryRow {
            id: id.clone(),
            canonical_root: root.display().to_string(),
            display_path: root.display().to_string(),
            git_dir: root.join(".git").display().to_string(),
            common_git_dir: root.join(".git").display().to_string(),
            created_at: "2026-10-01T00:00:00.000Z".into(),
            last_opened_at: None,
            last_checked_at: None,
            last_tab: None,
            status: None,
            error: None,
            last_fetch_at: None,
            last_fetch_error: None,
            remote_url: Some(format!("git@example.com:team/{name}.git")),
            forge_override: None,
        };
        self.core
            .call(move |conn| {
                db::insert_repository(conn, &row)?;
                db::set_remote_url(conn, &row.id, row.remote_url.as_deref())
            })
            .await
            .unwrap();
        id
    }
}

/// The ID of the live note at a vault path.
pub async fn note_id_at(h: &Harness, rel: &str) -> String {
    let rel2 = rel.to_string();
    let vault = h.notes.vault().unwrap().id;
    h.core
        .call(move |conn| {
            Ok(conn.query_row(
                "SELECT id FROM notes WHERE vault_id = ?1 AND relative_path = ?2 AND missing_at IS NULL",
                [&vault, &rel2],
                |r| r.get::<_, String>(0),
            )?)
        })
        .await
        .unwrap_or_else(|e| panic!("no live note at {rel}: {e}"))
}

/// Poll `check` until it holds, failing after ten seconds.
pub async fn wait_for<F, Fut>(what: &str, check: F)
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let started = Instant::now();
    while !check().await {
        if started.elapsed() > Duration::from_secs(10) {
            panic!("timed out waiting for {what}");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
