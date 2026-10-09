//! Host jobs (SPEC.md, Remote hosts, Host jobs): a remote host's install,
//! upgrade, image build, and test run in the background as numbered steps
//! the window follows, with the build's output, cancel until the install
//! begins, and the last job kept on disk so it survives a restart.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::watch;

use super::hosts::AgentHostService;
use super::runs::AgentRunService;
use super::settings::AgentSettingsService;
use crate::models::{
    now_rfc3339, AgentHost, AppError, AppResult, ErrorCode, HostJob, HostJobKind, HostJobState,
    HostJobStep, HostJobStepState,
};

/// Sends a job to the window each time it changes.
pub type HostJobEmitter = Arc<dyn Fn(&HostJob) + Send + Sync>;
/// Says a job ended, as a title and a body, when the window is not active.
pub type HostJobNotifier = Arc<dyn Fn(String, String) + Send + Sync>;

/// Lines of output the window gets with each change.
const TAIL_LINES: usize = 40;
/// The whole log stops growing here; a build prints far less.
const LOG_BYTES: usize = 4 * 1024 * 1024;
/// Output alone is sent to the window at most this often.
const EMIT_EVERY: Duration = Duration::from_millis(250);

/// The steps of Install and Upgrade, in order (`AgentHostService::deploy`).
pub mod install {
    pub const REACH: usize = 0;
    pub const BUILD: usize = 1;
    pub const COPY: usize = 2;
    pub const WAIT: usize = 3;
    pub const INSTALL: usize = 4;
    pub const CHECK: usize = 5;
    pub const COUNT: usize = 6;
}

/// The steps of Build image (`AgentHostService::build_image`).
pub mod image {
    pub const BUILD: usize = 0;
    pub const PROBE: usize = 1;
    pub const COUNT: usize = 2;
}

fn step(title: String, detail: &str) -> HostJobStep {
    HostJobStep {
        title,
        detail: detail.to_string(),
        state: HostJobStepState::Waiting,
        started_at: None,
        ended_at: None,
        progress: None,
    }
}

pub fn install_steps(name: &str, user: &str) -> Vec<HostJobStep> {
    vec![
        step(
            format!("Reach {name}"),
            &format!("SSH as {user}, sudo without a password, and its processor"),
        ),
        step(
            "Build the run controller".into(),
            "On this Mac, in Docker. Reused when this Brainiac already built it.",
        ),
        step(
            format!("Copy it to {name} and check its SHA-256"),
            "Over SSH, into /tmp",
        ),
        step(
            format!("Wait until no run is live on {name}"),
            "Restarting the controller would end a live run",
        ),
        step(
            "Install the program and its service".into(),
            "With sudo: the actions you approved when you added the host",
        ),
        step(
            "Check that it answers".into(),
            "The installation and protocol Brainiac expects",
        ),
    ]
}

pub fn image_steps(name: &str) -> Vec<HostJobStep> {
    vec![
        step(
            format!("Build the image on {name}"),
            "Claude Code, its ACP adapter, and OpenCode, from the Dockerfile in Settings",
        ),
        step(
            "Check its Docker engine".into(),
            "It must attach the workspace's loop devices",
        ),
    ]
}

/// One step per profile to test, named for it; one "Test a run" when Add
/// host's setup found none ready.
pub fn test_steps(name: &str, profiles: &[String]) -> Vec<HostJobStep> {
    let detail = format!(
        "Starts a short run, sends a prompt, cancels, and collects; the token or key goes to {name}"
    );
    if profiles.is_empty() {
        return vec![step("Test a run".into(), &detail)];
    }
    profiles
        .iter()
        .map(|profile| step(format!("Test {profile}"), &detail))
        .collect()
}

/// A job's steps. `profiles` names the profiles its tests are for.
pub fn steps_for(
    kind: HostJobKind,
    name: &str,
    user: &str,
    profiles: &[String],
) -> Vec<HostJobStep> {
    match kind {
        HostJobKind::Install | HostJobKind::Upgrade => install_steps(name, user),
        HostJobKind::BuildImage => image_steps(name),
        HostJobKind::Test => test_steps(name, profiles),
        HostJobKind::Setup => {
            let mut steps = install_steps(name, user);
            steps.extend(image_steps(name));
            steps.extend(test_steps(name, profiles));
            steps
        }
    }
}

