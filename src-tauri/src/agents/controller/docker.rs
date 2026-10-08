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
    AttachContainerOptionsBuilder, CreateContainerOptionsBuilder, DownloadFromContainerOptions,
    InspectContainerOptions, ListContainersOptionsBuilder, RemoveContainerOptionsBuilder,
    RemoveVolumeOptionsBuilder, StartContainerOptions, StopContainerOptionsBuilder,
    UploadToContainerOptionsBuilder, WaitContainerOptions,
};
use bollard::Docker;
use bytes::Bytes;
use futures_util::StreamExt;
use tokio::io::{AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, OwnedSemaphorePermit, Semaphore};

use super::archive::{Expected, Sink, TarReader};
use super::protocol::CollectManifest;
use crate::models::{AgentKind, AgentProvider, AppError, AppResult, RunPermissions};

pub const LABEL_INSTALLATION: &str = "org.brainiac.installation";
pub const LABEL_RUN: &str = "org.brainiac.run";
pub const LABEL_ATTEMPT: &str = "org.brainiac.attempt";
pub const LABEL_ROLE: &str = "org.brainiac.role";
/// On a workspace volume: the loop device it was created on.
pub const LABEL_DEVICE: &str = "org.brainiac.device";
/// One volume per engine holds the workspaces' image files, so a run's
/// workspace is a filesystem of exactly its size (docs/architecture.md,
/// Agent runs — v0.5, Workspace size).
pub const STORE_VOLUME: &str = "brainiac-workspaces";
/// What a helper may print.
const MAX_HELPER_OUTPUT: usize = 64 << 10;

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
/// The collector hashes every file of the workspace and writes the bundle.
const COLLECT_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// A preview of a live run gives up sooner.
pub const PREVIEW_TIMEOUT: Duration = Duration::from_secs(5 * 60);
/// The result bundle holds only what the start does not: new objects.
const MAX_RESULT_BYTES: u64 = 2 << 30;
const MAX_MANIFEST_BYTES: u64 = 16 << 20;

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
    /// The workspace's filesystem is made at exactly this size.
    pub workspace_gib: u32,
    /// The container's environment, from [`launch_env`]: which agent to
    /// start and its model and settings. Never the credential.
    pub env: Vec<String>,
    /// Copied to `/opt/brainiac/input/input.bundle` before the container starts.
    pub bundle: PathBuf,
    /// Becomes `true` when the run is stopped while its container is being
    /// made: the container is then not started.
    pub cancel: tokio::sync::watch::Receiver<bool>,
}

/// The run container's environment (docs/architecture.md, Agent runs —
/// v0.5, Agents): which agent the entrypoint starts, and how. Nothing in it
/// is a secret; the credential goes once to stdin.
///
/// Claude Code reads its model from `ANTHROPIC_MODEL`, which wins over the
/// repository's settings. OpenCode reads `OPENCODE_CONFIG_CONTENT`, which
/// the image's managed configuration (`/etc/opencode/opencode.json`) still
/// overrides, so the providers' addresses stay pinned: the model as
/// `<provider>/<model>`, only that provider, and in Ask before actions, its
/// edits, commands, and fetches asked for first. In Act without asking it
/// keeps its own defaults, and the controller allows what it still asks.
pub fn launch_env(
    agent: AgentKind,
    provider: AgentProvider,
    model: &str,
    permissions: RunPermissions,
) -> Vec<String> {
    match agent {
        AgentKind::ClaudeCode if model.is_empty() => Vec::new(),
        AgentKind::ClaudeCode => vec![format!("ANTHROPIC_MODEL={model}")],
        AgentKind::Opencode => {
            let mut config = serde_json::json!({ "enabled_providers": [provider.as_str()] });
            if !model.is_empty() {
                config["model"] = format!("{}/{model}", provider.as_str()).into();
            }
            if permissions == RunPermissions::Ask {
                config["permission"] =
                    serde_json::json!({ "edit": "ask", "bash": "ask", "webfetch": "ask" });
            }
            vec![
                "BRAINIAC_AGENT=opencode".to_string(),
                format!("OPENCODE_CONFIG_CONTENT={config}"),
            ]
        }
    }
}

