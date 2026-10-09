//! `ExplanationService` (SPEC.md, section 14; docs/architecture.md,
//! Explaining changes — v0.6): the Explain dialog, an explanation from its
//! explain run to its checked file, and what the panel and Settings →
//! Explanations change.
//!
//! An explanation follows its explain run through the run service's change
//! events: when the agent's turn ends, its file is read from the container
//! and checked; a file that fails gets one follow-up turn; then the run is
//! cancelled, and the run service discards its container and files, keeping
//! its row and journal for How it was written.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::future::BoxFuture;
use sha2::{Digest, Sha256};
use tokio::sync::broadcast;

use super::check::{self, Attempt};
use super::known;
use super::prompt::{self, PromptSubject, Reader};
use super::store::{self, Ending, ExplanationRow};
use super::subject::EMPTY_TREE;
use crate::agents::runs::{ExplainRun, ExplainStart};
use crate::agents::settings::{profile_name, run_missing};
use crate::agents::{AgentRunService, AgentSettingsService, RunArtifacts};
use crate::db::{self, Db};
use crate::models::{
    now_rfc3339, AgentKind, AgentRun, AgentSettings, AppError, AppResult, ConceptKind,
    CreateNoteRequest, ErrorCode, ExplainCost, ExplainDepth, ExplainDialog, ExplainEstimate,
    ExplainHostOption, ExplainProfileOption, ExplainStep, ExplainSubject, ExplainSubjectKind,
    ExplanationChangedEvent, ExplanationPlacement, ExplanationRecord, ExplanationSettings,
    ExplanationSettingsView, ExplanationState, ExplanationSummary, NotePlacement, NoteSummary,
    RunActivity, RunCollection, RunOutcome, RunPhase, RunStartStep, StartExplanationRequest,
};
use crate::notes::NoteService;
use crate::sharing::CodeSharingService;
use crate::workspaces::RepositoryService;

pub type ExplanationEmitter = Arc<dyn Fn(ExplanationChangedEvent) + Send + Sync>;
/// Says that an explanation ended, as a macOS notification when Brainiac is
/// not the active app (SPEC.md, Explain: While it works).
pub type ExplanationNotifier = Arc<dyn Fn(String, String) + Send + Sync>;

/// What Explain needs to know of a pull request, as the pull request
/// service (`forge`) last read it from the provider or its cache.
#[derive(Debug, Clone)]
pub struct PullRequestFacts {
    /// The repository registration that tracks the pull request.
    pub repository_id: String,
    pub number: u64,
    pub title: String,
    pub head_sha: String,
    /// The target branch's tip when the pull request was read.
    pub base_sha: String,
    pub source_branch: String,
    pub target_branch: String,
    /// The source branch is in another repository, such as a fork.
    pub from_fork: bool,
    /// Written by the account's user.
    pub mine: bool,
    pub author: String,
}

/// Reads a pull request by its reference. A `BoxFuture` is a future behind a
/// pointer, which is how a stored closure can be async; the closure keeps this
/// module from depending on `forge`, which is built first.
pub type PullRequestReader =
    Arc<dyn Fn(String) -> BoxFuture<'static, AppResult<PullRequestFacts>> + Send + Sync>;

const FROM_FORK: &str = "This pull request comes from another repository, such as a fork: its agent settings would run with your token, so it is not explained.";

/// How long an explanation waits for its run's next change before it looks
/// again anyway.
const LOOK_AGAIN: Duration = Duration::from_secs(20);
/// The dialog's estimate is the middle of at most this many explanations.
const ESTIMATE_FROM: u32 = 10;

pub struct ExplanationService {
    /// `brainiac.db`: known concepts and Settings → Explanations.
    core: Db,
    /// `history.db`: the explanations.
    history: Db,
    runs: Arc<AgentRunService>,
    agent_settings: Arc<AgentSettingsService>,
    /// Whether a repository's code may go to a provider, shared with runs.
    sharing: Arc<CodeSharingService>,
    artifacts: Arc<RunArtifacts>,
    repositories: Arc<RepositoryService>,
    notes: Arc<NoteService>,
    emitter: ExplanationEmitter,
    notifier: Mutex<Option<ExplanationNotifier>>,
    pull_requests: Mutex<Option<PullRequestReader>>,
    /// The run service's change events, by run ID.
    run_events: broadcast::Sender<String>,
    /// One task per working explanation.
    driving: Mutex<HashMap<String, tokio::task::JoinHandle<()>>>,
}

/// What a subject resolves to in the repository.
struct Resolved {
    base: String,
    tip: String,
    title: String,
    start: ExplainStart,
    /// "commit 1a2b3c4d", "the branch feature/x", for the prompt.
    what: String,
    root_commit: bool,
}

