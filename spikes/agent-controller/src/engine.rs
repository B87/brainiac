//! Docker Engine API for one non-TTY attach. The CLI is not used: a TTY
//! and the daemon's default json-file log driver are easy to turn on by accident.

use std::collections::HashMap;
use std::pin::Pin;

use bollard::container::LogOutput;
use bollard::query_parameters::{
    AttachContainerOptionsBuilder, CreateContainerOptionsBuilder, ListContainersOptionsBuilder,
    RemoveContainerOptionsBuilder, RemoveVolumeOptionsBuilder, StopContainerOptionsBuilder,
};
use bollard::Docker;
use bytes::Bytes;
use futures_util::StreamExt;
use tokio::io::{AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;

use crate::state::{CLAUDE_IMAGE, IMAGE, LABEL_KEY, LABEL_VALUE, RUN_LABEL};

/// Collected entries are bounded so a hostile workspace cannot fill the
/// controller's memory through the collector.
const COLLECT_MAX_LINES: usize = 10_000;

/// Read-only walk of the workspace. It runs Python from the stub image, not
/// the workload's entrypoint or Git. Small UTF-8 files carry their text so
/// the Mac can compare it with what the agent reported.
const COLLECTOR: &str = r#"
import hashlib, json, os
root = "/workspace"
for dirpath, dirnames, filenames in os.walk(root, followlinks=False):
    dirnames.sort()
    if ".git" in dirnames:
        dirnames.remove(".git")
    for name in sorted(filenames):
        path = os.path.join(dirpath, name)
        rel = os.path.relpath(path, root)
        if os.path.islink(path):
            print(json.dumps({"path": rel, "kind": "symlink", "target": os.readlink(path)}))
            continue
        if not os.path.isfile(path):
            print(json.dumps({"path": rel, "kind": "skip"}))
            continue
        data = open(path, "rb").read()
        entry = {"path": rel, "kind": "file", "size": len(data), "sha256": hashlib.sha256(data).hexdigest()}
        if len(data) <= 65536:
            try:
                entry["text"] = data.decode("utf-8")
            except UnicodeDecodeError:
                pass
        print(json.dumps(entry))
print(json.dumps({"kind": "end"}))
"#;

/// The volume that holds one run's `/workspace`. It outlives the container,
/// so stopping or losing the workload does not lose the agent's files.
pub fn work_volume(run_id: &str) -> String {
    format!("brainiac-spike-work-{run_id}")
}

pub struct Attachment {
    pub container_id: String,
    pub tty: bool,
    pub log_driver: String,
    pub lines: mpsc::UnboundedReceiver<Line>,
    pub stdin: mpsc::UnboundedSender<Vec<u8>>,
}

pub enum Line {
    Stdout(String),
    Closed {
        stderr_bytes: u64,
        stderr_tail: String,
    },
}

struct Launch {
    image: &'static str,
    network_mode: Option<String>,
    memory: i64,
    pids: i64,
    user: Option<String>,
    working_dir: Option<String>,
    cap_drop: Option<Vec<String>>,
    tmpfs: Option<HashMap<String, String>>,
}

/// `Docker` is cheap to clone: it shares the HTTP client. Callers clone it
/// so a lock around the journal is not held while a request is in flight.
#[derive(Clone)]
pub struct Engine {
    docker: Docker,
}

impl Engine {
    pub fn connect() -> anyhow::Result<Self> {
        // `connect_with_socket_defaults` always uses `/var/run/docker.sock` and
        // ignores `DOCKER_HOST`. On a Mac that socket is whichever engine last
        // claimed it, so the local checks pass the engine's own socket here.
        let docker = Docker::connect_with_defaults()
            .map_err(|err| anyhow::anyhow!("Docker socket: {err}"))?;
        Ok(Self { docker })
    }

    /// Operating system and storage driver, with no socket path.
    pub async fn describe(&self) -> String {
        match self.docker.info().await {
            Ok(info) => format!(
                "{} {}",
                info.operating_system.unwrap_or_default(),
                info.driver.unwrap_or_default()
            ),
            Err(_) => "unknown".into(),
        }
    }

    /// Stop every workload container and keep it. A stopped container and
    /// its volume are the run's work until the user collects or discards it.
    /// `t(2)` is the grace period before Docker sends SIGKILL.
    pub async fn stop_labeled(&self) -> anyhow::Result<usize> {
        let ids = self.labeled_ids(false).await?;
        let count = ids.len();
        for id in ids {
            let stop = StopContainerOptionsBuilder::new().t(2).build();
            let _ = self.docker.stop_container(&id, Some(stop)).await;
        }
        Ok(count)
    }

    /// `false` only when the engine reports the container stopped or gone.
    pub async fn is_running(&self, container_id: &str) -> anyhow::Result<bool> {
        match self
            .docker
            .inspect_container(
                container_id,
                None::<bollard::query_parameters::InspectContainerOptions>,
            )
            .await
        {
            Ok(inspected) => Ok(inspected
                .state
                .and_then(|state| state.running)
                .unwrap_or(true)),
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404, ..
            }) => Ok(false),
            Err(err) => anyhow::bail!("inspect: {err}"),
        }
    }

    /// `true` while any workload container exists, running or stopped.
    pub async fn any_labeled(&self) -> anyhow::Result<bool> {
        Ok(!self.labeled_ids(true).await?.is_empty())
    }

    /// Remove one run's container and its workspace volume. Only an explicit
    /// discard, or a start that failed before the agent ran, calls this.
    pub async fn remove_run(&self, container_id: &str, volume: &str) -> anyhow::Result<()> {
        let remove = RemoveContainerOptionsBuilder::new().force(true).build();
        match self
            .docker
            .remove_container(container_id, Some(remove))
            .await
        {
            Ok(()) => {}
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404, ..
            }) => {}
            Err(err) => anyhow::bail!("remove container: {err}"),
        }
        let remove = RemoveVolumeOptionsBuilder::new().force(true).build();
        match self.docker.remove_volume(volume, Some(remove)).await {
            Ok(()) => Ok(()),
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404, ..
            }) => Ok(()),
            Err(err) => anyhow::bail!("remove volume: {err}"),
        }
    }

    /// Run the collector against a stopped run's volume: read-only mount, no
    /// network, no capabilities, no log driver. Returns one JSON line per
    /// entry. The workload container is not started again.
    pub async fn collect(&self, volume: &str) -> anyhow::Result<Vec<String>> {
        let mut labels = HashMap::new();
        labels.insert("brainiac.spike.collector".to_string(), "1".to_string());
        let host = bollard::models::HostConfig {
            log_config: Some(bollard::models::HostConfigLogConfig {
                typ: Some("none".to_string()),
                ..Default::default()
            }),
            network_mode: Some("none".to_string()),
            cap_drop: Some(vec!["ALL".to_string()]),
            security_opt: Some(vec!["no-new-privileges:true".to_string()]),
            privileged: Some(false),
            readonly_rootfs: Some(true),
            memory: Some(256 * 1024 * 1024),
            pids_limit: Some(32),
            mounts: Some(vec![bollard::models::Mount {
                target: Some("/workspace".to_string()),
                source: Some(volume.to_string()),
                typ: Some(bollard::models::MountTypeEnum::VOLUME),
                read_only: Some(true),
                ..Default::default()
            }]),
            ..Default::default()
        };
        let body = bollard::models::ContainerCreateBody {
            image: Some(IMAGE.to_string()),
            entrypoint: Some(vec!["python3".to_string()]),
            cmd: Some(vec!["-c".to_string(), COLLECTOR.to_string()]),
            user: Some("nobody".to_string()),
            tty: Some(false),
            attach_stdout: Some(true),
            attach_stderr: Some(true),
            labels: Some(labels),
            host_config: Some(host),
            ..Default::default()
        };
        let created = self
            .docker
            .create_container(
                None::<bollard::query_parameters::CreateContainerOptions>,
                body,
            )
            .await
            .map_err(|err| anyhow::anyhow!("create collector: {err}"))?;
        let id = created.id;
        let result = self.run_collector(&id).await;
        let remove = RemoveContainerOptionsBuilder::new().force(true).build();
        let _ = self.docker.remove_container(&id, Some(remove)).await;
        result
    }

    async fn run_collector(&self, id: &str) -> anyhow::Result<Vec<String>> {
        // Attach before start so no output is missed; the log driver is none.
        let attach = AttachContainerOptionsBuilder::new()
            .stream(true)
            .stdout(true)
            .stderr(true)
            .logs(false)
            .build();
        let attached = self
            .docker
            .attach_container(id, Some(attach))
            .await
            .map_err(|err| anyhow::anyhow!("attach collector: {err}"))?;
        self.docker
            .start_container(id, None::<bollard::query_parameters::StartContainerOptions>)
            .await
            .map_err(|err| anyhow::anyhow!("start collector: {err}"))?;
        let mut output = attached.output;
        let mut buf = Vec::new();
        let mut lines = Vec::new();
        let read = async {
            while let Some(item) = output.next().await {
                let Ok(LogOutput::StdOut { message }) = item else {
                    continue;
                };
                buf.extend_from_slice(&message);
                while let Some(pos) = buf.iter().position(|byte| *byte == b'\n') {
                    let line: Vec<u8> = buf.drain(..=pos).collect();
                    lines.push(String::from_utf8_lossy(&line[..line.len() - 1]).into_owned());
                    if lines.len() > COLLECT_MAX_LINES {
                        anyhow::bail!("collector output is over {COLLECT_MAX_LINES} entries");
                    }
                }
            }
            Ok(())
        };
        tokio::time::timeout(std::time::Duration::from_secs(60), read)
            .await
            .map_err(|_| anyhow::anyhow!("collector timed out"))??;
        // The last line proves the walk finished rather than being cut off.
        if lines.last().map(String::as_str) != Some(r#"{"kind": "end"}"#) {
            anyhow::bail!("collector did not finish its walk");
        }
        lines.pop();
        Ok(lines)
    }

    async fn labeled_ids(&self, all: bool) -> anyhow::Result<Vec<String>> {
        let mut filters = HashMap::new();
        filters.insert(
            "label".to_string(),
            vec![format!("{LABEL_KEY}={LABEL_VALUE}")],
        );
        let options = ListContainersOptionsBuilder::new()
            .all(all)
            .filters(&filters)
            .build();
        let listed = self
            .docker
            .list_containers(Some(options))
            .await
            .map_err(|err| anyhow::anyhow!("list containers: {err}"))?;
        Ok(listed.into_iter().filter_map(|item| item.id).collect())
    }

    pub async fn start_stub(&self, run_id: &str) -> anyhow::Result<Attachment> {
        self.start(
            run_id,
            Launch {
                image: IMAGE,
                network_mode: Some("none".to_string()),
                memory: 128 * 1024 * 1024,
                pids: 64,
                user: None,
                working_dir: None,
                cap_drop: Some(vec!["ALL".to_string()]),
                tmpfs: None,
            },
        )
        .await
    }

    /// Claude needs outbound HTTPS. Nothing from the Mac is mounted.
    /// Home and `/tmp` are tmpfs so login files do not land on the host disk.
    pub async fn start_claude(&self, run_id: &str) -> anyhow::Result<Attachment> {
        let mut tmpfs = HashMap::new();
        tmpfs.insert(
            "/home/node".to_string(),
            "rw,nosuid,nodev,size=64m,uid=1000,gid=1000,mode=0700".to_string(),
        );
        tmpfs.insert(
            "/tmp".to_string(),
            "rw,nosuid,nodev,size=256m,mode=1777".to_string(),
        );
        self.start(
            run_id,
            Launch {
                image: CLAUDE_IMAGE,
                network_mode: None,
                memory: 3 * 1024 * 1024 * 1024,
                pids: 512,
                user: Some("node".to_string()),
                working_dir: Some("/workspace".to_string()),
                cap_drop: None,
                tmpfs: Some(tmpfs),
            },
        )
        .await
    }

    /// `/workspace` is a fresh named volume for this run. Nothing from the
    /// host is mounted. A previous run's container blocks the start: its work
    /// stays until it is collected or discarded, never replaced silently.
    async fn start(&self, run_id: &str, launch: Launch) -> anyhow::Result<Attachment> {
        if self.any_labeled().await? {
            anyhow::bail!("a previous run's work is kept; collect or discard it first");
        }
        let mut labels = HashMap::new();
        labels.insert(LABEL_KEY.to_string(), LABEL_VALUE.to_string());
        labels.insert(RUN_LABEL.to_string(), run_id.to_string());
        let volume = work_volume(run_id);
        self.docker
            .create_volume(bollard::models::VolumeCreateOptions {
                name: Some(volume.clone()),
                labels: Some(labels.clone()),
                ..Default::default()
            })
            .await
            .map_err(|err| anyhow::anyhow!("create workspace volume: {err}"))?;
        let host = bollard::models::HostConfig {
            log_config: Some(bollard::models::HostConfigLogConfig {
                typ: Some("none".to_string()),
                ..Default::default()
            }),
            network_mode: launch.network_mode,
            cap_drop: launch.cap_drop,
            security_opt: Some(vec!["no-new-privileges:true".to_string()]),
            privileged: Some(false),
            memory: Some(launch.memory),
            pids_limit: Some(launch.pids),
            tmpfs: launch.tmpfs,
            mounts: Some(vec![bollard::models::Mount {
                target: Some("/workspace".to_string()),
                source: Some(volume.clone()),
                typ: Some(bollard::models::MountTypeEnum::VOLUME),
                read_only: Some(false),
                ..Default::default()
            }]),
            restart_policy: Some(bollard::models::RestartPolicy {
                name: Some(bollard::models::RestartPolicyNameEnum::NO),
                maximum_retry_count: None,
            }),
            ..Default::default()
        };
        let body = bollard::models::ContainerCreateBody {
            image: Some(launch.image.to_string()),
            // Keep stdin open after this attach drops, so controller death
            // does not silently end the process. The guard has to stop it.
            open_stdin: Some(true),
            stdin_once: Some(false),
            tty: Some(false),
            attach_stdin: Some(true),
            attach_stdout: Some(true),
            attach_stderr: Some(true),
            user: launch.user,
            working_dir: launch.working_dir,
            labels: Some(labels),
            host_config: Some(host),
            ..Default::default()
        };
        let image = launch.image;
        let created = self
            .docker
            .create_container(
                Some(
                    CreateContainerOptionsBuilder::new()
                        .name(&format!("brainiac-spike-agent-{run_id}"))
                        .build(),
                ),
                body,
            )
            .await;
        let created = match created {
            Ok(created) => created,
            Err(err) => {
                let remove = RemoveVolumeOptionsBuilder::new().force(true).build();
                let _ = self.docker.remove_volume(&volume, Some(remove)).await;
                anyhow::bail!("create container (is {image} built?): {err}");
            }
        };
        let id = created.id;
        if let Err(err) = self
            .docker
            .start_container(
                &id,
                None::<bollard::query_parameters::StartContainerOptions>,
            )
            .await
        {
            let _ = self.remove_run(&id, &volume).await;
            return Err(anyhow::anyhow!("start container: {err}"));
        }
        let inspected = self
            .docker
            .inspect_container(
                &id,
                None::<bollard::query_parameters::InspectContainerOptions>,
            )
            .await
            .map_err(|err| anyhow::anyhow!("inspect container: {err}"))?;
        let tty = inspected
            .config
            .and_then(|config| config.tty)
            .unwrap_or(true);
        let log_driver = inspected
            .host_config
            .and_then(|host| host.log_config)
            .and_then(|log| log.typ)
            .unwrap_or_else(|| "unknown".to_string());
        let attach = AttachContainerOptionsBuilder::new()
            .stream(true)
            .stdin(true)
            .stdout(true)
            .stderr(true)
            .logs(false)
            .build();
        let attached = self
            .docker
            .attach_container(&id, Some(attach))
            .await
            .map_err(|err| anyhow::anyhow!("attach: {err}"))?;
        let (line_tx, lines) = mpsc::unbounded_channel();
        let (stdin_tx, stdin_rx) = mpsc::unbounded_channel();
        tokio::spawn(read_output(attached.output, line_tx));
        tokio::spawn(write_stdin(attached.input, stdin_rx));
        Ok(Attachment {
            container_id: id,
            tty,
            log_driver,
            lines,
            stdin: stdin_tx,
        })
    }
}

