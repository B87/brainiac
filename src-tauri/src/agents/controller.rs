//! `RunController`: `brainiac runner`, the process that owns agent runs
//! (docs/architecture.md, Agent runs — v0.5). It outlives the app: Brainiac
//! starts it when it needs it and reconnects to it, and it exits on its own
//! once no run is live and no client has asked anything for a while.
//!
//! For each run it alone writes to the agent, records each command's intent
//! before forwarding it and its outcome before answering, answers
//! permissions, enforces the deadline, and stops the container — keeping it
//! and its volume, which hold the work. A client that disconnects changes
//! nothing; a controller that restarts marks its live runs interrupted and
//! stops them rather than resuming what it no longer knows.

pub mod acp;
pub mod archive;
pub mod docker;
pub mod guard;
pub mod journal;
pub mod protocol;
pub mod state;

use std::collections::HashMap;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime};

use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{mpsc, oneshot, watch, Notify};

use self::acp::{Command, RunSink, Session};
use self::docker::{CollectSpec, LaunchSpec, Running, Workloads};
use self::journal::{journal_path, Append, Redactor, TraceJournal};
use self::protocol::{
    valid_id, Activity, CollectManifest, Credential, Delivery, EventBody, EventPage, Outcome,
    PendingPermission, Phase, Request, Response, RunStatus, StartRun, StopReason, MAX_LINE_BYTES,
    MAX_PROMPT_BYTES, PROTOCOL,
};
use self::state::{Ledger, RunRecord, StateDir};
use crate::models::{AppError, AppResult, ErrorCode};

/// How long a long poll for events may wait.
const MAX_WAIT: Duration = Duration::from_secs(30);
/// Creating the container and copying a large history into it.
const LAUNCH_WAIT: Duration = Duration::from_secs(20 * 60);
/// How often an unconfirmed stop is tried again.
const STOP_RETRY: Duration = Duration::from_secs(10);
/// A gap this large between the wall clock and the monotonic clock is a
/// sleep of this Mac (docs/design/agent-runs.md, Spike record: collection,
/// quota, and local wake).
const SUSPEND_GAP: Duration = Duration::from_secs(15);
/// Files a collection may be asked to add.
const MAX_INCLUDE: usize = 1000;
/// Memory for the collector's container, which hashes files one at a time.
const COLLECTOR_MEMORY_MIB: u32 = 2048;
/// The shortest and longest time limits a run may have.
const MIN_TIME_LIMIT: u64 = 1;
const MAX_TIME_LIMIT: u64 = 8 * 60 * 60;

