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
    CollectManifest, Delivery, EventPage, Request, Response, RunStatus, StartRun, StopReason,
    MAX_LINE_BYTES, PROTOCOL,
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
/// A collection hashes the whole working tree and copies the result out.
const COLLECT_TIMEOUT: Duration = Duration::from_secs(40 * 60);

/// What the controller said when the app connected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControllerInfo {
    pub protocol: u32,
    pub installation: String,
    pub build: String,
    pub pid: u32,
}

enum Link {
    /// This Mac's controller, on its Unix socket.
    Local {
        state: StateDir,
        socket: PathBuf,
        /// Start `brainiac runner` when no controller answers.
        launch: bool,
    },
    /// A host's controller, through an SSH stream-local forward. The SSH
    /// process is only the transport (SPEC.md, Remote hosts).
    Remote {
        target: super::ssh::Target,
        token: String,
        /// Empty until Deploy has recorded the installation.
        installation: String,
        forward: PathBuf,
    },
}

pub struct RunRuntime {
    link: Link,
    /// How long a request may wait. Tests use a short one so a stuck host
    /// does not hold the suite.
    call_timeout: Duration,
}

impl RunRuntime {
    /// The app's controller: `agent-runs/controller/` in the data folder.
    pub fn new(data_dir: &Path, identifier: &str) -> Self {
        let state = StateDir::new(data_dir.join("agent-runs").join("controller"));
        let socket = socket_path(state.root(), identifier);
        Self {
            link: Link::Local {
                state,
                socket,
                launch: true,
            },
            call_timeout: CALL_TIMEOUT,
        }
    }

    /// A controller started by someone else (tests): never launched from here.
    pub fn attach(state: StateDir, socket: PathBuf) -> Self {
        Self {
            link: Link::Local {
                state,
                socket,
                launch: false,
            },
            call_timeout: CALL_TIMEOUT,
        }
    }

    /// A remote controller. `installation` is empty until the first Hello
    /// is recorded; after that a different installation is a failure.
    pub fn remote(
        target: super::ssh::Target,
        token: String,
        installation: String,
        forward: PathBuf,
    ) -> Self {
        Self {
            link: Link::Remote {
                target,
                token,
                installation,
                forward,
            },
            call_timeout: CALL_TIMEOUT,
        }
    }

    pub fn with_call_timeout(mut self, timeout: Duration) -> Self {
        self.call_timeout = timeout;
        self
    }

    fn is_remote(&self) -> bool {
        matches!(self.link, Link::Remote { .. })
    }

    /// The controller, if one is running now; never starts one.
    pub async fn running(&self) -> Option<ControllerInfo> {
        if self.is_remote() {
            return self.connect_remote().await.ok().map(|(_, info)| info);
        }
        self.try_connect().await.ok().map(|(_, info)| info)
    }