/// One job as it runs: what the window sees, its cancel signal, and its log.
/// Cloning shares the same job (`Arc`), so the task running it and the
/// service that lists or cancels it see one state.
#[derive(Clone)]
pub struct JobProgress {
    inner: Arc<JobInner>,
}

struct JobInner {
    job: Mutex<HostJob>,
    cancel: watch::Sender<bool>,
    emitter: Option<HostJobEmitter>,
    /// The host's folder, where `last-job.json` and `last-job.log` live.
    dir: Option<PathBuf>,
    log: Mutex<LogState>,
}

struct LogState {
    file: Option<std::fs::File>,
    written: usize,
    last_emit: Option<Instant>,
}

impl JobProgress {
    /// A job just started, with its log emptied. `dir` and `emitter` are
    /// `None` in tests that only follow the state.
    pub fn new(job: HostJob, dir: Option<PathBuf>, emitter: Option<HostJobEmitter>) -> Self {
        let file = dir.as_ref().and_then(|d| {
            std::fs::create_dir_all(d).ok()?;
            std::fs::File::create(d.join(LOG_FILE)).ok()
        });
        let progress = Self::from_parts(job, dir, emitter, file);
        progress.changed();
        progress
    }

    /// A job read back from disk; it does not run again.
    fn ended(job: HostJob, dir: PathBuf) -> Self {
        Self::from_parts(job, Some(dir), None, None)
    }

    fn from_parts(
        job: HostJob,
        dir: Option<PathBuf>,
        emitter: Option<HostJobEmitter>,
        file: Option<std::fs::File>,
    ) -> Self {
        let (cancel, _) = watch::channel(false);
        Self {
            inner: Arc::new(JobInner {
                job: Mutex::new(job),
                cancel,
                emitter,
                dir,
                log: Mutex::new(LogState {
                    file,
                    written: 0,
                    last_emit: None,
                }),
            }),
        }
    }