/// RFC 3339 in UTC, to the second.
pub fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn wall(time: SystemTime) -> String {
    chrono::DateTime::<chrono::Utc>::from(time).to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

pub(crate) fn conflict(message: &str) -> AppError {
    AppError::new(ErrorCode::Conflict, message)
}

fn not_live() -> AppError {
    conflict("The run is not running.")
}

pub struct Config {
    pub state: StateDir,
    pub socket: PathBuf,
    /// Exit after this long with no live run and no request; `None` never.
    pub idle_exit: Option<Duration>,
    /// Start `brainiac runner guard` beside the controller.
    pub spawn_guard: bool,
}

/// What a run's session or the engine tells the controller.
enum Signal {
    Failed { run_id: String, message: String },
    Closed { run_id: String },
}

/// What a stop has to finish once it is decided.
struct Stop {
    session: Option<mpsc::UnboundedSender<Command>>,
    /// The container is still being made: `prepare` stops it once it exists.
    launching: bool,
}

/// Time since this Mac started, counting the time it slept, and not moved
/// when the clock is set: macOS's `CLOCK_MONOTONIC` (Linux's
/// `CLOCK_BOOTTIME`). `Instant` is not enough on its own: on macOS it stops
/// while the Mac sleeps.
fn since_boot() -> Duration {
    #[cfg(target_os = "macos")]
    let clock = libc::CLOCK_MONOTONIC;
    #[cfg(not(target_os = "macos"))]
    let clock = libc::CLOCK_BOOTTIME;
    let mut now = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // `unsafe` because every libc call is, to Rust; `clock_gettime` only
    // writes the struct it is given.
    unsafe { libc::clock_gettime(clock, &mut now) };
    Duration::new(now.tv_sec as u64, now.tv_nsec as u32)
}

/// The deadline, armed when the run is accepted.
struct Clock {
    /// `since_boot` at acceptance: the deadline counts the Mac's sleep.
    armed: Duration,
    /// When it was accepted by the clock that stops during sleep, to tell
    /// a deadline reached while the Mac slept.
    armed_awake: Instant,
    limit: Duration,
}

impl Clock {
    fn new(limit: Duration) -> Self {
        Self {
            armed: since_boot(),
            armed_awake: Instant::now(),
            limit,
        }
    }

    /// `Some(asleep)` once the run is past its time limit.
    fn expired(&self) -> Option<bool> {
        past_limit(
            self.limit,
            since_boot().saturating_sub(self.armed),
            self.armed_awake.elapsed(),
        )
    }
}

/// `Some(asleep)` once `elapsed` (sleep included) reaches the limit;
/// `asleep` when the Mac slept at least `SUSPEND_GAP` and the time it was
/// awake had not reached the limit: it expired while the Mac slept, and the
/// first check after waking stops it.
fn past_limit(limit: Duration, elapsed: Duration, awake: Duration) -> Option<bool> {
    (elapsed >= limit).then(|| elapsed.saturating_sub(awake) >= SUSPEND_GAP && awake < limit)
}

struct RunInner {
    record: RunRecord,
    journal: TraceJournal,
    ledger: Ledger,
    /// The run's known values, in memory only, while its session lives.
    redactor: Option<Arc<Redactor>>,
    session: Option<mpsc::UnboundedSender<Command>>,
    activity: Activity,
    permissions: Vec<PendingPermission>,
    clock: Option<Clock>,
    /// The container is being created; its stop waits for that to finish.
    launching: bool,
    /// Tells a container still being made that the run stopped.
    cancel_launch: Option<watch::Sender<bool>>,
    /// Commands begun and not yet settled: a repeat waits for the outcome.
    inflight: HashSet<String>,
    last_stop_try: Option<Instant>,
    journal_failed: bool,
    /// The collector is running against the run's volume.
    collecting: bool,
}

pub struct Run {
    id: String,
    dir: PathBuf,
    // `Mutex` (not async): it is never held across an `.await`, so a slow
    // engine never blocks a reader of the run's state.
    inner: Mutex<RunInner>,
    /// The journal's last sequence; long polls wait on it.
    cursor: watch::Sender<u64>,
    /// One stop attempt at a time.
    stopping: AtomicBool,
    /// Woken when a command settles.
    settled: Notify,
    signals: mpsc::UnboundedSender<Signal>,
}

impl Run {
    fn lock(&self) -> MutexGuard<'_, RunInner> {
        self.inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// Append an event, filtered by the run's known values.
    fn append(&self, mut body: EventBody, control: bool) {
        let mut inner = self.lock();
        if let Some(redactor) = &inner.redactor {
            redactor.redact_event(&mut body);
        }
        match inner.journal.append(now(), body, control) {
            Ok(Append::Stored(seq)) => {
                self.cursor.send_replace(seq);
            }
            Ok(Append::Full) | Err(_) if !inner.journal_failed => {
                inner.journal_failed = true;
                let _ = self.signals.send(Signal::Failed {
                    run_id: self.id.clone(),
                    message: "The conversation reached its 64 MB limit or could not be saved, so the run was stopped and its work kept."
                        .into(),
                });
            }
            _ => {}
        }
    }

    /// Record a command's outcome (`None`: nothing was written, so it is
    /// forgotten), and wake any repeat of it that waits.
    fn settle(&self, command_id: &str, outcome: Option<Delivery>) -> AppResult<()> {
        let result = {
            let mut inner = self.lock();
            inner.inflight.remove(command_id);
            match outcome {
                Some(outcome) => inner.ledger.finish(command_id, outcome),
                None => inner.ledger.forget(command_id),
            }
        };
        self.settled.notify_waiters();
        Ok(result?)
    }

    fn save(inner: &RunInner, dir: &std::path::Path) {
        if let Err(e) = inner.record.save(dir) {
            tracing::warn!(error = %e, "could not save a run record");
        }
    }

    fn status(&self) -> RunStatus {
        let inner = self.lock();
        let mut status = inner.record.status(inner.journal.cursor());
        if inner.record.phase == Phase::Running {
            status.activity = inner.activity;
            status.permissions = inner.permissions.clone();
        }
        status
    }
}

impl RunSink for Run {
    fn record(&self, body: EventBody) {
        self.append(body, false);
    }

    fn ready(&self, session_id: &str, model: Option<&str>) {
        let mut inner = self.lock();
        inner.record.session_id = Some(session_id.to_string());
        inner.record.model = model.map(str::to_string);
        if inner.record.phase == Phase::Preparing {
            inner.record.phase = Phase::Running;
            inner.activity = Activity::Idle;
        }
        Run::save(&inner, &self.dir);
    }

    fn activity(&self, activity: Activity, turn: u32) {
        let mut inner = self.lock();
        inner.activity = activity;
        if inner.record.turn != turn {
            inner.record.turn = turn;
            Run::save(&inner, &self.dir);
        }
    }

    fn asked(&self, permission: PendingPermission) {
        // Filtered and cut like the journal's copy: `Status` answers with it.
        let mut body = EventBody::Permission(permission);
        let mut inner = self.lock();
        if let Some(redactor) = &inner.redactor {
            redactor.redact_event(&mut body);
        }
        for text in body.texts_mut() {
            journal::clip(text, journal::MAX_TEXT_BYTES);
        }
        if let EventBody::Permission(permission) = body {
            inner.permissions.push(permission);
        }
    }

    fn answered(&self, permission_id: &str) {
        self.lock()
            .permissions
            .retain(|p| p.permission_id != permission_id);
    }

    fn failed(&self, message: String) {
        let _ = self.signals.send(Signal::Failed {
            run_id: self.id.clone(),
            message,
        });
    }

    fn closed(&self) {
        {
            let mut inner = self.lock();
            inner.session = None;
            inner.redactor = None;
            inner.permissions.clear();
        }
        let _ = self.signals.send(Signal::Closed {
            run_id: self.id.clone(),
        });
    }
}

pub struct Controller<W: Workloads> {
    state: StateDir,
    installation: String,
    workloads: W,
    runs: Mutex<HashMap<String, Arc<Run>>>,
    signals: mpsc::UnboundedSender<Signal>,
    connections: Arc<AtomicUsize>,
    last_request: Mutex<Instant>,
    /// Set, under the runs' lock, once the controller decided to exit: no
    /// run is accepted after it.
    closing: AtomicBool,
    shutdown: Notify,
    /// Every task the controller started, stopped when `serve` ends.
    tasks: Mutex<Vec<tokio::task::AbortHandle>>,
}

impl<W: Workloads> Controller<W> {
    /// Start a task the controller owns: it ends when the controller does,
    /// so nothing of a stopped controller keeps writing to its files.
    fn spawn(&self, task: impl std::future::Future<Output = ()> + Send + 'static) {
        let handle = tokio::spawn(task).abort_handle();
        let mut tasks = self.tasks.lock().unwrap_or_else(|p| p.into_inner());
        tasks.retain(|t| !t.is_finished());
        tasks.push(handle);
    }

    fn abort_all(&self) {
        for task in self
            .tasks
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .drain(..)
        {
            task.abort();
        }
    }
    /// Load every run an earlier controller left. A run that was live is not
    /// resumed: it is interrupted, and its container is stopped.
    fn load(
        state: StateDir,
        installation: String,
        workloads: W,
    ) -> AppResult<(Arc<Self>, mpsc::UnboundedReceiver<Signal>)> {
        let (signals, receiver) = mpsc::unbounded_channel();
        let mut runs = HashMap::new();
        for id in state.run_ids() {
            let Some(mut record) = state.load_run(&id) else {
                tracing::warn!(run = %id, "a run's record cannot be read; its container is not stopped by this controller");
                continue;
            };
            let dir = state.run_dir(&id);
            // A file that cannot be read is set aside, so one damaged run
            // does not keep the controller, and every other run, from starting.
            let journal = TraceJournal::open(journal_path(&dir)).or_else(|e| {
                tracing::warn!(run = %id, error = %e, "a run's journal was set aside");
                set_aside(&journal_path(&dir))?;
                TraceJournal::open(journal_path(&dir))
            })?;
            let ledger = Ledger::open(&dir).or_else(|e| {
                tracing::warn!(run = %id, error = %e, "a run's commands were set aside");
                set_aside(&dir.join("commands.json"))?;
                Ledger::open(&dir)
            })?;
            let interrupted = matches!(record.phase, Phase::Preparing | Phase::Running);
            if interrupted {
                record.phase = Phase::Stopping;
                record.outcome = Some(Outcome::Interrupted);
                record.error = Some(
                    "Brainiac's run controller stopped while the run was live. Updates after the last one recorded may be missing."
                        .into(),
                );
                record.save(&dir)?;
            }
            let cursor = journal.cursor();
            let run = Arc::new(Run {
                id: id.clone(),
                dir,
                inner: Mutex::new(RunInner {
                    record,
                    journal,
                    ledger,
                    redactor: None,
                    session: None,
                    activity: Activity::Stopping,
                    permissions: Vec::new(),
                    clock: None,
                    launching: false,
                    cancel_launch: None,
                    inflight: HashSet::new(),
                    last_stop_try: None,
                    journal_failed: false,
                    collecting: false,
                }),
                cursor: watch::Sender::new(cursor),
                stopping: AtomicBool::new(false),
                settled: Notify::new(),
                signals: signals.clone(),
            });
            if interrupted {
                run.append(
                    EventBody::Stopping {
                        outcome: Outcome::Interrupted,
                    },
                    true,
                );
            }
            runs.insert(id, run);
        }
        let controller = Arc::new(Self {
            state,
            installation,
            workloads,
            runs: Mutex::new(runs),
            signals,
            connections: Arc::new(AtomicUsize::new(0)),
            last_request: Mutex::new(Instant::now()),
            closing: AtomicBool::new(false),
            shutdown: Notify::new(),
            tasks: Mutex::new(Vec::new()),
        });
        Ok((controller, receiver))
    }

    fn runs(&self) -> MutexGuard<'_, HashMap<String, Arc<Run>>> {
        self.runs
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    fn run(&self, run_id: &str) -> AppResult<Arc<Run>> {
        self.runs()
            .get(run_id)
            .cloned()
            .ok_or_else(|| AppError::not_found("The run controller does not know this run."))
    }

    /// Decide to exit if no run is live, under the runs' lock so a start
    /// cannot slip in between. `true` when the controller is now closing.
    fn close_if_idle(&self) -> bool {
        let runs = self.runs();
        if live_in(&runs) > 0 {
            return false;
        }
        self.closing.store(true, Ordering::SeqCst);
        drop(runs);
        self.shutdown.notify_one();
        true
    }

    async fn dispatch(self: &Arc<Self>, request: Request) -> Response {
        *self.last_request.lock().unwrap_or_else(|p| p.into_inner()) = Instant::now();
        let result = match request {
            Request::Hello { .. } => Err(AppError::validation("Already connected.")),
            Request::Start(start) => self.start(start).map(|run| Response::Run { run }),
            Request::Status => {
                let mut runs: Vec<RunStatus> = self.runs().values().map(|r| r.status()).collect();
                runs.sort_by(|a, b| a.accepted_at.cmp(&b.accepted_at));
                Ok(Response::Runs { runs })
            }
            Request::Events {
                run_id,
                after,
                wait_ms,
            } => self
                .events(&run_id, after, wait_ms)
                .await
                .map(|page| Response::Events { page }),
            Request::Prompt {
                run_id,
                command_id,
                text,
            } => self
                .prompt(&run_id, &command_id, text)
                .await
                .map(|outcome| Response::Ack {
                    command_id,
                    outcome,
                }),
            Request::Permit {
                run_id,
                command_id,
                permission_id,
                allow,
            } => self
                .permit(&run_id, &command_id, &permission_id, allow)
                .await
                .map(|outcome| Response::Ack {
                    command_id,
                    outcome,
                }),
            Request::Stop { run_id, reason } => self
                .request_stop(&run_id, reason)
                .map(|run| Response::Run { run }),
            Request::Collect {
                run_id,
                include,
                out_dir,
                image,
            } => self
                .collect(&run_id, include, out_dir, image)
                .await
                .map(|manifest| Response::Collected { manifest }),
            Request::Discard { run_id, image } => {
                self.discard(&run_id, image).await.map(|()| Response::Done)
            }
            Request::Shutdown => {
                if self.close_if_idle() {
                    Ok(Response::Done)
                } else {
                    Err(conflict("Runs are live; the run controller keeps running."))
                }
            }
        };
        result.unwrap_or_else(|error| Response::Error { error })
    }

    fn start(self: &Arc<Self>, start: StartRun) -> AppResult<RunStatus> {
        check_start(&start)?;
        let mut runs = self.runs();
        if self.closing.load(Ordering::SeqCst) {
            return Err(AppError::dependency(
                "The run controller is shutting down; try again.",
            ));
        }
        if let Some(run) = runs.get(&start.run_id).cloned() {
            let same = run.lock().record.attempt == start.attempt;
            drop(runs);
            return if same {
                Ok(run.status())
            } else {
                Err(conflict("This run was already started."))
            };
        }
        let dir = self.state.run_dir(&start.run_id);
        let armed_wall = SystemTime::now();
        let limit = Duration::from_secs(start.time_limit_secs);
        let record = RunRecord {
            run_id: start.run_id.clone(),
            attempt: start.attempt,
            engine_socket: start.engine_socket.clone(),
            image: start.image.clone(),
            permissions: start.permissions,
            start_commit: start.start_commit.clone(),
            bundle: start.bundle.clone(),
            workspace_gib: start.workspace_gib,
            phase: Phase::Preparing,
            outcome: None,
            stop_confirmed: false,
            // From here the engine may hold something of the run.
            kept: true,
            container_id: None,
            volume: format!("brainiac-run-{}", start.run_id),
            session_id: None,
            model: None,
            turn: 0,
            accepted_at: wall(armed_wall),
            deadline_at: wall(armed_wall + limit),
            time_limit_secs: start.time_limit_secs,
            ended_at: None,
            expired_asleep: false,
            error: None,
        };
        record.save(&dir)?;
        let mut ledger = Ledger::open(&dir)?;
        ledger.begin(&start.prompt_id)?;
        let journal = TraceJournal::open(journal_path(&dir))?;
        let run = Arc::new(Run {
            id: start.run_id.clone(),
            dir,
            inner: Mutex::new(RunInner {
                record,
                journal,
                ledger,
                redactor: None,
                session: None,
                activity: Activity::Preparing,
                permissions: Vec::new(),
                // Armed now, when the controller accepts the start; no client
                // connection keeps it or cancels it.
                clock: Some(Clock::new(limit)),
                launching: true,
                cancel_launch: None,
                inflight: HashSet::from([start.prompt_id.clone()]),
                last_stop_try: None,
                journal_failed: false,
                collecting: false,
            }),
            cursor: watch::Sender::new(0),
            stopping: AtomicBool::new(false),
            settled: Notify::new(),
            signals: self.signals.clone(),
        });
        runs.insert(start.run_id.clone(), Arc::clone(&run));
        drop(runs);
        let deadline_at = run.lock().record.deadline_at.clone();
        run.append(
            EventBody::Accepted {
                deadline_at,
                permissions: start.permissions,
            },
            true,
        );
        let status = run.status();
        let controller = Arc::clone(self);
        self.spawn(async move { controller.prepare(run, start).await });
        Ok(status)
    }

    /// Create the container, open the session, and send the run's prompt.
    async fn prepare(self: Arc<Self>, run: Arc<Run>, start: StartRun) {
        let StartRun {
            credential,
            prompt_id,
            prompt,
            permissions,
            ..
        } = start.clone();
        let mut redactor = Redactor::new();
        redactor.register(&credential.value, credential.key.as_str());
        let redactor = Arc::new(redactor);
        run.lock().redactor = Some(Arc::clone(&redactor));
        let frame = frame(credential);
        let (cancel_launch, cancelled) = watch::channel(false);
        run.lock().cancel_launch = Some(cancel_launch);
        let spec = LaunchSpec {
            engine_socket: start.engine_socket.clone(),
            image: start.image.clone(),
            installation: self.installation.clone(),
            run_id: start.run_id.clone(),
            attempt: start.attempt,
            volume: run.lock().record.volume.clone(),
            cpus: start.cpus,
            memory_mib: start.memory_mib,
            workspace_gib: start.workspace_gib,
            model: start.model.clone(),
            bundle: start.bundle.clone(),
            cancel: cancelled,
        };
        let launched = match tokio::time::timeout(LAUNCH_WAIT, self.workloads.launch(spec)).await {
            Ok(result) => result,
            Err(_) => Err(AppError::timeout(
                "The run's container did not start within 20 minutes.",
            )),
        };
        let (commands, receiver) = mpsc::unbounded_channel();
        // The lock is let go before anything below waits on the engine.
        let attached = {
            let mut inner = run.lock();
            inner.launching = false;
            inner.cancel_launch = None;
            match launched {
                Ok(attached) => {
                    inner.record.container_id = Some(attached.container_id.clone());
                    Run::save(&inner, &run.dir);
                    let live = inner.record.phase == Phase::Preparing;
                    if live {
                        // In the same step as `launching`, so a stop sees either
                        // a container being made or a session to cancel.
                        inner.session = Some(commands.clone());
                    }
                    Ok(live.then_some(attached))
                }
                Err(error) => Err(error),
            }
        };
        let attached = match attached {
            Ok(Some(attached)) => attached,
            // Stopped while the container was being made: stop it now.
            Ok(None) => {
                let mut frame = frame;
                frame.fill(0);
                let _ = run.settle(&prompt_id, None);
                self.finish_stop(&run).await;
                return;
            }
            Err(error) => {
                tracing::warn!(error = %error, details = ?error.details, "a run did not start");
                let mut frame = frame;
                frame.fill(0);
                let _ = run.settle(&prompt_id, None);
                self.stop_run(&run, Outcome::Failed, Some(error.message), false)
                    .await;
                // A stop asked for while the container was being made.
                self.finish_stop(&run).await;
                return;
            }
        };
        let session = Session::new(Arc::clone(&run), redactor, attached.stdin, permissions);
        self.spawn(session.run(frame, attached.output, receiver));
        let (done, answer) = oneshot::channel();
        let sent = commands.send(Command::Prompt {
            command_id: prompt_id.clone(),
            text: prompt,
            first: true,
            done,
        });
        let outcome = match (sent, answer.await) {
            (Ok(()), Ok(Ok(()))) => Some(Delivery::Delivered),
            (Ok(()), Err(_)) => Some(Delivery::Uncertain),
            _ => None,
        };
        let _ = run.settle(&prompt_id, outcome);
    }

    /// Ask a run to stop; the stop itself goes on in the background.
    fn request_stop(self: &Arc<Self>, run_id: &str, reason: StopReason) -> AppResult<RunStatus> {
        let run = self.run(run_id)?;
        let refusal = {
            let inner = run.lock();
            match inner.record.phase {
                Phase::Ended | Phase::Stopping => Some(None),
                Phase::Preparing if reason == StopReason::Finish => Some(Some(
                    "Finish is possible once the agent is ready; cancel the run instead.",
                )),
                Phase::Running
                    if reason == StopReason::Finish
                        && !matches!(inner.activity, Activity::Idle | Activity::PlanLimit) =>
                {
                    Some(Some(
                        "Finish and collect is possible only while the agent waits for a prompt.",
                    ))
                }
                _ => None,
            }
        };
        // The lock is let go first: `status` takes it again.
        match refusal {
            Some(None) => return Ok(run.status()),
            Some(Some(message)) => return Err(conflict(message)),
            None => {}
        }
        let outcome = match reason {
            StopReason::Finish => Outcome::Finished,
            StopReason::Cancel => Outcome::Cancelled,
        };
        // Answer with the run stopping, without waiting for the engine.
        self.stop_later(&run, outcome, None, false);
        Ok(run.status())
    }

    /// Stop a run with this outcome: the session cancels its turn and every
    /// pending permission, then the engine stops the container and keeps it.
    async fn stop_run(
        self: &Arc<Self>,
        run: &Arc<Run>,
        outcome: Outcome,
        error: Option<String>,
        asleep: bool,
    ) {
        if let Some(stop) = self.begin_stop(run, outcome, error, asleep) {
            self.complete_stop(run, stop).await;
        }
    }

    /// The decision to stop, at once and in order: the first outcome wins.
    /// `None` when the run is already stopping or ended.
    fn begin_stop(
        &self,
        run: &Run,
        outcome: Outcome,
        error: Option<String>,
        asleep: bool,
    ) -> Option<Stop> {
        let stop = {
            let mut inner = run.lock();
            if matches!(inner.record.phase, Phase::Stopping | Phase::Ended) {
                return None;
            }
            inner.record.phase = Phase::Stopping;
            inner.record.outcome = Some(outcome);
            inner.record.error = error;
            inner.record.expired_asleep = asleep;
            inner.activity = Activity::Stopping;
            inner.clock = None;
            Run::save(&inner, &run.dir);
            if let Some(cancel) = &inner.cancel_launch {
                // The container being made is not started; `prepare` then stops
                // what was made.
                let _ = cancel.send(true);
            }
            Stop {
                session: inner.session.clone(),
                launching: inner.launching,
            }
        };
        run.append(EventBody::Stopping { outcome }, true);
        Some(stop)
    }

    /// Begin a stop now and finish it in the background.
    fn stop_later(
        self: &Arc<Self>,
        run: &Arc<Run>,
        outcome: Outcome,
        error: Option<String>,
        asleep: bool,
    ) {
        if let Some(stop) = self.begin_stop(run, outcome, error, asleep) {
            let controller = Arc::clone(self);
            let run = Arc::clone(run);
            self.spawn(async move { controller.complete_stop(&run, stop).await });
        }
    }

    async fn complete_stop(self: &Arc<Self>, run: &Arc<Run>, stop: Stop) {
        let Stop { session, launching } = stop;
        if let Some(session) = session {
            let (done, cancelled) = oneshot::channel();
            if session.send(Command::Cancel { done }).is_ok() {
                let _ = tokio::time::timeout(Duration::from_secs(2), cancelled).await;
            }
        }
        // A container still being made is stopped once it exists.
        if !launching {
            self.finish_stop(run).await;
        }
    }

    /// Stop the run's containers and confirm it with the engine. Until the
    /// engine confirms, the run stays stopping and this is tried again.
    async fn finish_stop(self: &Arc<Self>, run: &Arc<Run>) {
        if run.stopping.swap(true, Ordering::SeqCst) {
            return;
        }
        let socket = {
            let mut inner = run.lock();
            inner.last_stop_try = Some(Instant::now());
            (inner.record.phase == Phase::Stopping).then(|| inner.record.engine_socket.clone())
        };
        if let Some(socket) = socket {
            let confirmed = match self
                .workloads
                .stop(&socket, &self.installation, &run.id)
                .await
            {
                Ok(()) => {
                    self.workloads
                        .running(&socket, &self.installation, &run.id)
                        .await
                }
                Err(error) => Err(error),
            };
            let ended = {
                let mut inner = run.lock();
                match confirmed {
                    Ok(Running::No) => {
                        inner.record.phase = Phase::Ended;
                        inner.record.stop_confirmed = true;
                        inner.record.ended_at = Some(now());
                        inner.activity = Activity::Ended;
                        inner.session = None;
                        inner.permissions.clear();
                        Run::save(&inner, &run.dir);
                        Some((
                            inner.record.outcome.unwrap_or(Outcome::Interrupted),
                            inner.record.error.clone(),
                        ))
                    }
                    Ok(Running::Yes) => None,
                    Err(error) => {
                        tracing::warn!(error = %error, details = ?error.details, "a run's stop is not confirmed");
                        None
                    }
                }
            };
            if let Some((outcome, message)) = ended {
                run.append(EventBody::Ended { outcome, message }, true);
            }
        }
        run.stopping.store(false, Ordering::SeqCst);
    }

    async fn events(&self, run_id: &str, after: u64, wait_ms: u64) -> AppResult<EventPage> {
        let run = self.run(run_id)?;
        let mut cursor = run.cursor.subscribe();
        let wait = Duration::from_millis(wait_ms).min(MAX_WAIT);
        if !wait.is_zero() && *cursor.borrow() <= after {
            let _ = tokio::time::timeout(wait, cursor.wait_for(|c| *c > after)).await;
        }
        let inner = run.lock();
        Ok(EventPage {
            run_id: run_id.to_string(),
            events: inner.journal.after(after)?,
            cursor: inner.journal.cursor(),
        })
    }

    async fn prompt(&self, run_id: &str, command_id: &str, text: String) -> AppResult<Delivery> {
        if !valid_id(command_id) {
            return Err(AppError::validation("Invalid command ID."));
        }
        check_prompt(&text)?;
        let run = self.run(run_id)?;
        // The outcome check and the intent are one step under the lock, so
        // two repeats arriving together cannot both send.
        let session = {
            let mut inner = run.lock();
            match inner.ledger.outcome(command_id) {
                Some(outcome) => Err(outcome),
                None => {
                    if inner.record.phase != Phase::Running {
                        return Err(not_live());
                    }
                    if !matches!(inner.activity, Activity::Idle | Activity::PlanLimit) {
                        return Err(conflict(
                            "The agent is working; send the next prompt when its turn ends.",
                        ));
                    }
                    let session = inner.session.clone().ok_or_else(not_live)?;
                    inner.ledger.begin(command_id)?;
                    inner.inflight.insert(command_id.to_string());
                    Ok(session)
                }
            }
        };
        let session = match session {
            Ok(session) => session,
            Err(seen) => return Ok(self.first_outcome(&run, command_id, seen).await),
        };
        let (done, answer) = oneshot::channel();
        let sent = session.send(Command::Prompt {
            command_id: command_id.to_string(),
            text,
            first: false,
            done,
        });
        self.settle(&run, command_id, sent.is_ok(), answer).await
    }

    async fn permit(
        &self,
        run_id: &str,
        command_id: &str,
        permission_id: &str,
        allow: bool,
    ) -> AppResult<Delivery> {
        if !valid_id(command_id) || !valid_id(permission_id) {
            return Err(AppError::validation("Invalid command or permission ID."));
        }
        let run = self.run(run_id)?;
        // The outcome check and the intent are one step under the lock, so
        // two repeats arriving together cannot both send.
        let session = {
            let mut inner = run.lock();
            match inner.ledger.outcome(command_id) {
                Some(outcome) => Err(outcome),
                None => {
                    if inner.record.phase != Phase::Running {
                        // After Cancel or expiry a late answer has no effect.
                        return Err(not_live());
                    }
                    if !inner
                        .permissions
                        .iter()
                        .any(|p| p.permission_id == permission_id)
                    {
                        return Err(AppError::not_found("That permission is no longer waiting."));
                    }
                    let session = inner.session.clone().ok_or_else(not_live)?;
                    inner.ledger.begin(command_id)?;
                    inner.inflight.insert(command_id.to_string());
                    Ok(session)
                }
            }
        };
        let session = match session {
            Ok(session) => session,
            Err(seen) => return Ok(self.first_outcome(&run, command_id, seen).await),
        };
        let (done, answer) = oneshot::channel();
        let sent = session.send(Command::Permit {
            permission_id: permission_id.to_string(),
            allow,
            done,
        });
        self.settle(&run, command_id, sent.is_ok(), answer).await
    }

    /// The outcome of a command seen before. One still on its way waits
    /// for its outcome, so a repeat after a timeout gets the same answer as
    /// the first; only a command an earlier controller never settled stays
    /// uncertain.
    async fn first_outcome(&self, run: &Run, command_id: &str, seen: Delivery) -> Delivery {
        let deadline = tokio::time::Instant::now() + MAX_WAIT;
        loop {
            // Created under the lock, before it is let go, so a settle in
            // between is not missed.
            let wait = {
                let inner = run.lock();
                if !inner.inflight.contains(command_id) {
                    return inner.ledger.outcome(command_id).unwrap_or(seen);
                }
                run.settled.notified()
            };
            if tokio::time::timeout_at(deadline, wait).await.is_err() {
                return Delivery::Uncertain;
            }
        }
    }

    /// Record what became of a command the session was given.
    async fn settle(
        &self,
        run: &Run,
        command_id: &str,
        sent: bool,
        answer: oneshot::Receiver<AppResult<()>>,
    ) -> AppResult<Delivery> {
        let answer = if sent {
            answer.await.ok()
        } else {
            Some(Err(not_live()))
        };
        match answer {
            Some(Ok(())) => {
                run.settle(command_id, Some(Delivery::Delivered))?;
                Ok(Delivery::Delivered)
            }
            // Refused before anything was written: the same ID may try again.
            Some(Err(error)) => {
                run.settle(command_id, None)?;
                Err(error)
            }
            // The session ended while it had the command.
            None => {
                run.settle(command_id, Some(Delivery::Uncertain))?;
                Ok(Delivery::Uncertain)
            }
        }
    }

    /// Collect a stopped run's working tree: the collector container runs
    /// against the run's volume, read only, and its bundle lands in
    /// `out_dir`. The volume is unchanged, so it can be repeated.
    async fn collect(
        &self,
        run_id: &str,
        include: Vec<String>,
        out_dir: PathBuf,
        fallback_image: Option<String>,
    ) -> AppResult<CollectManifest> {
        let run = self.run(run_id)?;
        if include.len() > MAX_INCLUDE || include.iter().any(|p| !safe_relative(p)) {
            return Err(AppError::validation(
                "The files to add must be paths inside the workspace, at most 1,000 of them.",
            ));
        }
        if !out_dir.is_absolute() || !out_dir.is_dir() {
            return Err(AppError::validation(
                "The collection needs an existing folder to write into.",
            ));
        }
        let spec = {
            let mut inner = run.lock();
            if inner.record.phase != Phase::Ended || !inner.record.stop_confirmed {
                return Err(conflict(
                    "The run has not stopped; its work can be collected once it has.",
                ));
            }
            if !inner.record.kept {
                return Err(conflict("The run's work was discarded."));
            }
            if inner.record.start_commit.is_empty() {
                return Err(conflict("This run's start commit is not known."));
            }
            if inner.collecting {
                return Err(conflict("The run's work is already being collected."));
            }
            inner.collecting = true;
            CollectSpec {
                engine_socket: inner.record.engine_socket.clone(),
                image: inner.record.image.clone(),
                fallback_image,
                installation: self.installation.clone(),
                run_id: run_id.to_string(),
                attempt: inner.record.attempt,
                volume: inner.record.volume.clone(),
                memory_mib: COLLECTOR_MEMORY_MIB,
                bundle: inner.record.bundle.clone(),
                start_commit: inner.record.start_commit.clone(),
                include,
                out_dir,
            }
        };
        let result = self.workloads.collect(spec).await;
        run.lock().collecting = false;
        result
    }

    async fn discard(&self, run_id: &str, fallback_image: Option<String>) -> AppResult<()> {
        let run = self.run(run_id)?;
        let (socket, volume, image) = {
            let inner = run.lock();
            if inner.record.phase != Phase::Ended || !inner.record.stop_confirmed {
                return Err(conflict(
                    "The run has not stopped; it cannot be discarded yet.",
                ));
            }
            if inner.collecting {
                return Err(conflict(
                    "The run's work is being collected; discard it afterwards.",
                ));
            }
            (
                inner.record.engine_socket.clone(),
                inner.record.volume.clone(),
                inner.record.image.clone(),
            )
        };
        self.workloads
            .discard(
                &socket,
                &self.installation,
                run_id,
                &volume,
                &image,
                fallback_image.as_deref(),
            )
            .await?;
        {
            let mut inner = run.lock();
            inner.record.kept = false;
            Run::save(&inner, &run.dir);
        }
        self.runs().remove(run_id);
        std::fs::remove_dir_all(&run.dir)?;
        Ok(())
    }

    /// Once a second: expire runs past their time limit, retry unconfirmed
    /// stops, and exit when idle.
    async fn tick(self: &Arc<Self>, idle_exit: Option<Duration>) {
        let runs: Vec<Arc<Run>> = self.runs().values().cloned().collect();
        for run in runs {
            let (expired, retry) = {
                let inner = run.lock();
                let expired = inner.clock.as_ref().and_then(Clock::expired);
                let retry = inner.record.phase == Phase::Stopping
                    && !inner.launching
                    && inner
                        .last_stop_try
                        .is_none_or(|t| t.elapsed() >= STOP_RETRY);
                (expired, retry)
            };
            if let Some(asleep) = expired {
                self.stop_later(&run, Outcome::Expired, None, asleep);
            } else if retry {
                let controller = Arc::clone(self);
                self.spawn(async move { controller.finish_stop(&run).await });
            }
        }
        if let Some(idle) = idle_exit {
            let quiet = self
                .last_request
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .elapsed()
                >= idle;
            if quiet && self.connections.load(Ordering::SeqCst) == 0 {
                self.close_if_idle();
            }
        }
    }

    /// Apply a session's signal. Signals come in order, and each decision is
    /// made before the next is read: a failure reported just before the
    /// stream closed stays a failure.
    fn signal(self: &Arc<Self>, signal: Signal) {
        match signal {
            Signal::Failed { run_id, message } => {
                if let Ok(run) = self.run(&run_id) {
                    self.stop_later(&run, Outcome::Failed, Some(message), false);
                }
            }
            Signal::Closed { run_id } => {
                let Ok(run) = self.run(&run_id) else { return };
                let phase = run.lock().record.phase;
                match phase {
                    // The container ended on its own.
                    Phase::Preparing | Phase::Running => {
                        let message = "The agent's container stopped unexpectedly.".to_string();
                        self.stop_later(&run, Outcome::Interrupted, Some(message), false);
                    }
                    Phase::Stopping => {
                        let controller = Arc::clone(self);
                        self.spawn(async move { controller.finish_stop(&run).await });
                    }
                    Phase::Ended => {}
                }
            }
        }
    }

    /// Stop every live run as interrupted: the controller is being stopped
    /// (the Mac logs out or restarts).
    async fn interrupt_all(self: &Arc<Self>) {
        let runs: Vec<Arc<Run>> = self.runs().values().cloned().collect();
        let stops = runs.iter().map(|run| {
            self.stop_run(
                run,
                Outcome::Interrupted,
                Some(
                    "Brainiac's run controller was stopped, as when the Mac logs out or restarts."
                        .into(),
                ),
                false,
            )
        });
        futures_util::future::join_all(stops).await;
    }
}

fn check_start(start: &StartRun) -> AppResult<()> {
    if !valid_id(&start.run_id) || !valid_id(&start.prompt_id) || start.attempt == 0 {
        return Err(AppError::validation("Invalid run, attempt, or prompt ID."));
    }
    if !(MIN_TIME_LIMIT..=MAX_TIME_LIMIT).contains(&start.time_limit_secs) {
        return Err(AppError::validation(
            "A run's time limit is at most 8 hours.",
        ));
    }
    if !(1..=64).contains(&start.cpus)
        || !(512..=262_144).contains(&start.memory_mib)
        || !(1..=500).contains(&start.workspace_gib)
    {
        return Err(AppError::validation(
            "The run's CPU, memory, or workspace limit is out of range.",
        ));
    }
    if start.engine_socket.is_empty() || start.image.is_empty() {
        return Err(AppError::validation(
            "The run needs an engine and an image.",
        ));
    }
    if !start.bundle.is_file() {
        return Err(AppError::not_found("The run's start was not exported."));
    }
    let value = &start.credential.value;
    if !(16..=4096).contains(&value.len()) || !value.chars().all(|c| c.is_ascii_graphic()) {
        return Err(AppError::validation(
            "The token or key is not one line of printable characters.",
        ));
    }
    check_prompt(&start.prompt)
}

fn check_prompt(text: &str) -> AppResult<()> {
    if text.trim().is_empty() {
        return Err(AppError::validation("The prompt is empty."));
    }
    if text.len() > MAX_PROMPT_BYTES {
        return Err(AppError::validation("The prompt is over 100 KB."));
    }
    Ok(())
}

/// The credential frame (docs/architecture.md, Agent runs — v0.5,
/// Credentials): `BRB1`, a 4-byte big-endian length, then JSON with exactly
/// the credential's key. The caller's copy of the value is overwritten.
/// A path inside the workspace: relative, no `..`, no empty part.
fn safe_relative(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 4096
        && !path.starts_with('/')
        && !path.contains('\0')
        && !path.contains('\n')
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

pub fn frame(credential: Credential) -> Vec<u8> {
    let Credential { key, value } = credential;
    let mut payload =
        serde_json::to_vec(&serde_json::json!({ key.as_str(): value })).expect("JSON serializes");
    let mut frame = Vec::with_capacity(8 + payload.len());
    frame.extend_from_slice(b"BRB1");
    frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    frame.extend_from_slice(&payload);
    payload.fill(0);
    let mut value = value.into_bytes();
    value.fill(0);
    frame
}

/// Read one line of at most `max` bytes; `None` at the end of the stream.
pub async fn read_line<R: AsyncRead + Unpin>(
    reader: &mut BufReader<R>,
    max: usize,
) -> std::io::Result<Option<Vec<u8>>> {
    let mut line = Vec::new();
    let n = (&mut *reader)
        .take(max as u64 + 1)
        .read_until(b'\n', &mut line)
        .await?;
    if n == 0 {
        return Ok(None);
    }
    if line.last() != Some(&b'\n') {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            if n > max {
                "line too long"
            } else {
                "line cut short"
            },
        ));
    }
    line.pop();
    Ok(Some(line))
}

