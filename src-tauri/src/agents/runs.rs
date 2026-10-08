//! `AgentRunService` (SPEC.md, section 13; docs/architecture.md, Agent runs
//! — v0.5): runs as the app sees them. It starts a run (checks, the copy of
//! the start, the credential read on the Mac, the controller's acceptance),
//! follows it through the run controller, mirrors its journal to
//! `agent-runs/<run-id>/trace.jsonl`, keeps each run's row in `history.db`
//! as the controller's last confirmed projection, and drives the user's
//! actions: prompts, permissions, Cancel, Finish and collect, collection,
//! review, Delete, cleanup, and retention.
//!
//! The controller's records are authoritative for what a run is doing; the
//! row is what Brainiac last confirmed. A run is never stopped because the
//! app disconnected, and a Cancel asked for while the controller could not
//! be reached is sent when it can be.

mod store;

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::de::DeserializeOwned;

use super::controller::protocol::{
    Credential, EventBody, RunStatus, StartRun, StopReason, MAX_PROMPT_BYTES,
};
use super::runtime::StartError;
use super::settings::{self, AgentSettingsService};
use super::{RunArtifacts, RunRuntime};
use crate::credentials::{CredentialService, Resolution};
use crate::db::Db;
use crate::models::{
    now_rfc3339, AgentHost, AgentKind, AgentProfile, AgentRun, AgentRunChangedEvent, AgentRunList,
    AgentSettings, AgentTestResult, AgentTestStep, AppError, AppResult, DiffResult, ErrorCode,
    LeftOutFile, RunActivity, RunBranchCommand, RunChanges, RunCollection, RunControllerStatus,
    RunDiffRequest, RunEvent, RunEventPage, RunOutcome, RunPermissionRequest, RunPermissions,
    RunPhase, RunPreview, RunStartStep, StartRunRequest,
};
use crate::workspaces::RepositoryService;

pub use store::RunRow;

/// How long one poll for events waits at the controller.
const POLL_WAIT: Duration = Duration::from_secs(25);
/// How long to wait before trying the controller again after it failed.
const RETRY_AFTER: Duration = Duration::from_secs(5);
/// How long a collection waits for the one before it to finish its cleanup.
const COLLECT_HANDOVER: Duration = Duration::from_secs(30);
/// Events the window gets per page.
const PAGE: usize = 500;
/// A title is the prompt's first line, this long.
const TITLE_CHARS: usize = 80;
/// Ended runs are removed after this long, unless work waits for a decision.
pub const RETENTION: chrono::Duration = chrono::Duration::days(30);
/// The test's run: a short time limit, and a prompt that needs no tool.
const TEST_TIME_LIMIT_MINUTES: u32 = 30;
const TEST_PROMPT: &str = "Reply with the single word ready and nothing else.";

pub type RunEmitter = Arc<dyn Fn(AgentRunChangedEvent) + Send + Sync>;

/// A repository's name and root, answered later.
// A boxed future: a trait used through `dyn` cannot have `async fn`, so the
// method returns its future boxed, with `Send` so it runs on any thread.
pub type Located<'a> =
    std::pin::Pin<Box<dyn std::future::Future<Output = AppResult<(String, PathBuf)>> + Send + 'a>>;

/// Where a run's repository is, and what it is called. The service asks
/// for a repository only through this, so tests can stand in for the
/// workspace service.
pub trait RepositoryLookup: Send + Sync {
    fn locate(&self, repository_id: &str) -> Located<'_>;
}

impl RepositoryLookup for RepositoryService {
    fn locate(&self, repository_id: &str) -> Located<'_> {
        let id = repository_id.to_string();
        Box::pin(async move {
            let summary = self.summary(&id).await?;
            Ok((summary.name, PathBuf::from(summary.canonical_root)))
        })
    }
}

/// The last Changes so far of a run, in `agent-runs/<run-id>/preview.json`:
/// derived, and gone with the run's folder.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct PreviewRecord {
    commit: Option<String>,
    taken_at: Option<String>,
    turn: u32,
    left_out: u32,
    error: Option<String>,
    /// Read once when it was taken: the window asks for the preview each
    /// time the run changes, which is every page while the agent streams.
    #[serde(default)]
    files: Vec<crate::models::CommitFile>,
}

/// Everything about one run held in memory: its mirrored journal.
struct Journal {
    events: Vec<RunEvent>,
    /// Loaded from `trace.jsonl` already.
    loaded: bool,
}

/// A run being handed to its controller (`AgentRunService::launch`).
struct Starting {
    step: RunStartStep,
    /// Cancel was asked for meanwhile.
    cancelled: bool,
}

pub struct AgentRunService {
    history: Db,
    settings: Arc<AgentSettingsService>,
    // `Arc` because accounts, connections, and agents share one credentials layer.
    credentials: Arc<CredentialService>,
    artifacts: Arc<RunArtifacts>,
    runtime: Arc<RunRuntime>,
    /// Remote hosts, when the app has them. Tests leave this empty.
    hosts: Mutex<Option<Arc<super::hosts::AgentHostService>>>,
    /// Whether each remote host answered last. This Mac uses `connected`.
    links: Mutex<HashMap<String, bool>>,
    /// A test run's controller, when it is not this Mac's.
    test_runtimes: Mutex<HashMap<String, Arc<RunRuntime>>>,
    repositories: Arc<dyn RepositoryLookup>,
    emitter: RunEmitter,
    /// One sync task per live run, by run ID.
    syncing: Mutex<HashMap<String, tokio::task::JoinHandle<()>>>,
    journals: Mutex<HashMap<String, Arc<Mutex<Journal>>>>,
    /// One collection at a time per run.
    collecting: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// One preview (Changes so far) at a time per run.
    previewing: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// Runs whose turn ended while a preview was being taken: one more
    /// follows it, so the last turn is always read.
    preview_again: Mutex<std::collections::HashSet<String>>,
    /// Runs Start run answered for and still hands to their controller.
    starting: Mutex<HashMap<String, Starting>>,
    /// The last controller call answered.
    connected: std::sync::atomic::AtomicBool,
}

impl AgentRunService {
    pub fn new(
        history: Db,
        settings: Arc<AgentSettingsService>,
        credentials: Arc<CredentialService>,
        artifacts: Arc<RunArtifacts>,
        runtime: Arc<RunRuntime>,
        repositories: Arc<dyn RepositoryLookup>,
        emitter: RunEmitter,
    ) -> Arc<Self> {
        Arc::new(AgentRunService {
            history,
            settings,
            credentials,
            artifacts,
            runtime,
            hosts: Mutex::new(None),
            links: Mutex::new(HashMap::new()),
            test_runtimes: Mutex::new(HashMap::new()),
            repositories,
            emitter,
            syncing: Mutex::new(HashMap::new()),
            journals: Mutex::new(HashMap::new()),
            collecting: Mutex::new(HashMap::new()),
            previewing: Mutex::new(HashMap::new()),
            preview_again: Mutex::new(std::collections::HashSet::new()),
            starting: Mutex::new(HashMap::new()),
            connected: std::sync::atomic::AtomicBool::new(false),
        })
    }

    fn emit(&self, run_id: &str, deleted: bool) {
        (self.emitter)(AgentRunChangedEvent {
            run_id: run_id.to_string(),
            deleted,
        });
    }

    // -----------------------------------------------------------------------
    // Reading
    // -----------------------------------------------------------------------

    pub async fn list(&self) -> AppResult<AgentRunList> {
        // Steps first: a start's answer is in its row before its step goes.
        let steps = self.starting_steps();
        let runs = self.history.call(store::list).await?;
        let connected = self.connected.load(std::sync::atomic::Ordering::SeqCst);
        Ok(AgentRunList {
            runs: runs
                .into_iter()
                .map(|r| {
                    let step = steps.get(&r.id).copied();
                    self.present(r, step)
                })
                .collect(),
            // The local controller. A remote host that is down does not
            // change this.
            controller_running: connected,
        })
    }

    pub async fn get(&self, run_id: &str) -> AppResult<AgentRun> {
        let step = self.starting_steps().get(run_id).copied();
        let row = self.row(run_id).await?;
        Ok(self.present(row, step))
    }

    fn link_up(&self, host_id: &str) -> bool {
        if host_id.is_empty() || host_id == super::hosts::LOCAL_ID {
            self.connected.load(std::sync::atomic::Ordering::SeqCst)
        } else {
            self.links
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .get(host_id)
                .copied()
                .unwrap_or(false)
        }
    }

    fn set_link(&self, host_id: &str, up: bool) {
        if host_id.is_empty() || host_id == super::hosts::LOCAL_ID {
            self.connected
                .store(up, std::sync::atomic::Ordering::SeqCst);
        } else {
            self.links
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .insert(host_id.to_string(), up);
        }
    }

    async fn runtime_for(&self, host_id: &str) -> AppResult<Arc<RunRuntime>> {
        if host_id.is_empty() || host_id == super::hosts::LOCAL_ID {
            return Ok(Arc::clone(&self.runtime));
        }
        let hosts = self
            .hosts
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
            .ok_or_else(|| AppError::dependency("Remote hosts are not available."))?;
        hosts.runtime(host_id).await
    }

