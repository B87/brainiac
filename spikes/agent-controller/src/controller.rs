//! The process that owns the agent attach. Client tasks only read and write
//! the journal under a mutex; they never hold that mutex across a Docker
//! call. A sleeping client is not in this loop at all, because responses are
//! sent only when a request arrives.

use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime};

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};
use tokio::time::{timeout, Duration};

use crate::acp::{self, Note, PermitChoice, PromptJob};
use crate::engine::{work_volume, Engine, Line};
use crate::journal::Journal;
use crate::ledger::{Begin, Ledger};
use crate::messages::{Request, Response};
use crate::state::{RunRecord, StateDir, PORT};

const REPLAY_LIMIT: usize = 200;

struct Hub {
    state: StateDir,
    engine: Engine,
    journal: Journal,
    ledger: Ledger,
    run: Option<LiveRun>,
    starting: bool,
    interrupted: bool,
    stdin: Option<mpsc::UnboundedSender<Vec<u8>>>,
    /// In-memory filtering material for this session. It is not written to
    /// the journal, the ledger, or the run record.
    secret: Option<String>,
    claude: Option<ClaudeLink>,
    /// One permission the agent has asked for and the user has not answered.
    /// It stays while the Mac is disconnected. Expiry clears it.
    pending: Option<String>,
    /// Monotonic instant when this run must stop. `None` for a Claude start
    /// in this spike. A dropped client does not clear it.
    deadline: Option<Instant>,
    /// Set once `expire` has been entered for this run, so the monotonic
    /// timer and the wake check cannot both stop it.
    expiry_for: Option<String>,
}

struct ClaudeLink {
    prompts: mpsc::UnboundedSender<PromptJob>,
    permits: mpsc::UnboundedSender<PermitChoice>,
}

struct LiveRun {
    #[allow(dead_code)]
    run_id: String,
    container_id: String,
    tty: bool,
    log_driver: String,
}

pub async fn serve(state: StateDir) -> anyhow::Result<()> {
    state.ensure()?;
    let token = state.token()?;
    // The pid file is written before any container work so the guard can
    // tell a live controller from a crashed one.
    state.write_pid()?;
    let engine = Engine::connect()?;
    let described = engine.describe().await;
    let interrupted = mark_interrupted(&state);
    // Stop, never remove: an interrupted run's container and volume are its
    // work until the user collects or discards it.
    let stopped = engine.stop_labeled().await?;
    eprintln!(
        "controller listening on 127.0.0.1:{PORT} stopped={stopped} interrupted={interrupted} engine={described}"
    );

    let hub = Arc::new(Mutex::new(Hub {
        journal: Journal::open(state.journal_path())?,
        ledger: Ledger::open(state.ledger_path())?,
        run: None,
        starting: false,
        interrupted,
        stdin: None,
        secret: None,
        claude: None,
        pending: None,
        deadline: None,
        expiry_for: None,
        state,
        engine,
    }));

    let listener = TcpListener::bind(("127.0.0.1", PORT)).await?;
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            _ = terminate.recv() => break,
            accepted = listener.accept() => {
                let (socket, _) = accepted?;
                let hub = Arc::clone(&hub);
                let token = token.clone();
                tokio::spawn(async move {
                    if let Err(err) = connection(socket, hub, token).await {
                        eprintln!("client closed: {err}");
                    }
                });
            }
        }
    }
    // A live run ends here because the controller stops, not because the
    // agent finished: record that before the attach closes and clears it.
    let engine = {
        let guard = lock(&hub);
        mark_interrupted(&guard.state);
        guard.engine.clone()
    };
    let _ = engine.stop_labeled().await;
    lock(&hub).state.remove_pid()?;
    eprintln!("controller stopped");
    Ok(())
}

fn mark_interrupted(state: &StateDir) -> bool {
    let Some(mut run) = state.load_run() else {
        return false;
    };
    if !run.live {
        return run.interrupted;
    }
    run.live = false;
    run.interrupted = true;
    let _ = state.save_run(&run);
    true
}

/// `Arc<Mutex<_>>` is how connection tasks and the attach reader share one
/// journal. The lock is not held across a `.await`.
fn lock(hub: &Mutex<Hub>) -> std::sync::MutexGuard<'_, Hub> {
    hub.lock().unwrap_or_else(|poison| poison.into_inner())
}

