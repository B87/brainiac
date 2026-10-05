//! `DockerEngine`: a run's container on the chosen engine, through the
//! Docker Engine API over that engine's socket (docs/architecture.md, Agent
//! runs — v0.5, Docker). The CLI is not used: a TTY or the daemon's default
//! log driver is too easy to turn on by accident.
//!
//! Everything a run owns carries labels (installation, run, attempt, role),
//! and is found by them, so a create whose answer was lost is still stopped.

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bollard::container::LogOutput;
use bollard::models::{
    ContainerCreateBody, HostConfig, HostConfigLogConfig, Mount, MountTypeEnum, RestartPolicy,
    RestartPolicyNameEnum, VolumeCreateOptions,
};
use bollard::query_parameters::{
    AttachContainerOptionsBuilder, CreateContainerOptionsBuilder, InspectContainerOptions,
    ListContainersOptionsBuilder, RemoveContainerOptionsBuilder, RemoveVolumeOptionsBuilder,
    StartContainerOptions, StopContainerOptionsBuilder, UploadToContainerOptionsBuilder,
};
use bollard::Docker;
use bytes::Bytes;
use futures_util::StreamExt;
use tokio::io::{AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, OwnedSemaphorePermit, Semaphore};

use crate::models::{AppError, AppResult};

pub const LABEL_INSTALLATION: &str = "org.brainiac.installation";
pub const LABEL_RUN: &str = "org.brainiac.run";
pub const LABEL_ATTEMPT: &str = "org.brainiac.attempt";
pub const LABEL_ROLE: &str = "org.brainiac.role";

/// The longest line the agent may write; a longer one fails the run.
pub const MAX_FRAME_BYTES: usize = 16 << 20;
/// How much of a run's output may wait in memory for the controller.
const BUFFERED_BYTES: usize = 32 << 20;
/// How long a stopping container gets after SIGTERM before SIGKILL.
const STOP_GRACE_SECS: i32 = 10;
/// Requests other than the attach stream and the upload.
const REQUEST_TIMEOUT_SECS: u64 = 60;
/// Copying a large history into the container.
const UPLOAD_TIMEOUT: Duration = Duration::from_secs(15 * 60);
/// ustar sizes are 11 octal digits.
const MAX_BUNDLE_BYTES: u64 = (1 << 33) - 1;

/// What a run's container is made from.
#[derive(Debug, Clone)]
pub struct LaunchSpec {
    pub engine_socket: String,
    pub image: String,
    pub installation: String,
    pub run_id: String,
    pub attempt: u32,
    pub volume: String,
    pub cpus: u32,
    pub memory_mib: u32,
    /// Copied to `/opt/brainiac/input/input.bundle` before the container starts.
    pub bundle: PathBuf,
    /// Becomes `true` when the run is stopped while its container is being
    /// made: the container is then not started.
    pub cancel: tokio::sync::watch::Receiver<bool>,
}

/// What the agent's stdout gave, a line at a time.
#[derive(Debug)]
pub enum Output {
    Line(Line),
    /// A line over `MAX_FRAME_BYTES`; nothing more is read.
    Oversized,
    /// The attach ended: the container stopped or the engine went away.
    Closed,
}

/// One line of the agent's output. While it is held it counts against its
/// run's budget of buffered output, so a fast agent cannot fill the
/// controller's memory: the reader waits, and the engine holds the rest.
#[derive(Debug)]
pub struct Line {
    pub bytes: Vec<u8>,
    _held: Option<OwnedSemaphorePermit>,
}

impl From<Vec<u8>> for Line {
    fn from(bytes: Vec<u8>) -> Self {
        Self { bytes, _held: None }
    }
}

/// A started container, attached.
pub struct Attached {
    pub container_id: String,
    pub output: mpsc::Receiver<Output>,
    pub stdin: mpsc::UnboundedSender<Vec<u8>>,
}

/// Whether anything of a run is running, by the engine's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Running {
    Yes,
    No,
}

