//! `RunRuntime`: how the app reaches its run controller (docs/architecture.md,
//! Agent runs — v0.5). It starts `brainiac runner` when none answers, so the
//! controller is a separate process that outlives the app, and it hides the
//! socket, the token, and the protocol from the rest of the app.
//!
//! Each call opens its own connection. Nothing the app does by closing one —
//! quitting, sleeping — reaches a run.

use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::io::BufReader;
use tokio::net::UnixStream;

use super::controller::protocol::{
    Delivery, EventPage, Request, Response, RunStatus, StartRun, StopReason, MAX_LINE_BYTES,
    PROTOCOL,
};
use super::controller::state::StateDir;
use super::controller::{read_line, write_line};
use crate::mcp::{current_uid, fallback_dir, MAX_SOCKET_PATH};
use crate::models::{AppError, AppResult, ErrorCode};

/// How long a newly started controller has to answer.
const START_WAIT: Duration = Duration::from_secs(10);
const RETRY_EVERY: Duration = Duration::from_millis(100);
/// Requests answer at once; only a long poll for events waits.
const CALL_TIMEOUT: Duration = Duration::from_secs(30);

/// What the controller said when the app connected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControllerInfo {
    pub protocol: u32,
    pub installation: String,
    pub build: String,
    pub pid: u32,
}

pub struct RunRuntime {
    state: StateDir,
    socket: PathBuf,
    /// Start `brainiac runner` (this executable) when no controller answers.
    launch: bool,
}

impl RunRuntime {
    /// The app's controller: `agent-runs/controller/` in the data folder.
    pub fn new(data_dir: &Path, identifier: &str) -> Self {
        let state = StateDir::new(data_dir.join("agent-runs").join("controller"));
        let socket = socket_path(state.root(), identifier);
        Self {
            state,
            socket,
            launch: true,
        }
    }

    /// A controller started by someone else (tests): never launched from here.
    pub fn attach(state: StateDir, socket: PathBuf) -> Self {
        Self {
            state,
            socket,
            launch: false,
        }
    }

    /// The controller, if one is running now; never starts one.
    pub async fn running(&self) -> Option<ControllerInfo> {
        self.try_connect().await.ok().map(|(_, info)| info)
    }

    /// Connect, starting the controller first when none answers. A
    /// controller of another protocol is replaced only when it has no live
    /// runs; otherwise it keeps them.
    async fn connect(&self) -> AppResult<Connection> {
        match self.try_connect().await {
            Ok((connection, info)) if info.protocol == PROTOCOL => return Ok(connection),
            Ok((mut connection, _)) => {
                if !matches!(
                    connection.call(&Request::Shutdown, CALL_TIMEOUT).await?,
                    Response::Done
                ) {
                    return Err(AppError::new(
                        ErrorCode::Conflict,
                        "A run controller from another version of Brainiac is still running runs. Wait for them to end.",
                    ));
                }
                self.wait_gone().await;
            }
            Err(_) if !self.launch => {
                return Err(AppError::dependency("The run controller is not running."));
            }
            Err(_) => {}
        }
        self.spawn()?;
        let deadline = tokio::time::Instant::now() + START_WAIT;
        loop {
            tokio::time::sleep(RETRY_EVERY).await;
            match self.try_connect().await {
                Ok((connection, info)) if info.protocol == PROTOCOL => return Ok(connection),
                Ok(_) => {
                    return Err(AppError::dependency(
                        "The run controller started with another protocol.",
                    ))
                }
                Err(e) if tokio::time::Instant::now() >= deadline => {
                    return Err(AppError::dependency("The run controller did not start.")
                        .with_details(format!(
                            "{} (log: {})",
                            e.message,
                            self.state.log_path().display()
                        )));
                }
                Err(_) => {}
            }
        }
    }

    async fn try_connect(&self) -> AppResult<(Connection, ControllerInfo)> {
        // Only a socket this user owns: in the `/tmp` fallback, another user
        // could otherwise put one of theirs in its place.
        let meta = std::fs::symlink_metadata(&self.socket)?;
        if !meta.file_type().is_socket() || meta.uid() != current_uid() {
            return Err(AppError::new(
                ErrorCode::PermissionDenied,
                "That is not the run controller's socket.",
            ));
        }
        let stream = UnixStream::connect(&self.socket).await?;
        let (read, write) = stream.into_split();
        let mut connection = Connection {
            reader: BufReader::new(read),
            writer: write,
        };
        let token = self.state.token()?;
        let hello = Request::Hello {
            token,
            protocol: PROTOCOL,
        };
        match connection.call(&hello, CALL_TIMEOUT).await? {
            Response::Welcome {
                protocol,
                installation,
                build,
                pid,
            } => Ok((
                connection,
                ControllerInfo {
                    protocol,
                    installation,
                    build,
                    pid,
                },
            )),
            Response::Error { error } => Err(error),
            _ => Err(unexpected()),
        }
    }

    async fn wait_gone(&self) {
        let deadline = tokio::time::Instant::now() + START_WAIT;
        while tokio::time::Instant::now() < deadline
            && UnixStream::connect(&self.socket).await.is_ok()
        {
            tokio::time::sleep(RETRY_EVERY).await;
        }
    }