async fn connection(socket: TcpStream, hub: Arc<Mutex<Hub>>, token: String) -> anyhow::Result<()> {
    let _ = socket.set_nodelay(true);
    let (read, mut write) = socket.into_split();
    let mut read = BufReader::new(read);
    let mut line = String::new();
    read_request_line(&mut read, &mut line).await?;
    let authed = serde_json::from_str::<serde_json::Value>(&line)
        .ok()
        .and_then(|value| {
            value
                .get("token")
                .and_then(|token| token.as_str())
                .map(str::to_string)
        });
    line.clear();
    if authed.as_deref() != Some(token.as_str()) {
        write_response(
            &mut write,
            &Response::Err {
                message: "unauthorized".into(),
            },
        )
        .await?;
        return Ok(());
    }
    loop {
        if read_request_line(&mut read, &mut line).await.is_err() {
            return Ok(());
        }
        let request = match serde_json::from_str::<Request>(&line) {
            Ok(request) => request,
            Err(_) => {
                write_response(
                    &mut write,
                    &Response::Err {
                        message: "unrecognized request".into(),
                    },
                )
                .await?;
                line.clear();
                continue;
            }
        };
        line.clear();
        let response = dispatch(&hub, request).await;
        write_response(&mut write, &response).await?;
    }
}

async fn read_request_line(
    read: &mut BufReader<tokio::net::tcp::OwnedReadHalf>,
    line: &mut String,
) -> anyhow::Result<()> {
    line.clear();
    let n = read.read_line(line).await?;
    if n == 0 {
        anyhow::bail!("client closed");
    }
    if n > 64 * 1024 {
        anyhow::bail!("request too large");
    }
    Ok(())
}

async fn write_response(
    write: &mut tokio::net::tcp::OwnedWriteHalf,
    response: &Response,
) -> anyhow::Result<()> {
    let mut bytes = serde_json::to_vec(response)?;
    bytes.push(b'\n');
    write.write_all(&bytes).await?;
    Ok(())
}

async fn dispatch(hub: &Arc<Mutex<Hub>>, request: Request) -> Response {
    match request {
        Request::Status => status(hub),
        Request::Start { deadline_secs } => start(hub, deadline_secs).await,
        Request::StartClaude {
            token,
            allow_tools,
            hold_permissions,
        } => start_claude(hub, token, allow_tools, hold_permissions).await,
        Request::Replay { after } => replay(hub, after),
        Request::Follow { id, text } => follow(hub, &id, &text).await,
        Request::Resolve { id } => resolve(hub, &id),
        Request::Permit { id, choice } => permit(hub, &id, &choice).await,
        Request::Collect => collect(hub).await,
        Request::Discard => discard(hub).await,
    }
}

fn status(hub: &Mutex<Hub>) -> Response {
    let hub = lock(hub);
    let record = hub.state.load_run();
    let permission_id = hub.pending.clone();
    let waiting = if hub.run.is_some() {
        Some(
            if permission_id.is_some() {
                "permission"
            } else {
                "idle"
            }
            .to_string(),
        )
    } else {
        None
    };
    Response::Status {
        run_id: record.as_ref().map(|run| run.run_id.clone()),
        running: hub.run.is_some(),
        cursor: hub.journal.cursor(),
        tty: hub
            .run
            .as_ref()
            .map(|run| run.tty)
            .or_else(|| record.as_ref().map(|run| run.tty)),
        log_driver: hub
            .run
            .as_ref()
            .map(|run| run.log_driver.clone())
            .or_else(|| record.as_ref().map(|run| run.log_driver.clone())),
        interrupted: hub.interrupted,
        waiting,
        permission_id,
        expired: record.as_ref().is_some_and(|run| run.expired),
        kept: record.is_some_and(|run| run.kept()),
    }
}

fn replay(hub: &Mutex<Hub>, after: u64) -> Response {
    let hub = lock(hub);
    match hub.journal.replay_after(after, REPLAY_LIMIT) {
        Ok((events, truncated)) => Response::Replay {
            run_id: hub.state.load_run().map(|run| run.run_id),
            cursor: hub.journal.cursor(),
            truncated,
            events,
        },
        Err(err) => Response::Err {
            message: format!("replay failed: {err}"),
        },
    }
}