impl ExplanationService {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        core: Db,
        history: Db,
        runs: Arc<AgentRunService>,
        agent_settings: Arc<AgentSettingsService>,
        sharing: Arc<CodeSharingService>,
        artifacts: Arc<RunArtifacts>,
        repositories: Arc<RepositoryService>,
        notes: Arc<NoteService>,
        emitter: ExplanationEmitter,
        run_events: broadcast::Sender<String>,
    ) -> Arc<Self> {
        Arc::new(ExplanationService {
            core,
            history,
            runs,
            agent_settings,
            sharing,
            artifacts,
            repositories,
            notes,
            emitter,
            notifier: Mutex::new(None),
            pull_requests: Mutex::new(None),
            run_events,
            driving: Mutex::new(HashMap::new()),
        })
    }

    pub fn set_notifier(&self, notifier: ExplanationNotifier) {
        *self.notifier.lock().unwrap_or_else(|p| p.into_inner()) = Some(notifier);
    }

    pub fn set_pull_requests(&self, reader: PullRequestReader) {
        *self.pull_requests.lock().unwrap_or_else(|p| p.into_inner()) = Some(reader);
    }

    async fn pull_request(&self, reference: &str) -> AppResult<PullRequestFacts> {
        let reader = self
            .pull_requests
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
            .ok_or_else(|| AppError::dependency("Pull requests are not available."))?;
        reader(reference.to_string()).await
    }

    fn emit(&self, id: &str, repository_id: &str, deleted: bool) {
        (self.emitter)(ExplanationChangedEvent {
            id: id.to_string(),
            repository_id: repository_id.to_string(),
            deleted,
        });
    }

    fn notify(&self, title: String, body: String) {
        let notifier = self
            .notifier
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        if let Some(notifier) = notifier {
            notifier(title, body);
        }
    }

    async fn row(&self, id: &str) -> AppResult<ExplanationRow> {
        let id = id.to_string();
        self.history
            .call(move |conn| store::get(conn, &id))
            .await?
            .ok_or_else(|| AppError::not_found("There is no such explanation."))
    }

    pub async fn settings(&self) -> AppResult<ExplanationSettings> {
        self.core.call(|conn| store::load_settings(conn)).await
    }

    // -----------------------------------------------------------------------
    // Reading
    // -----------------------------------------------------------------------

    pub async fn get(&self, id: &str) -> AppResult<ExplanationRecord> {
        let row = self.row(id).await?;
        Ok(self.present(row).await)
    }

    /// Every explanation of a subject, newest first.
    pub async fn for_subject(
        &self,
        repository_id: &str,
        subject: ExplainSubject,
    ) -> AppResult<Vec<ExplanationRecord>> {
        let rows = self.rows_for(repository_id, &subject).await?;
        let mut records = Vec::with_capacity(rows.len());
        for row in rows {
            records.push(self.present(row).await);
        }
        Ok(records)
    }

    /// A subject's explanations, newest first; then, for a branch or a pull
    /// request, the ready ones of the other with the same changes, so one
    /// explanation serves both (SPEC.md, section 14, Pull requests).
    async fn rows_for(
        &self,
        repository_id: &str,
        subject: &ExplainSubject,
    ) -> AppResult<Vec<ExplanationRow>> {
        let mut rows = {
            let (repository, subject) = (repository_id.to_string(), subject.clone());
            self.history
                .call(move |conn| store::for_subject(conn, &repository, &subject))
                .await?
        };
        if !shares_changes(subject.kind) {
            return Ok(rows);
        }
        let Ok(summary) = self.repositories.summary(repository_id).await else {
            return Ok(rows);
        };
        let root = PathBuf::from(&summary.canonical_root);
        let Ok(range) = self.resolve(repository_id, &root, subject).await else {
            return Ok(rows);
        };
        let repository = repository_id.to_string();
        let shared = self
            .history
            .call(move |conn| store::for_range(conn, &repository, &range.base, &range.tip))
            .await?;
        for row in shared {
            if row.subject != *subject
                && shares_changes(row.subject.kind)
                && row.state == ExplanationState::Ready
                && !rows.iter().any(|r| r.id == row.id)
            {
                rows.push(row);
            }
        }
        Ok(rows)
    }

    /// The row as the panel sees it: a working one with its run's progress.
    async fn present(&self, row: ExplanationRow) -> ExplanationRecord {
        let mut files_read = Vec::new();
        let mut deadline_at = None;
        if row.state == ExplanationState::Working {
            if let Some(run_id) = &row.run_id {
                files_read = self.runs.files_read(run_id).await;
                deadline_at = self.runs.get(run_id).await.ok().and_then(|r| r.deadline_at);
            }
        }
        ExplanationRecord {
            id: row.id,
            repository_id: row.repository_id,
            repository_name: row.repository_name,
            subject: row.subject,
            title: row.title,
            base: row.base,
            tip: row.tip,
            profile_id: row.profile_id,
            agent: row.agent,
            provider: row.provider,
            payment: row.payment,
            host_id: row.host_id,
            host_name: row.host_name,
            model: row.model,
            depth: row.depth,
            questions: row.questions,
            time_limit_minutes: row.time_limit_minutes,
            state: row.state,
            step: row.step,
            error: row.error,
            errors: row.errors,
            run_id: row.run_id,
            files_read,
            cost: row.cost,
            duration_secs: row.duration_secs,
            created_at: row.created_at,
            deadline_at,
            ended_at: row.ended_at,
            explanation: row.explanation,
            hidden: row.hidden,
        }
    }

    fn summary(&self, row: &ExplanationRow) -> ExplanationSummary {
        let trace = row
            .run_id
            .as_ref()
            .and_then(|r| std::fs::metadata(self.artifacts.run_dir(r).join("trace.jsonl")).ok())
            .map_or(0, |m| m.len());
        ExplanationSummary {
            id: row.id.clone(),
            repository_id: row.repository_id.clone(),
            repository_name: row.repository_name.clone(),
            subject: row.subject.clone(),
            title: row.title.clone(),
            profile_id: row.profile_id.clone(),
            agent: row.agent,
            provider: row.provider,
            payment: row.payment,
            model: row.model.clone(),
            depth: row.depth,
            state: row.state,
            created_at: row.created_at.clone(),
            cost: row.cost.clone(),
            duration_secs: row.duration_secs,
            size_bytes: row.explanation_bytes + trace,
        }
    }

    // -----------------------------------------------------------------------
    // The dialog
    // -----------------------------------------------------------------------

    /// The profiles and hosts an explanation can use, each with what it
    /// still lacks.
    fn profile_options(&self, agents: &AgentSettings) -> Vec<ExplainProfileOption> {
        agents
            .profiles
            .iter()
            .map(|profile| ExplainProfileOption {
                profile_id: profile.id.clone(),
                name: profile_name(profile),
                agent: profile.agent,
                provider: profile.provider,
                payment: profile.payment,
                hosts: agents
                    .hosts
                    .iter()
                    .filter(|h| h.approved || h.kind == "local")
                    .map(|host| ExplainHostOption {
                        host_id: host.id.clone(),
                        name: host.name.clone(),
                        missing: run_missing(profile, host),
                        busy: self
                            .runs
                            .host_busy(&host.id)
                            .then(|| "A job is in progress on this host.".to_string()),
                    })
                    .collect(),
            })
            .collect()
    }

    /// Settings' profile and host, else the last run's, else the first
    /// profile ready on this Mac.
    async fn default_agent(
        &self,
        settings: &ExplanationSettings,
        agents: &AgentSettings,
    ) -> (Option<String>, Option<String>) {
        let exists = |p: &str| agents.profiles.iter().any(|x| x.id == p);
        if let Some(profile) = settings.profile_id.as_deref().filter(|p| exists(p)) {
            let host = settings
                .host_id
                .clone()
                .filter(|h| agents.hosts.iter().any(|x| &x.id == h))
                .unwrap_or_else(|| crate::agents::hosts::LOCAL_ID.to_string());
            return (Some(profile.to_string()), Some(host));
        }
        if let Ok(list) = self.runs.list().await {
            if let Some(run) = list.runs.iter().find(|r| exists(&r.profile_id)) {
                return (Some(run.profile_id.clone()), Some(run.host_id.clone()));
            }
        }
        let local = agents
            .hosts
            .iter()
            .find(|h| h.id == crate::agents::hosts::LOCAL_ID);
        let ready = agents
            .profiles
            .iter()
            .find(|p| local.is_some_and(|host| run_missing(p, host).is_empty()));
        match ready.or_else(|| agents.profiles.first()) {
            Some(p) => (
                Some(p.id.clone()),
                Some(crate::agents::hosts::LOCAL_ID.to_string()),
            ),
            None => (None, None),
        }
    }

    /// The model an explanation asks for at a depth.
    fn model_for(
        settings: &ExplanationSettings,
        agents: &AgentSettings,
        profile_id: &str,
        depth: ExplainDepth,
    ) -> String {
        let profile = agents.profiles.iter().find(|p| p.id == profile_id);
        match profile {
            Some(p) if p.agent == AgentKind::ClaudeCode => match depth {
                ExplainDepth::Brief => settings.brief_model.clone(),
                ExplainDepth::TeachMe => settings.teach_me_model.clone(),
                ExplainDepth::Deep => settings.deep_model.clone(),
            },
            Some(p) => p.model.clone(),
            None => String::new(),
        }
    }

    fn minutes_for(settings: &ExplanationSettings, depth: ExplainDepth) -> u32 {
        match depth {
            ExplainDepth::Brief => settings.brief_minutes,
            ExplainDepth::TeachMe => settings.teach_me_minutes,
            ExplainDepth::Deep => settings.deep_minutes,
        }
        .clamp(1, 120)
    }

    /// The Explain dialog's contents for a subject.
    pub async fn dialog(
        &self,
        repository_id: &str,
        subject: ExplainSubject,
    ) -> AppResult<ExplainDialog> {
        let settings = self.settings().await?;
        let agents = self.agent_settings.get().await?;
        let summary = self.repositories.summary(repository_id).await?;
        let resolved = self
            .resolve(
                repository_id,
                &PathBuf::from(&summary.canonical_root),
                &subject,
            )
            .await;
        let pull_request = match subject.kind {
            ExplainSubjectKind::PullRequest => self.pull_request(&subject.reference).await.ok(),
            _ => None,
        };
        let title = match (&resolved, &pull_request) {
            (Ok(r), _) => r.title.clone(),
            (Err(_), Some(pr)) => format!("#{} {}", pr.number, pr.title),
            (Err(_), None) => subject.reference.clone(),
        };
        let blocked = resolved.as_ref().err().map(|e| e.message.clone());
        // A pull request whose head is not here yet is explained after a fetch.
        let fetch_first = matches!(&resolved, Err(e) if e.code == ErrorCode::NotFound)
            && pull_request.as_ref().is_some_and(|pr| !pr.from_fork);
        let head_author = pull_request
            .as_ref()
            .filter(|pr| !pr.mine)
            .map(|pr| pr.author.clone());
        let mut providers: Vec<crate::models::AgentProvider> = Vec::new();
        for p in &agents.profiles {
            if !providers.contains(&p.provider) {
                providers.push(p.provider);
            }
        }
        let question = self.sharing.question(repository_id, &providers).await?;
        let existing: Vec<ExplanationSummary> = self
            .rows_for(repository_id, &subject)
            .await?
            .iter()
            .map(|row| self.summary(row))
            .collect();
        let mut estimates = Vec::new();
        for profile in &agents.profiles {
            for depth in [
                ExplainDepth::Brief,
                ExplainDepth::TeachMe,
                ExplainDepth::Deep,
            ] {
                let model = Self::model_for(&settings, &agents, &profile.id, depth);
                if let Some(estimate) = self.estimate(&profile.id, &model, depth).await? {
                    estimates.push(estimate);
                }
            }
        }
        let (profile_id, host_id) = self.default_agent(&settings, &agents).await;
        Ok(ExplainDialog {
            repository_id: repository_id.to_string(),
            repository_name: summary.name,
            subject,
            title,
            profiles: self.profile_options(&agents),
            profile_id,
            host_id,
            depth: settings.default_depth,
            settings,
            consents: question.consents,
            workspaces: question.workspaces,
            existing,
            estimates,
            blocked,
            fetch_first,
            head_author,
        })
    }

    /// The middle time and cost of the last finished explanations.
    async fn estimate(
        &self,
        profile_id: &str,
        model: &str,
        depth: ExplainDepth,
    ) -> AppResult<Option<ExplainEstimate>> {
        let (profile, model_name) = (profile_id.to_string(), model.to_string());
        let mut usage = self
            .history
            .call(move |conn| {
                store::recent_usage(conn, &profile, &model_name, depth, ESTIMATE_FROM)
            })
            .await?;
        if usage.is_empty() {
            return Ok(None);
        }
        let from = usage.len() as u32;
        let mut durations: Vec<u32> = usage.iter().map(|(d, _)| *d).collect();
        durations.sort_unstable();
        let duration_secs = durations[durations.len() / 2];
        let currency = usage
            .iter()
            .find_map(|(_, c)| c.as_ref().map(|c| c.currency.clone()));
        let cost = currency.and_then(|currency| {
            usage.retain(|(_, c)| c.as_ref().is_some_and(|c| c.currency == currency));
            let mut costs: Vec<u64> = usage
                .iter()
                .filter_map(|(_, c)| c.as_ref().map(|c| c.micros))
                .collect();
            costs.sort_unstable();
            costs
                .get(costs.len() / 2)
                .map(|&micros| ExplainCost { micros, currency })
        });
        Ok(Some(ExplainEstimate {
            profile_id: profile_id.to_string(),
            depth,
            duration_secs,
            cost,
            from,
        }))
    }

    // -----------------------------------------------------------------------
    // Explain
    // -----------------------------------------------------------------------

    /// What a subject is in the repository: its range, title, and start.
    async fn resolve(
        &self,
        repository_id: &str,
        root: &Path,
        subject: &ExplainSubject,
    ) -> AppResult<Resolved> {
        let git = |args: Vec<String>| {
            let root = root.to_path_buf();
            async move {
                let args: Vec<&str> = args.iter().map(String::as_str).collect();
                self.artifacts
                    .read_git(&root, &args)
                    .await
                    .map(|out| out.trim().to_string())
            }
        };
        match subject.kind {
            ExplainSubjectKind::Commit => {
                crate::git::validate_revision(&subject.reference)?;
                let tip = git(vec![
                    "rev-parse".into(),
                    "--verify".into(),
                    "--end-of-options".into(),
                    format!("{}^{{commit}}", subject.reference),
                ])
                .await
                .map_err(|_| AppError::not_found("That commit is not in the repository."))?;
                let parents = git(vec![
                    "rev-list".into(),
                    "--parents".into(),
                    "-n".into(),
                    "1".into(),
                    "--end-of-options".into(),
                    tip.clone(),
                    "--".into(),
                ])
                .await?;
                let base = parents.split_whitespace().nth(1).map(str::to_string);
                let title = git(vec![
                    "log".into(),
                    "-1".into(),
                    "--no-show-signature".into(),
                    "--format=%s".into(),
                    tip.clone(),
                    "--".into(),
                ])
                .await?;
                Ok(Resolved {
                    root_commit: base.is_none(),
                    base: base.unwrap_or_else(|| EMPTY_TREE.to_string()),
                    what: format!("commit {}", &tip[..tip.len().min(12)]),
                    start: ExplainStart::Commit(tip.clone()),
                    tip,
                    title,
                })
            }
            ExplainSubjectKind::Branch => {
                let name = subject.reference.as_str();
                if !(name.starts_with("refs/heads/") || name.starts_with("refs/remotes/")) {
                    return Err(AppError::validation("Explain a branch by its full name."));
                }
                crate::git::validate_revision(name)?;
                let tip = git(vec![
                    "rev-parse".into(),
                    "--verify".into(),
                    "--end-of-options".into(),
                    format!("{name}^{{commit}}"),
                ])
                .await
                .map_err(|_| AppError::not_found("That branch is gone."))?;
                let base_ref = self.default_branch(root).await?;
                if base_ref == name {
                    return Err(AppError::validation(
                        "This is the default branch: explain one of its commits instead.",
                    ));
                }
                let base = git(vec![
                    "merge-base".into(),
                    "--end-of-options".into(),
                    base_ref.clone(),
                    tip.clone(),
                ])
                .await
                .map_err(|_| {
                    AppError::validation("This branch shares no history with the default branch.")
                })?;
                if base == tip {
                    return Err(AppError::validation(
                        "This branch has no changes against the default branch.",
                    ));
                }
                let short = short_ref(name);
                Ok(Resolved {
                    what: format!(
                        "the branch {short}, compared with the default branch {}",
                        short_ref(&base_ref)
                    ),
                    title: short.to_string(),
                    start: ExplainStart::Commit(tip.clone()),
                    base,
                    tip,
                    root_commit: false,
                })
            }
            ExplainSubjectKind::PullRequest => {
                let pr = self.pull_request(&subject.reference).await?;
                if pr.repository_id != repository_id {
                    return Err(AppError::validation(
                        "That pull request is of another repository.",
                    ));
                }
                if pr.from_fork {
                    return Err(AppError::validation(FROM_FORK));
                }
                // The head and the target's tip must both be here: the
                // agent copies commits from this repository.
                let local = |sha: String| {
                    let git = &git;
                    async move {
                        crate::git::validate_revision(&sha)?;
                        git(vec![
                            "rev-parse".into(),
                            "--verify".into(),
                            "--end-of-options".into(),
                            format!("{sha}^{{commit}}"),
                        ])
                        .await
                        .map_err(|_| {
                            AppError::not_found(format!(
                                "Commit {} of the pull request is not on this Mac yet: fetch, then Explain.",
                                &sha[..sha.len().min(7)]
                            ))
                        })
                    }
                };
                let tip = local(pr.head_sha.clone()).await?;
                let target = local(pr.base_sha.clone()).await?;
                let base = git(vec![
                    "merge-base".into(),
                    "--end-of-options".into(),
                    target,
                    tip.clone(),
                ])
                .await
                .map_err(|_| {
                    AppError::validation(format!(
                        "This pull request shares no history with {}.",
                        pr.target_branch
                    ))
                })?;
                if base == tip {
                    return Err(AppError::validation(format!(
                        "This pull request has no changes against {}.",
                        pr.target_branch
                    )));
                }
                Ok(Resolved {
                    what: format!(
                        "pull request #{}, the branch {} to be merged into {}, compared with {} from their merge base",
                        pr.number, pr.source_branch, pr.target_branch, pr.target_branch
                    ),
                    title: format!("#{} {}", pr.number, pr.title),
                    start: ExplainStart::Commit(tip.clone()),
                    base,
                    tip,
                    root_commit: false,
                })
            }
            ExplainSubjectKind::Run => {
                let run = self.runs.get(&subject.reference).await?;
                if run.repository_id != repository_id {
                    return Err(AppError::validation("That run is of another repository."));
                }
                if run.collection != RunCollection::Ready {
                    return Err(AppError::validation(
                        "Only a collected run's result can be explained.",
                    ));
                }
                let tip = run
                    .result_commit
                    .clone()
                    .ok_or_else(|| AppError::validation("The run has no result to explain."))?;
                if tip == run.start_commit {
                    return Err(AppError::validation("The run changed nothing."));
                }
                Ok(Resolved {
                    what: "an agent run's result (one commit on top of its start)".to_string(),
                    title: run.title.clone(),
                    start: ExplainStart::RunResult(tip.clone()),
                    base: run.start_commit,
                    tip,
                    root_commit: false,
                })
            }
        }
    }

    /// The default branch's full ref name, as the branch comparison uses it.
    async fn default_branch(&self, root: &Path) -> AppResult<String> {
        let git = self
            .repositories
            .git_service()
            .ok_or_else(|| AppError::dependency("Git was not found."))?;
        let (_, base) = git.list_refs_with_base(root).await?;
        base.ok_or_else(|| {
            AppError::validation("This repository has no default branch to compare with.")
        })
    }

    /// **Explain**: start an explanation, replacing one of the same subject,
    /// agent, and depth.
    pub async fn start(
        self: &Arc<Self>,
        request: StartExplanationRequest,
    ) -> AppResult<ExplanationRecord> {
        let settings = self.settings().await?;
        let agents = self.agent_settings.get().await?;
        let profile = agents
            .profiles
            .iter()
            .find(|p| p.id == request.profile_id)
            .cloned()
            .ok_or_else(|| AppError::validation("Choose an agent for the explanation."))?;
        let host = agents
            .hosts
            .iter()
            .find(|h| h.id == request.host_id)
            .cloned()
            .ok_or_else(|| AppError::validation("Choose a host for the explanation."))?;
        let summary = self.repositories.summary(&request.repository_id).await?;
        // The repository's answer (SPEC.md, section 13, Code sharing), before
        // anything is copied; the dialog asked when there was none.
        self.sharing
            .require(&request.repository_id, profile.provider)
            .await?;
        let root = PathBuf::from(&summary.canonical_root);
        let resolved = self
            .resolve(&request.repository_id, &root, &request.subject)
            .await?;

        // One at a time per subject; Explain again replaces the same one.
        let (repository, subject) = (request.repository_id.clone(), request.subject.clone());
        let others = self
            .history
            .call(move |conn| store::for_subject(conn, &repository, &subject))
            .await?;
        if others.iter().any(|o| o.state == ExplanationState::Working) {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "This is being explained already.",
            ));
        }
        if let Some(same) = others
            .iter()
            .find(|o| o.profile_id == profile.id && o.depth == request.depth)
        {
            self.remove(same).await?;
        }

        let model = Self::model_for(&settings, &agents, &profile.id, request.depth);
        let minutes = Self::minutes_for(&settings, request.depth);
        let known = {
            let repository = request.repository_id.clone();
            let refs = self
                .core
                .call(move |conn| store::known_refs(conn, &repository))
                .await?;
            known::file(&refs)
        };
        let text = prompt::prompt(
            &PromptSubject {
                what: &resolved.what,
                base: &resolved.base,
                tip: &resolved.tip,
                root: resolved.root_commit,
            },
            &Reader {
                levels: &settings.levels,
                known_count: known.as_ref().map_or(0, |k| k.count),
            },
            request.depth,
            request.questions,
        );
        let id = uuid::Uuid::new_v4().to_string();
        let row = ExplanationRow {
            id: id.clone(),
            repository_id: request.repository_id.clone(),
            repository_name: summary.name.clone(),
            subject: request.subject.clone(),
            title: resolved.title.clone(),
            base: resolved.base.clone(),
            tip: resolved.tip.clone(),
            profile_id: profile.id.clone(),
            agent: profile.agent,
            provider: profile.provider,
            payment: profile.payment,
            host_id: host.id.clone(),
            host_name: host.name.clone(),
            model: model.clone(),
            depth: request.depth,
            questions: request.questions,
            time_limit_minutes: minutes,
            state: ExplanationState::Working,
            step: Some(ExplainStep::Copying),
            error: None,
            errors: Vec::new(),
            run_id: None,
            explanation: None,
            hidden: Vec::new(),
            cost: None,
            duration_secs: None,
            created_at: now_rfc3339(),
            ended_at: None,
            explanation_bytes: 0,
        };
        self.history
            .call({
                let row = row.clone();
                move |conn| store::insert(conn, &row)
            })
            .await?;
        // A start the run service refuses (Settings, the repository, the
        // host) leaves nothing behind and answers in the dialog.
        let started = self
            .runs
            .start_explain(ExplainRun {
                repository_id: request.repository_id.clone(),
                profile_id: profile.id.clone(),
                host_id: host.id.clone(),
                start: resolved.start,
                prompt: text,
                title: format!("Explain {}", resolved.title),
                model,
                time_limit_minutes: minutes,
                known_concepts: known.map(|k| k.text),
            })
            .await;
        let run_id = match started {
            Ok(run_id) => run_id,
            Err(e) => {
                let gone = id.clone();
                let _ = self
                    .history
                    .call(move |conn| store::delete(conn, &gone))
                    .await;
                return Err(e);
            }
        };
        {
            let (id, run_id) = (id.clone(), run_id.clone());
            self.history
                .call(move |conn| store::set_run(conn, &id, &run_id))
                .await?;
        }
        self.spawn_drive(&id);
        self.emit(&id, &request.repository_id, false);
        self.get(&id).await
    }

    /// **Cancel** a working explanation; it ends cancelled once its run does.
    pub async fn cancel(&self, id: &str) -> AppResult<ExplanationRecord> {
        let row = self.row(id).await?;
        if row.state != ExplanationState::Working {
            return self.get(id).await;
        }
        if let Some(run_id) = &row.run_id {
            self.runs.cancel(run_id).await?;
        }
        self.get(id).await
    }

    // -----------------------------------------------------------------------
    // Following the explain run
    // -----------------------------------------------------------------------

    /// At launch: follow every explanation left working.
    pub async fn reconnect(self: &Arc<Self>) {
        let rows = match self.history.call(store::list).await {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!(error = %e, "the explanations could not be listed");
                return;
            }
        };
        for row in rows {
            if row.state != ExplanationState::Working {
                continue;
            }
            if row.run_id.is_none() {
                let ending = Ending {
                    state: ExplanationState::Failed,
                    error: Some("Brainiac quit before the explanation started.".into()),
                    errors: Vec::new(),
                    explanation: None,
                    cost: None,
                    duration_secs: None,
                };
                let id = row.id.clone();
                let _ = self
                    .history
                    .call(move |conn| store::end(conn, &id, &ending))
                    .await;
                continue;
            }
            self.spawn_drive(&row.id);
        }
    }

    fn spawn_drive(self: &Arc<Self>, id: &str) {
        let mut driving = self.driving.lock().unwrap_or_else(|p| p.into_inner());
        if driving.get(id).is_some_and(|t| !t.is_finished()) {
            return;
        }
        let service = Arc::clone(self);
        let explanation = id.to_string();
        driving.insert(
            id.to_string(),
            tokio::spawn(async move { service.drive(&explanation).await }),
        );
    }

    /// Follow one explanation's run until the explanation ended and the run
    /// did too, so its time and cost are final.
    async fn drive(self: Arc<Self>, id: &str) {
        let mut events = self.run_events.subscribe();
        // The turn last checked, and whether the follow-up turn was sent: a
        // follow-up's prompt makes turn 2, so a restart reads the turn count.
        let mut checked_turn = 0u32;
        let mut followed_up = false;
        let mut last_files = 0usize;
        loop {
            let Ok(row) = self.row(id).await else {
                return;
            };
            let Some(run_id) = row.run_id.clone() else {
                return;
            };
            let run = match self.runs.get(&run_id).await {
                Ok(run) => run,
                Err(e) if e.code == ErrorCode::NotFound => {
                    self.finish(&row, failed("The explain run is gone.", Vec::new()), None)
                        .await;
                    return;
                }
                Err(_) => {
                    self.wait(&mut events, &run_id).await;
                    continue;
                }
            };
            if row.state == ExplanationState::Working {
                if run.turn >= 2 {
                    followed_up = true;
                }
                if let Some(step) = step_of(&run, followed_up) {
                    let id_ = id.to_string();
                    let changed = self
                        .history
                        .call(move |conn| store::set_step(conn, &id_, step))
                        .await
                        .unwrap_or(false);
                    let files = self.runs.files_read(&run_id).await.len();
                    if changed || files != last_files {
                        last_files = files;
                        self.emit(id, &row.repository_id, false);
                    }
                }
                if run.phase == RunPhase::Running
                    && run.activity == RunActivity::Idle
                    && run.turn > checked_turn
                {
                    checked_turn = run.turn;
                    let attempt = if followed_up {
                        Attempt::Last
                    } else {
                        Attempt::First
                    };
                    match self.check_turn(&row, &run_id, attempt).await {
                        Ok(Some(ending)) => {
                            self.finish(&row, ending, Some(&run)).await;
                            let _ = self.runs.cancel(&run_id).await;
                        }
                        Ok(None) => followed_up = true,
                        Err(e) => {
                            self.finish(&row, failed(&e.message, Vec::new()), Some(&run))
                                .await;
                            let _ = self.runs.cancel(&run_id).await;
                        }
                    }
                    continue;
                }
                if run.activity == RunActivity::PlanLimit {
                    self.finish(
                        &row,
                        failed(
                            "Plan limit reached. Brainiac does not know when it resets; try again later.",
                            Vec::new(),
                        ),
                        Some(&run),
                    )
                    .await;
                    let _ = self.runs.cancel(&run_id).await;
                    continue;
                }
                if run.phase == RunPhase::Ended {
                    let ending = ended_early(&run, row.time_limit_minutes);
                    self.finish(&row, ending, Some(&run)).await;
                    continue;
                }
            } else if run.phase == RunPhase::Ended {
                // The explanation ended before its run: its time and cost
                // are final now.
                let cost = self.cost_of(&run_id).await;
                let duration = duration_of(&run);
                let id_ = id.to_string();
                let _ = self
                    .history
                    .call(move |conn| store::set_usage(conn, &id_, cost.as_ref(), duration))
                    .await;
                self.emit(id, &row.repository_id, false);
                return;
            }
            self.wait(&mut events, &run_id).await;
        }
    }

    /// Wait for the run's next change, or a while.
    async fn wait(&self, events: &mut broadcast::Receiver<String>, run_id: &str) {
        let deadline = tokio::time::Instant::now() + LOOK_AGAIN;
        loop {
            match tokio::time::timeout_at(deadline, events.recv()).await {
                Ok(Ok(changed)) if changed == run_id => return,
                Ok(Ok(_)) => continue,
                // Missed events: look now.
                Ok(Err(broadcast::error::RecvError::Lagged(_))) => return,
                Ok(Err(broadcast::error::RecvError::Closed)) => {
                    tokio::time::sleep_until(deadline).await;
                    return;
                }
                Err(_) => return,
            }
        }
    }

    /// Read and check the file the turn wrote. `Ok(None)`: the follow-up
    /// turn was sent.
    async fn check_turn(
        &self,
        row: &ExplanationRow,
        run_id: &str,
        attempt: Attempt,
    ) -> AppResult<Option<Ending>> {
        {
            let id = row.id.clone();
            let _ = self
                .history
                .call(move |conn| store::set_step(conn, &id, ExplainStep::Checking))
                .await;
            self.emit(&row.id, &row.repository_id, false);
        }
        let text = self.runs.read_explanation(run_id).await?;
        let result = match text {
            None => Err(check::Failed {
                errors: vec![
                    "There is no .brainiac/explanation.json: write it as the prompt asked."
                        .to_string(),
                ],
            }),
            Some(text) => match check::parse(&text) {
                Err(failed) => Err(failed),
                Ok(draft) => {
                    let repo = self.artifacts.repository_dir(&row.repository_id);
                    let subject = self
                        .artifacts
                        .read_subject(&repo, &row.base, &row.tip)
                        .await?;
                    // Only cited files are read: the cap on files read must
                    // not cut a cited one among a large change's others.
                    let paths = draft.cited_paths();
                    let files = self.artifacts.read_files(&repo, &row.tip, &paths).await?;
                    // The ledger as it is now: a concept forgotten while the
                    // agent worked is not counted as left out.
                    let known = {
                        let repository = row.repository_id.clone();
                        self.core
                            .call(move |conn| store::known_refs(conn, &repository))
                            .await?
                    };
                    check::check(draft, &subject, &files, &known, attempt)
                }
            },
        };
        match (result, attempt) {
            (Ok(checked), _) => Ok(Some(Ending {
                state: ExplanationState::Ready,
                error: None,
                errors: Vec::new(),
                explanation: Some(checked.explanation),
                cost: None,
                duration_secs: None,
            })),
            (Err(failed), Attempt::First) => {
                self.runs
                    .prompt(run_id, &prompt::follow_up(&failed.errors))
                    .await?;
                let id = row.id.clone();
                let _ = self
                    .history
                    .call(move |conn| store::set_step(conn, &id, ExplainStep::FollowUp))
                    .await;
                self.emit(&row.id, &row.repository_id, false);
                Ok(None)
            }
            (Err(failed), Attempt::Last) => Ok(Some(failed_with(
                "The agent's file did not match the schema after a second turn.",
                failed.errors,
            ))),
        }
    }

    async fn cost_of(&self, run_id: &str) -> Option<ExplainCost> {
        self.runs
            .reported_cost(run_id)
            .await
            .map(|(micros, currency)| ExplainCost { micros, currency })
    }

    /// End a working explanation with its time and cost so far, and say so.
    async fn finish(&self, row: &ExplanationRow, mut ending: Ending, run: Option<&AgentRun>) {
        if let Some(run_id) = &row.run_id {
            ending.cost = self.cost_of(run_id).await;
        }
        ending.duration_secs = run.and_then(duration_of).or_else(|| {
            chrono::DateTime::parse_from_rfc3339(&row.created_at)
                .ok()
                .map(|t| {
                    (chrono::Utc::now() - t.with_timezone(&chrono::Utc))
                        .num_seconds()
                        .max(0) as u32
                })
        });
        let state = ending.state;
        let id = row.id.clone();
        let ended = self
            .history
            .call(move |conn| store::end(conn, &id, &ending))
            .await
            .unwrap_or(false);
        if !ended {
            return;
        }
        self.emit(&row.id, &row.repository_id, false);
        let what = format!("{} · {}", row.repository_name, row.title);
        match state {
            ExplanationState::Ready => self.notify("Explanation ready".into(), what),
            ExplanationState::Failed => self.notify("Explanation failed".into(), what),
            _ => {}
        }
    }

    // -----------------------------------------------------------------------
    // The panel
    // -----------------------------------------------------------------------

    /// Remove an explanation's row and its run.
    async fn remove(&self, row: &ExplanationRow) -> AppResult<()> {
        if row.state == ExplanationState::Working {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "The explanation is still being written. Cancel it first.",
            ));
        }
        if let Some(run_id) = &row.run_id {
            match self.runs.delete(run_id).await {
                Ok(()) => {}
                Err(e) if e.code == ErrorCode::NotFound => {}
                Err(e) => return Err(e),
            }
        }
        let id = row.id.clone();
        self.history
            .call(move |conn| store::delete(conn, &id))
            .await?;
        self.emit(&row.id, &row.repository_id, true);
        Ok(())
    }

    /// **Delete explanation…**
    pub async fn delete(&self, id: &str) -> AppResult<()> {
        let row = self.row(id).await?;
        self.remove(&row).await
    }

    /// **Delete all** in Settings; one still being written is left.
    pub async fn delete_all(&self) -> AppResult<u32> {
        let rows = self.history.call(store::list).await?;
        let mut deleted = 0;
        for row in rows {
            if row.state == ExplanationState::Working {
                continue;
            }
            self.remove(&row).await?;
            deleted += 1;
        }
        Ok(deleted)
    }

    /// **Not a problem** on a disagreement, or **Show** to bring it back.
    pub async fn set_hidden(
        &self,
        id: &str,
        index: u32,
        hidden: bool,
    ) -> AppResult<ExplanationRecord> {
        let row = self.row(id).await?;
        let mut list = row.hidden.clone();
        list.retain(|&i| i != index);
        if hidden {
            list.push(index);
            list.sort_unstable();
        }
        let id_ = id.to_string();
        self.history
            .call(move |conn| store::set_hidden(conn, &id_, &list))
            .await?;
        self.emit(id, &row.repository_id, false);
        self.get(id).await
    }

    /// **I know this**: a concept goes into the ledger; a project pattern
    /// with its repository. Given the explanation it is in, it keeps that
    /// explanation's words for it and where it was learned. Returns the
    /// concept's ID, for Undo.
    pub async fn learn(
        &self,
        repository_id: &str,
        kind: ConceptKind,
        name: &str,
        explanation_id: Option<&str>,
    ) -> AppResult<String> {
        let mut origin = store::ConceptOrigin {
            learned_in: repository_id.to_string(),
            ..Default::default()
        };
        if let Some(explanation_id) = explanation_id {
            let row = self.row(explanation_id).await?;
            origin.learned_from = subject_short(&row);
            origin.explanation_id = Some(row.id.clone());
            origin.description = row
                .explanation
                .as_ref()
                .and_then(|e| e.concepts.iter().find(|c| c.kind == kind && c.name == name))
                .map(|c| c.explanation.clone())
                .unwrap_or_default();
        }
        let (repository, name) = (repository_id.to_string(), name.to_string());
        let id = self
            .core
            .call(move |conn| store::add_concept(conn, kind, &name, Some(&repository), &origin))
            .await?;
        self.emit("", repository_id, false);
        Ok(id)
    }

    /// **Undo** or **Remove**.
    pub async fn forget_concept(&self, id: &str) -> AppResult<()> {
        let id = id.to_string();
        self.core
            .call(move |conn| store::remove_concept(conn, &id))
            .await?;
        self.emit("", "", false);
        Ok(())
    }

    /// **Edit** a concept's name and kind; its old name stays as a name that
    /// stands for it.
    pub async fn edit_concept(&self, id: &str, name: &str, kind: ConceptKind) -> AppResult<()> {
        let (id, name) = (id.to_string(), name.to_string());
        self.core
            .call(move |conn| store::edit_concept(conn, &id, &name, kind))
            .await?;
        self.emit("", "", false);
        Ok(())
    }

    /// **Merge** one concept into another.
    pub async fn merge_concept(&self, from: &str, into: &str) -> AppResult<()> {
        let (from, into) = (from.to_string(), into.to_string());
        self.core
            .call(move |conn| store::merge_concept(conn, &from, &into))
            .await?;
        self.emit("", "", false);
        Ok(())
    }

    /// Where a branch explanation's notes are on the branch now. A commit's
    /// or a run's never moves.
    pub async fn placement(&self, id: &str) -> AppResult<ExplanationPlacement> {
        let row = self.row(id).await?;
        let explanation = row
            .explanation
            .clone()
            .ok_or_else(|| AppError::validation("The explanation is not ready."))?;
        let as_written = |moved: bool, gone: bool, tip: String| ExplanationPlacement {
            explanation_id: row.id.clone(),
            tip,
            moved,
            gone,
            notes: explanation
                .notes
                .iter()
                .enumerate()
                .map(|(i, n)| NotePlacement {
                    index: i as u32,
                    start: n.start,
                    end: n.end,
                    out_of_date: moved,
                })
                .collect(),
            uncovered: Vec::new(),
        };
        if !shares_changes(row.subject.kind) {
            return Ok(as_written(false, false, row.tip.clone()));
        }
        let summary = self.repositories.summary(&row.repository_id).await?;
        let root = PathBuf::from(&summary.canonical_root);
        // Where the branch or the pull request is now.
        let now = match self.resolve(&row.repository_id, &root, &row.subject).await {
            Ok(now) => now,
            // A pull request's new head that is not on this Mac yet: its
            // notes cannot be placed, so all are out of date until a fetch.
            Err(e)
                if row.subject.kind == ExplainSubjectKind::PullRequest
                    && e.code == ErrorCode::NotFound =>
            {
                return Ok(as_written(true, false, row.tip.clone()));
            }
            // Deleted, merged, or no longer comparable.
            Err(_) => return Ok(as_written(true, true, row.tip.clone())),
        };
        if now.tip == row.tip {
            return Ok(as_written(false, false, now.tip));
        }
        let (base, tip) = (now.base, now.tip);
        let subject = self.artifacts.read_subject(&root, &base, &tip).await?;
        let paths: BTreeSet<String> = explanation.notes.iter().map(|n| n.path.clone()).collect();
        let files = self.artifacts.read_files(&root, &tip, &paths).await?;
        let notes = explanation
            .notes
            .iter()
            .enumerate()
            .map(|(i, note)| {
                let found = subject.file(&note.path).and_then(|file| {
                    let text = files.get(&note.path)?;
                    find_lines(
                        text,
                        &note.lines_hash,
                        note.start,
                        note.end - note.start + 1,
                    )
                    .filter(|(s, e)| file.changed.iter().any(|c| c.touches(*s, *e)))
                });
                match found {
                    Some((start, end)) => NotePlacement {
                        index: i as u32,
                        start,
                        end,
                        out_of_date: false,
                    },
                    None => NotePlacement {
                        index: i as u32,
                        start: note.start,
                        end: note.end,
                        out_of_date: true,
                    },
                }
            })
            .collect();
        let uncovered = subject
            .files
            .iter()
            .filter(|f| !explanation.tour.iter().any(|s| s.path == f.path))
            .map(|f| f.path.clone())
            .collect();
        Ok(ExplanationPlacement {
            explanation_id: row.id,
            tip,
            moved: true,
            gone: false,
            notes,
            uncovered,
        })
    }

    /// **Save as note**: a copy in the vault, linked to the repository.
    pub async fn save_as_note(&self, id: &str) -> AppResult<NoteSummary> {
        let row = self.row(id).await?;
        let explanation = row
            .explanation
            .clone()
            .ok_or_else(|| AppError::validation("The explanation is not ready."))?;
        let short = match row.subject.kind {
            ExplainSubjectKind::Commit => row.tip[..row.tip.len().min(8)].to_string(),
            // A file name cannot hold a slash.
            ExplainSubjectKind::Branch => short_ref(&row.subject.reference).replace('/', "-"),
            ExplainSubjectKind::Run => format!("run {}", &row.tip[..row.tip.len().min(8)]),
            ExplainSubjectKind::PullRequest => {
                format!(
                    "#{}",
                    row.subject.reference.rsplit('#').next().unwrap_or_default()
                )
            }
        };
        // A pull request's title already starts with its number.
        let subject_title = row
            .title
            .strip_prefix(&format!("{short} "))
            .unwrap_or(&row.title);
        let title = format!("{short} — {subject_title}");
        let folder = format!("Explanations/{}", sanitize_folder(&row.repository_name));
        let subject_word = match row.subject.kind {
            ExplainSubjectKind::Commit => "commit",
            ExplainSubjectKind::Branch => "branch",
            ExplainSubjectKind::Run => "run",
            ExplainSubjectKind::PullRequest => "pull request",
        };
        let frontmatter = vec![
            ("repository".to_string(), row.repository_name.clone()),
            (
                "subject".to_string(),
                format!("{subject_word} {}", row.subject.reference),
            ),
            ("tip".to_string(), row.tip.clone()),
            (
                "date".to_string(),
                row.ended_at.clone().unwrap_or(row.created_at.clone()),
            ),
            ("agent".to_string(), agent_word(row.agent).to_string()),
            ("model".to_string(), row.model.clone()),
        ];
        let body = super::markdown::render(&title, &explanation);
        self.notes
            .create_with_body(
                CreateNoteRequest {
                    folder: Some(folder),
                    title: Some(title),
                    repository_id: Some(row.repository_id.clone()),
                },
                frontmatter,
                body,
            )
            .await
    }

    // -----------------------------------------------------------------------
    // Settings → Explanations
    // -----------------------------------------------------------------------

    pub async fn settings_view(&self) -> AppResult<ExplanationSettingsView> {
        let settings = self.settings().await?;
        let agents = self.agent_settings.get().await?;
        let (mut concepts, repository_names) = self
            .core
            .call(|conn| {
                let concepts = store::concepts(conn)?;
                let repositories: HashMap<String, String> = db::list_repositories(conn)?
                    .into_iter()
                    .map(|r| {
                        let name = Path::new(&r.display_path)
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or(r.display_path.clone());
                        (r.id, name)
                    })
                    .collect();
                Ok((concepts, repositories))
            })
            .await?;
        for concept in &mut concepts {
            concept.repository_name = concept
                .repository_id
                .as_ref()
                .and_then(|r| repository_names.get(r).cloned());
            concept.learned_in_name = concept
                .learned_in
                .as_ref()
                .and_then(|r| repository_names.get(r).cloned());
        }
        let rows = self.history.call(store::list).await?;
        let stored: Vec<ExplanationSummary> = rows.iter().map(|r| self.summary(r)).collect();
        let stored_bytes = stored.iter().map(|s| s.size_bytes).sum();
        let mut stored_cost: Vec<ExplainCost> = Vec::new();
        for cost in stored.iter().filter_map(|s| s.cost.as_ref()) {
            match stored_cost.iter_mut().find(|c| c.currency == cost.currency) {
                Some(total) => total.micros += cost.micros,
                None => stored_cost.push(cost.clone()),
            }
        }
        Ok(ExplanationSettingsView {
            settings,
            profiles: self.profile_options(&agents),
            concepts,
            stored,
            stored_bytes,
            stored_cost,
        })
    }

    pub async fn save_settings(&self, settings: ExplanationSettings) -> AppResult<()> {
        for minutes in [
            settings.brief_minutes,
            settings.teach_me_minutes,
            settings.deep_minutes,
        ] {
            if !(1..=120).contains(&minutes) {
                return Err(AppError::validation(
                    "A time limit is between 1 and 120 minutes.",
                ));
            }
        }
        for model in [
            &settings.brief_model,
            &settings.teach_me_model,
            &settings.deep_model,
        ] {
            crate::agents::settings::check_model(AgentKind::ClaudeCode, model)?;
        }
        self.core
            .call(move |conn| store::save_settings(conn, &settings))
            .await
    }
}