    /// Start `brainiac runner` on its own: its own process group, so a
    /// Ctrl-C in the terminal that started the app does not reach it, and
    /// nothing the app holds open. Its errors go to `runner.log`.
    fn spawn(&self) -> AppResult<()> {
        use std::os::unix::fs::OpenOptionsExt;
        use std::os::unix::process::CommandExt;
        self.state.ensure()?;
        // Created before the controller starts, so the app can read it.
        self.state.token()?;
        let log = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o600)
            .open(self.state.log_path())?;
        let mut child = std::process::Command::new(std::env::current_exe()?)
            .arg("runner")
            .arg("--state")
            .arg(self.state.root())
            .arg("--socket")
            .arg(&self.socket)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(log)
            .process_group(0)
            .spawn()?;
        // Waited for on a thread of its own, so it does not linger as a
        // zombie once it exits.
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        Ok(())
    }

    async fn call(&self, request: Request, timeout: Duration) -> AppResult<Response> {
        let mut connection = self.connect().await?;
        match connection.call(&request, timeout).await? {
            Response::Error { error } => Err(error),
            response => Ok(response),
        }
    }

    /// Hand a run to the controller. It answers once the run is accepted and
    /// its deadline armed; preparing it goes on there.
    pub async fn start(&self, start: StartRun) -> AppResult<RunStatus> {
        match self.call(Request::Start(start), CALL_TIMEOUT).await? {
            Response::Run { run } => Ok(run),
            _ => Err(unexpected()),
        }
    }

    pub async fn runs(&self) -> AppResult<Vec<RunStatus>> {
        match self.call(Request::Status, CALL_TIMEOUT).await? {
            Response::Runs { runs } => Ok(runs),
            _ => Err(unexpected()),
        }
    }

    /// Events after `after`, waiting up to `wait` for the first one.
    pub async fn events(&self, run_id: &str, after: u64, wait: Duration) -> AppResult<EventPage> {
        let request = Request::Events {
            run_id: run_id.to_string(),
            after,
            wait_ms: wait.as_millis() as u64,
        };
        match self.call(request, wait + CALL_TIMEOUT).await? {
            Response::Events { page } => Ok(page),
            _ => Err(unexpected()),
        }
    }

    pub async fn prompt(&self, run_id: &str, command_id: &str, text: &str) -> AppResult<Delivery> {
        let request = Request::Prompt {
            run_id: run_id.to_string(),
            command_id: command_id.to_string(),
            text: text.to_string(),
        };
        self.ack(request).await
    }

    pub async fn permit(
        &self,
        run_id: &str,
        command_id: &str,
        permission_id: &str,
        allow: bool,
    ) -> AppResult<Delivery> {
        let request = Request::Permit {
            run_id: run_id.to_string(),
            command_id: command_id.to_string(),
            permission_id: permission_id.to_string(),
            allow,
        };
        self.ack(request).await
    }

    async fn ack(&self, request: Request) -> AppResult<Delivery> {
        match self.call(request, CALL_TIMEOUT).await? {
            Response::Ack { outcome, .. } => Ok(outcome),
            _ => Err(unexpected()),
        }
    }

    pub async fn stop(&self, run_id: &str, reason: StopReason) -> AppResult<RunStatus> {
        let request = Request::Stop {
            run_id: run_id.to_string(),
            reason,
        };
        match self.call(request, CALL_TIMEOUT).await? {
            Response::Run { run } => Ok(run),
            _ => Err(unexpected()),
        }
    }

    pub async fn discard(&self, run_id: &str) -> AppResult<()> {
        match self
            .call(
                Request::Discard {
                    run_id: run_id.to_string(),
                },
                CALL_TIMEOUT * 2,
            )
            .await?
        {
            Response::Done => Ok(()),
            _ => Err(unexpected()),
        }
    }
}

/// `runner.sock` in the controller's folder, or, when that path is too long
/// for a socket, one in `/tmp/brainiac-<uid>/`.
pub fn socket_path(state: &Path, identifier: &str) -> PathBuf {
    let preferred = state.join("runner.sock");
    if preferred.as_os_str().len() <= MAX_SOCKET_PATH {
        preferred
    } else {
        fallback_dir().join(format!("{identifier}-runner.sock"))
    }
}

fn unexpected() -> AppError {
    AppError::dependency("The run controller answered something unexpected.")
}

struct Connection {
    reader: BufReader<tokio::net::unix::OwnedReadHalf>,
    writer: tokio::net::unix::OwnedWriteHalf,
}

impl Connection {
    async fn call(&mut self, request: &Request, timeout: Duration) -> AppResult<Response> {
        let exchange = async {
            write_line(&mut self.writer, request).await?;
            let line = read_line(&mut self.reader, MAX_LINE_BYTES)
                .await?
                .ok_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "the controller closed the connection",
                    )
                })?;
            serde_json::from_slice::<Response>(&line).map_err(std::io::Error::other)
        };
        match tokio::time::timeout(timeout, exchange).await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(e)) => Err(
                AppError::dependency("The run controller could not be reached.")
                    .with_details(e.to_string()),
            ),
            Err(_) => Err(AppError::timeout("The run controller did not answer.")),
        }
    }
}