/// What the collector's container is made from.
#[derive(Debug, Clone)]
pub struct CollectSpec {
    pub engine_socket: String,
    /// The run's own image, which holds the collector and the helper.
    pub image: String,
    /// The current image, used when the run's is no longer on the engine:
    /// a rebuild after an update does not strand older runs.
    pub fallback_image: Option<String>,
    pub installation: String,
    pub run_id: String,
    pub attempt: u32,
    /// The run's workspace, mounted read only.
    pub volume: String,
    pub memory_mib: u32,
    /// The run's input bundle: the start, cloned again by the collector.
    pub bundle: PathBuf,
    pub start_commit: String,
    /// Left-out paths to collect this time.
    pub include: Vec<String>,
    /// Where `result.bundle` is written, on the Mac.
    pub out_dir: PathBuf,
    /// A preview of a running run: its container is not stopped, and its
    /// workspace is mounted and attached already, so it is read as it is.
    pub live: bool,
    /// For a preview: becomes `true` when the run starts stopping. The
    /// preview's container is then not started, so a stop that listed the
    /// run's running containers before it existed cannot miss it.
    pub cancel: Option<tokio::sync::watch::Receiver<bool>>,
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
    /// Before anything of a run is made: the run's image is on the engine.
    /// A cleanup of unused images on the engine's machine can remove it.
    fn check_image(&self, socket: &str, image: &str) -> impl Future<Output = AppResult<()>> + Send;
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
    /// Run the collector against the run's stopped volume (or, `live`, its
    /// running one) and bring its result to `out_dir`.
    fn collect(&self, spec: CollectSpec)
        -> impl Future<Output = AppResult<CollectManifest>> + Send;
    /// Remove the run's stopped containers, its volume, and its workspace
    /// file. `fallback_image` is as in `CollectSpec`.
    fn discard(
        &self,
        socket: &str,
        installation: &str,
        run_id: &str,
        volume: &str,
        image: &str,
        fallback_image: Option<&str>,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// What this engine is, and whether `losetup` works when `image` is set.
    fn probe(
        &self,
        _socket: &str,
        _image: Option<&str>,
    ) -> impl Future<Output = AppResult<super::protocol::EngineReport>> + Send {
        std::future::ready(Err(AppError::dependency(
            "This engine is not probed from the run controller.",
        )))
    }

    /// Build the run image from a tar context.
    fn build_image(
        &self,
        _socket: &str,
        _tag: &str,
        _context: Vec<u8>,
    ) -> impl Future<Output = AppResult<String>> + Send {
        std::future::ready(Err(AppError::dependency(
            "This engine does not build the run image.",
        )))
    }
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
        self.make_workspace(docker, spec, &labels).await?;
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
            env: (!spec.env.is_empty()).then(|| spec.env.clone()),
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
        upload_input(docker, &id, &spec.bundle, None).await?;
        Ok(id)
    }
}

impl DockerEngine {
    /// Run a short shell script in a privileged helper container of the
    /// run's image, with the workspace store mounted at `/store`, and return
    /// what it printed. Privileged, because it attaches loop devices in the
    /// engine's VM; it has no network, no credential, and nothing of a run.
    async fn helper(
        &self,
        docker: &Docker,
        image: &str,
        installation: &str,
        run_id: &str,
        script: &str,
    ) -> AppResult<String> {
        let (exit, printed) = self
            .helper_status(docker, image, installation, run_id, script)
            .await?;
        if exit != 0 {
            return Err(helper_failed(exit));
        }
        Ok(printed)
    }