fn resolve(hub: &Mutex<Hub>, id: &str) -> Response {
    let hub = lock(hub);
    let outcome = hub.ledger.outcome(id).unwrap_or("unknown").to_string();
    Response::Ack {
        id: id.to_string(),
        outcome,
    }
}

async fn start(hub: &Arc<Mutex<Hub>>, deadline_secs: u64) -> Response {
    if !(1..=3600).contains(&deadline_secs) {
        return Response::Err {
            message: "deadline must be 1 to 3600 seconds".into(),
        };
    }
    {
        let mut guard = lock(hub);
        if guard.run.is_some() || guard.starting {
            return Response::Err {
                message: "a session is already running".into(),
            };
        }
        if let Some(refusal) = kept_refusal(&guard) {
            return refusal;
        }
        guard.starting = true;
        guard.pending = None;
        guard.deadline = None;
        guard.expiry_for = None;
    }
    let engine = lock(hub).engine.clone();
    let run_id = random_id();
    let attachment = match engine.start_stub(&run_id).await {
        Ok(attachment) => attachment,
        Err(err) => {
            lock(hub).starting = false;
            return Response::Err {
                message: err.to_string(),
            };
        }
    };
    if attachment.tty || attachment.log_driver != "none" {
        let _ = engine
            .remove_run(&attachment.container_id, &work_volume(&run_id))
            .await;
        lock(hub).starting = false;
        return Response::Err {
            message: format!(
                "refusing tty={} log={}",
                attachment.tty, attachment.log_driver
            ),
        };
    }
    let record = RunRecord {
        run_id: run_id.clone(),
        container_id: attachment.container_id.clone(),
        tty: attachment.tty,
        log_driver: attachment.log_driver.clone(),
        live: true,
        interrupted: false,
        expired: false,
        volume: Some(work_volume(&run_id)),
    };
    let cursor = {
        let mut guard = lock(hub);
        if let Err(err) = guard.state.save_run(&record) {
            guard.starting = false;
            return Response::Err {
                message: format!("could not record the run: {err}"),
            };
        }
        let event = match guard.journal.append("session", &format!("start {run_id}")) {
            Ok(event) => event,
            Err(err) => {
                guard.starting = false;
                return Response::Err {
                    message: format!("could not journal the start: {err}"),
                };
            }
        };
        let _ = guard
            .journal
            .append("session", &format!("deadline {deadline_secs}s"));
        guard.interrupted = false;
        let _ = guard.ledger.clear();
        guard.stdin = Some(attachment.stdin);
        guard.pending = None;
        // Instant is monotonic: changing the host's wall clock does not move it.
        guard.deadline = Some(Instant::now() + Duration::from_secs(deadline_secs));
        guard.expiry_for = None;
        guard.starting = false;
        guard.run = Some(LiveRun {
            run_id: run_id.clone(),
            container_id: attachment.container_id.clone(),
            tty: attachment.tty,
            log_driver: attachment.log_driver,
        });
        event.seq
    };
    // Journal lines as they arrive, with no client connected. A sleeping
    // Mac is not in this task.
    spawn_journal(Arc::clone(hub), attachment.container_id, attachment.lines);
    let armed = Instant::now();
    let armed_wall = SystemTime::now();
    spawn_deadline(Arc::clone(hub), run_id.clone(), deadline_secs);
    spawn_resume(
        Arc::clone(hub),
        run_id.clone(),
        armed,
        armed_wall,
        deadline_secs,
    );
    Response::Started {
        run_id,
        cursor,
        tty: false,
        log_driver: "none".into(),
    }
}