/// What the controller needs from an engine. The controller is written
/// against this trait so its tests can stand in for Docker.
// `impl Future + Send` in a trait: each implementation returns its own
// future type, and `Send` lets the controller run it on any worker thread.
pub trait Workloads: Send + Sync + 'static {
    /// Create the volume and the container, copy the bundle in, start, attach.
    fn launch(&self, spec: LaunchSpec) -> impl Future<Output = AppResult<Attached>> + Send;
    /// Stop every container of the run, and keep them.
    fn stop(
        &self,
        socket: &str,
        installation: &str,
        run_id: &str,
    ) -> impl Future<Output = AppResult<()>> + Send;
    /// `No` only when the engine says none of the run's containers runs.
    fn running(
        &self,
        socket: &str,
        installation: &str,
        run_id: &str,
    ) -> impl Future<Output = AppResult<Running>> + Send;
    /// Remove the run's stopped containers and its volume.
    fn discard(
        &self,
        socket: &str,
        installation: &str,
        run_id: &str,
        volume: &str,
    ) -> impl Future<Output = AppResult<()>> + Send;
}

/// One client per engine socket.
#[derive(Default)]
pub struct DockerEngine {
    clients: Mutex<HashMap<String, Docker>>,
}

impl DockerEngine {
    pub fn new() -> Self {
        Self::default()
    }

    fn client(&self, socket: &str) -> AppResult<Docker> {
        let mut clients = self.clients.lock().expect("docker clients");
        if let Some(docker) = clients.get(socket) {
            return Ok(docker.clone());
        }
        let docker =
            Docker::connect_with_unix(socket, REQUEST_TIMEOUT_SECS, bollard::API_DEFAULT_VERSION)
                .map_err(|e| engine_error("The engine cannot be reached.", e))?;
        clients.insert(socket.to_string(), docker.clone());
        Ok(docker)
    }

    async fn run_containers(
        &self,
        docker: &Docker,
        installation: &str,
        run_id: &str,
        all: bool,
    ) -> AppResult<Vec<String>> {
        let mut filters = HashMap::new();
        filters.insert(
            "label".to_string(),
            vec![
                format!("{LABEL_INSTALLATION}={installation}"),
                format!("{LABEL_RUN}={run_id}"),
            ],
        );
        let options = ListContainersOptionsBuilder::new()
            .all(all)
            .filters(&filters)
            .build();
        let listed = docker
            .list_containers(Some(options))
            .await
            .map_err(|e| engine_error("The engine did not list the run's containers.", e))?;
        Ok(listed.into_iter().filter_map(|c| c.id).collect())
    }