/// `refs/heads/x` → `x`; `refs/remotes/origin/x` → `origin/x`.
/// A branch and a pull request can have the same changes, and then share
/// one explanation; a commit's or a run's are its own.
fn shares_changes(kind: ExplainSubjectKind) -> bool {
    matches!(
        kind,
        ExplainSubjectKind::Branch | ExplainSubjectKind::PullRequest
    )
}

fn short_ref(name: &str) -> &str {
    name.strip_prefix("refs/heads/")
        .or_else(|| name.strip_prefix("refs/remotes/"))
        .unwrap_or(name)
}

fn agent_word(agent: AgentKind) -> &'static str {
    match agent {
        AgentKind::ClaudeCode => "Claude Code",
        AgentKind::Opencode => "OpenCode",
    }
}

/// A vault folder name from a repository's name.
fn sanitize_folder(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c == '/' || c == '\\' || c == ':' {
                '-'
            } else {
                c
            }
        })
        .collect();
    let cleaned = cleaned.trim().trim_start_matches('.').to_string();
    if cleaned.is_empty() {
        "repository".into()
    } else {
        cleaned
    }
}

fn failed(message: &str, errors: Vec<String>) -> Ending {
    failed_with(message, errors)
}

fn failed_with(message: &str, errors: Vec<String>) -> Ending {
    Ending {
        state: ExplanationState::Failed,
        error: Some(message.to_string()),
        errors,
        explanation: None,
        cost: None,
        duration_secs: None,
    }
}