    pub fn snapshot(&self) -> HostJob {
        self.lock().clone()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HostJob> {
        self.inner.job.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn running(&self) -> bool {
        self.lock().state == HostJobState::Running
    }

    /// Change the job under its lock, then save it and tell the window.
    fn update(&self, change: impl FnOnce(&mut HostJob)) {
        change(&mut self.lock());
        self.changed();
    }

    fn changed(&self) {
        let job = self.snapshot();
        if let Some(dir) = &self.inner.dir {
            if let Err(e) = save(dir, &job) {
                tracing::warn!(error = %e, "could not save a host job");
            }
        }
        if let Some(emit) = &self.inner.emitter {
            emit(&job);
        }
        self.inner
            .log
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .last_emit = Some(Instant::now());
    }

    pub fn begin(&self, index: usize) {
        self.update(|job| {
            if let Some(step) = job.steps.get_mut(index) {
                step.state = HostJobStepState::Running;
                step.started_at = Some(now_rfc3339());
            }
        });
    }

    /// Ends a step; `detail` replaces what it said it would do.
    pub fn finish(&self, index: usize, detail: Option<String>) {
        self.update(|job| {
            if let Some(step) = job.steps.get_mut(index) {
                if step.started_at.is_none() {
                    step.started_at = Some(now_rfc3339());
                }
                step.state = HostJobStepState::Done;
                step.ended_at = Some(now_rfc3339());
                step.progress = None;
                if let Some(detail) = detail {
                    step.detail = detail;
                }
            }
        });
    }

    pub fn skip(&self, index: usize, detail: &str) {
        self.update(|job| {
            if let Some(step) = job.steps.get_mut(index) {
                step.state = HostJobStepState::Skipped;
                step.detail = detail.to_string();
            }
        });
    }

    pub fn retitle(&self, index: usize, title: String, detail: String) {
        self.update(|job| {
            if let Some(step) = job.steps.get_mut(index) {
                step.title = title;
                step.detail = detail;
            }
        });
    }

    /// How far a running step got, in words. Sent at most every 250 ms.
    pub fn progress(&self, index: usize, text: String) {
        if let Some(step) = self.lock().steps.get_mut(index) {
            step.progress = Some(text);
        }
        self.maybe_emit();
    }

    /// Replaces the steps from `index` on, for a test whose steps are known
    /// only once it ran.
    pub fn replace_steps(&self, index: usize, steps: Vec<HostJobStep>) {
        self.update(|job| {
            job.steps.truncate(index);
            job.steps.extend(steps);
        });
    }

    /// One line of output: kept whole in the log file, and the last lines
    /// in the job.
    pub fn line(&self, line: &str) {
        {
            let mut job = self.lock();
            job.log_tail.push(line.to_string());
            let excess = job.log_tail.len().saturating_sub(TAIL_LINES);
            job.log_tail.drain(..excess);
        }
        {
            let mut log = self.inner.log.lock().unwrap_or_else(|p| p.into_inner());
            if log.written < LOG_BYTES {
                let written = log.written + line.len() + 1;
                if let Some(file) = log.file.as_mut() {
                    let _ = writeln!(file, "{line}");
                    if written >= LOG_BYTES {
                        let _ = writeln!(file, "[The rest of the output was not kept.]");
                    }
                }
                log.written = written;
            }
        }
        self.maybe_emit();
    }

    fn maybe_emit(&self) {
        let due = {
            let log = self.inner.log.lock().unwrap_or_else(|p| p.into_inner());
            log.last_emit.is_none_or(|at| at.elapsed() >= EMIT_EVERY)
        };
        if due {
            self.changed();
        }
    }

    pub fn set_builds(&self, from: Option<String>, to: Option<String>) {
        self.update(|job| {
            job.from_build = from;
            job.to_build = to;
        });
    }

    /// Cancel is refused from here on: the host is about to change.
    pub fn close_cancel(&self) {
        self.update(|job| job.cancellable = false);
    }

    /// Asks the job to stop. Refused once it can no longer leave the host
    /// as it was.
    pub fn cancel(&self) -> AppResult<()> {
        let job = self.lock();
        if job.state != HostJobState::Running {
            return Err(AppError::validation("This job already ended."));
        }
        if !job.cancellable {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "The install has begun, so this job finishes on its own.",
            ));
        }
        // `send_replace` stores the value even when no step is listening yet.
        self.inner.cancel.send_replace(true);
        Ok(())
    }

    pub fn cancelled(&self) -> bool {
        *self.inner.cancel.borrow()
    }

    /// A receiver a long step waits on alongside its work.
    pub fn cancel_signal(&self) -> watch::Receiver<bool> {
        self.inner.cancel.subscribe()
    }

    /// `Err(Cancelled)` once the user asked to cancel, so a step can stop
    /// with `?` between commands.
    pub fn check_cancelled(&self) -> AppResult<()> {
        if self.cancelled() {
            Err(cancelled())
        } else {
            Ok(())
        }
    }

    /// Records how the job ended: the running step takes the error, and
    /// the steps after it are marked not run.
    pub fn end(&self, result: &AppResult<()>) {
        self.update(|job| end_job(job, result));
    }
}

pub fn cancelled() -> AppError {
    AppError::new(ErrorCode::Cancelled, "Cancelled.")
}

fn end_job(job: &mut HostJob, result: &AppResult<()>) {
    let now = now_rfc3339();
    job.ended_at = Some(now.clone());
    job.cancellable = false;
    let cancelled = matches!(result, Err(e) if e.code == ErrorCode::Cancelled);
    job.state = match result {
        Ok(()) => HostJobState::Succeeded,
        Err(_) if cancelled => HostJobState::Cancelled,
        Err(_) => HostJobState::Failed,
    };
    if let Err(e) = result {
        if !cancelled {
            job.error = Some(e.message.clone());
            job.error_details = e.details.clone();
        }
    }
    let mut failed = false;
    for step in &mut job.steps {
        match step.state {
            HostJobStepState::Running => {
                step.ended_at = Some(now.clone());
                step.progress = None;
                if cancelled {
                    step.state = HostJobStepState::Skipped;
                    step.detail = "Cancelled".into();
                } else if result.is_err() {
                    step.state = HostJobStepState::Failed;
                    failed = true;
                } else {
                    step.state = HostJobStepState::Done;
                }
            }
            HostJobStepState::Waiting => {
                step.state = HostJobStepState::Skipped;
                if cancelled || failed || result.is_err() {
                    step.detail = "Not run".into();
                }
            }
            _ => {}
        }
    }
}