async fn start_claude(
    hub: &Arc<Mutex<Hub>>,
    token: String,
    allow_tools: bool,
    hold_permissions: bool,
) -> Response {
    if !acp::valid_token(&token) {
        return Response::Err {
            message: "credential must be one line of printable ASCII".into(),
        };
    }
    {
        let mut guard = lock(hub);
        if guard.run.is_some() || guard.starting {
            return Response::Err {
                message: "a session is already running".into(),
            };
        }
        if let Some(refusal) = kept_refusal(&guard) {
            return refusal;
        }
        guard.starting = true;
        guard.pending = None;
        guard.deadline = None;
        guard.secret = Some(token.clone());
    }
    let engine = lock(hub).engine.clone();
    let run_id = random_id();
    let volume = work_volume(&run_id);
    let attachment = match engine.start_claude(&run_id).await {
        Ok(attachment) => attachment,
        Err(err) => {
            fail_start(hub);
            return Response::Err {
                message: redact_hub(hub, &err.to_string()),
            };
        }
    };
    if attachment.tty || attachment.log_driver != "none" {
        let _ = engine.remove_run(&attachment.container_id, &volume).await;
        fail_start(hub);
        return Response::Err {
            message: format!(
                "refusing tty={} log={}",
                attachment.tty, attachment.log_driver
            ),
        };
    }
    let frame = match acp::bootstrap_frame(&token) {
        Ok(frame) => frame,
        Err(err) => {
            let _ = engine.remove_run(&attachment.container_id, &volume).await;
            fail_start(hub);
            return Response::Err {
                message: err.to_string(),
            };
        }
    };
    // Drop the request's copy. The hub keeps one value for redaction, and
    // the frame is what the entrypoint reads.
    drop(token);
    let (prompt_tx, prompt_rx) = mpsc::unbounded_channel();
    let (permit_tx, permit_rx) = mpsc::unbounded_channel();
    let (note_tx, note_rx) = mpsc::unbounded_channel();
    // A oneshot carries the single session-id result from the ACP task
    // back to this request. It is not a stream of events.
    let (ready_tx, ready_rx) = oneshot::channel();
    let container_id = attachment.container_id.clone();
    // The note task keeps its own copy so a later wipe cannot uncover a
    // line that was still queued.
    spawn_notes(
        Arc::clone(hub),
        container_id.clone(),
        note_rx,
        lock(hub).secret.clone().unwrap_or_default(),
    );
    tokio::spawn(acp::drive(acp::Drive {
        lines: attachment.lines,
        stdin: attachment.stdin.clone(),
        prompts: prompt_rx,
        notes: note_tx,
        ready: ready_tx,
        allow_tools,
        hold_permissions,
        permits: permit_rx,
    }));
    if attachment.stdin.send(frame).is_err() {
        let _ = engine.remove_run(&container_id, &volume).await;
        fail_start(hub);
        return Response::Err {
            message: "could not write the credential frame".into(),
        };
    }
    let session_id = match timeout(Duration::from_secs(240), ready_rx).await {
        Ok(Ok(Ok(session_id))) => session_id,
        Ok(Ok(Err(message))) => {
            let _ = engine.remove_run(&container_id, &volume).await;
            let message = redact_hub(hub, &message);
            fail_start(hub);
            return Response::Err { message };
        }
        _ => {
            let _ = engine.remove_run(&container_id, &volume).await;
            fail_start(hub);
            return Response::Err {
                message: "timed out waiting for the ACP session".into(),
            };
        }
    };
    let record = RunRecord {
        run_id: run_id.clone(),
        container_id: container_id.clone(),
        tty: false,
        log_driver: attachment.log_driver.clone(),
        live: true,
        interrupted: false,
        expired: false,
        volume: Some(volume),
    };
    let cursor = {
        let mut guard = lock(hub);
        if let Err(err) = guard.state.save_run(&record) {
            guard.starting = false;
            wipe_secret(&mut guard.secret);
            return Response::Err {
                message: format!("could not record the run: {err}"),
            };
        }
        let event = match guard.journal.append("session", &format!("start {run_id}")) {
            Ok(event) => event,
            Err(err) => {
                guard.starting = false;
                wipe_secret(&mut guard.secret);
                return Response::Err {
                    message: format!("could not journal the start: {err}"),
                };
            }
        };
        let _ = guard
            .journal
            .append("acp", &format!("claude-session {session_id}"));
        guard.interrupted = false;
        let _ = guard.ledger.clear();
        guard.starting = false;
        guard.pending = None;
        guard.deadline = None;
        guard.stdin = None;
        guard.claude = Some(ClaudeLink {
            prompts: prompt_tx,
            permits: permit_tx,
        });
        guard.run = Some(LiveRun {
            run_id: run_id.clone(),
            container_id,
            tty: false,
            log_driver: attachment.log_driver,
        });
        event.seq
    };
    Response::Started {
        run_id,
        cursor,
        tty: false,
        log_driver: "none".into(),
    }
}

fn fail_start(hub: &Mutex<Hub>) {
    let mut guard = lock(hub);
    guard.starting = false;
    wipe_secret(&mut guard.secret);
}