/// The step a working explanation is on, from its run.
fn step_of(run: &AgentRun, followed_up: bool) -> Option<ExplainStep> {
    match run.phase {
        RunPhase::Preparing => Some(match run.starting {
            Some(RunStartStep::Copy) | Some(RunStartStep::Send) => ExplainStep::Copying,
            _ => ExplainStep::Starting,
        }),
        RunPhase::Running => match run.activity {
            RunActivity::Working | RunActivity::Permission if followed_up => {
                Some(ExplainStep::FollowUp)
            }
            RunActivity::Working | RunActivity::Permission => Some(ExplainStep::Reading),
            RunActivity::Preparing => Some(ExplainStep::Starting),
            _ => None,
        },
        _ => None,
    }
}

/// Why a run that ended before a checked file ended the explanation.
fn ended_early(run: &AgentRun, minutes: u32) -> Ending {
    match run.outcome {
        Some(RunOutcome::Cancelled) => Ending {
            state: ExplanationState::Cancelled,
            error: None,
            errors: Vec::new(),
            explanation: None,
            cost: None,
            duration_secs: None,
        },
        Some(RunOutcome::Expired) if run.expired_asleep => {
            failed("Expired while this Mac slept.", Vec::new())
        }
        Some(RunOutcome::Expired) => failed(
            &format!("It took longer than the time limit of {minutes} minutes."),
            Vec::new(),
        ),
        Some(RunOutcome::Interrupted) => failed(
            "The explain run was interrupted: the engine, its container, or the run controller stopped.",
            Vec::new(),
        ),
        _ => failed(
            run.error
                .as_deref()
                .unwrap_or("The agent stopped before writing an explanation that passed the checks."),
            Vec::new(),
        ),
    }
}