async fn read_output(
    mut output: Pin<
        Box<dyn futures_util::Stream<Item = Result<LogOutput, bollard::errors::Error>> + Send>,
    >,
    tx: mpsc::UnboundedSender<Line>,
) {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut stderr_bytes = 0u64;
    while let Some(item) = output.next().await {
        let Ok(item) = item else {
            break;
        };
        match item {
            LogOutput::StdOut { message } => push_lines(&mut stdout, &message, &tx),
            LogOutput::StdErr { message } => {
                stderr_bytes = stderr_bytes.saturating_add(message.len() as u64);
                push_tail(&mut stderr, &message);
            }
            LogOutput::Console { message } => push_lines(&mut stdout, &message, &tx),
            LogOutput::StdIn { .. } => {}
        }
    }
    let _ = tx.send(Line::Closed {
        stderr_bytes,
        stderr_tail: String::from_utf8_lossy(&stderr).into_owned(),
    });
}

fn push_tail(buf: &mut Vec<u8>, chunk: &Bytes) {
    buf.extend_from_slice(chunk);
    if buf.len() > 4096 {
        let extra = buf.len() - 4096;
        buf.drain(..extra);
    }
}

fn push_lines(buf: &mut Vec<u8>, chunk: &Bytes, tx: &mpsc::UnboundedSender<Line>) {
    buf.extend_from_slice(chunk);
    while let Some(pos) = buf.iter().position(|byte| *byte == b'\n') {
        let line: Vec<u8> = buf.drain(..=pos).collect();
        let text = String::from_utf8_lossy(&line[..line.len().saturating_sub(1)]);
        if tx.send(Line::Stdout(text.to_string())).is_err() {
            return;
        }
    }
    if buf.len() > 1024 * 1024 {
        buf.clear();
    }
}

async fn write_stdin(
    mut input: Pin<Box<dyn AsyncWrite + Send>>,
    mut rx: mpsc::UnboundedReceiver<Vec<u8>>,
) {
    while let Some(chunk) = rx.recv().await {
        if input.write_all(&chunk).await.is_err() {
            break;
        }
        if input.flush().await.is_err() {
            break;
        }
    }
}