    /// Connect, starting the controller first when none answers. A
    /// controller of another protocol is replaced only when it has no live
    /// runs; otherwise it keeps them.
    async fn connect(&self) -> AppResult<Connection> {
        if self.is_remote() {
            return self
                .connect_remote()
                .await
                .map(|(connection, _)| connection);
        }
        let (_, _, launch) = self.local();
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
            Err(_) if !launch => {
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
                    let (state, _, _) = self.local();
                    return Err(AppError::dependency("The run controller did not start.")
                        .with_details(format!(
                            "{} (log: {})",
                            e.message,
                            state.log_path().display()
                        )));
                }
                Err(_) => {}
            }
        }
    }

    fn local(&self) -> (&StateDir, &Path, bool) {
        match &self.link {
            Link::Local {
                state,
                socket,
                launch,
            } => (state, socket, *launch),
            Link::Remote { .. } => unreachable!("a remote runtime has no local controller"),
        }
    }

    async fn try_connect(&self) -> AppResult<(Connection, ControllerInfo)> {
        let timeout = self.call_timeout;
        let (state, socket, _) = self.local();
        // Only a socket this user owns: in the `/tmp` fallback, another user
        // could otherwise put one of theirs in its place.
        let meta = std::fs::symlink_metadata(socket)?;
        if !meta.file_type().is_socket() || meta.uid() != current_uid() {
            return Err(AppError::new(
                ErrorCode::PermissionDenied,
                "That is not the run controller's socket.",
            ));
        }
        let stream = UnixStream::connect(socket).await?;
        let (read, write) = stream.into_split();
        let mut connection = Connection {
            reader: BufReader::new(read),
            writer: write,
            forward: None,
        };
        let token = state.token()?;
        let hello = Request::Hello {
            token,
            protocol: PROTOCOL,
        };
        match connection.call(&hello, timeout).await? {
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
        let (_, socket, _) = self.local();
        let socket = socket.to_path_buf();
        let deadline = tokio::time::Instant::now() + START_WAIT;
        while tokio::time::Instant::now() < deadline && UnixStream::connect(&socket).await.is_ok() {
            tokio::time::sleep(RETRY_EVERY).await;
        }
    }

    /// Start `brainiac runner` on its own: its own process group, so a
    /// Ctrl-C in the terminal that started the app does not reach it, and
    /// nothing the app holds open. Its errors go to `runner.log`.
    fn spawn(&self) -> AppResult<()> {
        use std::os::unix::fs::OpenOptionsExt;
        use std::os::unix::process::CommandExt;
        let (state, socket, _) = self.local();
        state.ensure()?;
        // Created before the controller starts, so the app can read it.
        state.token()?;
        let log = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o600)
            .open(state.log_path())?;
        let (exe, mut args) = super::controller::runner_command()?;
        args.extend([
            "--state".into(),
            state.root().display().to_string(),
            "--socket".into(),
            socket.display().to_string(),
        ]);
        let mut child = std::process::Command::new(exe)
            .args(args)
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
        match self.exchange(request, timeout).await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(answered)) | Err(answered) => Err(answered),
        }
    }

    /// One request: the outer error is the transport's (the controller was
    /// not reached, or did not answer in time), the inner one the
    /// controller's own answer.
    async fn exchange(
        &self,
        request: Request,
        timeout: Duration,
    ) -> AppResult<Result<Response, AppError>> {
        let mut connection = self.connect().await?;
        Ok(match connection.call(&request, timeout).await? {
            Response::Error { error } => Err(error),
            response => Ok(response),
        })
    }

    /// Hand a run to the controller. It answers once the run is accepted and
    /// its deadline armed; preparing it goes on there.
    pub async fn start(&self, start: StartRun) -> Result<RunStatus, StartError> {
        if self.is_remote() {
            return self.start_remote(start).await;
        }
        match self
            .exchange(Request::Start(start), self.call_timeout)
            .await
        {
            Ok(Ok(Response::Run { run })) => Ok(run),
            Ok(Ok(_)) => Err(StartError::Unanswered(unexpected())),
            Ok(Err(refused)) => Err(StartError::Refused(refused)),
            Err(transport) => Err(StartError::Unanswered(transport)),
        }
    }

    pub async fn runs(&self) -> AppResult<Vec<RunStatus>> {
        match self.call(Request::Status, self.call_timeout).await? {
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
        match self.call(request, self.call_timeout).await? {
            Response::Ack { outcome, .. } => Ok(outcome),
            _ => Err(unexpected()),
        }
    }

    pub async fn stop(&self, run_id: &str, reason: StopReason) -> AppResult<RunStatus> {
        let request = Request::Stop {
            run_id: run_id.to_string(),
            reason,
        };
        match self.call(request, self.call_timeout).await? {
            Response::Run { run } => Ok(run),
            _ => Err(unexpected()),
        }
    }

    /// Collect a stopped run's working tree into `out_dir/result.bundle`.
    /// `image` is the current image, for a run whose own is gone.
    pub async fn collect(
        &self,
        run_id: &str,
        include: Vec<String>,
        out_dir: &Path,
        image: Option<String>,
    ) -> AppResult<CollectManifest> {
        if self.is_remote() {
            return self.collect_remote(run_id, include, out_dir, image).await;
        }
        let request = Request::Collect {
            run_id: run_id.to_string(),
            include,
            out_dir: out_dir.to_path_buf(),
            image,
        };
        match self.call(request, COLLECT_TIMEOUT).await? {
            Response::Collected { manifest } => Ok(manifest),
            _ => Err(unexpected()),
        }
    }

    pub async fn discard(&self, run_id: &str, image: Option<String>) -> AppResult<()> {
        match self
            .call(
                Request::Discard {
                    run_id: run_id.to_string(),
                    image,
                },
                CALL_TIMEOUT * 2,
            )
            .await?
        {
            Response::Done => Ok(()),
            _ => Err(unexpected()),
        }
    }

    /// Open the SSH forward and check the installation and protocol.
    async fn connect_remote(&self) -> AppResult<(Connection, ControllerInfo)> {
        let Link::Remote {
            target,
            token,
            installation,
            forward,
        } = &self.link
        else {
            return Err(AppError::dependency("This run is not on a remote host."));
        };
        // The `/tmp` fallback must be this user's and closed to others.
        // `create_dir_all` would leave that folder open.
        crate::mcp::prepare_socket_dir(forward)?;
        let args = super::ssh::forward_args(target, forward);
        use std::os::unix::process::CommandExt;
        let child = std::process::Command::new("ssh")
            .args(&args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .process_group(0)
            .spawn()
            .map_err(|e| {
                AppError::dependency("SSH could not be started.").with_details(e.to_string())
            })?;
        let held = Forward { child };
        let deadline = tokio::time::Instant::now() + super::ssh::CONNECT_TIMEOUT;
        let stream = loop {
            if tokio::time::Instant::now() >= deadline {
                return Err(AppError::dependency(
                    "The host's run controller could not be reached.",
                ));
            }
            match UnixStream::connect(forward).await {
                Ok(stream) => break stream,
                Err(_) => tokio::time::sleep(RETRY_EVERY).await,
            }
        };
        let (read, write) = stream.into_split();
        let mut connection = Connection {
            reader: BufReader::new(read),
            writer: write,
            forward: Some(held),
        };
        let hello = Request::Hello {
            token: token.clone(),
            protocol: PROTOCOL,
        };
        match connection.call(&hello, self.call_timeout).await? {
            Response::Welcome {
                protocol,
                installation: got,
                build,
                pid,
            } => {
                if protocol != PROTOCOL {
                    return Err(AppError::dependency(
                        "The host's run controller speaks another protocol.",
                    ));
                }
                if !installation.is_empty() && got != *installation {
                    return Err(AppError::dependency(
                        "The host's run controller is not the one Brainiac installed.",
                    ));
                }
                Ok((
                    connection,
                    ControllerInfo {
                        protocol,
                        installation: got,
                        build,
                        pid,
                    },
                ))
            }
            Response::Error { error } => Err(error),
            _ => Err(unexpected()),
        }
    }

    async fn start_remote(&self, mut start: StartRun) -> Result<RunStatus, StartError> {
        let bytes = tokio::fs::read(&start.bundle).await.map_err(|e| {
            StartError::Refused(
                AppError::io("The run's start was not exported.").with_details(e.to_string()),
            )
        })?;
        let (mut connection, _) = self.connect_remote().await.map_err(StartError::Refused)?;
        let path =
            match upload_bundle(&mut connection, &start.run_id, &bytes, self.call_timeout).await {
                Ok(path) => path,
                Err(e) => return Err(StartError::Refused(e)),
            };
        start.bundle = PathBuf::from(path);
        match connection
            .call(&Request::Start(start), self.call_timeout)
            .await
        {
            Ok(Response::Run { run }) => Ok(run),
            Ok(Response::Error { error }) => Err(StartError::Refused(error)),
            Ok(_) => Err(StartError::Unanswered(unexpected())),
            Err(e) => Err(StartError::Unanswered(e)),
        }
    }

    async fn collect_remote(
        &self,
        run_id: &str,
        include: Vec<String>,
        out_dir: &Path,
        image: Option<String>,
    ) -> AppResult<CollectManifest> {
        let (mut connection, _) = self.connect_remote().await?;
        let manifest = match connection
            .call(
                &Request::CollectHere {
                    run_id: run_id.to_string(),
                    include,
                    image,
                },
                COLLECT_TIMEOUT,
            )
            .await?
        {
            Response::Collected { manifest } => manifest,
            Response::Error { error } => return Err(error),
            _ => return Err(unexpected()),
        };
        if manifest.result == manifest.start {
            return Ok(manifest);
        }
        let dest = out_dir.join("result.bundle");
        let mut file = tokio::fs::File::create(&dest).await?;
        let mut offset = 0u64;
        loop {
            match connection
                .call(
                    &Request::ReadBundle {
                        run_id: run_id.to_string(),
                        offset,
                        max: super::controller::protocol::TRANSFER_CHUNK as u32,
                    },
                    self.call_timeout,
                )
                .await?
            {
                Response::Chunk { total, hex, .. } => {
                    if hex.is_empty() || offset >= total {
                        break;
                    }
                    let bytes = super::controller::protocol::decode_hex(&hex)
                        .ok_or_else(|| AppError::io("The collected bundle could not be read."))?;
                    use tokio::io::AsyncWriteExt;
                    file.write_all(&bytes).await?;
                    offset += bytes.len() as u64;
                    if offset >= total {
                        break;
                    }
                }
                Response::Error { error } => return Err(error),
                _ => return Err(unexpected()),
            }
        }
        Ok(manifest)
    }

    pub async fn probe_engine(
        &self,
        socket: &str,
        image: Option<String>,
    ) -> AppResult<super::controller::protocol::EngineReport> {
        match self
            .call(
                Request::ProbeEngine {
                    socket: socket.to_string(),
                    image,
                },
                self.call_timeout,
            )
            .await?
        {
            Response::Engine { report } => Ok(report),
            _ => Err(unexpected()),
        }
    }

    pub async fn build_image_remote(
        &self,
        socket: &str,
        tag: &str,
        context: &[u8],
    ) -> AppResult<String> {
        let hex = super::controller::protocol::encode_hex(context);
        match self
            .call(
                Request::BuildImage {
                    socket: socket.to_string(),
                    tag: tag.to_string(),
                    hex,
                },
                COLLECT_TIMEOUT,
            )
            .await?
        {
            Response::Image { id } => Ok(id),
            _ => Err(unexpected()),
        }
    }
}

async fn upload_bundle(
    connection: &mut Connection,
    run_id: &str,
    bytes: &[u8],
    timeout: Duration,
) -> AppResult<String> {
    let total = bytes.len() as u64;
    let mut offset = 0usize;
    let mut path = String::new();
    while offset < bytes.len() {
        let end = (offset + super::controller::protocol::TRANSFER_CHUNK).min(bytes.len());
        let hex = super::controller::protocol::encode_hex(&bytes[offset..end]);
        match connection
            .call(
                &Request::PutBundle {
                    run_id: run_id.to_string(),
                    offset: offset as u64,
                    total,
                    hex,
                },
                timeout,
            )
            .await?
        {
            Response::Uploaded { path: got, .. } => path = got,
            Response::Error { error } => return Err(error),
            _ => return Err(unexpected()),
        }
        offset = end;
    }
    if path.is_empty() {
        return Err(AppError::dependency("The host did not accept the bundle."));
    }
    Ok(path)
}

/// Why a start did not give a run. A refusal means the controller saw the
/// request and made nothing; an unanswered start may have been accepted
/// before the socket or the timeout failed, so the run must be followed.
#[derive(Debug)]
pub enum StartError {
    Refused(AppError),
    Unanswered(AppError),
}

impl StartError {
    pub fn into_error(self) -> AppError {
        match self {
            StartError::Refused(e) | StartError::Unanswered(e) => e,
        }
    }
}

// `From` lets `?` turn a `StartError` into the app's one error type.
impl From<StartError> for AppError {
    fn from(e: StartError) -> Self {
        e.into_error()
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

/// The stream-local forward's socket on this Mac. The host folder lives
/// under `Application Support`, whose space makes OpenSSH reject the
/// forward, and that path is often too long for a socket.
pub fn forward_socket(host_dir: &Path, host_id: &str) -> PathBuf {
    let preferred = host_dir.join("forward.sock");
    let text = preferred.to_string_lossy();
    if !text.contains(' ') && preferred.as_os_str().len() <= MAX_SOCKET_PATH {
        preferred
    } else {
        fallback_dir().join(format!("{host_id}-forward.sock"))
    }
}

fn unexpected() -> AppError {
    AppError::dependency("The run controller answered something unexpected.")
}

/// An SSH forward. Dropping it stops the process group, so a closed
/// connection does not leave `ssh` running.
struct Forward {
    child: std::process::Child,
}

impl Drop for Forward {
    fn drop(&mut self) {
        let pid = self.child.id() as i32;
        // `process_group(0)` made this pid the group leader. Killing the
        // group stops ssh and the forward together. `kill` is unsafe because
        // it is a raw system call.
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
        let _ = self.child.wait();
    }
}

struct Connection {
    reader: BufReader<tokio::net::unix::OwnedReadHalf>,
    writer: tokio::net::unix::OwnedWriteHalf,
    /// Dropped with the connection, which stops the SSH forward.
    #[allow(dead_code)]
    forward: Option<Forward>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_forward_under_application_support_moves_to_tmp() {
        let path = forward_socket(Path::new("/Users/ada/Application Support/hosts/abc"), "abc");
        let text = path.to_string_lossy();
        assert!(!text.contains(' '));
        assert!(text.ends_with("abc-forward.sock"));
    }
}