/// A job that was running when Brainiac quit: it is not running any more.
fn interrupted(mut job: HostJob) -> HostJob {
    if job.state != HostJobState::Running {
        return job;
    }
    let result = Err(AppError::dependency(
        "Brainiac quit while this job ran. Look at the host's setup, and run it again if a step is missing.",
    ));
    end_job(&mut job, &result);
    job.state = HostJobState::Interrupted;
    job
}

const JOB_FILE: &str = "last-job.json";
const LOG_FILE: &str = "last-job.log";

/// Written beside, then renamed over, so a crash leaves the old one whole.
fn save(dir: &Path, job: &HostJob) -> AppResult<()> {
    std::fs::create_dir_all(dir)?;
    let text = serde_json::to_vec_pretty(job)
        .map_err(|e| AppError::db("A host job could not be saved.").with_details(e.to_string()))?;
    let next = dir.join(format!("{JOB_FILE}.next"));
    std::fs::write(&next, text)?;
    std::fs::rename(next, dir.join(JOB_FILE))?;
    Ok(())
}

fn load(dir: &Path) -> Option<HostJob> {
    let text = std::fs::read(dir.join(JOB_FILE)).ok()?;
    serde_json::from_slice(&text).ok()
}

/// Starts, lists, and cancels host jobs. One at a time per host.
pub struct HostJobService {
    hosts: Arc<AgentHostService>,
    runs: Arc<AgentRunService>,
    settings: Arc<AgentSettingsService>,
    hosts_dir: PathBuf,
    /// The last job of each host, running or ended, by host ID.
    jobs: Mutex<HashMap<String, JobProgress>>,
    emitter: HostJobEmitter,
    notifier: HostJobNotifier,
}

impl HostJobService {
    /// Reads each host's last job; one that was running is marked
    /// interrupted, since Brainiac quit under it.
    pub fn new(
        hosts: Arc<AgentHostService>,
        runs: Arc<AgentRunService>,
        settings: Arc<AgentSettingsService>,
        data_dir: &Path,
        emitter: HostJobEmitter,
        notifier: HostJobNotifier,
    ) -> Self {
        let hosts_dir = data_dir.join("agent-runs").join("hosts");
        let mut jobs = HashMap::new();
        for entry in std::fs::read_dir(&hosts_dir)
            .into_iter()
            .flatten()
            .flatten()
        {
            let dir = entry.path();
            let Some(job) = load(&dir) else { continue };
            let was_running = job.state == HostJobState::Running;
            let job = interrupted(job);
            if was_running {
                let _ = save(&dir, &job);
            }
            jobs.insert(job.host_id.clone(), JobProgress::ended(job, dir));
        }
        Self {
            hosts,
            runs,
            settings,
            hosts_dir,
            jobs: Mutex::new(jobs),
            emitter,
            notifier,
        }
    }

    /// Each host's last job.
    pub fn list(&self) -> Vec<HostJob> {
        let mut jobs: Vec<HostJob> = self
            .jobs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .values()
            .map(JobProgress::snapshot)
            .collect();
        jobs.sort_by(|a, b| b.started_at.cmp(&a.started_at));
        jobs
    }