pub async fn write_line<T: serde::Serialize, W: tokio::io::AsyncWrite + Unpin>(
    writer: &mut W,
    value: &T,
) -> std::io::Result<()> {
    let mut bytes = serde_json::to_vec(value).map_err(std::io::Error::other)?;
    bytes.push(b'\n');
    writer.write_all(&bytes).await?;
    writer.flush().await
}

/// Compare two tokens without stopping at the first difference.
fn same_token(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

async fn connection<W: Workloads>(
    controller: Arc<Controller<W>>,
    stream: UnixStream,
    token: Arc<String>,
) {
    // Only this user's processes: the socket is mode 600, and this checks it.
    if stream.peer_cred().map(|c| c.uid()).ok() != Some(crate::mcp::current_uid()) {
        return;
    }
    let (read, mut write) = stream.into_split();
    let mut reader = BufReader::new(read);
    // A client that never says hello is let go, so it cannot keep the
    // controller from exiting when idle.
    let hello = tokio::time::timeout(Duration::from_secs(10), read_line(&mut reader, 4096)).await;
    let welcomed = match hello.unwrap_or(Ok(None)) {
        Ok(Some(line)) => matches!(
            serde_json::from_slice::<Request>(&line),
            Ok(Request::Hello { token: given, .. }) if same_token(&given, &token)
        ),
        _ => false,
    };
    let first = if welcomed {
        Response::Welcome {
            protocol: PROTOCOL,
            installation: controller.installation.clone(),
            build: env!("CARGO_PKG_VERSION").to_string(),
            pid: std::process::id(),
        }
    } else {
        Response::Error {
            error: AppError::new(ErrorCode::PermissionDenied, "Not authorized."),
        }
    };
    if write_line(&mut write, &first).await.is_err() || !welcomed {
        return;
    }
    loop {
        let line = match read_line(&mut reader, MAX_LINE_BYTES).await {
            Ok(Some(line)) => line,
            _ => return,
        };
        let response = match serde_json::from_slice::<Request>(&line) {
            Ok(request) => controller.dispatch(request).await,
            Err(_) => Response::Error {
                error: AppError::validation("The run controller did not understand the request."),
            },
        };
        if write_line(&mut write, &response).await.is_err() {
            return;
        }
    }
}

/// Counts a connection from when it is accepted until its task ends.
struct Counted(Arc<AtomicUsize>);

impl Counted {
    fn new(count: &Arc<AtomicUsize>) -> Self {
        count.fetch_add(1, Ordering::SeqCst);
        Self(Arc::clone(count))
    }
}

// `Drop` runs when the connection's task ends, however it ends.
impl Drop for Counted {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Stops every task of a controller when `serve` returns or is dropped.
struct Owned<W: Workloads>(Arc<Controller<W>>);

impl<W: Workloads> Drop for Owned<W> {
    fn drop(&mut self) {
        self.0.abort_all();
    }
}

/// Set a damaged file aside, next to where it was.
fn set_aside(path: &std::path::Path) -> std::io::Result<()> {
    let mut aside = path.as_os_str().to_owned();
    aside.push(".damaged");
    std::fs::rename(path, aside)
}

/// Runs the controller must stay for: not ended, or being collected.
fn live_in(runs: &HashMap<String, Arc<Run>>) -> usize {
    runs.values()
        .filter(|run| {
            let inner = run.lock();
            inner.record.phase != Phase::Ended || inner.collecting
        })
        .count()
}

/// Bind the controller's socket. The lock is held, so a socket already
/// there is an earlier controller's.
fn bind(socket: &std::path::Path) -> AppResult<UnixListener> {
    use std::os::unix::fs::PermissionsExt;
    if socket.as_os_str().len() > crate::mcp::MAX_SOCKET_PATH {
        return Err(AppError::validation(
            "The run controller's socket path is too long for a socket.",
        )
        .with_details(socket.display().to_string()));
    }
    crate::mcp::prepare_socket_dir(socket)?;
    if std::fs::symlink_metadata(socket).is_ok() {
        if !crate::mcp::owned_socket(socket) {
            return Err(AppError::new(
                ErrorCode::PermissionDenied,
                "Something other than the run controller's socket is in its place.",
            )
            .with_details(socket.display().to_string()));
        }
        std::fs::remove_file(socket)?;
    }
    let listener = UnixListener::bind(socket)?;
    std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

/// Run the controller until it is idle, asked to shut down, or stopped.
pub async fn serve<W: Workloads>(config: Config, workloads: W) -> AppResult<()> {
    config.state.ensure()?;
    // An exiting controller's guard holds the lock while it checks what was
    // left; a new controller waits for it rather than giving up.
    let mut tries = 0;
    let _lock = loop {
        match config.state.lock()? {
            Some(lock) => break lock,
            None if tries < 50 => {
                tries += 1;
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            None => return Err(conflict("Another run controller is already running.")),
        }
    };
    let token = Arc::new(config.state.token()?);
    let installation = config.state.installation()?;
    config.state.write_pid()?;
    let listener = bind(&config.socket)?;
    let (controller, mut signals) =
        Controller::load(config.state.clone(), installation, workloads)?;
    let _owned = Owned(Arc::clone(&controller));

    // Runs an earlier controller left live are stopped now.
    controller.tick(None).await;
    if config.spawn_guard {
        guard::spawn(&config.state)?;
    }
    let signal_controller = Arc::clone(&controller);
    controller.spawn(async move {
        while let Some(signal) = signals.recv().await {
            signal_controller.signal(signal);
        }
    });
    let ticker = Arc::clone(&controller);
    let idle_exit = config.idle_exit;
    controller.spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        loop {
            interval.tick().await;
            ticker.tick(idle_exit).await;
        }
    });

    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    let stopped = loop {
        tokio::select! {
            _ = controller.shutdown.notified() => break false,
            _ = terminate.recv() => break true,
            _ = interrupt.recv() => break true,
            accepted = listener.accept() => {
                let Ok((stream, _)) = accepted else { continue };
                // Counted before its task starts, so an idle check in between
                // still sees it.
                let counted = Counted::new(&controller.connections);
                let connected = Arc::clone(&controller);
                let token = Arc::clone(&token);
                controller.spawn(async move {
                    let _counted = counted;
                    connection(connected, stream, token).await;
                });
            }
        }
    };
    if stopped {
        let _ = tokio::time::timeout(Duration::from_secs(25), controller.interrupt_all()).await;
    }
    let _ = std::fs::remove_file(&config.socket);
    let _ = std::fs::remove_file(config.state.pid_path());
    Ok(())
}

/// `brainiac runner …`: the controller, or its guard. Returns the exit code.
pub fn main(args: &[String]) -> i32 {
    let value = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let Some(state) = value("--state").map(StateDir::new) else {
        eprintln!("brainiac runner: --state is required");
        return 2;
    };
    // Warnings go to stderr, which the app points at `runner.log`. They
    // carry safe messages and engine details, never a prompt or credential.
    let _ = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .with_env_filter(tracing_subscriber::EnvFilter::new("warn"))
        .try_init();
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("brainiac runner: {e}");
            return 1;
        }
    };
    let result = if args.first().map(String::as_str) == Some("guard") {
        let Some(pid) = value("--pid").and_then(|p| p.parse().ok()) else {
            eprintln!("brainiac runner guard: --pid is required");
            return 2;
        };
        runtime.block_on(guard::watch(state, pid, docker::DockerEngine::new()));
        Ok(())
    } else {
        let Some(socket) = value("--socket").map(PathBuf::from) else {
            eprintln!("brainiac runner: --socket is required");
            return 2;
        };
        let config = Config {
            state,
            socket,
            idle_exit: Some(Duration::from_secs(10 * 60)),
            spawn_guard: true,
        };
        runtime.block_on(serve(config, docker::DockerEngine::new()))
    };
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("brainiac runner: {}", e.message);
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sleep_past_the_deadline_expires_the_run_on_wake() {
        // Measured on a real Mac sleep: about 393 s passed while the Mac was
        // awake for 30 s of them.
        let limit = Duration::from_secs(60);
        assert_eq!(
            past_limit(limit, Duration::from_secs(393), Duration::from_secs(30)),
            Some(true)
        );
        // Reached while awake.
        assert_eq!(
            past_limit(limit, Duration::from_secs(61), Duration::from_secs(61)),
            Some(false)
        );
    }

    #[test]
    fn a_short_sleep_does_not_expire_the_run() {
        assert_eq!(
            past_limit(
                Duration::from_secs(3600),
                Duration::from_secs(400),
                Duration::from_secs(30)
            ),
            None
        );
    }

    #[test]
    fn the_deadline_clock_counts_at_least_the_awake_time() {
        // Setting the Mac's clock moves neither; sleep moves only `since_boot`.
        let clock = Clock::new(Duration::from_secs(3600));
        std::thread::sleep(Duration::from_millis(50));
        let elapsed = since_boot().saturating_sub(clock.armed);
        assert!(elapsed >= Duration::from_millis(45), "{elapsed:?}");
        assert_eq!(clock.expired(), None);
    }

    #[test]
    fn the_frame_carries_exactly_the_credentials_key() {
        let frame = frame(Credential {
            key: protocol::CredentialKey::ClaudeCodeOauthToken,
            value: "sk-ant-oat01-example-token-value".into(),
        });
        assert_eq!(&frame[..4], b"BRB1");
        let len = u32::from_be_bytes(frame[4..8].try_into().unwrap()) as usize;
        assert_eq!(frame.len(), 8 + len);
        let value: serde_json::Value = serde_json::from_slice(&frame[8..]).unwrap();
        let object = value.as_object().unwrap();
        assert_eq!(object.len(), 1);
        assert_eq!(
            object["CLAUDE_CODE_OAUTH_TOKEN"],
            "sk-ant-oat01-example-token-value"
        );
    }

    #[test]
    fn tokens_compare_whole() {
        assert!(same_token("abc", "abc"));
        assert!(!same_token("abc", "abd"));
        assert!(!same_token("abc", "abcd"));
    }
}