    async fn create_and_start(&self, docker: &Docker, spec: &LaunchSpec) -> AppResult<String> {
        let labels = labels(spec);
        docker
            .create_volume(VolumeCreateOptions {
                name: Some(spec.volume.clone()),
                labels: Some(labels.clone()),
                ..Default::default()
            })
            .await
            .map_err(|e| engine_error("The run's workspace could not be created.", e))?;
        let mut tmpfs = HashMap::new();
        // Home and /tmp are in memory: Claude Code's own files never reach a disk.
        tmpfs.insert(
            "/home/node".to_string(),
            "rw,nosuid,nodev,size=256m,uid=1000,gid=1000,mode=0700".to_string(),
        );
        tmpfs.insert(
            "/tmp".to_string(),
            "rw,nosuid,nodev,size=512m,mode=1777".to_string(),
        );
        let host = HostConfig {
            log_config: Some(HostConfigLogConfig {
                typ: Some("none".to_string()),
                ..Default::default()
            }),
            cap_drop: Some(vec!["ALL".to_string()]),
            security_opt: Some(vec!["no-new-privileges:true".to_string()]),
            privileged: Some(false),
            nano_cpus: Some(i64::from(spec.cpus) * 1_000_000_000),
            memory: Some(i64::from(spec.memory_mib) << 20),
            pids_limit: Some(1024),
            tmpfs: Some(tmpfs),
            mounts: Some(vec![Mount {
                target: Some("/workspace".to_string()),
                source: Some(spec.volume.clone()),
                typ: Some(MountTypeEnum::VOLUME),
                read_only: Some(false),
                ..Default::default()
            }]),
            restart_policy: Some(RestartPolicy {
                name: Some(RestartPolicyNameEnum::NO),
                maximum_retry_count: None,
            }),
            ..Default::default()
        };
        let body = ContainerCreateBody {
            image: Some(spec.image.clone()),
            // Stdin stays open when the attach drops, so a controller that
            // dies does not end the agent quietly: the guard stops it.
            open_stdin: Some(true),
            stdin_once: Some(false),
            tty: Some(false),
            attach_stdin: Some(true),
            attach_stdout: Some(true),
            attach_stderr: Some(true),
            user: Some("node".to_string()),
            working_dir: Some("/workspace".to_string()),
            labels: Some(labels),
            host_config: Some(host),
            ..Default::default()
        };
        let name = format!("brainiac-run-{}-{}", spec.run_id, spec.attempt);
        let created = docker
            .create_container(
                Some(CreateContainerOptionsBuilder::new().name(&name).build()),
                body,
            )
            .await
            .map_err(|e| {
                engine_error(
                    "The run's container could not be created. Is the image built?",
                    e,
                )
            })?;
        let id = created.id;
        // Check what the engine made, not what was asked: a TTY or a log
        // driver would put the agent's protocol, and the credential frame,
        // somewhere else.
        let inspected = docker
            .inspect_container(&id, None::<InspectContainerOptions>)
            .await
            .map_err(|e| engine_error("The run's container could not be inspected.", e))?;
        let tty = inspected
            .config
            .as_ref()
            .and_then(|c| c.tty)
            .unwrap_or(true);
        let log = inspected
            .host_config
            .as_ref()
            .and_then(|h| h.log_config.as_ref())
            .and_then(|l| l.typ.clone())
            .unwrap_or_default();
        if tty || log != "none" {
            return Err(AppError::dependency(
                "The engine changed the run's container settings (terminal or logging), so the run was not started.",
            ));
        }
        upload_bundle(docker, &id, &spec.bundle).await?;
        Ok(id)
    }
}

impl Workloads for DockerEngine {
    async fn launch(&self, spec: LaunchSpec) -> AppResult<Attached> {
        let docker = self.client(&spec.engine_socket)?;
        let mut cancel = spec.cancel.clone();
        // Stopping the run drops the copy where it is; what was made is
        // stopped by labels, and removed with the run's other resources.
        let id = tokio::select! {
            made = self.create_and_start(&docker, &spec) => made?,
            _ = cancel.wait_for(|stop| *stop) => return Err(stopped_while_made()),
        };
        // Attach before start, so no line is missed: the log driver is off
        // and there is nothing to read back later.
        let attach = AttachContainerOptionsBuilder::new()
            .stream(true)
            .stdin(true)
            .stdout(true)
            .stderr(true)
            .logs(false)
            .build();
        let attached = docker
            .attach_container(&id, Some(attach))
            .await
            .map_err(|e| engine_error("The run's container could not be attached.", e))?;
        if *spec.cancel.borrow() {
            return Err(stopped_while_made());
        }
        docker
            .start_container(&id, None::<StartContainerOptions>)
            .await
            .map_err(|e| engine_error("The run's container did not start.", e))?;
        // A bounded channel: if the controller falls behind, the reader waits
        // and the engine holds the agent's output, rather than memory growing.
        let (output_tx, output) = mpsc::channel(256);
        let (stdin, stdin_rx) = mpsc::unbounded_channel();
        let budget = Arc::new(Semaphore::new(BUFFERED_BYTES));
        tokio::spawn(read_output(attached.output, output_tx, budget));
        tokio::spawn(write_stdin(attached.input, stdin_rx));
        Ok(Attached {
            container_id: id,
            output,
            stdin,
        })
    }