    /// The whole output of a host's last job.
    pub fn log(&self, host_id: &str) -> AppResult<String> {
        let path = self.hosts_dir.join(safe_id(host_id)?).join(LOG_FILE);
        match std::fs::read(&path) {
            Ok(bytes) => Ok(String::from_utf8_lossy(&bytes).into_owned()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
            Err(e) => Err(e.into()),
        }
    }

    pub fn cancel(&self, host_id: &str) -> AppResult<HostJob> {
        let job = self
            .current(host_id)
            .ok_or_else(|| AppError::not_found("This host has no job running."))?;
        job.cancel()?;
        Ok(job.snapshot())
    }

    fn current(&self, host_id: &str) -> Option<JobProgress> {
        self.jobs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(host_id)
            .cloned()
    }

    /// True while a host has a job running; Remove waits for it.
    pub fn busy(&self, host_id: &str) -> bool {
        self.current(host_id).is_some_and(|j| j.running())
    }

    /// Remove (SPEC.md, Remote hosts) refuses a host with a job running,
    /// and forgets the host's last job with the host.
    pub async fn remove(&self, host_id: &str) -> AppResult<()> {
        if self.busy(host_id) {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "This host has a job running. Wait for it to end, or cancel it.",
            ));
        }
        self.hosts.remove(host_id).await?;
        self.jobs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(host_id);
        Ok(())
    }

    /// Starts a job and returns at once; the window follows it by events.
    // `self: &Arc<Self>` lets the background task keep the service alive.
    /// `profile_id` is the profile Test runs; Add host's setup tests every
    /// profile that is ready (its token or key, and model).
    pub async fn start(
        self: &Arc<Self>,
        host_id: &str,
        kind: HostJobKind,
        profile_id: Option<String>,
    ) -> AppResult<HostJob> {
        let host = self.hosts.host(host_id).await?;
        check_start(&host, kind)?;
        let dir = self.hosts_dir.join(safe_id(host_id)?);
        let profiles = self.settings.profiles().await?;
        let tested: Vec<_> = match kind {
            HostJobKind::Test => {
                let id =
                    profile_id.ok_or_else(|| AppError::validation("Choose the agent to test."))?;
                let profile = profiles
                    .into_iter()
                    .find(|p| p.id == id)
                    .ok_or_else(|| AppError::not_found("There is no such agent profile."))?;
                if let Some(first) = profile.missing.first() {
                    return Err(AppError::validation(format!(
                        "Settings → Agents is not ready: {first}"
                    )));
                }
                vec![profile]
            }
            HostJobKind::Setup => profiles
                .into_iter()
                .filter(|p| p.missing.is_empty())
                .collect(),
            _ => Vec::new(),
        };
        let profile_ids: Vec<String> = tested.iter().map(|p| p.id.clone()).collect();
        let names: Vec<String> = tested.iter().map(super::settings::profile_name).collect();
        let job = HostJob {
            id: uuid::Uuid::new_v4().simple().to_string(),
            host_id: host.id.clone(),
            host_name: host.name.clone(),
            kind,
            state: HostJobState::Running,
            started_at: now_rfc3339(),
            ended_at: None,
            steps: steps_for(
                kind,
                &host.name,
                host.ssh_user.as_deref().unwrap_or("the SSH user"),
                &names,
            ),
            log_tail: Vec::new(),
            error: None,
            error_details: None,
            cancellable: matches!(
                kind,
                HostJobKind::Install | HostJobKind::Upgrade | HostJobKind::Setup
            ),
            from_build: host.controller_build.clone(),
            to_build: match kind {
                HostJobKind::Install | HostJobKind::Upgrade | HostJobKind::Setup => {
                    host.available_build.clone()
                }
                _ => None,
            },
            profile_ids: profile_ids.clone(),
        };
        let progress = {
            // Checked and inserted under one lock, so two clicks start one job.
            let mut jobs = self.jobs.lock().unwrap_or_else(|p| p.into_inner());
            if jobs.get(host_id).is_some_and(|j| j.running()) {
                return Err(AppError::new(
                    ErrorCode::Conflict,
                    format!("{} already has a job running.", host.name),
                ));
            }
            let progress = JobProgress::new(job, Some(dir), Some(Arc::clone(&self.emitter)));
            jobs.insert(host_id.to_string(), progress.clone());
            progress
        };
        let service = Arc::clone(self);
        let id = host_id.to_string();
        let running = progress.clone();
        tokio::spawn(async move {
            let result = service.run(&id, kind, &profile_ids, &running).await;
            running.end(&result);
            let job = running.snapshot();
            if let Some((title, body)) = outcome(&job) {
                (service.notifier)(title, body);
            }
        });
        Ok(progress.snapshot())
    }

    async fn run(
        &self,
        id: &str,
        kind: HostJobKind,
        profiles: &[String],
        job: &JobProgress,
    ) -> AppResult<()> {
        match kind {
            HostJobKind::Install | HostJobKind::Upgrade => {
                self.hosts.deploy(id, job, 0).await.map(|_| ())
            }
            HostJobKind::BuildImage => self.hosts.build_image(id, job, 0).await.map(|_| ()),
            HostJobKind::Test => match profiles.first() {
                Some(profile) => self.test(id, profile, job, 0, true).await,
                None => Err(AppError::validation("Choose the agent to test.")),
            },
            HostJobKind::Setup => {
                self.hosts.deploy(id, job, 0).await?;
                self.hosts.build_image(id, job, install::COUNT).await?;
                let at = install::COUNT + image::COUNT;
                if profiles.is_empty() {
                    job.skip(
                        at,
                        "Not run yet: no agent is set up. Add a token or key in Settings → Agents.",
                    );
                    return Ok(());
                }
                for (i, profile) in profiles.iter().enumerate() {
                    let last = i + 1 == profiles.len();
                    self.test(id, profile, job, at + i, last).await?;
                }
                Ok(())
            }
        }
    }

    /// One profile's test, as the step at `at`. The job's last step opens
    /// into the test's own steps; an earlier one keeps its place, so the
    /// steps after it stay where they are.
    async fn test(
        &self,
        id: &str,
        profile: &str,
        job: &JobProgress,
        at: usize,
        last: bool,
    ) -> AppResult<()> {
        job.begin(at);
        let result = self.runs.test_host(id, profile).await?;
        if !last {
            let failed = result.steps.iter().find(|s| !s.passed);
            return match failed {
                None => {
                    job.finish(at, Some("Passed".into()));
                    Ok(())
                }
                Some(step) => Err(AppError::dependency(format!(
                    "The test did not pass, so runs cannot start on this host yet. {}: {}",
                    step.name,
                    step.detail.clone().unwrap_or_default()
                ))),
            };
        }
        let started = job
            .snapshot()
            .steps
            .get(at)
            .and_then(|s| s.started_at.clone());
        let now = now_rfc3339();
        let steps = result
            .steps
            .iter()
            .map(|s| HostJobStep {
                title: s.name.clone(),
                detail: s.detail.clone().unwrap_or_default(),
                state: if s.passed {
                    HostJobStepState::Done
                } else {
                    HostJobStepState::Failed
                },
                started_at: started.clone(),
                ended_at: Some(now.clone()),
                progress: None,
            })
            .collect();
        job.replace_steps(at, steps);
        if result.passed {
            Ok(())
        } else {
            Err(AppError::dependency(
                "The test did not pass, so runs cannot start on this host yet.",
            ))
        }
    }
}