    /// The controller for this run. A run with no row (a local test) uses
    /// this Mac.
    async fn rt(&self, run_id: &str) -> AppResult<Arc<RunRuntime>> {
        if let Some(runtime) = self
            .test_runtimes
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(run_id)
            .cloned()
        {
            return Ok(runtime);
        }
        let host = match self.row(run_id).await {
            Ok(row) => row.host_id,
            Err(_) => super::hosts::LOCAL_ID.to_string(),
        };
        self.runtime_for(&host).await
    }

    fn refuse_if_upgrading(&self, host_id: &str) -> AppResult<()> {
        let upgrading = self
            .hosts
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
            .is_some_and(|hosts| hosts.is_deploying(host_id));
        if upgrading {
            Err(upgrading_conflict())
        } else {
            Ok(())
        }
    }

    async fn row(&self, run_id: &str) -> AppResult<RunRow> {
        let id = run_id.to_string();
        self.history
            .call(move |conn| store::get(conn, &id))
            .await?
            .ok_or_else(|| AppError::not_found("There is no such run."))
    }

    /// The conversation after `after`, from the mirrored journal.
    pub async fn events(&self, run_id: &str, after: u64) -> AppResult<RunEventPage> {
        let row = self.row(run_id).await?;
        let journal = self.journal(run_id).await?;
        let journal = journal.lock().unwrap_or_else(|p| p.into_inner());
        let events: Vec<RunEvent> = journal
            .events
            .iter()
            .filter(|e| e.seq > after)
            .take(PAGE)
            .cloned()
            .collect();
        Ok(RunEventPage {
            run_id: run_id.to_string(),
            events,
            cursor: row.cursor,
        })
    }

    /// The run's journal in memory, read from its mirror the first time.
    async fn journal(&self, run_id: &str) -> AppResult<Arc<Mutex<Journal>>> {
        let journal = {
            let mut journals = self.journals.lock().unwrap_or_else(|p| p.into_inner());
            Arc::clone(journals.entry(run_id.to_string()).or_insert_with(|| {
                Arc::new(Mutex::new(Journal {
                    events: Vec::new(),
                    loaded: false,
                }))
            }))
        };
        let path = self.trace_path(run_id);
        let load = {
            let journal = journal.lock().unwrap_or_else(|p| p.into_inner());
            !journal.loaded
        };
        if load {
            let text = tokio::task::spawn_blocking(move || std::fs::read_to_string(&path))
                .await
                .map_err(|e| AppError::io(e.to_string()))?;
            let mut loaded = Vec::new();
            if let Ok(text) = text {
                for line in text.lines() {
                    if let Ok(event) = serde_json::from_str::<RunEvent>(line) {
                        loaded.push(event);
                    }
                }
            }
            let mut journal = journal.lock().unwrap_or_else(|p| p.into_inner());
            if !journal.loaded {
                // Events mirrored while the file was read come after it.
                let first_new = journal.events.first().map(|e| e.seq);
                let mut events = loaded;
                if let Some(first) = first_new {
                    events.retain(|e| e.seq < first);
                }
                events.append(&mut journal.events);
                journal.events = events;
                journal.loaded = true;
            }
        }
        Ok(journal)
    }

    fn trace_path(&self, run_id: &str) -> PathBuf {
        self.artifacts.run_dir(run_id).join("trace.jsonl")
    }

    /// Settings → Agents: whether the run controller is running now.
    pub async fn controller_status(&self) -> RunControllerStatus {
        match self.runtime.running().await {
            Some(info) => {
                let live = self
                    .history
                    .call(store::list)
                    .await
                    .map(|rows| rows.iter().filter(|r| r.phase != RunPhase::Ended).count())
                    .unwrap_or(0);
                RunControllerStatus {
                    running: true,
                    pid: Some(info.pid),
                    live_runs: live as u32,
                }
            }
            None => RunControllerStatus {
                running: false,
                pid: None,
                live_runs: 0,
            },
        }
    }

    // -----------------------------------------------------------------------
    // Starting
    // -----------------------------------------------------------------------

    /// Attach the host service. The local controller is already set.
    pub fn set_hosts(&self, hosts: Arc<super::hosts::AgentHostService>) {
        *self.hosts.lock().unwrap_or_else(|p| p.into_inner()) = Some(hosts);
    }

    /// New run, **Start run**: refuse what Settings still lacks, copy the
    /// start, read the credential, hand the run to the controller.
    pub async fn start(self: &Arc<Self>, request: StartRunRequest) -> AppResult<AgentRun> {
        let settings = self.settings.get().await?;
        let host_id = if request.host_id.is_empty() {
            super::hosts::LOCAL_ID.to_string()
        } else {
            request.host_id.clone()
        };
        let remote = host_id != super::hosts::LOCAL_ID;
        let profile = settings
            .profiles
            .iter()
            .find(|p| p.id == request.profile_id)
            .cloned()
            .ok_or_else(|| AppError::validation("Choose an agent for the run."))?;
        let host = settings
            .hosts
            .iter()
            .find(|h| h.id == host_id)
            .cloned()
            .ok_or_else(|| AppError::validation("Choose a host for the run."))?;
        if let Some(first) = settings::run_missing(&profile, &host).first() {
            return Err(AppError::validation(format!(
                "Settings → Agents is not ready: {first}"
            )));
        }
        let prompt = request.prompt.trim().to_string();
        if prompt.is_empty() {
            return Err(AppError::validation("Write a prompt for the agent."));
        }
        if prompt.len() > MAX_PROMPT_BYTES {
            return Err(AppError::validation("The prompt is over 100 KB."));
        }
        let time_limit = settings::in_range(
            "The time limit",
            request.time_limit_minutes,
            settings::TIME_LIMIT_MINUTES,
        )?;
        let cpus = settings::in_range("CPUs", request.cpus, settings::CPUS)?;
        let memory = settings::in_range("Memory in MiB", request.memory_mib, settings::MEMORY_MIB)?;
        let workspace = settings::in_range(
            "The workspace in GiB",
            request.workspace_gib,
            settings::WORKSPACE_GIB,
        )?;
        let model = settings::check_model(profile.agent, &request.model)?;
        if model.is_empty() && profile.agent == AgentKind::Opencode {
            return Err(AppError::validation("Choose the model for the run."));
        }
        let (name, root) = self.repositories.locate(&request.repository_id).await?;
        let preview = self
            .artifacts
            .preview(&request.repository_id, &root, &request.start_commit)
            .await?;
        if preview.commit != request.start_commit {
            return Err(AppError::validation(
                "Start from a full commit ID, as the preview showed it.",
            ));
        }
        let run_id = uuid::Uuid::new_v4().to_string();
        // `run_missing` above promised the image, and on this Mac the engine.
        let image = host
            .image
            .clone()
            .ok_or_else(|| AppError::validation("Build the image first."))?;
        let (socket, engine_name) = if remote {
            self.refuse_if_upgrading(&host_id)?;
            (
                "/var/run/docker.sock".to_string(),
                host.engine_name.clone().unwrap_or_else(|| "Docker".into()),
            )
        } else {
            let socket = settings
                .engine_socket
                .clone()
                .ok_or_else(|| AppError::validation("Choose where runs execute first."))?;
            let name = super::engine::name_of(&socket);
            (socket, name)
        };
        let (image_name, image_id, host_name) = (image.name, image.id, host.name);
        // The credential is read on the Mac, after everything that could be
        // refused, and goes once to the controller.
        let credential = self.credential(&profile).await?;
        let title = title_of(&prompt, &credential.value);
        let row = RunRow::new(store::NewRun {
            id: run_id.clone(),
            repository_id: request.repository_id.clone(),
            repository_name: name,
            title,
            start_commit: preview.commit.clone(),
            start_subject: preview.subject.clone(),
            profile_id: profile.id.clone(),
            agent: profile.agent,
            provider: profile.provider,
            payment: profile.payment,
            credential_source: crate::credentials::describe(
                &profile.credential_source,
                &settings::credential_owner(&profile.id),
            ),
            host_id: host_id.clone(),
            host_name,
            engine_socket: socket.clone(),
            engine_name,
            image_name: image_name.clone(),
            image_id,
            permissions: request.permissions,
            time_limit_minutes: time_limit,
            cpus,
            memory_mib: memory,
            workspace_gib: workspace,
            model: model.clone(),
        });
        // The row is written before the controller is asked, so a start
        // whose answer is lost is still a run Brainiac knows. Its step is
        // set first, so the window never sees the run without one.
        self.starting
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(
                run_id.clone(),
                Starting {
                    step: RunStartStep::Copy,
                    cancelled: false,
                },
            );
        if let Err(e) = self
            .history
            .call({
                let row = row.clone();
                move |conn| store::insert(conn, &row)
            })
            .await
        {
            self.end_starting(&run_id);
            return Err(e);
        }
        self.emit(&run_id, false);
        let start = StartRun {
            run_id: run_id.clone(),
            attempt: 1,
            engine_socket: socket,
            image: image_name.clone(),
            cpus,
            memory_mib: memory,
            workspace_gib: workspace,
            time_limit_secs: u64::from(time_limit) * 60,
            permissions: request.permissions,
            start_commit: preview.commit,
            // Set once the start is copied, after this answers.
            bundle: PathBuf::new(),
            prompt_id: format!("{run_id}-prompt-1"),
            prompt,
            credential,
            agent: profile.agent,
            provider: profile.provider,
            model,
        };
        // Copying the start and reaching a remote host can take minutes, so
        // they go on after this answers: the window opens the run at once
        // and shows each step (SPEC.md, The run: Starting).
        let service = Arc::clone(self);
        let repository_id = request.repository_id.clone();
        tokio::spawn(async move {
            service.launch(start, host_id, repository_id, root).await;
        });
        self.get(&run_id).await
    }