    async fn stop(&self, socket: &str, installation: &str, run_id: &str) -> AppResult<()> {
        let docker = self.client(socket)?;
        for id in self
            .run_containers(&docker, installation, run_id, false)
            .await?
        {
            let options = StopContainerOptionsBuilder::new()
                .t(STOP_GRACE_SECS)
                .build();
            // The stop waits for the grace period, longer than other requests.
            let docker = docker.clone().with_timeout(Duration::from_secs(
                REQUEST_TIMEOUT_SECS + STOP_GRACE_SECS as u64,
            ));
            match docker.stop_container(&id, Some(options)).await {
                Ok(()) => {}
                Err(bollard::errors::Error::DockerResponseServerError {
                    status_code: 304 | 404,
                    ..
                }) => {}
                Err(e) => {
                    return Err(engine_error(
                        "The engine did not stop the run's container.",
                        e,
                    ))
                }
            }
        }
        Ok(())
    }

    async fn running(&self, socket: &str, installation: &str, run_id: &str) -> AppResult<Running> {
        let docker = self.client(socket)?;
        let running = self
            .run_containers(&docker, installation, run_id, false)
            .await?;
        Ok(if running.is_empty() {
            Running::No
        } else {
            Running::Yes
        })
    }

    async fn discard(
        &self,
        socket: &str,
        installation: &str,
        run_id: &str,
        volume: &str,
    ) -> AppResult<()> {
        let docker = self.client(socket)?;
        if !self
            .run_containers(&docker, installation, run_id, false)
            .await?
            .is_empty()
        {
            return Err(super::conflict("The run is still running; stop it first."));
        }
        for id in self
            .run_containers(&docker, installation, run_id, true)
            .await?
        {
            let options = RemoveContainerOptionsBuilder::new().build();
            match docker.remove_container(&id, Some(options)).await {
                Ok(())
                | Err(bollard::errors::Error::DockerResponseServerError {
                    status_code: 404, ..
                }) => {}
                Err(e) => {
                    return Err(engine_error(
                        "The engine did not remove the run's container.",
                        e,
                    ))
                }
            }
        }
        // Only the run's own volume: its name is the run's, and its labels say so.
        let owned = match docker.inspect_volume(volume).await {
            Ok(v) => {
                v.labels.get(LABEL_RUN).map(String::as_str) == Some(run_id)
                    && v.labels.get(LABEL_INSTALLATION).map(String::as_str) == Some(installation)
            }
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404, ..
            }) => return Ok(()),
            Err(e) => {
                return Err(engine_error(
                    "The engine did not show the run's workspace.",
                    e,
                ))
            }
        };
        if !owned {
            return Err(super::conflict(
                "A workspace with the run's name belongs to something else; it was left alone.",
            ));
        }
        match docker
            .remove_volume(volume, Some(RemoveVolumeOptionsBuilder::new().build()))
            .await
        {
            Ok(())
            | Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404, ..
            }) => Ok(()),
            Err(e) => Err(engine_error(
                "The engine did not remove the run's workspace.",
                e,
            )),
        }
    }
}

fn labels(spec: &LaunchSpec) -> HashMap<String, String> {
    HashMap::from([
        (LABEL_INSTALLATION.to_string(), spec.installation.clone()),
        (LABEL_RUN.to_string(), spec.run_id.clone()),
        (LABEL_ATTEMPT.to_string(), spec.attempt.to_string()),
        (LABEL_ROLE.to_string(), "agent".to_string()),
    ])
}

fn stopped_while_made() -> AppError {
    AppError::new(
        crate::models::ErrorCode::Cancelled,
        "The run was stopped before its container started.",
    )
}

/// Engine errors keep the engine's words in the details, never the message.
fn engine_error(message: &str, error: bollard::errors::Error) -> AppError {
    AppError::dependency(message).with_details(error.to_string())
}