fn redact_hub(hub: &Mutex<Hub>, text: &str) -> String {
    let guard = lock(hub);
    acp::redact(text, guard.secret.as_deref())
}

fn wipe_secret(secret: &mut Option<String>) {
    if let Some(value) = secret.take() {
        let mut bytes = value.into_bytes();
        bytes.fill(0);
    }
}

fn spawn_notes(
    hub: Arc<Mutex<Hub>>,
    container_id: String,
    mut notes: mpsc::UnboundedReceiver<Note>,
    mut secret: String,
) {
    tokio::spawn(async move {
        while let Some(note) = notes.recv().await {
            let mut guard = lock(&hub);
            match note {
                Note::Log { kind, text } => {
                    let text = acp::redact(&text, Some(secret.as_str()));
                    if let Some(id) = text.strip_prefix("perm:pending:") {
                        guard.pending = Some(id.to_string());
                    } else if let Some(id) = text
                        .strip_prefix("perm:allowed:")
                        .or_else(|| text.strip_prefix("perm:cancelled:"))
                    {
                        if guard.pending.as_deref() == Some(id) {
                            guard.pending = None;
                        }
                    }
                    let _ = guard.journal.append(&kind, &text);
                }
                Note::Closed => {
                    let same = guard
                        .run
                        .as_ref()
                        .is_some_and(|run| run.container_id == container_id);
                    if same {
                        if let Some(mut record) = guard.state.load_run() {
                            record.live = false;
                            let _ = guard.state.save_run(&record);
                        }
                        guard.run = None;
                        guard.claude = None;
                        guard.stdin = None;
                        guard.pending = None;
                        guard.deadline = None;
                        wipe_secret(&mut guard.secret);
                    }
                    wipe_secret_string(&mut secret);
                    return;
                }
            }
        }
        wipe_secret_string(&mut secret);
    });
}

fn wipe_secret_string(secret: &mut String) {
    let mut bytes = std::mem::take(secret).into_bytes();
    bytes.fill(0);
}

fn spawn_journal(
    hub: Arc<Mutex<Hub>>,
    container_id: String,
    mut lines: mpsc::UnboundedReceiver<Line>,
) {
    tokio::spawn(async move {
        while let Some(line) = lines.recv().await {
            match line {
                Line::Stdout(text) => {
                    let mut guard = lock(&hub);
                    note_stdout(&mut guard, &text);
                }
                Line::Closed { stderr_bytes, .. } => {
                    let mut guard = lock(&hub);
                    let same = guard
                        .run
                        .as_ref()
                        .is_some_and(|run| run.container_id == container_id);
                    let _ = guard.journal.append(
                        "session",
                        &format!("attach closed stderr_bytes={stderr_bytes}"),
                    );
                    if same {
                        if let Some(mut record) = guard.state.load_run() {
                            record.live = false;
                            let _ = guard.state.save_run(&record);
                        }
                        guard.run = None;
                        guard.stdin = None;
                        guard.pending = None;
                        guard.deadline = None;
                    }
                    return;
                }
            }
        }
    });
}

fn note_stdout(guard: &mut Hub, text: &str) {
    let kind = if text.starts_with("beat:") {
        "beat"
    } else if text.starts_with("follow:") {
        "follow"
    } else if let Some(id) = text.strip_prefix("perm:pending:") {
        guard.pending = Some(id.to_string());
        "permission"
    } else if let Some(id) = text
        .strip_prefix("perm:allowed:")
        .or_else(|| text.strip_prefix("perm:cancelled:"))
    {
        if guard.pending.as_deref() == Some(id) {
            guard.pending = None;
        }
        "permission"
    } else if text.starts_with("perm:") {
        "permission"
    } else {
        "out"
    };
    let _ = guard.journal.append(kind, text);
}

/// The timer lives on the controller, not on the client connection that
/// started the run. `sleep` uses the runtime's monotonic clock, so a
/// wall-clock step does not move it and a dropped SSH session does not
/// cancel it. Startup already stops a run it finds live, and does not arm
/// this timer again for that run.
fn spawn_deadline(hub: Arc<Mutex<Hub>>, run_id: String, secs: u64) {
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(secs)).await;
        expire(hub, run_id).await;
    });
}