/// Refuses a job its host is not ready for, before anything starts.
fn check_start(host: &AgentHost, kind: HostJobKind) -> AppResult<()> {
    if host.kind != "ssh" {
        return Err(AppError::validation(
            "This Mac builds and tests from its own page.",
        ));
    }
    if !host.approved {
        return Err(AppError::validation(
            "Confirm this host's key before anything runs on it.",
        ));
    }
    match kind {
        HostJobKind::Install | HostJobKind::Setup => Ok(()),
        HostJobKind::Upgrade | HostJobKind::BuildImage if !host.installed => Err(
            AppError::validation("Install the run controller on this host first."),
        ),
        HostJobKind::Test if host.image.as_ref().is_none_or(|i| !i.current) => Err(
            AppError::validation("Build the image on this host before testing it."),
        ),
        _ => Ok(()),
    }
}

/// A host ID names a folder: only what Brainiac itself makes is accepted.
fn safe_id(id: &str) -> AppResult<&str> {
    if !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        Ok(id)
    } else {
        Err(AppError::validation("That host is not saved."))
    }
}

/// What a job's end says, in the window and in a notification. A cancelled
/// job says nothing: the user asked for it.
pub fn outcome(job: &HostJob) -> Option<(String, String)> {
    let name = &job.host_name;
    let took = took(job);
    match job.state {
        HostJobState::Succeeded => Some(match job.kind {
            HostJobKind::Upgrade => (
                format!("{name} is upgraded"),
                format!(
                    "{}in {took}. Runs can start there again.",
                    job.to_build
                        .as_ref()
                        .map(|b| format!("Build {b}, "))
                        .unwrap_or_default()
                ),
            ),
            HostJobKind::Install => (
                format!("The run controller is installed on {name}"),
                format!("In {took}. Build the image there next."),
            ),
            HostJobKind::BuildImage => (
                format!("The image is built on {name}"),
                format!("In {took}. Test the host next."),
            ),
            HostJobKind::Test => (
                format!("{name} passed its test"),
                "Runs can start there.".into(),
            ),
            HostJobKind::Setup => {
                let skipped = job
                    .steps
                    .last()
                    .filter(|s| s.state == HostJobStepState::Skipped);
                match skipped {
                    Some(step) => (
                        format!("{name} is installed, and its image built"),
                        format!("Its test waits. {}", step.detail),
                    ),
                    None => (
                        format!("{name} is ready"),
                        format!("Set up in {took}. Runs can start there."),
                    ),
                }
            }
        }),
        HostJobState::Failed | HostJobState::Interrupted => {
            let at = job
                .steps
                .iter()
                .position(|s| s.state == HostJobStepState::Failed)
                .map(|i| format!(" at step {} of {}", i + 1, job.steps.len()))
                .unwrap_or_default();
            Some((
                format!("{}{at}", failed_title(job)),
                job.error.clone().unwrap_or_default(),
            ))
        }
        HostJobState::Cancelled | HostJobState::Running => None,
    }
}