    /// The rest of Start run: copy the start, hand the run to its
    /// controller, and record how that went. A refusal ends the run as
    /// failed with the reason; a Cancel that came first ends it as
    /// cancelled; one that came while the controller was asked is sent now.
    async fn launch(
        self: Arc<Self>,
        start: StartRun,
        host_id: String,
        repository_id: String,
        root: PathBuf,
    ) {
        let run_id = start.run_id.clone();
        let result = self.hand_over(start, &host_id, &repository_id, &root).await;
        let id = run_id.clone();
        // The controller's answer goes into the row before the step is
        // dropped, so the window never sees a started run without either.
        let applied = match &result {
            Ok(status) => self.apply_status(&run_id, status).await,
            Err(_) => Ok(()),
        };
        let cancelled = self.end_starting(&run_id);
        let recorded = match result {
            Ok(_) => {
                self.set_link(&host_id, true);
                let flagged = if cancelled {
                    self.history
                        .call(move |conn| store::set_cancel_requested(conn, &id, true))
                        .await
                } else {
                    Ok(())
                };
                self.spawn_sync(&run_id);
                applied.and(flagged)
            }
            Err(StartError::Refused(_)) if cancelled => {
                self.history
                    .call(move |conn| store::mark_cancelled_unstarted(conn, &id))
                    .await
            }
            // An answered error means nothing was made.
            Err(StartError::Refused(e)) => {
                tracing::warn!(run = %run_id, error = %e, details = ?e.details, "the run was not started");
                let message = e.message;
                self.history
                    .call(move |conn| store::mark_refused(conn, &id, &message))
                    .await
            }
            // An unanswered start (a timeout, the socket) may have been
            // accepted before the failure, so the run stays live and is
            // followed: the controller answers a start it never saw with
            // "unknown run", which ends it.
            Err(StartError::Unanswered(e)) => {
                tracing::warn!(run = %run_id, error = %e, details = ?e.details, "a start was not answered");
                self.set_link(&host_id, false);
                if cancelled {
                    let _ = self
                        .history
                        .call(move |conn| store::set_cancel_requested(conn, &id, true))
                        .await;
                }
                self.spawn_sync(&run_id);
                Ok(())
            }
        };
        if let Err(e) = recorded {
            tracing::warn!(run = %run_id, error = %e, "the run's start could not be recorded");
        }
        self.emit(&run_id, false);
    }

    /// Copy the start, reach the host, and ask its controller, saying each
    /// step as it begins.
    async fn hand_over(
        &self,
        mut start: StartRun,
        host_id: &str,
        repository_id: &str,
        root: &Path,
    ) -> Result<RunStatus, StartError> {
        let run_id = start.run_id.clone();
        let exported = self
            .artifacts
            .export(repository_id, root, &start.start_commit, &run_id)
            .await
            .map_err(StartError::Refused)?;
        start.bundle = exported.bundle;
        let remote = host_id != super::hosts::LOCAL_ID;
        if remote && !self.set_starting(&run_id, RunStartStep::Send) {
            return Err(StartError::Refused(super::runtime::cancelled_start()));
        }
        let runtime = self
            .runtime_for(host_id)
            .await
            .map_err(StartError::Refused)?;
        // An upgrade that began while the start was copied.
        if remote {
            self.refuse_if_upgrading(host_id)
                .map_err(StartError::Refused)?;
        }
        runtime
            .start_reporting(start, &|step| self.set_starting(&run_id, step))
            .await
    }

    /// Record the step a starting run is on, and emit it. False once Cancel
    /// was asked for: the start stops before that step.
    fn set_starting(&self, run_id: &str, step: RunStartStep) -> bool {
        let go = match self
            .starting
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get_mut(run_id)
        {
            Some(starting) => {
                starting.step = step;
                !starting.cancelled
            }
            None => false,
        };
        self.emit(run_id, false);
        go
    }

    /// The start is over, one way or another. True when Cancel was asked
    /// for while it went on.
    fn end_starting(&self, run_id: &str) -> bool {
        self.starting
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(run_id)
            .is_some_and(|s| s.cancelled)
    }

    /// Cancel a run still being handed to its controller. False when its
    /// start is over, and Cancel goes to the controller as usual.
    fn cancel_starting(&self, run_id: &str) -> bool {
        match self
            .starting
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get_mut(run_id)
        {
            Some(starting) => {
                starting.cancelled = true;
                true
            }
            None => false,
        }
    }

    /// The step of each start that goes on. Read before the rows: `launch`
    /// writes the controller's answer into the row before it drops the
    /// step, so a run is never seen with neither.
    fn starting_steps(&self) -> HashMap<String, RunStartStep> {
        self.starting
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .map(|(id, s)| (id.clone(), s.step))
            .collect()
    }

    /// The row as the window sees it, with the step of a start that goes on.
    fn present(&self, row: RunRow, step: Option<RunStartStep>) -> AgentRun {
        // A starting run is being talked to, not lost.
        let up = step.is_some() || self.link_up(&row.host_id);
        let mut run = row.into_run(up);
        run.starting = step;
        run
    }

    /// The current image's name, handed to the controller with a collection
    /// or a discard: a kept run whose own image was removed from the engine
    /// (a rebuild after an update, a prune) uses it for its helper and
    /// collector instead.
    async fn current_image(&self) -> Option<String> {
        self.settings
            .get()
            .await
            .ok()?
            .hosts
            .into_iter()
            .find(|h| h.id == super::hosts::LOCAL_ID)?
            .image
            .map(|i| i.name)
    }

    async fn current_image_for(&self, run_id: &str) -> Option<String> {
        let row = self.row(run_id).await.ok()?;
        if row.host_id.is_empty() || row.host_id == super::hosts::LOCAL_ID {
            return self.current_image().await;
        }
        self.settings
            .get()
            .await
            .ok()?
            .hosts
            .into_iter()
            .find_map(|h| {
                if h.id == row.host_id {
                    h.image.map(|i| i.name)
                } else {
                    None
                }
            })
    }

    /// The profile's token or key, read through the credentials layer.
    async fn credential(&self, profile: &AgentProfile) -> AppResult<Credential> {
        let binding = AgentSettingsService::binding(profile);
        let lease = match self.credentials.resolve(&binding).await? {
            Resolution::Secret(lease) => lease,
            Resolution::NoCredential | Resolution::InputRequired => {
                return Err(AppError::new(
                    ErrorCode::Unauthenticated,
                    "Settings → Agents has no token or key for runs.",
                ))
            }
        };
        let value = String::from_utf8(lease.bytes().expose().to_vec()).map_err(|_| {
            AppError::new(ErrorCode::Unauthenticated, "The token or key is not text.")
        })?;
        let value = settings::normalize_credential(profile.provider, profile.payment, &value)?;
        Ok(Credential {
            key: settings::credential_key(profile),
            value,
        })
    }

    // -----------------------------------------------------------------------
    // Following the controller
    // -----------------------------------------------------------------------