/// While the Mac is suspended this loop does not run, and `Instant` does
/// not advance. The first tick after wake sees the wall clock ahead of the
/// monotonic clock. A gap of at least 15 seconds is treated as a suspend,
/// the same threshold `hold` uses to notice a wake. A smaller step is left
/// to the monotonic timer so an ordinary clock adjustment does not expire
/// the run. Work between the kernel resume and this tick is the disclosed gap.
fn spawn_resume(
    hub: Arc<Mutex<Hub>>,
    run_id: String,
    armed: Instant,
    armed_wall: SystemTime,
    secs: u64,
) {
    let deadline = Duration::from_secs(secs);
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let still = lock(&hub)
                .run
                .as_ref()
                .is_some_and(|run| run.run_id == run_id);
            if !still {
                return;
            }
            let mono_elapsed = armed.elapsed();
            let wall_elapsed = armed_wall.elapsed().unwrap_or_default();
            if resume_expires(deadline, mono_elapsed, wall_elapsed) {
                expire(hub, run_id).await;
                return;
            }
        }
    });
}

/// `true` when a suspend gap is long enough that the wall clock has passed
/// the deadline while the monotonic clock has not.
fn resume_expires(deadline: Duration, mono_elapsed: Duration, wall_elapsed: Duration) -> bool {
    let gap = wall_elapsed.saturating_sub(mono_elapsed);
    gap >= Duration::from_secs(15) && wall_elapsed >= deadline
}

async fn expire(hub: Arc<Mutex<Hub>>, run_id: String) {
    let cancel = {
        let mut guard = lock(&hub);
        let same = guard.run.as_ref().is_some_and(|run| run.run_id == run_id);
        if !same || guard.expiry_for.as_deref() == Some(run_id.as_str()) {
            return;
        }
        guard.expiry_for = Some(run_id.clone());
        let pending = guard.pending.take();
        guard.deadline = None;
        if let Some(id) = &pending {
            let _ = guard
                .journal
                .append("permission", &format!("cancelled {id}"));
        }
        let _ = guard.journal.append("session", "expired");
        if let Some(mut record) = guard.state.load_run() {
            if record.run_id == run_id {
                record.live = false;
                record.expired = true;
                let _ = guard.state.save_run(&record);
            }
        }
        let claude = pending
            .clone()
            .and_then(|id| guard.claude.as_ref().map(|link| (id, link.permits.clone())));
        (pending.and(guard.stdin.clone()), claude)
    };
    let (cancel, claude_cancel) = cancel;
    if let Some((id, permits)) = claude_cancel {
        let _ = permits.send(PermitChoice { id, allow: false });
    }
    if let Some(stdin) = cancel {
        let _ = stdin.send(b"perm:cancel\n".to_vec());
    }
    let engine = lock(&hub).engine.clone();
    let _ = engine.stop_labeled().await;
}

/// A start is refused while the previous run's work is kept. Replacing it
/// would be a silent discard.
fn kept_refusal(guard: &Hub) -> Option<Response> {
    if guard.run.is_some() || guard.starting {
        return None;
    }
    guard
        .state
        .load_run()
        .filter(RunRecord::kept)
        .map(|_| Response::Err {
            message: "the previous run's work is kept; collect or discard it first".into(),
        })
}

/// The ended run's kept record, or the reason there is nothing to act on.
async fn ended_run(hub: &Mutex<Hub>, engine: &Engine) -> Result<RunRecord, Response> {
    let record = {
        let guard = lock(hub);
        if guard.run.is_some() || guard.starting {
            return Err(Response::Err {
                message: "the run is still live; stop it first".into(),
            });
        }
        guard.state.load_run().filter(RunRecord::kept)
    };
    let Some(record) = record else {
        return Err(Response::Err {
            message: "no kept work".into(),
        });
    };
    // Read or remove the volume only after the engine confirms the workload
    // is not running. An unknown state is not treated as stopped.
    match engine.is_running(&record.container_id).await {
        Ok(false) => Ok(record),
        Ok(true) => Err(Response::Err {
            message: "the workload is still running".into(),
        }),
        Err(err) => Err(Response::Err {
            message: format!("cannot confirm the workload stopped: {err}"),
        }),
    }
}