fn failed_title(job: &HostJob) -> String {
    let name = &job.host_name;
    match job.kind {
        HostJobKind::Upgrade => format!("{name}'s upgrade failed"),
        HostJobKind::Install => format!("The install on {name} failed"),
        HostJobKind::BuildImage => format!("The image build on {name} failed"),
        HostJobKind::Test => format!("{name}'s test did not pass"),
        HostJobKind::Setup => format!("Setting up {name} failed"),
    }
}

/// "19 minutes", "40 seconds": how long a job took.
fn took(job: &HostJob) -> String {
    let parse = |t: &str| chrono::DateTime::parse_from_rfc3339(t).ok();
    let seconds = match (
        parse(&job.started_at),
        job.ended_at.as_deref().and_then(parse),
    ) {
        (Some(start), Some(end)) => (end - start).num_seconds().max(0),
        _ => 0,
    };
    duration_words(seconds)
}

pub fn duration_words(seconds: i64) -> String {
    if seconds < 60 {
        format!(
            "{seconds} {}",
            if seconds == 1 { "second" } else { "seconds" }
        )
    } else {
        let minutes = (seconds + 30) / 60;
        format!(
            "{minutes} {}",
            if minutes == 1 { "minute" } else { "minutes" }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(kind: HostJobKind) -> HostJob {
        HostJob {
            id: "job".into(),
            host_id: "h1".into(),
            host_name: "build-01".into(),
            kind,
            state: HostJobState::Running,
            started_at: "2026-10-07T19:02:00Z".into(),
            ended_at: None,
            steps: steps_for(kind, "build-01", "ci", &[]),
            log_tail: Vec::new(),
            error: None,
            error_details: None,
            cancellable: true,
            from_build: Some("a1f3c9e".into()),
            to_build: Some("7c2e51a".into()),
            profile_ids: Vec::new(),
        }
    }

    #[test]
    fn a_failed_step_takes_the_error_and_the_rest_are_not_run() {
        let progress = JobProgress::new(job(HostJobKind::Upgrade), None, None);
        progress.begin(install::REACH);
        progress.finish(install::REACH, None);
        progress.begin(install::BUILD);
        progress.finish(install::BUILD, None);
        progress.begin(install::COPY);
        progress.end(&Err(AppError::dependency("The copy did not match.")
            .with_details("expected abc".to_string())));
        let ended = progress.snapshot();
        assert_eq!(ended.state, HostJobState::Failed);
        assert_eq!(ended.steps[install::COPY].state, HostJobStepState::Failed);
        assert_eq!(ended.steps[install::WAIT].state, HostJobStepState::Skipped);
        assert_eq!(ended.error.as_deref(), Some("The copy did not match."));
        let (title, _) = outcome(&ended).unwrap();
        assert_eq!(title, "build-01's upgrade failed at step 3 of 6");
    }

    #[test]
    fn cancel_works_until_the_install_begins() {
        let progress = JobProgress::new(job(HostJobKind::Upgrade), None, None);
        let signal = progress.cancel_signal();
        progress.begin(install::BUILD);
        progress.cancel().unwrap();
        assert!(*signal.borrow());
        assert!(progress.check_cancelled().is_err());
        progress.end(&Err(cancelled()));
        let ended = progress.snapshot();
        assert_eq!(ended.state, HostJobState::Cancelled);
        assert_eq!(ended.steps[install::BUILD].state, HostJobStepState::Skipped);
        assert!(ended.error.is_none());
        assert!(outcome(&ended).is_none());

        let installing = JobProgress::new(job(HostJobKind::Upgrade), None, None);
        installing.close_cancel();
        assert_eq!(installing.cancel().unwrap_err().code, ErrorCode::Conflict);
        assert!(!installing.cancelled());
    }

    #[test]
    fn the_log_keeps_its_last_lines_and_the_whole_output_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let progress = JobProgress::new(
            job(HostJobKind::Install),
            Some(dir.path().to_path_buf()),
            None,
        );
        for n in 0..100 {
            progress.line(&format!("   Compiling crate{n} v1.0.0"));
        }
        let tail = progress.snapshot().log_tail;
        assert_eq!(tail.len(), TAIL_LINES);
        assert_eq!(tail.last().unwrap(), "   Compiling crate99 v1.0.0");
        let log = std::fs::read_to_string(dir.path().join(LOG_FILE)).unwrap();
        assert!(log.contains("crate0 ") && log.contains("crate99 "));
    }

    #[test]
    fn a_job_brainiac_quit_under_is_interrupted_when_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let progress = JobProgress::new(
            job(HostJobKind::Upgrade),
            Some(dir.path().to_path_buf()),
            None,
        );
        progress.begin(install::REACH);
        let read = interrupted(load(dir.path()).unwrap());
        assert_eq!(read.state, HostJobState::Interrupted);
        assert_eq!(read.steps[install::REACH].state, HostJobStepState::Failed);
        assert_eq!(read.steps[install::CHECK].state, HostJobStepState::Skipped);
        assert!(read.error.unwrap().contains("Brainiac quit"));
    }

    #[test]
    fn a_setup_whose_test_waits_says_why() {
        let progress = JobProgress::new(job(HostJobKind::Setup), None, None);
        for i in 0..install::COUNT + image::COUNT {
            progress.finish(i, None);
        }
        progress.skip(
            install::COUNT + image::COUNT,
            "Not run yet: Add an API key.",
        );
        progress.end(&Ok(()));
        let ended = progress.snapshot();
        assert_eq!(ended.state, HostJobState::Succeeded);
        let (title, body) = outcome(&ended).unwrap();
        assert_eq!(title, "build-01 is installed, and its image built");
        assert!(body.contains("Add an API key"));
    }

    #[test]
    fn durations_read_as_words() {
        assert_eq!(duration_words(40), "40 seconds");
        assert_eq!(duration_words(19 * 60 + 10), "19 minutes");
        assert_eq!(duration_words(60), "1 minute");
    }

    #[test]
    fn only_a_host_id_brainiac_made_names_a_folder() {
        assert!(safe_id("4f1c0a9e").is_ok());
        assert!(safe_id("../etc").is_err());
        assert!(safe_id("").is_err());
    }
}