/// How long the run took, from its acceptance to its end (or now).
fn duration_of(run: &AgentRun) -> Option<u32> {
    let parse = |t: &str| {
        chrono::DateTime::parse_from_rfc3339(t)
            .ok()
            .map(|t| t.with_timezone(&chrono::Utc))
    };
    let start = run.accepted_at.as_deref().and_then(parse)?;
    let end = run
        .ended_at
        .as_deref()
        .and_then(parse)
        .unwrap_or_else(chrono::Utc::now);
    Some((end - start).num_seconds().max(0) as u32)
}

/// The span of `count` lines whose hash is `hash`, nearest to `near`.
fn find_lines(text: &str, hash: &str, near: u32, count: u32) -> Option<(u32, u32)> {
    let text = text.replace("\r\n", "\n");
    let lines: Vec<&str> = text.split('\n').collect();
    // A file that ends with a line break has no last, empty line.
    let len = if text.ends_with('\n') {
        lines.len() - 1
    } else {
        lines.len()
    };
    let count = count as usize;
    if count == 0 || count > len {
        return None;
    }
    let mut best: Option<(u32, u32)> = None;
    for start in 0..=(len - count) {
        let span = lines[start..start + count].join("\n");
        if format!("{:x}", Sha256::digest(span.as_bytes())) == hash {
            let candidate = (start as u32 + 1, (start + count) as u32);
            let distance = |s: u32| s.abs_diff(near);
            if best.is_none_or(|b| distance(candidate.0) < distance(b.0)) {
                best = Some(candidate);
            }
        }
    }
    best
}