async fn collect(hub: &Arc<Mutex<Hub>>) -> Response {
    let engine = lock(hub).engine.clone();
    let record = match ended_run(hub, &engine).await {
        Ok(record) => record,
        Err(response) => return response,
    };
    let volume = record.volume.clone().unwrap_or_default();
    match engine.collect(&volume).await {
        Ok(entries) => {
            let _ = lock(hub).journal.append(
                "session",
                &format!("collected {} entries {}", entries.len(), record.run_id),
            );
            Response::Collected {
                run_id: record.run_id,
                entries,
            }
        }
        // The volume is untouched on failure, so collection can be retried.
        Err(err) => Response::Err {
            message: format!("collection failed: {err}"),
        },
    }
}

async fn discard(hub: &Arc<Mutex<Hub>>) -> Response {
    let engine = lock(hub).engine.clone();
    let mut record = match ended_run(hub, &engine).await {
        Ok(record) => record,
        Err(response) => return response,
    };
    let volume = record.volume.clone().unwrap_or_default();
    if let Err(err) = engine.remove_run(&record.container_id, &volume).await {
        return Response::Err {
            message: format!("discard failed: {err}"),
        };
    }
    record.volume = None;
    let mut guard = lock(hub);
    if let Err(err) = guard.state.save_run(&record) {
        return Response::Err {
            message: format!("could not record the discard: {err}"),
        };
    }
    let _ = guard
        .journal
        .append("session", &format!("discarded {}", record.run_id));
    Response::Ack {
        id: record.run_id,
        outcome: "discarded".into(),
    }
}

enum Answer {
    Stub(mpsc::UnboundedSender<Vec<u8>>),
    Claude(mpsc::UnboundedSender<PermitChoice>),
}

async fn permit(hub: &Arc<Mutex<Hub>>, id: &str, choice: &str) -> Response {
    if choice != "allow" && choice != "cancel" {
        return Response::Err {
            message: "choice must be allow or cancel".into(),
        };
    }
    if !valid_id(id) {
        return Response::Err {
            message: "command id must be 1-64 letters, digits, _ or -".into(),
        };
    }
    let stdin = {
        let mut guard = lock(hub);
        if guard.run.is_none() {
            return Response::Err {
                message: "no running session".into(),
            };
        }
        if guard
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            return Response::Err {
                message: "the run deadline has passed".into(),
            };
        }
        if let Some(outcome) = guard.ledger.outcome(id) {
            return Response::Ack {
                id: id.to_string(),
                outcome: outcome.to_string(),
            };
        }
        if guard.pending.as_deref() != Some(id) {
            return Response::Err {
                message: "no pending permission".into(),
            };
        }
        match guard.ledger.begin(id) {
            Ok(Begin::Fresh) => {
                guard.pending = None;
                if let Some(link) = &guard.claude {
                    Answer::Claude(link.permits.clone())
                } else if let Some(stdin) = guard.stdin.clone() {
                    Answer::Stub(stdin)
                } else {
                    return Response::Ack {
                        id: id.to_string(),
                        outcome: "uncertain".into(),
                    };
                }
            }
            Ok(Begin::Delivered) => {
                return Response::Ack {
                    id: id.to_string(),
                    outcome: "delivered".into(),
                };
            }
            Ok(Begin::Uncertain) => {
                return Response::Ack {
                    id: id.to_string(),
                    outcome: "uncertain".into(),
                };
            }
            Err(err) => {
                return Response::Err {
                    message: format!("ledger: {err}"),
                };
            }
        }
    };
    let sent = match stdin {
        Answer::Stub(stdin) => {
            let line = if choice == "allow" {
                "perm:allow\n"
            } else {
                "perm:cancel\n"
            };
            stdin.send(line.as_bytes().to_vec()).is_ok()
        }
        Answer::Claude(permits) => permits
            .send(PermitChoice {
                id: id.to_string(),
                allow: choice == "allow",
            })
            .is_ok(),
    };
    if !sent {
        return Response::Ack {
            id: id.to_string(),
            outcome: "uncertain".into(),
        };
    }
    {
        let mut guard = lock(hub);
        if let Err(err) = guard.ledger.commit_delivered(id) {
            return Response::Err {
                message: format!("could not record delivery: {err}"),
            };
        }
    }
    Response::Ack {
        id: id.to_string(),
        outcome: "delivered".into(),
    }
}