/// Copy the bundle into the created container as a one-file tar, read from
/// disk a piece at a time.
async fn upload_bundle(docker: &Docker, id: &str, bundle: &std::path::Path) -> AppResult<()> {
    let file = tokio::fs::File::open(bundle)
        .await
        .map_err(|e| AppError::from(e).with_details(bundle.display().to_string()))?;
    let size = file.metadata().await?.len();
    if size > MAX_BUNDLE_BYTES {
        return Err(AppError::validation(
            "The repository's history is over 8 GB, too large for a run.",
        ));
    }
    let header =
        Bytes::copy_from_slice(&crate::agents::image::header("input.bundle", size as usize));
    let padding = (512 - (size % 512) as usize) % 512 + 1024;
    // `unfold` turns the file into a stream of pieces: the header, the
    // file's bytes, then the padding and the two empty blocks that end a
    // tar. A read error, or a file shorter than it was, fails the upload
    // rather than sending a cut-off archive.
    let pieces = futures_util::stream::unfold((file, 0u64), move |(mut file, sent)| async move {
        if sent >= size {
            return None;
        }
        let mut buf = vec![0u8; (1 << 20).min((size - sent) as usize)];
        match file.read(&mut buf).await {
            Ok(0) => Some((
                Err(std::io::Error::other(
                    "the bundle changed while it was copied",
                )),
                (file, size),
            )),
            Ok(n) => {
                buf.truncate(n);
                Some((Ok(Bytes::from(buf)), (file, sent + n as u64)))
            }
            Err(e) => Some((Err(e), (file, size))),
        }
    });
    let body = futures_util::stream::once(async move { Ok(header) })
        .chain(pieces)
        .chain(futures_util::stream::once(async move {
            Ok(Bytes::from(vec![0u8; padding]))
        }));
    let options = UploadToContainerOptionsBuilder::new()
        .path("/opt/brainiac/input")
        .build();
    docker
        .clone()
        .with_timeout(UPLOAD_TIMEOUT)
        .upload_to_container(id, Some(options), bollard::body_try_stream(body))
        .await
        .map_err(|e| engine_error("The run's start could not be copied into its container.", e))
}

type OutputStream =
    Pin<Box<dyn futures_util::Stream<Item = Result<LogOutput, bollard::errors::Error>> + Send>>;

/// Split stdout into lines for the controller. Stderr is drained and
/// dropped: it is never stored or shown (it can hold anything the agent
/// printed, the credential included).
async fn read_output(mut output: OutputStream, tx: mpsc::Sender<Output>, budget: Arc<Semaphore>) {
    let mut line = Vec::new();
    while let Some(item) = output.next().await {
        let Ok(item) = item else { break };
        let chunk = match item {
            LogOutput::StdOut { message } | LogOutput::Console { message } => message,
            LogOutput::StdErr { .. } | LogOutput::StdIn { .. } => continue,
        };
        let mut rest: &[u8] = &chunk;
        while let Some(pos) = rest.iter().position(|b| *b == b'\n') {
            line.extend_from_slice(&rest[..pos]);
            rest = &rest[pos + 1..];
            if line.len() > MAX_FRAME_BYTES {
                let _ = tx.send(Output::Oversized).await;
                return;
            }
            let bytes = std::mem::take(&mut line);
            // At most `MAX_FRAME_BYTES`, under the budget, so it fits a `u32`.
            let Ok(held) = Arc::clone(&budget)
                .acquire_many_owned(bytes.len().max(1) as u32)
                .await
            else {
                return;
            };
            let line = Line {
                bytes,
                _held: Some(held),
            };
            if tx.send(Output::Line(line)).await.is_err() {
                return;
            }
        }
        line.extend_from_slice(rest);
        if line.len() > MAX_FRAME_BYTES {
            let _ = tx.send(Output::Oversized).await;
            return;
        }
    }
    let _ = tx.send(Output::Closed).await;
}

async fn write_stdin(
    mut input: Pin<Box<dyn AsyncWrite + Send>>,
    mut rx: mpsc::UnboundedReceiver<Vec<u8>>,
) {
    while let Some(mut chunk) = rx.recv().await {
        let written = input.write_all(&chunk).await.is_ok() && input.flush().await.is_ok();
        // The first chunk is the credential frame: overwrite each chunk once written.
        chunk.fill(0);
        if !written {
            break;
        }
    }
}