    /// At launch: reconnect to this installation's controller for every run
    /// that is not over, and finish what was left: collections, and cancels
    /// asked for while the controller could not be reached.
    pub async fn reconnect(self: &Arc<Self>) {
        let rows = match self.history.call(store::list).await {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!(error = %e, "the runs could not be listed");
                return;
            }
        };
        for row in &rows {
            // A collection Brainiac quit in the middle of is not resumed: it
            // failed, and Retry collection runs it again.
            if row.collection == RunCollection::Collecting {
                let _ = self
                    .set_collection(
                        &row.id,
                        RunCollection::Failed,
                        Some(
                            "Brainiac quit while the work was collected. Retry the collection."
                                .into(),
                        ),
                    )
                    .await;
            }
            if row.needs_sync() {
                self.spawn_sync(&row.id);
            }
        }
        // A Settings test Brainiac quit in the middle of has no row. Stop it
        // on whichever controller still has it: this Mac, and each installed
        // host. One host that does not answer does not block the others.
        let known: Vec<String> = rows.iter().map(|row| row.id.clone()).collect();
        self.sweep_test_runs(self.runtime.as_ref(), &known).await;
        let hosts = self.hosts.lock().unwrap_or_else(|p| p.into_inner()).clone();
        if let Some(hosts) = hosts {
            if let Ok(ids) = hosts.installed_ids().await {
                let mut tasks = Vec::new();
                for id in ids {
                    let hosts = Arc::clone(&hosts);
                    let service = Arc::clone(self);
                    let known = known.clone();
                    tasks.push(tokio::spawn(async move {
                        let Ok(runtime) = hosts.runtime(&id).await else {
                            return;
                        };
                        service.sweep_test_runs(runtime.as_ref(), &known).await;
                    }));
                }
                for task in tasks {
                    let _ = task.await;
                }
            }
        }
    }

    /// Stop and discard a Settings test the controller still has and Brainiac
    /// has no row for. Discard waits until the stop is confirmed, because a
    /// live container is refused.
    async fn sweep_test_runs(&self, runtime: &RunRuntime, known: &[String]) {
        use super::controller::protocol::Phase;
        let Ok(runs) = runtime.runs().await else {
            return;
        };
        for run in runs {
            let orphan =
                run.run_id.starts_with("test-") && !known.iter().any(|id| id == &run.run_id);
            if !orphan {
                continue;
            }
            if run.phase != Phase::Ended || !run.stop_confirmed {
                let _ = runtime.stop(&run.run_id, StopReason::Cancel).await;
                for _ in 0..50 {
                    let Ok(now) = runtime.runs().await else {
                        break;
                    };
                    let Some(status) = now.into_iter().find(|r| r.run_id == run.run_id) else {
                        break;
                    };
                    if status.phase == Phase::Ended && status.stop_confirmed {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            }
            let _ = runtime.discard(&run.run_id, None).await;
            let _ = self.artifacts.remove_run("test", &run.run_id).await;
        }
    }

    fn spawn_sync(self: &Arc<Self>, run_id: &str) {
        let mut syncing = self.syncing.lock().unwrap_or_else(|p| p.into_inner());
        if syncing.get(run_id).is_some_and(|t| !t.is_finished()) {
            return;
        }
        let service = Arc::clone(self);
        let id = run_id.to_string();
        syncing.insert(
            run_id.to_string(),
            tokio::spawn(async move { service.sync(&id).await }),
        );
    }

    /// Follow one run until it ended and everything of it is mirrored.
    async fn sync(self: Arc<Self>, run_id: &str) {
        loop {
            let Ok(row) = self.row(run_id).await else {
                return;
            };
            if !row.needs_sync() {
                return;
            }
            // A Cancel asked for while the controller could not be reached.
            let runtime = match self.runtime_for(&row.host_id).await {
                Ok(runtime) => runtime,
                Err(_) => {
                    self.set_link(&row.host_id, false);
                    tokio::time::sleep(RETRY_AFTER).await;
                    continue;
                }
            };
            if row.cancel_requested && row.phase != RunPhase::Ended {
                match runtime.stop(run_id, StopReason::Cancel).await {
                    Ok(status) => {
                        let id = run_id.to_string();
                        let _ = self
                            .history
                            .call(move |conn| store::set_cancel_requested(conn, &id, false))
                            .await;
                        let _ = self.apply_status(run_id, &status).await;
                    }
                    Err(e) if e.code == ErrorCode::NotFound => {
                        let id = run_id.to_string();
                        let _ = self
                            .history
                            .call(move |conn| store::mark_lost(conn, &id))
                            .await;
                        self.emit(run_id, false);
                        return;
                    }
                    Err(e) if e.code == ErrorCode::Conflict => {
                        // Already ended or stopping: nothing to cancel.
                        let id = run_id.to_string();
                        let _ = self
                            .history
                            .call(move |conn| store::set_cancel_requested(conn, &id, false))
                            .await;
                    }
                    Err(_) => {
                        self.set_link(&row.host_id, false);
                        tokio::time::sleep(RETRY_AFTER).await;
                        continue;
                    }
                }
            }
            // An ended run has little left to say: a short wait, so what
            // follows its end (collection, the end of this task) is not
            // held up by a long poll.
            let wait = if row.phase == RunPhase::Ended {
                Duration::from_secs(1)
            } else {
                POLL_WAIT
            };
            match runtime.events(run_id, row.cursor, wait).await {
                Ok(page) => {
                    self.set_link(&row.host_id, true);
                    let last = page.events.last().map(|e| e.seq);
                    let turn_ended = page
                        .events
                        .iter()
                        .any(|e| matches!(e.body, EventBody::TurnEnded { .. }));
                    if !page.events.is_empty() {
                        if let Err(e) = self.mirror(run_id, page.events).await {
                            tracing::warn!(run = %run_id, error = %e, "the run's journal could not be mirrored");
                            tokio::time::sleep(RETRY_AFTER).await;
                            continue;
                        }
                    }
                    let status = match self.status_of(run_id).await {
                        Ok(status) => status,
                        Err(_) => {
                            self.set_link(&row.host_id, false);
                            tokio::time::sleep(RETRY_AFTER).await;
                            continue;
                        }
                    };
                    if let Err(e) = self.apply_status(run_id, &status).await {
                        tracing::warn!(run = %run_id, error = %e, "the run's row could not be updated");
                    }
                    // Changes so far, after each turn of a run still working.
                    if turn_ended && status.phase == super::controller::protocol::Phase::Running {
                        self.spawn_preview(run_id);
                    }
                    let caught_up = status.cursor <= last.unwrap_or(row.cursor);
                    if status.phase == super::controller::protocol::Phase::Ended && caught_up {
                        self.after_end(run_id).await;
                        if self.row(run_id).await.is_ok_and(|r| !r.needs_sync()) {
                            return;
                        }
                    }
                }
                Err(e) => {
                    if e.code == ErrorCode::NotFound {
                        // The controller does not know the run: an earlier
                        // controller's records are gone with it.
                        let id = run_id.to_string();
                        let _ = self
                            .history
                            .call(move |conn| store::mark_lost(conn, &id))
                            .await;
                        self.emit(run_id, false);
                        return;
                    }
                    self.set_link(&row.host_id, false);
                    self.emit(run_id, false);
                    tokio::time::sleep(RETRY_AFTER).await;
                }
            }
        }
    }

    async fn status_of(&self, run_id: &str) -> AppResult<RunStatus> {
        self.rt(run_id)
            .await?
            .runs()
            .await?
            .into_iter()
            .find(|r| r.run_id == run_id)
            .ok_or_else(|| AppError::not_found("The run controller does not know this run."))
    }

    /// Append events to `trace.jsonl`, then advance the row's cursor: the
    /// cursor moves only after the Mac's own copy is on disk.
    async fn mirror(
        &self,
        run_id: &str,
        events: Vec<super::controller::protocol::Event>,
    ) -> AppResult<()> {
        // An event this build cannot read is kept as a notice, so the
        // cursor still moves past it.
        let converted: Vec<RunEvent> = events
            .into_iter()
            .map(|e| {
                convert(&e).unwrap_or(RunEvent {
                    seq: e.seq,
                    at: e.at.clone(),
                    body: crate::models::RunEventBody::Notice {
                        text: "An update Brainiac could not read was skipped.".into(),
                    },
                })
            })
            .collect();
        let Some(last) = converted.last().map(|e| e.seq) else {
            return Ok(());
        };
        let path = self.trace_path(run_id);
        let lines: Vec<String> = converted
            .iter()
            .filter_map(|e| serde_json::to_string(e).ok())
            .collect();
        tokio::task::spawn_blocking(move || -> std::io::Result<()> {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            use std::os::unix::fs::OpenOptionsExt;
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .mode(0o600)
                .open(&path)?;
            for line in lines {
                file.write_all(line.as_bytes())?;
                file.write_all(b"\n")?;
            }
            file.sync_data()
        })
        .await
        .map_err(|e| AppError::io(e.to_string()))??;
        let journal = self.journal(run_id).await?;
        {
            let mut journal = journal.lock().unwrap_or_else(|p| p.into_inner());
            let known = journal.events.last().map(|e| e.seq).unwrap_or(0);
            journal
                .events
                .extend(converted.into_iter().filter(|e| e.seq > known));
        }
        let id = run_id.to_string();
        self.history
            .call(move |conn| store::set_cursor(conn, &id, last))
            .await?;
        // The window reads the mirror on this: a turn streams without any
        // change in the run's state.
        self.emit(run_id, false);
        Ok(())
    }

    /// The controller's status becomes the row's projection.
    async fn apply_status(&self, run_id: &str, status: &RunStatus) -> AppResult<()> {
        let id = run_id.to_string();
        let projection = store::Projection::from_status(status);
        let changed = self
            .history
            .call(move |conn| store::apply(conn, &id, &projection))
            .await?;
        if changed {
            self.emit(run_id, false);
        }
        Ok(())
    }

    /// Once a run ended and its stop is confirmed: collect its work, unless
    /// it was interrupted (then Collect work is the user's). A run that
    /// failed before anything was made on the engine has no work; the
    /// controller's copy of its start and journal goes now, the journal
    /// being mirrored.
    async fn after_end(self: &Arc<Self>, run_id: &str) {
        let Ok(row) = self.row(run_id).await else {
            return;
        };
        if row.phase == RunPhase::Ended && row.stop_confirmed && !row.kept {
            if let Err(e) = self.discard_at_controller(run_id).await {
                tracing::warn!(run = %run_id, error = %e, "the controller's files for a run were not removed");
            }
        } else if row.phase == RunPhase::Ended
            && row.stop_confirmed
            && row.kept
            && row.collection == RunCollection::None
            && row.outcome != Some(RunOutcome::Interrupted)
        {
            if let Err(e) = self.collect(run_id, Vec::new()).await {
                tracing::warn!(run = %run_id, error = %e, "the run's work was not collected");
            }
        }
    }

    // -----------------------------------------------------------------------
    // Steering
    // -----------------------------------------------------------------------

    /// **Send** the next prompt, only while the run is idle.
    pub async fn prompt(&self, run_id: &str, text: &str) -> AppResult<AgentRun> {
        let row = self.row(run_id).await?;
        let text = text.trim();
        if text.is_empty() {
            return Err(AppError::validation("Write a prompt for the agent."));
        }
        if text.len() > MAX_PROMPT_BYTES {
            return Err(AppError::validation("The prompt is over 100 KB."));
        }
        if row.phase != RunPhase::Running {
            return Err(conflict("The run is not running."));
        }
        // One ID per prompt the user sends, so a retry of this call never
        // sends it twice; the controller keeps the first outcome.
        let command_id = format!("prompt-{}", uuid::Uuid::new_v4());
        self.rt(run_id)
            .await?
            .prompt(run_id, &command_id, text)
            .await?;
        self.set_link(&row.host_id, true);
        self.refresh(run_id).await
    }

    /// **Allow once** or **Reject** a pending permission.
    pub async fn permit(
        &self,
        run_id: &str,
        permission_id: &str,
        allow: bool,
    ) -> AppResult<AgentRun> {
        let row = self.row(run_id).await?;
        if row.phase != RunPhase::Running {
            return Err(conflict("The run is not running."));
        }
        // The same answer to the same request has the same ID, so it is
        // written to the agent once however often it is sent.
        let command_id = format!("permit-{}", short_hash(&format!("{permission_id}:{allow}")));
        self.rt(run_id)
            .await?
            .permit(run_id, &command_id, permission_id, allow)
            .await?;
        self.refresh(run_id).await
    }

    /// **Cancel run**: sent now, or remembered until the controller answers.
    pub async fn cancel(self: &Arc<Self>, run_id: &str) -> AppResult<AgentRun> {
        let row = self.row(run_id).await?;
        if row.phase == RunPhase::Ended {
            return Ok(row.into_run(true));
        }
        // Still being handed to its controller: the start stops before its
        // next step, or the Cancel is sent once the controller has it.
        if self.cancel_starting(run_id) {
            let id = run_id.to_string();
            self.history
                .call(move |conn| store::set_cancel_requested(conn, &id, true))
                .await?;
            self.emit(run_id, false);
            return self.get(run_id).await;
        }
        match self
            .rt(run_id)
            .await?
            .stop(run_id, StopReason::Cancel)
            .await
        {
            Ok(status) => {
                self.apply_status(run_id, &status).await?;
                self.spawn_sync(run_id);
                self.get(run_id).await
            }
            Err(e) if e.code == ErrorCode::Conflict || e.code == ErrorCode::Validation => Err(e),
            Err(_) => {
                self.set_link(&row.host_id, false);
                let id = run_id.to_string();
                self.history
                    .call(move |conn| store::set_cancel_requested(conn, &id, true))
                    .await?;
                self.emit(run_id, false);
                self.spawn_sync(run_id);
                self.get(run_id).await
            }
        }
    }

    /// **Finish and collect**: only while the agent waits for a prompt.
    pub async fn finish(self: &Arc<Self>, run_id: &str) -> AppResult<AgentRun> {
        let row = self.row(run_id).await?;
        if row.phase != RunPhase::Running {
            return Err(conflict("The run is not running."));
        }
        let status = self
            .rt(run_id)
            .await?
            .stop(run_id, StopReason::Finish)
            .await?;
        self.apply_status(run_id, &status).await?;
        self.spawn_sync(run_id);
        self.get(run_id).await
    }

    /// The controller's view of the run now, into the row.
    async fn refresh(&self, run_id: &str) -> AppResult<AgentRun> {
        if let Ok(status) = self.status_of(run_id).await {
            self.apply_status(run_id, &status).await?;
        }
        self.get(run_id).await
    }

    // -----------------------------------------------------------------------
    // Collecting and reviewing
    // -----------------------------------------------------------------------

    /// Collect the stopped run's working tree (**Collect work**, **Retry
    /// collection**, **Choose files to add…** with `include`), verify and
    /// import the snapshot, and clean up when nothing waits for a decision.
    pub async fn collect(
        self: &Arc<Self>,
        run_id: &str,
        include: Vec<String>,
    ) -> AppResult<AgentRun> {
        let lock = Arc::clone(
            self.collecting
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .entry(run_id.to_string())
                .or_default(),
        );
        let busy = || conflict("The run's work is already being collected.");
        let _held = match lock.try_lock() {
            Ok(held) => held,
            // The row says what was collected a moment before the collection
            // that wrote it lets go (a cleanup may follow); a collection asked
            // for then waits for it. One still running is refused at once.
            Err(_) => {
                if self.row(run_id).await?.collection == RunCollection::Collecting {
                    return Err(busy());
                }
                tokio::time::timeout(COLLECT_HANDOVER, lock.lock())
                    .await
                    .map_err(|_| busy())?
            }
        };
        // The left-out list names a folder with a trailing slash; the
        // collector takes it without one.
        let include: Vec<String> = include
            .into_iter()
            .map(|p| p.trim_end_matches('/').to_string())
            .collect();
        let row = self.row(run_id).await?;
        if row.phase != RunPhase::Ended || !row.stop_confirmed {
            return Err(conflict(
                "The run's work can be collected once the run has stopped.",
            ));
        }
        if !row.kept {
            return Err(conflict("The run's work is no longer on the engine."));
        }
        self.set_collection(run_id, RunCollection::Collecting, None)
            .await?;
        let out_dir = self.artifacts.run_dir(run_id);
        let collected = async {
            std::fs::create_dir_all(&out_dir)?;
            let image = self.current_image_for(run_id).await;
            let manifest = self
                .rt(run_id)
                .await?
                .collect(run_id, include, &out_dir, image)
                .await?;
            self.set_link(&row.host_id, true);
            if manifest.start != row.start_commit {
                return Err(AppError::io(
                    "The collector reported another start than the run's.",
                ));
            }
            let bundle = out_dir.join("result.bundle");
            let result = if manifest.result == manifest.start {
                manifest.result.clone()
            } else {
                self.artifacts
                    .import_result(
                        &row.repository_id,
                        run_id,
                        &bundle,
                        &row.start_commit,
                        &manifest.result,
                    )
                    .await?
            };
            let _ = std::fs::remove_file(&bundle);
            Ok::<_, AppError>((manifest, result))
        }
        .await;
        match collected {
            Ok((manifest, result)) => {
                let no_changes = result == row.start_commit;
                let left_out: Vec<LeftOutFile> = manifest
                    .left_out
                    .into_iter()
                    .map(|l| LeftOutFile {
                        path: l.path,
                        reason: l.reason,
                    })
                    .collect();
                let accepted = left_out.is_empty() && manifest.left_out_more == 0;
                let id = run_id.to_string();
                let outcome = store::Collected {
                    collection: if no_changes {
                        RunCollection::NoChanges
                    } else {
                        RunCollection::Ready
                    },
                    result_commit: result,
                    changed_files: manifest.changed_files,
                    left_out,
                    left_out_more: manifest.left_out_more,
                    accepted,
                };
                self.history
                    .call(move |conn| store::set_collected(conn, &id, &outcome))
                    .await?;
                self.drop_preview(run_id);
                self.emit(run_id, false);
                if accepted {
                    self.cleanup(run_id).await;
                }
            }
            Err(e) => {
                tracing::warn!(run = %run_id, error = %e, details = ?e.details, "collection failed");
                self.set_collection(run_id, RunCollection::Failed, Some(e.message.clone()))
                    .await?;
                return Err(e);
            }
        }
        self.get(run_id).await
    }

    async fn set_collection(
        &self,
        run_id: &str,
        state: RunCollection,
        error: Option<String>,
    ) -> AppResult<()> {
        let id = run_id.to_string();
        self.history
            .call(move |conn| store::set_collection(conn, &id, state, error.as_deref()))
            .await?;
        self.emit(run_id, false);
        Ok(())
    }

    /// **Keep this snapshot**: the left-out list is accepted; the stopped
    /// container and its files go.
    pub async fn accept_snapshot(self: &Arc<Self>, run_id: &str) -> AppResult<AgentRun> {
        let row = self.row(run_id).await?;
        if !matches!(
            row.collection,
            RunCollection::Ready | RunCollection::NoChanges
        ) {
            return Err(conflict("There is no collected snapshot to keep yet."));
        }
        let id = run_id.to_string();
        self.history
            .call(move |conn| store::set_accepted(conn, &id))
            .await?;
        self.emit(run_id, false);
        self.cleanup(run_id).await;
        self.get(run_id).await
    }

    /// Remove the run's container and volume from the engine, once nothing
    /// of its work waits for a decision. A failure stays as Cleanup pending.
    async fn cleanup(&self, run_id: &str) {
        let Ok(row) = self.row(run_id).await else {
            return;
        };
        if !row.kept {
            return;
        }
        let id = run_id.to_string();
        let result = self.discard_at_controller(run_id).await;
        let pending = match &result {
            Ok(()) => None,
            Err(e) => Some(e.message.clone()),
        };
        if let Err(e) = &result {
            tracing::warn!(run = %run_id, error = %e, details = ?e.details, "cleanup pending");
        }
        let _ = self
            .history
            .call(move |conn| store::set_cleanup(conn, &id, result.is_ok(), pending.as_deref()))
            .await;
        self.emit(run_id, false);
    }

    /// Remove the run's container, workspace, and record at the controller.
    /// A run the controller no longer knows has nothing left there to
    /// remove: its record went with an earlier discard whose answer was
    /// lost, or with the controller's folder.
    async fn discard_at_controller(&self, run_id: &str) -> AppResult<()> {
        let image = self.current_image_for(run_id).await;
        match self.rt(run_id).await?.discard(run_id, image).await {
            Err(e) if e.code == ErrorCode::NotFound => {
                tracing::warn!(run = %run_id, "the controller no longer knows the run; nothing left to remove");
                Ok(())
            }
            result => result,
        }
    }

    /// **Discard work…**: the stopped container and its files go, whatever
    /// was or was not collected.
    pub async fn discard(self: &Arc<Self>, run_id: &str) -> AppResult<AgentRun> {
        let row = self.row(run_id).await?;
        if row.phase != RunPhase::Ended || !row.stop_confirmed {
            return Err(conflict(
                "The run has not stopped, so its work cannot be discarded yet.",
            ));
        }
        if row.collection == RunCollection::Collecting {
            return Err(conflict(
                "The run's work is being collected; discard it afterwards.",
            ));
        }
        self.discard_at_controller(run_id).await?;
        let id = run_id.to_string();
        self.history
            .call(move |conn| store::set_cleanup(conn, &id, true, None))
            .await?;
        self.emit(run_id, false);
        self.get(run_id).await
    }

    /// **Retry cleanup**.
    pub async fn retry_cleanup(self: &Arc<Self>, run_id: &str) -> AppResult<AgentRun> {
        self.cleanup(run_id).await;
        self.get(run_id).await
    }

    // -----------------------------------------------------------------------
    // Changes so far
    // -----------------------------------------------------------------------

    fn preview_path(&self, run_id: &str) -> PathBuf {
        self.artifacts.run_dir(run_id).join("preview.json")
    }

    fn preview_lock(&self, run_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        Arc::clone(
            self.previewing
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .entry(run_id.to_string())
                .or_default(),
        )
    }

    /// The last preview taken, as recorded in the run's folder.
    fn preview_record(&self, run_id: &str) -> PreviewRecord {
        std::fs::read(self.preview_path(run_id))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    /// Take a preview in the background; one already being taken is
    /// followed by another. A run that stopped meanwhile is not an error
    /// worth showing.
    fn spawn_preview(self: &Arc<Self>, run_id: &str) {
        let service = Arc::clone(self);
        let id = run_id.to_string();
        tokio::spawn(async move {
            match service.take_preview(&id, false).await {
                Ok(()) => {}
                Err(e) if e.code == ErrorCode::Conflict => {}
                Err(e) => {
                    tracing::warn!(run = %id, error = %e, details = ?e.details, "changes so far were not read");
                }
            }
        });
    }

    /// **Refresh** in Changes so far: a provisional snapshot of the live
    /// run's working tree, now.
    pub async fn preview(self: &Arc<Self>, run_id: &str) -> AppResult<RunPreview> {
        self.take_preview(run_id, true).await?;
        self.preview_state(run_id).await
    }

    /// The collector reads the running workspace; its snapshot goes under
    /// the run's preview ref, never its result. A failure keeps the earlier
    /// preview and says why. `asked`: the user's Refresh, which waits for
    /// one already being taken instead of being refused.
    async fn take_preview(self: &Arc<Self>, run_id: &str, asked: bool) -> AppResult<()> {
        let lock = self.preview_lock(run_id);
        let _held = if asked {
            lock.lock().await
        } else {
            match lock.try_lock() {
                Ok(held) => held,
                Err(_) => {
                    self.preview_again
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .insert(run_id.to_string());
                    return Err(conflict("Changes so far are already being read."));
                }
            }
        };
        let row = self.row(run_id).await?;
        if row.phase != RunPhase::Running {
            return Err(conflict(
                "Changes so far can be shown while the run is working.",
            ));
        }
        self.emit(run_id, false);
        let dir = self.artifacts.run_dir(run_id).join("preview");
        let bundle = dir.join("result.bundle");
        let taken = async {
            std::fs::create_dir_all(&dir)?;
            let manifest = self.rt(run_id).await?.preview(run_id, &dir).await?;
            if manifest.start != row.start_commit {
                return Err(AppError::io(
                    "The collector reported another start than the run's.",
                ));
            }
            let commit = if manifest.result == manifest.start {
                manifest.result.clone()
            } else {
                self.artifacts
                    .import_preview(
                        &row.repository_id,
                        run_id,
                        &bundle,
                        &row.start_commit,
                        &manifest.result,
                    )
                    .await?
            };
            let left_out = manifest.left_out.len() as u32 + manifest.left_out_more;
            let files = if commit == row.start_commit {
                Vec::new()
            } else {
                self.artifacts
                    .changes(&row.repository_id, &row.start_commit, &commit, Some(run_id))
                    .await?
            };
            Ok::<_, AppError>((commit, left_out, files))
        }
        .await;
        // Imported or not, the bundle has served.
        let _ = std::fs::remove_file(&bundle);
        let mut record = self.preview_record(run_id);
        let outcome = match taken {
            Ok((commit, left_out, files)) => {
                record = PreviewRecord {
                    commit: Some(commit),
                    taken_at: Some(now_rfc3339()),
                    turn: row.turn,
                    left_out,
                    error: None,
                    files,
                };
                Ok(())
            }
            // A run that stopped while it was read: the earlier preview
            // stands, and the collection comes next.
            Err(e) if e.code == ErrorCode::Conflict => Err(e),
            Err(e) => {
                record.error = Some(e.message.clone());
                Err(e)
            }
        };
        // A run that ended meanwhile is collected next, and a deleted one
        // keeps its folder gone.
        let path = self.preview_path(run_id);
        if self
            .row(run_id)
            .await
            .is_ok_and(|r| r.phase != RunPhase::Ended)
        {
            if let Ok(bytes) = serde_json::to_vec(&record) {
                if let Err(e) = super::controller::state::write_atomic(&path, &bytes) {
                    tracing::warn!(run = %run_id, error = %e, "the preview's record was not saved");
                }
            }
        }
        drop(_held);
        self.emit(run_id, false);
        let again = self
            .preview_again
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(run_id);
        if again {
            self.spawn_preview(run_id);
        }
        outcome
    }

    /// Once the work is collected, Changes so far has served: its
    /// repository and record go.
    fn drop_preview(&self, run_id: &str) {
        let _ = std::fs::remove_dir_all(self.artifacts.preview_repository(run_id));
        let _ = std::fs::remove_dir_all(self.artifacts.run_dir(run_id).join("preview"));
        let _ = std::fs::remove_file(self.preview_path(run_id));
    }

    /// **Changes so far**: the last preview and its files against the start.
    pub async fn preview_state(&self, run_id: &str) -> AppResult<RunPreview> {
        let row = self.row(run_id).await?;
        let record = self.preview_record(run_id);
        Ok(RunPreview {
            run_id: run_id.to_string(),
            start_commit: row.start_commit,
            commit: record.commit,
            taken_at: record.taken_at,
            turn: record.turn,
            files: record.files,
            left_out: record.left_out,
            busy: self.preview_lock(run_id).try_lock().is_err(),
            error: record.error,
        })
    }

    /// **Changes**: the files the snapshot changed.
    pub async fn changes(&self, run_id: &str) -> AppResult<RunChanges> {
        let row = self.row(run_id).await?;
        let (start, result) = row.reviewable()?;
        let files = if result == start {
            Vec::new()
        } else {
            self.artifacts
                .changes(&row.repository_id, &start, &result, None)
                .await?
        };
        Ok(RunChanges {
            run_id: run_id.to_string(),
            start_commit: start,
            result_commit: result,
            files,
        })
    }

    pub async fn diff(&self, request: RunDiffRequest) -> AppResult<DiffResult> {
        let row = self.row(&request.run_id).await?;
        let (start, result) = if request.preview {
            let commit = self
                .preview_record(&request.run_id)
                .commit
                .ok_or_else(|| conflict("No changes so far have been read yet."))?;
            (row.start_commit.clone(), commit)
        } else {
            row.reviewable()?
        };
        self.artifacts
            .diff(
                &row.repository_id,
                (&start, &result),
                &request.path,
                request.old_path.as_deref(),
                request.options,
                request.preview.then_some(request.run_id.as_str()),
            )
            .await
    }

    /// **Copy patch** (`None`) or **Save patch…** (`Some(path)`). Either
    /// confirms the left-out list as Keep this snapshot does.
    pub async fn patch(
        self: &Arc<Self>,
        run_id: &str,
        to_file: Option<&Path>,
    ) -> AppResult<String> {
        let row = self.row(run_id).await?;
        let (start, result) = row.reviewable()?;
        if result == start {
            return Err(AppError::validation(
                "The run changed nothing: there is no patch.",
            ));
        }
        let patch = self
            .artifacts
            .patch(&row.repository_id, &start, &result, to_file)
            .await?;
        if !row.snapshot_accepted {
            let _ = self.accept_snapshot(run_id).await;
        }
        Ok(patch)
    }

    /// **Copy branch command**: a command that fetches the snapshot from
    /// Brainiac's repository into the user's as a new branch, for the user to
    /// run. Like a patch export, it confirms the left-out list.
    pub async fn branch_command(self: &Arc<Self>, run_id: &str) -> AppResult<RunBranchCommand> {
        let row = self.row(run_id).await?;
        let (start, result) = row.reviewable()?;
        if result == start {
            return Err(AppError::validation(
                "The run changed nothing: there is no branch to make.",
            ));
        }
        let branch = super::repository::branch_name(&row.title, run_id);
        let command = self
            .artifacts
            .branch_command(&row.repository_id, run_id, &branch);
        if !row.snapshot_accepted {
            let _ = self.accept_snapshot(run_id).await;
        }
        Ok(RunBranchCommand { branch, command })
    }

    // -----------------------------------------------------------------------
    // Deleting and keeping
    // -----------------------------------------------------------------------

    /// **Delete run…**: refused while the stop is not confirmed; removes the
    /// container and its files, then the conversation, the result, and the row.
    pub async fn delete(self: &Arc<Self>, run_id: &str) -> AppResult<()> {
        let row = self.row(run_id).await?;
        if row.phase != RunPhase::Ended || !row.stop_confirmed {
            return Err(conflict(
                "The run has not stopped, so it cannot be deleted yet. Cancel it first.",
            ));
        }
        if row.collection == RunCollection::Collecting {
            return Err(conflict(
                "The run's work is being collected; delete it afterwards.",
            ));
        }
        if row.kept {
            if let Err(e) = self.discard_at_controller(run_id).await {
                let id = run_id.to_string();
                let message = e.message.clone();
                let _ = self
                    .history
                    .call(move |conn| store::set_cleanup(conn, &id, false, Some(&message)))
                    .await;
                self.emit(run_id, false);
                return Err(AppError::new(
                    e.code,
                    format!(
                        "{} The run is kept with Cleanup pending until its container and files are removed.",
                        e.message
                    ),
                ));
            }
        }
        if let Some(task) = self
            .syncing
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(run_id)
        {
            task.abort();
        }
        self.artifacts
            .remove_run(&row.repository_id, run_id)
            .await?;
        self.journals
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(run_id);
        let id = run_id.to_string();
        self.history
            .call(move |conn| store::delete(conn, &id))
            .await?;
        self.emit(run_id, true);
        Ok(())
    }

    /// Once an hour: ended runs older than the retention go, unless their
    /// work still waits for a decision or their cleanup is pending.
    pub async fn retention_tick(self: &Arc<Self>) {
        let Ok(rows) = self.history.call(store::list).await else {
            return;
        };
        let cutoff = chrono::Utc::now() - RETENTION;
        for row in rows {
            let old = row
                .ended_at
                .as_deref()
                .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
                .is_some_and(|t| t.with_timezone(&chrono::Utc) < cutoff);
            if old
                && row.phase == RunPhase::Ended
                && row.stop_confirmed
                && !row.waits_for_decision()
            {
                if let Err(e) = self.delete(&row.id).await {
                    tracing::warn!(run = %row.id, error = %e, "an old run was not removed");
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Settings → Agents, Test
    // -----------------------------------------------------------------------

    /// **Test** of a profile on this Mac: a short run on a tiny repository
    /// of its own, a prompt, a cancel, and a collection. Records the pass for
    /// the profile and this Mac.
    pub async fn test(self: &Arc<Self>, profile_id: &str) -> AppResult<AgentTestResult> {
        let settings = self.settings.get().await?;
        let (profile, host) = test_subject(&settings, super::hosts::LOCAL_ID, profile_id)?;
        let mut steps = Vec::new();
        let mut step = |name: &str, result: AppResult<String>| -> bool {
            let passed = result.is_ok();
            steps.push(AgentTestStep {
                name: name.to_string(),
                passed,
                detail: match result {
                    Ok(detail) if detail.is_empty() => None,
                    Ok(detail) => Some(detail),
                    Err(e) => Some(e.message),
                },
            });
            passed
        };
        let socket = settings
            .engine_socket
            .clone()
            .ok_or_else(|| AppError::validation("Choose where runs execute first."))?;
        let image = host
            .image
            .clone()
            .ok_or_else(|| AppError::validation("Build the image first."))?;
        let run_id = format!("test-{}", uuid::Uuid::new_v4());
        let started = self
            .test_start(&profile, &socket, &image.name, &run_id)
            .await;
        let outcome = match started {
            Ok(()) => {
                step("Start a run", Ok(String::new()));
                self.test_follow(&run_id, &mut step).await
            }
            Err(e) => {
                step("Start a run", Err(e));
                Err(())
            }
        };
        // Stop the container first when the follow-up never reached Cancel.
        // Discard refuses a run that is still going.
        self.release_test(&run_id, Some(image.name)).await;
        let _ = self.artifacts.remove_run("test", &run_id).await;
        let passed = outcome.is_ok();
        if passed {
            self.settings
                .record_test(&host.id, &profile, &image.id, Some(&socket))
                .await?;
        }
        Ok(AgentTestResult {
            steps,
            passed,
            tested_at: now_rfc3339(),
        })
    }

    /// **Test** of a profile on an approved host. The token or key is sent
    /// to that host.
    pub async fn test_host(
        self: &Arc<Self>,
        host_id: &str,
        profile_id: &str,
    ) -> AppResult<AgentTestResult> {
        let settings = self.settings.get().await?;
        let (profile, host) = test_subject(&settings, host_id, profile_id)?;
        let image = host
            .image
            .clone()
            .ok_or_else(|| AppError::validation("Build the image on this host first."))?;
        let hosts = self
            .hosts
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
            .ok_or_else(|| AppError::dependency("Remote hosts are not available."))?;
        if hosts.is_deploying(host_id) {
            return Err(upgrading_conflict());
        }
        let _lease = hosts.begin_test(host_id);
        let runtime = self.runtime_for(host_id).await?;
        let run_id = format!("test-{}", uuid::Uuid::new_v4());
        self.test_runtimes
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(run_id.clone(), runtime);
        let mut steps = Vec::new();
        let mut step = |name: &str, result: AppResult<String>| -> bool {
            let passed = result.is_ok();
            steps.push(AgentTestStep {
                name: name.to_string(),
                passed,
                detail: match result {
                    Ok(detail) if detail.is_empty() => None,
                    Ok(detail) => Some(detail),
                    Err(e) => Some(e.message),
                },
            });
            passed
        };
        let started = self
            .test_start(&profile, "/var/run/docker.sock", &image.name, &run_id)
            .await;
        let outcome = match started {
            Ok(()) => {
                step("Start a run", Ok(String::new()));
                self.test_follow(&run_id, &mut step).await
            }
            Err(e) => {
                step("Start a run", Err(e));
                Err(())
            }
        };
        self.release_test(&run_id, Some(image.name.clone())).await;
        let _ = self.artifacts.remove_run("test", &run_id).await;
        self.test_runtimes
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&run_id);
        let passed = outcome.is_ok();
        if passed {
            self.settings
                .record_test(host_id, &profile, &image.id, None)
                .await?;
        }
        Ok(AgentTestResult {
            steps,
            passed,
            tested_at: now_rfc3339(),
        })
    }

    async fn test_start(
        &self,
        profile: &AgentProfile,
        socket: &str,
        image: &str,
        run_id: &str,
    ) -> AppResult<()> {
        // A repository made for the test, in the run's own folder.
        let dir = self.artifacts.run_dir(run_id);
        let source = dir.join("source");
        let (bundle, commit) = self.artifacts.test_bundle(&source, run_id).await?;
        let credential = self.credential(profile).await?;
        let start = StartRun {
            run_id: run_id.to_string(),
            attempt: 1,
            engine_socket: socket.to_string(),
            image: image.to_string(),
            cpus: profile.cpus.min(2),
            memory_mib: profile.memory_mib.min(4096).max(settings::MEMORY_MIB.0),
            workspace_gib: 1,
            time_limit_secs: u64::from(TEST_TIME_LIMIT_MINUTES) * 60,
            permissions: RunPermissions::Act,
            start_commit: commit,
            bundle,
            prompt_id: format!("{run_id}-prompt-1"),
            prompt: TEST_PROMPT.to_string(),
            credential,
            agent: profile.agent,
            provider: profile.provider,
            // The profile's model, so a name the plan or key cannot use fails here.
            model: profile.model.clone(),
        };
        self.rt(run_id).await?.start(start).await?;
        Ok(())
    }

    /// Stop a Settings test and discard it. Discard is refused while the
    /// container is still running, so this waits until the stop is confirmed.
    /// A follow-up that returned early would otherwise leave the container.
    async fn release_test(&self, run_id: &str, image: Option<String>) {
        use super::controller::protocol::Phase;
        let Ok(runtime) = self.rt(run_id).await else {
            return;
        };
        let _ = runtime.stop(run_id, StopReason::Cancel).await;
        for _ in 0..600 {
            match runtime.runs().await {
                Ok(runs) => {
                    let Some(status) = runs.into_iter().find(|r| r.run_id == run_id) else {
                        break;
                    };
                    if status.phase == Phase::Ended && status.stop_confirmed {
                        break;
                    }
                }
                Err(_) => break,
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        let _ = runtime.discard(run_id, image).await;
    }

    /// After the test's turn ended: `None` once the run is running and idle,
    /// or why it is not. The controller's failure follows the turn's end by
    /// a moment, so the status is read until it settles either way.
    async fn test_settled(&self, run_id: &str) -> Option<String> {
        use super::controller::protocol::{Activity, Phase};
        for _ in 0..15 {
            match self.status_of(run_id).await {
                Ok(status) if status.phase == Phase::Ended => {
                    return Some(status.error.unwrap_or_else(|| "the run ended".into()));
                }
                Ok(status) if status.phase == Phase::Stopping => {
                    return Some(
                        status
                            .error
                            .unwrap_or_else(|| "the run stopped after its reply".into()),
                    );
                }
                Ok(status)
                    if status.phase == Phase::Running && status.activity == Activity::Idle =>
                {
                    return None;
                }
                Ok(_) => {}
                Err(e) => return Some(e.message),
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        Some("The run did not settle after its reply.".into())
    }

    /// Wait for the test's turn to end, cancel, and collect.
    async fn test_follow(
        &self,
        run_id: &str,
        step: &mut impl FnMut(&str, AppResult<String>) -> bool,
    ) -> Result<(), ()> {
        use super::controller::protocol::{Activity, Phase};
        // Claude Code retries a refused key for about three minutes first.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(8 * 60);
        let mut cursor = 0;
        let mut reply = String::new();
        let mut failure: Option<String> = None;
        let mut replied = false;
        // Once the controller says the turn is over, one more page is read:
        // the status can say so before the events are fetched.
        let mut settled = false;
        while tokio::time::Instant::now() < deadline {
            // A host test is followed on that host. `self.runtime` is this Mac,
            // which does not know the run and would leave its container going.
            let runtime = match self.rt(run_id).await {
                Ok(runtime) => runtime,
                Err(e) => {
                    step("The agent answers a prompt", Err(e));
                    return Err(());
                }
            };
            let page = match runtime.events(run_id, cursor, Duration::from_secs(5)).await {
                Ok(page) => page,
                Err(e) => {
                    step("The agent answers a prompt", Err(e));
                    return Err(());
                }
            };
            for event in page.events {
                cursor = event.seq;
                match event.body {
                    EventBody::Message { text, .. } => reply.push_str(&text),
                    EventBody::TurnEnded { reason, .. } if reason != "error" => replied = true,
                    EventBody::Ended { message, outcome } => {
                        failure = Some(message.unwrap_or_else(|| format!("{outcome:?}")));
                    }
                    EventBody::Stopping { outcome } => {
                        failure = Some(format!("the run stopped: {outcome:?}"));
                    }
                    _ => {}
                }
            }
            if failure.is_some() || settled {
                break;
            }
            if replied {
                // A refused key can come back as the reply, and the
                // controller fails the run just after the turn ends: the run
                // must still be running and idle, not merely answered.
                failure = self.test_settled(run_id).await;
                break;
            }
            if let Ok(status) = self.status_of(run_id).await {
                if status.phase == Phase::Ended {
                    failure = Some(status.error.unwrap_or_else(|| "the run ended".into()));
                    break;
                }
                settled = status.phase == Phase::Running && status.activity == Activity::Idle;
            }
        }
        if failure.is_none() && super::controller::acp::credential_refused(&reply) {
            failure = Some(format!(
                "The key or token was refused: {}",
                reply.trim().chars().take(120).collect::<String>()
            ));
        }
        let answered = match failure {
            Some(message) => Err(AppError::validation(message)),
            None if replied => Ok(reply.trim().chars().take(80).collect()),
            None => Err(AppError::timeout(
                "The agent did not answer within eight minutes.",
            )),
        };
        if !step("The agent answers a prompt", answered) {
            return Err(());
        }
        let cancelled = async {
            self.rt(run_id)
                .await?
                .stop(run_id, StopReason::Cancel)
                .await?;
            for _ in 0..600 {
                let status = self.status_of(run_id).await?;
                if status.phase == Phase::Ended && status.stop_confirmed {
                    return Ok(String::new());
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            Err(AppError::timeout("The engine did not confirm the stop."))
        }
        .await;
        if !step("Cancel stops the container", cancelled) {
            return Err(());
        }
        let collected = async {
            let out = self.artifacts.run_dir(run_id);
            let manifest = self
                .rt(run_id)
                .await?
                .collect(run_id, Vec::new(), &out, None)
                .await?;
            Ok(format!("{} changed file(s)", manifest.changed_files))
        }
        .await;
        if !step("Collect the work", collected) {
            return Err(());
        }
        Ok(())
    }
}

fn upgrading_conflict() -> AppError {
    AppError::new(
        ErrorCode::Conflict,
        "This host's run controller is being installed or upgraded. Follow it in Settings → Agents, or start the run on another host.",
    )
}

/// The profile and host a Test runs, refused while anything but the test
/// itself is missing.
fn test_subject(
    settings: &AgentSettings,
    host_id: &str,
    profile_id: &str,
) -> AppResult<(AgentProfile, AgentHost)> {
    let profile = settings
        .profiles
        .iter()
        .find(|p| p.id == profile_id)
        .cloned()
        .ok_or_else(|| AppError::validation("Choose the agent to test."))?;
    let host = settings
        .hosts
        .iter()
        .find(|h| h.id == host_id)
        .cloned()
        .ok_or_else(|| AppError::validation("Choose a host."))?;
    if let Some(first) = profile.missing.iter().chain(&host.missing).next() {
        return Err(AppError::validation(format!(
            "Settings → Agents is not ready: {first}"
        )));
    }
    Ok((profile, host))
}

/// Sixteen hex digits of a SHA-256, for IDs that must stay short.
fn short_hash(text: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(text.as_bytes())
        .iter()
        .take(8)
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn conflict(message: &str) -> AppError {
    AppError::new(ErrorCode::Conflict, message)
}

/// The run's title: the prompt's first line, cut short, with the
/// credential's value removed should it be in there.
fn title_of(prompt: &str, secret: &str) -> String {
    let first = prompt.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let clean = if secret.len() >= 8 {
        first.replace(secret, "[credential]")
    } else {
        first.to_string()
    };
    let mut title: String = clean.trim().chars().take(TITLE_CHARS).collect();
    if clean.trim().chars().count() > TITLE_CHARS {
        title.push('…');
    }
    if title.is_empty() {
        "Untitled run".to_string()
    } else {
        title
    }
}

/// The protocol's event as the window's: the same shape, through JSON.
fn convert<T: DeserializeOwned>(event: &impl serde::Serialize) -> AppResult<T> {
    serde_json::to_value(event)
        .and_then(serde_json::from_value)
        .map_err(|e| AppError::io(e.to_string()))
}

pub(crate) fn permission_of(
    p: &super::controller::protocol::PendingPermission,
) -> Option<RunPermissionRequest> {
    convert(p).ok()
}

pub(crate) fn activity_of(a: super::controller::protocol::Activity) -> RunActivity {
    convert(&a).unwrap_or(RunActivity::Ended)
}

pub(crate) fn phase_of(p: super::controller::protocol::Phase) -> RunPhase {
    convert(&p).unwrap_or(RunPhase::Ended)
}

pub(crate) fn outcome_of(o: super::controller::protocol::Outcome) -> RunOutcome {
    convert(&o).unwrap_or(RunOutcome::Interrupted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_title_is_the_first_line_without_the_credential() {
        let key = "sk-ant-api03-SecretSecretSecret";
        assert_eq!(
            title_of(&format!("\n\n  Fix the build with {key} please\nmore"), key),
            "Fix the build with [credential] please"
        );
        let long = "x".repeat(100);
        let title = title_of(&long, "nope");
        assert_eq!(title.chars().count(), TITLE_CHARS + 1);
        assert!(title.ends_with('…'));
        assert_eq!(title_of("   ", "k"), "Untitled run");
    }
}