async fn follow(hub: &Arc<Mutex<Hub>>, id: &str, text: &str) -> Response {
    if !valid_id(id) {
        return Response::Err {
            message: "command id must be 1-64 letters, digits, _ or -".into(),
        };
    }
    if text.is_empty() || text.contains('\n') || text.len() > 1024 {
        return Response::Err {
            message: "follow-up text must be one non-empty line".into(),
        };
    }
    let sender = {
        let mut guard = lock(hub);
        if guard.run.is_none() {
            return Response::Err {
                message: "no running session".into(),
            };
        }
        if guard.pending.is_some() {
            return Response::Err {
                message: "a permission is waiting".into(),
            };
        }
        if guard
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            return Response::Err {
                message: "the run deadline has passed".into(),
            };
        }
        match guard.ledger.begin(id) {
            Ok(Begin::Fresh) => {
                if let Some(link) = &guard.claude {
                    Delivery::Claude(link.prompts.clone())
                } else if let Some(stdin) = &guard.stdin {
                    Delivery::Stub(stdin.clone())
                } else {
                    return Response::Ack {
                        id: id.to_string(),
                        outcome: "uncertain".into(),
                    };
                }
            }
            Ok(Begin::Delivered) => {
                return Response::Ack {
                    id: id.to_string(),
                    outcome: "delivered".into(),
                };
            }
            Ok(Begin::Uncertain) => {
                return Response::Ack {
                    id: id.to_string(),
                    outcome: "uncertain".into(),
                };
            }
            Err(err) => {
                return Response::Err {
                    message: format!("ledger: {err}"),
                }
            }
        }
    };
    match sender {
        Delivery::Stub(sender) => {
            if sender.send(format!("{text}\n").into_bytes()).is_err() {
                return Response::Ack {
                    id: id.to_string(),
                    outcome: "uncertain".into(),
                };
            }
        }
        Delivery::Claude(sender) => {
            let (done_tx, done_rx) = oneshot::channel();
            if sender
                .send(PromptJob {
                    text: text.to_string(),
                    done: done_tx,
                })
                .is_err()
            {
                return Response::Ack {
                    id: id.to_string(),
                    outcome: "uncertain".into(),
                };
            }
            let turn = match timeout(Duration::from_secs(240), done_rx).await {
                Ok(Ok(turn)) => turn,
                _ => {
                    return Response::Ack {
                        id: id.to_string(),
                        outcome: "uncertain".into(),
                    };
                }
            };
            if let Err(message) = turn {
                let message = redact_hub(hub, &message);
                let mut guard = lock(hub);
                if let Err(err) = guard.ledger.commit_delivered(id) {
                    return Response::Err {
                        message: format!("could not record delivery: {err}"),
                    };
                }
                return Response::Err { message };
            }
        }
    }
    {
        let mut guard = lock(hub);
        if let Err(err) = guard.ledger.commit_delivered(id) {
            return Response::Err {
                message: format!("could not record delivery: {err}"),
            };
        }
    }
    Response::Ack {
        id: id.to_string(),
        outcome: "delivered".into(),
    }
}

enum Delivery {
    Stub(mpsc::UnboundedSender<Vec<u8>>),
    Claude(mpsc::UnboundedSender<PromptJob>),
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}

fn random_id() -> String {
    let mut bytes = [0u8; 16];
    if let Ok(mut file) = std::fs::File::open("/dev/urandom") {
        use std::io::Read;
        let _ = file.read_exact(&mut bytes);
    }
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_suspend_gap_expires_a_deadline_the_monotonic_clock_has_not_reached() {
        // Measured on a real Mac sleep: the wall clock jumped by minutes
        // while Instant advanced only for the time the machine was awake.
        let deadline = Duration::from_secs(60);
        let mono = Duration::from_secs(30);
        let wall = Duration::from_secs(393);
        assert!(resume_expires(deadline, mono, wall));
    }

    #[test]
    fn a_small_clock_step_does_not_count_as_suspend() {
        let deadline = Duration::from_secs(60);
        assert!(!resume_expires(
            deadline,
            Duration::from_secs(10),
            Duration::from_secs(12),
        ));
    }

    #[test]
    fn resume_does_not_expire_a_deadline_that_is_still_ahead() {
        assert!(!resume_expires(
            Duration::from_secs(3600),
            Duration::from_secs(30),
            Duration::from_secs(50),
        ));
    }
}