/// A subject's short label, for where a concept was learned: a commit's
/// short hash, a branch's name, a pull request's number, or a run's.
fn subject_short(row: &ExplanationRow) -> String {
    let short = |s: &str| s[..s.len().min(7)].to_string();
    match row.subject.kind {
        ExplainSubjectKind::Commit => short(&row.tip),
        ExplainSubjectKind::Branch => short_ref(&row.subject.reference).to_string(),
        ExplainSubjectKind::Run => format!(
            "run {}",
            &row.subject.reference[..row.subject.reference.len().min(8)]
        ),
        ExplainSubjectKind::PullRequest => format!(
            "#{}",
            row.subject.reference.rsplit('#').next().unwrap_or_default()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_are_found_again_by_their_hash_nearest_where_they_were() {
        let text = "a\nb\nc\nb\nc\n";
        let hash = format!("{:x}", Sha256::digest("b\nc".as_bytes()));
        assert_eq!(find_lines(text, &hash, 4, 2), Some((4, 5)));
        assert_eq!(find_lines(text, &hash, 1, 2), Some((2, 3)));
        assert_eq!(find_lines(text, "nope", 1, 2), None);
        assert_eq!(find_lines("x", &hash, 1, 3), None);
    }

    #[test]
    fn refs_are_shortened_and_folders_cleaned() {
        assert_eq!(short_ref("refs/heads/feature/x"), "feature/x");
        assert_eq!(short_ref("refs/remotes/origin/main"), "origin/main");
        assert_eq!(sanitize_folder("../a/b"), "-a-b");
        assert_eq!(sanitize_folder(" "), "repository");
    }
}