    /// The helper's exit status and what it printed, for a caller whose
    /// script says something with its status.
    async fn helper_status(
        &self,
        docker: &Docker,
        image: &str,
        installation: &str,
        run_id: &str,
        script: &str,
    ) -> AppResult<(i64, String)> {
        // The scripts interpolate the run ID: only a name, never a path.
        if !super::protocol::valid_id(run_id) {
            return Err(AppError::validation("Invalid run ID."));
        }
        let labels = HashMap::from([
            (LABEL_INSTALLATION.to_string(), installation.to_string()),
            (LABEL_RUN.to_string(), run_id.to_string()),
            (LABEL_ROLE.to_string(), "helper".to_string()),
        ]);
        let host = HostConfig {
            log_config: Some(HostConfigLogConfig {
                typ: Some("none".to_string()),
                ..Default::default()
            }),
            network_mode: Some("none".to_string()),
            privileged: Some(true),
            pids_limit: Some(64),
            memory: Some(256 << 20),
            mounts: Some(vec![Mount {
                target: Some("/store".to_string()),
                source: Some(STORE_VOLUME.to_string()),
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
            image: Some(image.to_string()),
            entrypoint: Some(vec!["sh".to_string(), "-c".to_string()]),
            cmd: Some(vec![script.to_string()]),
            user: Some("root".to_string()),
            tty: Some(false),
            attach_stdout: Some(true),
            attach_stderr: Some(true),
            labels: Some(labels),
            host_config: Some(host),
            ..Default::default()
        };
        let name = format!("brainiac-helper-{run_id}-{}", now_suffix());
        let created = docker
            .create_container(
                Some(CreateContainerOptionsBuilder::new().name(&name).build()),
                body,
            )
            .await
            .map_err(|e| {
                engine_error(
                    "The workspace helper's container could not be created. Is the image built?",
                    e,
                )
            })?;
        let id = created.id;
        let result = self.run_helper(docker, &id).await;
        let _ = docker
            .remove_container(
                &id,
                Some(RemoveContainerOptionsBuilder::new().force(true).build()),
            )
            .await;
        result
    }

    async fn run_helper(&self, docker: &Docker, id: &str) -> AppResult<(i64, String)> {
        let attach = AttachContainerOptionsBuilder::new()
            .stream(true)
            .stdout(true)
            .stderr(true)
            .logs(false)
            .build();
        let attached = docker
            .attach_container(id, Some(attach))
            .await
            .map_err(|e| engine_error("The workspace helper could not be attached.", e))?;
        docker
            .start_container(id, None::<StartContainerOptions>)
            .await
            .map_err(|e| {
                engine_error(
                    "The engine did not start Brainiac's workspace helper. Runs need an engine that allows a privileged container of Brainiac's own.",
                    e,
                )
            })?;
        let mut output = attached.output;
        let mut printed = Vec::new();
        let read = async {
            while let Some(item) = output.next().await {
                let Ok(item) = item else { break };
                if let LogOutput::StdOut { message } = item {
                    if printed.len() + message.len() <= MAX_HELPER_OUTPUT {
                        printed.extend_from_slice(&message);
                    }
                }
            }
        };
        if tokio::time::timeout(Duration::from_secs(10 * 60), read)
            .await
            .is_err()
        {
            return Err(AppError::timeout(
                "The workspace helper did not finish within ten minutes.",
            ));
        }
        let mut waiting = docker.wait_container(
            id,
            Some(WaitContainerOptions {
                condition: "not-running".to_string(),
            }),
        );
        let exit = match waiting.next().await {
            Some(Ok(response)) => response.status_code,
            Some(Err(bollard::errors::Error::DockerContainerWaitError { code, .. })) => code,
            Some(Err(e)) => {
                return Err(engine_error(
                    "The engine did not say how the helper ended.",
                    e,
                ))
            }
            None => 0,
        };
        Ok((exit, String::from_utf8_lossy(&printed).trim().to_string()))
    }

    /// The image to run a kept run's helper and collector from: the run's
    /// own, or the current one when the run's was removed from the engine
    /// (a rebuild after an update, or a prune).
    async fn resolve_image(
        &self,
        docker: &Docker,
        image: &str,
        fallback: Option<&str>,
    ) -> AppResult<String> {
        for candidate in std::iter::once(image).chain(fallback) {
            match docker.inspect_image(candidate).await {
                Ok(_) => return Ok(candidate.to_string()),
                Err(bollard::errors::Error::DockerResponseServerError {
                    status_code: 404, ..
                }) => {}
                Err(e) => return Err(engine_error("The engine did not show the image.", e)),
            }
        }
        Err(AppError::dependency(
            "The run's image is no longer on the engine. Build the image in Settings → Agents, then try again.",
        ))
    }

    /// The volume that holds every workspace's image file on this engine.
    async fn ensure_store(&self, docker: &Docker, installation: &str) -> AppResult<()> {
        match docker.inspect_volume(STORE_VOLUME).await {
            Ok(v) if v.labels.get(LABEL_ROLE).map(String::as_str) == Some("store") => {
                return Ok(())
            }
            Ok(_) => {
                return Err(AppError::dependency(
                    "A volume named brainiac-workspaces on this engine is not Brainiac's; runs cannot use it.",
                ))
            }
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404, ..
            }) => {}
            Err(e) => {
                return Err(engine_error(
                    "The engine did not show the workspace store.",
                    e,
                ))
            }
        }
        docker
            .create_volume(VolumeCreateOptions {
                name: Some(STORE_VOLUME.to_string()),
                labels: Some(HashMap::from([
                    (LABEL_INSTALLATION.to_string(), installation.to_string()),
                    (LABEL_ROLE.to_string(), "store".to_string()),
                ])),
                ..Default::default()
            })
            .await
            .map_err(|e| engine_error("The workspace store could not be created.", e))?;
        Ok(())
    }

    /// A workspace of exactly the run's size: an ext4 filesystem in a file
    /// of the store, attached as a loop device by the helper, and a volume
    /// on that device. The engine refuses writes past the size.
    async fn make_workspace(
        &self,
        docker: &Docker,
        spec: &LaunchSpec,
        labels: &HashMap<String, String>,
    ) -> AppResult<()> {
        self.ensure_store(docker, &spec.installation).await?;
        let script = format!(
            "set -e; f=/store/{run}.img; rm -f \"$f\"; truncate -s {size}G \"$f\"; \
             mkfs.ext4 -q -F -E root_owner=1000:1000 \"$f\" >/dev/null; losetup -f --show \"$f\"",
            run = spec.run_id,
            size = spec.workspace_gib
        );
        let device = self
            .helper(
                docker,
                &spec.image,
                &spec.installation,
                &spec.run_id,
                &script,
            )
            .await?;
        if !device.starts_with("/dev/loop") {
            return Err(AppError::dependency(
                "The workspace helper did not attach a loop device.",
            ));
        }
        let mut labels = labels.clone();
        labels.insert(LABEL_DEVICE.to_string(), device.clone());
        docker
            .create_volume(VolumeCreateOptions {
                name: Some(spec.volume.clone()),
                driver: Some("local".to_string()),
                driver_opts: Some(HashMap::from([
                    ("type".to_string(), "ext4".to_string()),
                    ("device".to_string(), device),
                ])),
                labels: Some(labels),
                ..Default::default()
            })
            .await
            .map_err(|e| engine_error("The run's workspace could not be created.", e))?;
        Ok(())
    }

    /// Before a kept workspace is used again (the collector): the loop
    /// device may be gone after the engine restarted, so it is attached
    /// again, and the volume made anew on it when its device changed. The
    /// files are in the image file, which is unchanged by this.
    async fn ensure_workspace(
        &self,
        docker: &Docker,
        image: &str,
        installation: &str,
        run_id: &str,
        volume: &str,
    ) -> AppResult<()> {
        let script = format!(
            "set -e; f=/store/{run_id}.img; test -f \"$f\" || exit 3; \
             d=$(losetup -j \"$f\" | head -n 1 | cut -d: -f1); \
             [ -n \"$d\" ] || d=$(losetup -f --show \"$f\"); echo \"$d\""
        );
        let device = match self
            .helper_status(docker, image, installation, run_id, &script)
            .await
        {
            Ok((0, device)) => device,
            // The script's own answer: no workspace file for this run.
            Ok((3, _)) => {
                return Err(AppError::dependency(
                    "The run's workspace is not on the engine, so there is no work to collect. The run may have ended before its workspace was made, or the engine's data was removed.",
                ))
            }
            Ok((exit, _)) => {
                return Err(AppError::dependency(
                    "The run's workspace could not be reached, so its work was not collected.",
                )
                .with_details(helper_failed(exit).message))
            }
            Err(e) => {
                return Err(AppError::new(
                    e.code,
                    "The run's workspace could not be reached, so its work was not collected.",
                )
                .with_details(e.message))
            }
        };
        let current = match docker.inspect_volume(volume).await {
            Ok(v) => Some(v),
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404, ..
            }) => None,
            Err(e) => {
                return Err(engine_error(
                    "The engine did not show the run's workspace.",
                    e,
                ))
            }
        };
        let (labels, same) = match &current {
            Some(v) => (
                v.labels.clone(),
                v.options.get("device").map(String::as_str) == Some(device.as_str()),
            ),
            None => (
                HashMap::from([
                    (LABEL_INSTALLATION.to_string(), installation.to_string()),
                    (LABEL_RUN.to_string(), run_id.to_string()),
                    (LABEL_ROLE.to_string(), "agent".to_string()),
                ]),
                false,
            ),
        };
        if same {
            return Ok(());
        }
        if current.is_some() {
            // The engine keeps a volume that any container, stopped or not,
            // still names; the run's stopped containers go first. Their
            // layers hold nothing of the work: it is in the image file.
            for id in self
                .run_containers(docker, installation, run_id, true)
                .await?
            {
                let options = RemoveContainerOptionsBuilder::new().build();
                match docker.remove_container(&id, Some(options)).await {
                    Ok(())
                    | Err(bollard::errors::Error::DockerResponseServerError {
                        status_code: 404,
                        ..
                    }) => {}
                    Err(e) => return Err(engine_error(
                        "The run's stopped container could not be removed to reach its workspace.",
                        e,
                    )),
                }
            }
            docker
                .remove_volume(volume, Some(RemoveVolumeOptionsBuilder::new().build()))
                .await
                .map_err(|e| engine_error("The run's workspace could not be made again.", e))?;
        }
        let mut labels = labels;
        labels.insert(LABEL_DEVICE.to_string(), device.clone());
        docker
            .create_volume(VolumeCreateOptions {
                name: Some(volume.to_string()),
                driver: Some("local".to_string()),
                driver_opts: Some(HashMap::from([
                    ("type".to_string(), "ext4".to_string()),
                    ("device".to_string(), device),
                ])),
                labels: Some(labels),
                ..Default::default()
            })
            .await
            .map_err(|e| engine_error("The run's workspace could not be made again.", e))?;
        Ok(())
    }

    /// Detach the workspace's loop device and delete its image file.
    async fn remove_workspace_file(
        &self,
        docker: &Docker,
        image: &str,
        installation: &str,
        run_id: &str,
    ) -> AppResult<()> {
        let script = format!(
            "f=/store/{run_id}.img; for d in $(losetup -j \"$f\" 2>/dev/null | cut -d: -f1); do losetup -d \"$d\" || true; done; rm -f \"$f\""
        );
        self.helper(docker, image, installation, run_id, &script)
            .await
            .map(|_| ())
            .map_err(|e| {
                AppError::new(e.code, "The run's workspace file was not removed.")
                    .with_details(e.message)
            })
    }

    /// Remove a run's preview containers, running or not. One that started
    /// after the run's stop was confirmed would otherwise keep the run
    /// "running" for its collection and its Discard.
    async fn remove_previews(
        &self,
        docker: &Docker,
        installation: &str,
        run_id: &str,
    ) -> AppResult<()> {
        let mut filters = HashMap::new();
        filters.insert(
            "label".to_string(),
            vec![
                format!("{LABEL_INSTALLATION}={installation}"),
                format!("{LABEL_RUN}={run_id}"),
                format!("{LABEL_ROLE}=preview"),
            ],
        );
        let listed = docker
            .list_containers(Some(
                ListContainersOptionsBuilder::new()
                    .all(true)
                    .filters(&filters)
                    .build(),
            ))
            .await
            .map_err(|e| engine_error("The engine did not list the run's containers.", e))?;
        for id in listed.into_iter().filter_map(|c| c.id) {
            let options = RemoveContainerOptionsBuilder::new().force(true).build();
            match docker.remove_container(&id, Some(options)).await {
                Ok(())
                | Err(bollard::errors::Error::DockerResponseServerError {
                    status_code: 404, ..
                }) => {}
                Err(e) => {
                    return Err(engine_error(
                        "The engine did not remove a preview of the run's changes.",
                        e,
                    ))
                }
            }
        }
        Ok(())
    }

    /// The collector's container: the run's volume read only, no network,
    /// no credential, the same image with its collector as the entrypoint.
    async fn create_collector(&self, docker: &Docker, spec: &CollectSpec) -> AppResult<String> {
        let labels = HashMap::from([
            (LABEL_INSTALLATION.to_string(), spec.installation.clone()),
            (LABEL_RUN.to_string(), spec.run_id.clone()),
            (LABEL_ATTEMPT.to_string(), spec.attempt.to_string()),
            (
                LABEL_ROLE.to_string(),
                if spec.live { "preview" } else { "collector" }.to_string(),
            ),
        ]);
        let host = HostConfig {
            log_config: Some(HostConfigLogConfig {
                typ: Some("none".to_string()),
                ..Default::default()
            }),
            network_mode: Some("none".to_string()),
            cap_drop: Some(vec!["ALL".to_string()]),
            security_opt: Some(vec!["no-new-privileges:true".to_string()]),
            privileged: Some(false),
            memory: Some(i64::from(spec.memory_mib) << 20),
            pids_limit: Some(256),
            mounts: Some(vec![Mount {
                target: Some("/work".to_string()),
                source: Some(spec.volume.clone()),
                typ: Some(MountTypeEnum::VOLUME),
                read_only: Some(true),
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
            entrypoint: Some(vec![
                "node".to_string(),
                "/opt/claude/collector.mjs".to_string(),
            ]),
            cmd: Some(Vec::new()),
            tty: Some(false),
            open_stdin: Some(false),
            user: Some("node".to_string()),
            working_dir: Some("/scratch".to_string()),
            labels: Some(labels),
            host_config: Some(host),
            ..Default::default()
        };
        let name = format!(
            "brainiac-{}-{}-{}",
            if spec.live { "preview" } else { "collect" },
            spec.run_id,
            now_suffix()
        );
        let created = docker
            .create_container(
                Some(CreateContainerOptionsBuilder::new().name(&name).build()),
                body,
            )
            .await
            .map_err(|e| {
                engine_error(
                    "The collector's container could not be created. Is the image built?",
                    e,
                )
            })?;
        let id = created.id;
        // What the engine made: the volume must be read only and the
        // container off the network, or the collection is not trusted.
        let inspected = docker
            .inspect_container(&id, None::<InspectContainerOptions>)
            .await
            .map_err(|e| engine_error("The collector's container could not be inspected.", e))?;
        let host = inspected.host_config.unwrap_or_default();
        let read_only = inspected
            .mounts
            .unwrap_or_default()
            .iter()
            .any(|m| m.destination.as_deref() == Some("/work") && m.rw == Some(false));
        if host.network_mode.as_deref() != Some("none") || !read_only {
            return Err(AppError::dependency(
                "The engine changed the collector's container settings (network or mount), so the work was not collected.",
            ));
        }
        let params = serde_json::json!({
            "start": spec.start_commit,
            "include": spec.include,
        });
        upload_input(
            docker,
            &id,
            &spec.bundle,
            Some(("collect.json", params.to_string().into_bytes())),
        )
        .await?;
        Ok(id)
    }

    async fn run_collector(
        &self,
        docker: &Docker,
        id: &str,
        spec: &CollectSpec,
    ) -> AppResult<CollectManifest> {
        docker
            .start_container(id, None::<StartContainerOptions>)
            .await
            .map_err(|e| engine_error("The collector's container did not start.", e))?;
        // A preview reads a workspace the agent keeps changing; one that
        // takes long is not worth holding the run's next preview for.
        let limit = if spec.live {
            PREVIEW_TIMEOUT
        } else {
            COLLECT_TIMEOUT
        };
        let mut waiting = docker.clone().with_timeout(limit).wait_container(
            id,
            Some(WaitContainerOptions {
                condition: "not-running".to_string(),
            }),
        );
        let exit = match tokio::time::timeout(limit, waiting.next()).await {
            Ok(Some(Ok(response))) => response.status_code,
            // bollard reports a non-zero exit as an error carrying the code.
            Ok(Some(Err(bollard::errors::Error::DockerContainerWaitError { code, .. }))) => code,
            Ok(Some(Err(e))) => {
                return Err(engine_error(
                    "The engine did not say how the collector ended.",
                    e,
                ))
            }
            Ok(None) => return Err(AppError::dependency("The collector's container vanished.")),
            Err(_) => {
                let _ = docker
                    .stop_container(id, Some(StopContainerOptionsBuilder::new().t(5).build()))
                    .await;
                return Err(AppError::timeout(if spec.live {
                    "Reading the changes took over 5 minutes and was stopped."
                } else {
                    "Collecting the work took over 30 minutes and was stopped."
                }));
            }
        };
        let bundle = spec.out_dir.join("result.bundle");
        let partial = spec.out_dir.join("result.bundle.part");
        let _ = std::fs::remove_file(&bundle);
        let mut reader = TarReader::new(vec![
            Expected {
                name: "out/manifest.json",
                max_bytes: MAX_MANIFEST_BYTES,
                sink: Sink::Memory(Vec::new()),
                found: false,
            },
            Expected {
                name: "out/result.bundle",
                max_bytes: MAX_RESULT_BYTES,
                sink: Sink::File(partial.clone(), None),
                found: false,
            },
        ]);
        let mut download = docker
            .clone()
            .with_timeout(COLLECT_TIMEOUT)
            .download_from_container(
                id,
                Some(DownloadFromContainerOptions {
                    path: "/out".to_string(),
                }),
            );
        let read = async {
            while let Some(chunk) = download.next().await {
                let chunk = chunk
                    .map_err(|e| engine_error("The collector's result could not be read.", e))?;
                reader.feed(&chunk).await.map_err(|e| {
                    AppError::io("The collector's result is not what was expected.")
                        .with_details(e.to_string())
                })?;
            }
            reader.finish().map_err(|e| {
                AppError::io("The collector's result was cut short.").with_details(e.to_string())
            })
        };
        let entries = match read.await {
            Ok(entries) => entries,
            Err(e) => {
                let _ = std::fs::remove_file(&partial);
                return Err(e);
            }
        };
        let manifest_bytes = match &entries[0].sink {
            Sink::Memory(bytes) if entries[0].found => bytes.clone(),
            _ => {
                let _ = std::fs::remove_file(&partial);
                return Err(AppError::io(format!(
                    "The collector wrote no manifest (exit status {exit})."
                )));
            }
        };
        // A failed collection writes its reason and no result: read that first.
        let raw: serde_json::Value = serde_json::from_slice(&manifest_bytes).map_err(|e| {
            AppError::io("The collector's manifest could not be read.").with_details(e.to_string())
        })?;
        if let Some(error) = raw
            .get("error")
            .and_then(|e| e.as_str())
            .filter(|e| !e.is_empty())
        {
            let _ = std::fs::remove_file(&partial);
            return Err(AppError::io(error.to_string()));
        }
        if exit != 0 {
            let _ = std::fs::remove_file(&partial);
            return Err(AppError::io(format!(
                "The collector failed (exit status {exit})."
            )));
        }
        let manifest: CollectManifest = serde_json::from_value(raw).map_err(|e| {
            AppError::io("The collector's manifest could not be read.").with_details(e.to_string())
        })?;
        if manifest.result != manifest.start {
            if !entries[1].found {
                return Err(AppError::io("The collector wrote no result bundle."));
            }
            std::fs::rename(&partial, &bundle)?;
        } else {
            let _ = std::fs::remove_file(&partial);
        }
        Ok(manifest)
    }
}

fn now_suffix() -> String {
    format!(
        "{:x}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    )
}

impl Workloads for DockerEngine {
    async fn check_image(&self, socket: &str, image: &str) -> AppResult<()> {
        let docker = self.client(socket)?;
        match docker.inspect_image(image).await {
            Ok(_) => Ok(()),
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404, ..
            }) => Err(AppError::dependency(
                "The run's image is no longer on the engine, so the run did not start. A cleanup of unused images may have removed it: build the image in Settings → Agents, then start a new run.",
            )),
            Err(e) => Err(engine_error("The engine did not show the run's image.", e)),
        }
    }

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

    async fn collect(&self, spec: CollectSpec) -> AppResult<CollectManifest> {
        let docker = self.client(&spec.engine_socket)?;
        if !spec.live {
            self.remove_previews(&docker, &spec.installation, &spec.run_id)
                .await?;
        }
        if !spec.live
            && !self
                .run_containers(&docker, &spec.installation, &spec.run_id, false)
                .await?
                .is_empty()
        {
            return Err(super::conflict("The run is still running; stop it first."));
        }
        let image = self
            .resolve_image(&docker, &spec.image, spec.fallback_image.as_deref())
            .await?;
        // A running run's workspace is attached and mounted: the engine
        // mounts a volume once and gives each container its own read-only
        // view of it. Making it again would pull it from under the agent.
        if !spec.live {
            self.ensure_workspace(
                &docker,
                &image,
                &spec.installation,
                &spec.run_id,
                &spec.volume,
            )
            .await?;
        }
        let spec = CollectSpec { image, ..spec };
        let id = self.create_collector(&docker, &spec).await?;
        let stopped = spec.cancel.as_ref().is_some_and(|c| *c.borrow());
        let collected = if stopped {
            Err(super::conflict(
                "The run started stopping while its changes were being read.",
            ))
        } else {
            self.run_collector(&docker, &id, &spec).await
        };
        // The collector's container is the collection's own; it goes
        // whatever happened. The run's container and volume stay.
        let removed = docker
            .remove_container(
                &id,
                Some(RemoveContainerOptionsBuilder::new().force(true).build()),
            )
            .await;
        if let Err(e) = removed {
            tracing::warn!(error = %e, "the collector's container was not removed");
        }
        collected
    }

    async fn discard(
        &self,
        socket: &str,
        installation: &str,
        run_id: &str,
        volume: &str,
        image: &str,
        fallback_image: Option<&str>,
    ) -> AppResult<()> {
        let docker = self.client(socket)?;
        let image = self.resolve_image(&docker, image, fallback_image).await?;
        self.remove_previews(&docker, installation, run_id).await?;
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
        // Only the run's own volume: its name is the run's, and its labels
        // say so. A volume already gone (an earlier Discard that failed
        // after this step) still leaves the file and its device to remove.
        let owned = match docker.inspect_volume(volume).await {
            Ok(v) => Some(
                v.labels.get(LABEL_RUN).map(String::as_str) == Some(run_id)
                    && v.labels.get(LABEL_INSTALLATION).map(String::as_str) == Some(installation),
            ),
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404, ..
            }) => None,
            Err(e) => {
                return Err(engine_error(
                    "The engine did not show the run's workspace.",
                    e,
                ))
            }
        };
        if owned == Some(false) {
            return Err(super::conflict(
                "A workspace with the run's name belongs to something else; it was left alone.",
            ));
        }
        if owned.is_some() {
            match docker
                .remove_volume(volume, Some(RemoveVolumeOptionsBuilder::new().build()))
                .await
            {
                Ok(())
                | Err(bollard::errors::Error::DockerResponseServerError {
                    status_code: 404, ..
                }) => {}
                Err(e) => {
                    return Err(engine_error(
                        "The engine did not remove the run's workspace.",
                        e,
                    ))
                }
            }
        }
        self.remove_workspace_file(&docker, &image, installation, run_id)
            .await
    }

    async fn probe(
        &self,
        socket: &str,
        image: Option<&str>,
    ) -> AppResult<super::protocol::EngineReport> {
        let docker = self.client(socket)?;
        let info = docker
            .info()
            .await
            .map_err(|e| engine_error("The engine did not answer.", e))?;
        let name = info
            .operating_system
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Docker".into());
        let linux = !name.to_ascii_lowercase().contains("windows");
        let mut loop_devices = false;
        if let Some(image) = image {
            self.ensure_store(&docker, "probe").await?;
            if super::protocol::valid_id("probe") {
                let script = "losetup -f";
                match self.helper(&docker, image, "probe", "probe", script).await {
                    Ok(device) if device.starts_with("/dev/loop") => loop_devices = true,
                    Ok(_) => {}
                    Err(_) => {}
                }
            }
        }
        let supported = linux && (image.is_none() || loop_devices);
        let problem = if !linux {
            Some("Runs on a remote host need a Linux Docker engine.".into())
        } else if image.is_some() && !loop_devices {
            Some("This engine cannot attach the workspace's loop devices.".into())
        } else {
            None
        };
        Ok(super::protocol::EngineReport {
            name,
            supported,
            problem,
            loop_devices,
        })
    }

    async fn build_image(&self, socket: &str, tag: &str, context: Vec<u8>) -> AppResult<String> {
        use futures_util::StreamExt;
        let docker = self.client(socket)?;
        let options = bollard::query_parameters::BuildImageOptionsBuilder::default()
            .dockerfile("Dockerfile")
            .t(tag)
            .pull("true")
            .rm(true)
            .forcerm(true)
            .labels(&std::collections::HashMap::from([(
                "org.brainiac.role".to_string(),
                "image".to_string(),
            )]))
            .build();
        let body = bollard::body_stream(futures_util::stream::iter(vec![bytes::Bytes::from(
            context,
        )]));
        let mut stream = docker.build_image(options, None, Some(body));
        let mut id = None;
        let mut error = None;
        while let Some(item) = stream.next().await {
            match item {
                Ok(info) => {
                    if let Some(err) = info.error.filter(|e| !e.is_empty()) {
                        error = Some(err);
                    }
                    if let Some(aux) = info.aux {
                        if let Some(found) = aux.id {
                            id = Some(found);
                        }
                    }
                }
                Err(e) => return Err(engine_error("The image could not be built.", e)),
            }
        }
        if let Some(error) = error {
            return Err(AppError::io("The image could not be built.").with_details(error));
        }
        id.ok_or_else(|| AppError::io("The engine built the image without an ID."))
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

fn helper_failed(exit: i64) -> AppError {
    AppError::dependency(format!(
        "Brainiac's workspace helper failed (exit status {exit}). The engine may not support loop devices."
    ))
}

/// Engine errors keep the engine's words in the details, never the message.
fn engine_error(message: &str, error: bollard::errors::Error) -> AppError {
    AppError::dependency(message).with_details(error.to_string())
}

/// Copy the bundle (and, for the collector, its parameters) into the created
/// container's `/opt/brainiac/input` as a tar, the bundle read from disk a
/// piece at a time.
async fn upload_input(
    docker: &Docker,
    id: &str,
    bundle: &std::path::Path,
    extra: Option<(&str, Vec<u8>)>,
) -> AppResult<()> {
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
    let padding = (512 - (size % 512) as usize) % 512;
    let mut trailer = vec![0u8; padding];
    if let Some((name, bytes)) = extra {
        trailer.extend_from_slice(&crate::agents::image::header(name, bytes.len()));
        let pad = (512 - bytes.len() % 512) % 512;
        trailer.extend_from_slice(&bytes);
        trailer.extend(std::iter::repeat_n(0u8, pad));
    }
    trailer.extend(std::iter::repeat_n(0u8, 1024));
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
            Ok(Bytes::from(trailer))
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
