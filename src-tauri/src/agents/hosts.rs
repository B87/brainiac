//! Remote run hosts (SPEC.md, Remote hosts): the run host role on an
//! approved machine (`crate::machines`), which owns the connection and its
//! key. Deploy installs the Linux run controller there as a systemd
//! service; the app reconnects to it and does not stop a healthy run by
//! disconnecting.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rusqlite::{params, Connection, OptionalExtension};

use super::controller::protocol::{Phase, RunStatus};
use super::host_jobs::{self, install, JobProgress};
use super::image;
use super::runner_binary;
use super::runtime::RunRuntime;
use crate::db::Db;
use crate::machines::ssh::{self, Target};
use crate::machines::{self, Approval, Machine, MachineService};
use crate::models::{
    now_rfc3339, AgentHost, AgentHostPreview, AgentImage, AppError, AppResult,
    ApproveAgentHostRequest, ErrorCode,
};

pub const LOCAL_ID: &str = "local";
const LOCAL_NAME: &str = "This Mac";
/// The group the controller's socket belongs to on a host.
const SERVICE_GROUP: &str = "brainiac";
const DOCKER_SOCKET: &str = "/var/run/docker.sock";
/// Where the program is copied before its digest is checked and installed.
const UPLOAD: &str = "/tmp/brainiac-runner.upload";
/// How often an upgrade that waits for a live run looks again.
const LIVE_POLL: std::time::Duration = std::time::Duration::from_secs(10);

const UNIT: &str = r#"[Unit]
Description=Brainiac run controller
After=docker.service
Requires=docker.service

[Service]
User=brainiac
Group=brainiac
SupplementaryGroups=docker
ExecStart=/usr/local/bin/brainiac-runner --service --state /var/lib/brainiac-runner --socket /var/lib/brainiac-runner/runner.sock
Restart=on-failure
NoNewPrivileges=true

[Install]
WantedBy=multi-user.target
"#;

/// What Deploy tells the user it will do, before it does it.
pub fn install_actions() -> Vec<String> {
    vec![
        "Create the user brainiac, in the docker group, if it does not exist.".into(),
        "Add the SSH user to the brainiac group, so it can reach the controller.".into(),
        "Install /usr/local/bin/brainiac-runner and check its digest.".into(),
        "Install and start brainiac-runner.service.".into(),
    ]
}

/// A live run blocks upgrade and removal.
pub fn live_run(phases: &[Phase]) -> bool {
    phases.iter().any(|p| *p != Phase::Ended)
}

/// Removal keeps the state directory while work or a cleanup is still there.
pub fn keep_state(kept: bool, cleanup: bool) -> bool {
    kept || cleanup
}

pub struct AgentHostService {
    db: Db,
    history: Db,
    data_dir: PathBuf,
    machines: Arc<MachineService>,
    /// Hosts with a Settings test in progress. That run has no history row,
    /// and Upgrade and Remove must still see it.
    live_tests: Mutex<HashSet<String>>,
    /// Hosts whose controller is being installed or restarted. A new run
    /// or test is refused until that finishes.
    deploying: Mutex<HashSet<String>>,
}

impl AgentHostService {
    pub fn new(db: Db, history: Db, data_dir: &Path, machines: Arc<MachineService>) -> Self {
        Self {
            db,
            history,
            data_dir: data_dir.to_path_buf(),
            machines,
            live_tests: Mutex::new(HashSet::new()),
            deploying: Mutex::new(HashSet::new()),
        }
    }

    /// True while Deploy or Upgrade is replacing this host's controller.
    pub(crate) fn is_deploying(&self, id: &str) -> bool {
        self.deploying
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .contains(id)
    }

    fn begin_deploy(&self, id: &str) -> DeployGuard<'_> {
        self.deploying
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(id.to_string());
        DeployGuard {
            hosts: self,
            id: id.to_string(),
        }
    }

    /// Installed, approved SSH hosts. Launch uses this to stop a Settings
    /// test that was still running when Brainiac quit.
    pub async fn installed_ids(&self) -> AppResult<Vec<String>> {
        self.db
            .call(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT h.id FROM agent_hosts h JOIN machines m ON m.id = h.machine_id
                     WHERE m.approved != 0 AND h.installation IS NOT NULL",
                )?;
                let ids = stmt.query_map([], |row| row.get(0))?;
                Ok(ids.collect::<rusqlite::Result<Vec<String>>>()?)
            })
            .await
    }

    /// Marks a Settings test until the returned lease is dropped.
    pub(crate) fn begin_test(self: &Arc<Self>, id: &str) -> TestLease {
        self.live_tests
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(id.to_string());
        TestLease {
            hosts: Arc::clone(self),
            id: id.to_string(),
        }
    }

    pub async fn preview(&self, host: &str, port: u16) -> AppResult<AgentHostPreview> {
        Ok(AgentHostPreview {
            fingerprint: self.machines.fingerprint(host, port).await?,
            actions: install_actions(),
        })
    }

    /// Approve the machine, then make it a run host. Confirming the key of
    /// a saved machine keeps its run host and everything installed there.
    pub async fn approve(&self, request: ApproveAgentHostRequest) -> AppResult<AgentHost> {
        let machine = self
            .machines
            .approve(Approval {
                name: request.name,
                user: request.user,
                host: request.host,
                port: request.port,
                identity_path: request.identity_path,
                fingerprint: request.fingerprint,
                accept_changed_key: request.accept_changed_key,
            })
            .await?;
        let id = self
            .db
            .call(move |conn| {
                let now = now_rfc3339();
                conn.execute(
                    "INSERT INTO agent_hosts (id, kind, machine_id, created_at, updated_at)
                     VALUES (?1, 'ssh', ?2, ?3, ?3)
                     ON CONFLICT(machine_id) DO UPDATE SET version = version + 1, updated_at = ?3",
                    params![uuid::Uuid::new_v4().simple().to_string(), machine.id, now],
                )?;
                Ok(conn.query_row(
                    "SELECT id FROM agent_hosts WHERE machine_id = ?1",
                    [&machine.id],
                    |r| r.get::<_, String>(0),
                )?)
            })
            .await?;
        self.host(&id).await
    }

    /// Install or Upgrade (SPEC.md, Remote hosts), as the six steps of
    /// `host_jobs::install`, numbered from `at` in `job`.
    pub async fn deploy(&self, id: &str, job: &JobProgress, at: usize) -> AppResult<AgentHost> {
        use install::*;
        let row = self.require(id).await?;
        if !row.approved() {
            return Err(AppError::validation(
                "Confirm this host before installing on it.",
            ));
        }
        let name = row.name();
        // A service that is already running keeps its old process across
        // `enable --now`. Replacing its program has to restart it.
        let restart = row.installation.is_some();
        let target = self.ssh_target(&row)?;

        job.begin(at + REACH);
        ssh::run("ssh", &ssh::ssh_args(&target, "sudo -n true")).await?;
        let uname = ssh::run("ssh", &ssh::ssh_args(&target, "uname -m")).await?;
        let uname = uname.trim().to_string();
        let platform = runner_binary::platform(&uname)?;
        job.finish(
            at + REACH,
            Some(format!(
                "SSH as {}, sudo without a password, an {uname} processor",
                target.user
            )),
        );
        // Held until this function returns, so a run or a test cannot start
        // on this host while its controller is being replaced. It is taken
        // before the build, so an upgrade the user chose is not overtaken by
        // a run started while it compiles.
        let _deploying = self.begin_deploy(id);

        job.retitle(
            at + BUILD,
            format!("Build the run controller for {platform}"),
            if runner_binary::emulated(&uname) {
                format!("On this Mac, in Docker, imitating an {uname} processor. The first build after a Brainiac update takes many minutes.")
            } else {
                "On this Mac, in Docker. The first build after a Brainiac update takes a few minutes.".to_string()
            },
        );
        job.begin(at + BUILD);
        let mut compiled = 0usize;
        let progress = job.clone();
        let mut on_line = move |line: &str| {
            progress.line(line);
            if runner_binary::compiled_crate(line).is_some() {
                compiled += 1;
                progress.progress(
                    at + BUILD,
                    format!(
                        "{compiled} {} compiled",
                        if compiled == 1 { "crate" } else { "crates" }
                    ),
                );
            }
        };
        let built =
            runner_binary::ensure(&self.cache_dir(), &uname, &mut on_line, job.cancel_signal())
                .await?;
        job.finish(
            at + BUILD,
            Some(if built.reused {
                "Already built by this Brainiac: reused".to_string()
            } else {
                "Built on this Mac, in Docker, and kept for the next host".to_string()
            }),
        );
        let binary = built.path;
        let digest = runner_binary::file_digest(&binary)?;
        job.check_cancelled()?;

        job.begin(at + COPY);
        ssh::run("scp", &ssh::scp_args(&target, &binary, UPLOAD)).await?;
        let uploaded = ssh::run(
            "ssh",
            &ssh::ssh_args(&target, &format!("sha256sum {UPLOAD}")),
        )
        .await?;
        if !digest_matches(&uploaded, &digest) {
            remove_upload(&target).await;
            return Err(AppError::dependency(format!(
                "The copy on {name} did not match the program Brainiac built. The copy was deleted; nothing was installed or restarted."
            ))
            .with_details(format!(
                "expected {digest}  brainiac-runner\n{name}: {}",
                uploaded.trim()
            )));
        }
        let size = std::fs::metadata(&binary).map(|m| m.len()).unwrap_or(0);
        job.finish(
            at + COPY,
            Some(format!(
                "{:.1} MB · SHA-256 {}… matches",
                size as f64 / 1_000_000.0,
                &digest[..12]
            )),
        );
        if job.cancelled() {
            remove_upload(&target).await;
            return Err(host_jobs::cancelled());
        }

        job.begin(at + WAIT);
        if restart {
            if let Err(e) = self.wait_for_no_live_run(id, &name, job, at + WAIT).await {
                remove_upload(&target).await;
                return Err(e);
            }
            job.finish(at + WAIT, Some(format!("No run is live on {name}")));
        } else {
            job.finish(at + WAIT, Some("No controller there yet".to_string()));
        }

        // From here the host changes: the job finishes on its own.
        job.close_cancel();
        job.begin(at + INSTALL);
        let user = group_command(
            &row.machine
                .as_ref()
                .map(|m| m.user.clone())
                .unwrap_or_default(),
        )?;
        for command in [
            "id brainiac >/dev/null 2>&1 || sudo -n useradd --system --create-home --shell /usr/sbin/nologin brainiac",
            "getent group docker >/dev/null && sudo -n usermod -aG docker brainiac || true",
            user.as_str(),
            "sudo -n mkdir -p /var/lib/brainiac-runner && sudo -n chown brainiac:brainiac /var/lib/brainiac-runner && sudo -n chmod 750 /var/lib/brainiac-runner",
        ] {
            ssh::run("ssh", &ssh::ssh_args(&target, command)).await?;
        }
        ssh::run(
            "ssh",
            &ssh::ssh_args(
                &target,
                &format!("sudo -n install -o root -g root -m 755 {UPLOAD} /usr/local/bin/brainiac-runner && rm -f {UPLOAD}"),
            ),
        )
        .await?;
        let installed = ssh::run(
            "ssh",
            &ssh::ssh_args(&target, "sha256sum /usr/local/bin/brainiac-runner"),
        )
        .await?;
        if !digest_matches(&installed, &digest) {
            let _ = ssh::run(
                "ssh",
                &ssh::ssh_args(&target, "sudo -n rm -f /usr/local/bin/brainiac-runner"),
            )
            .await;
            return Err(AppError::dependency(format!(
                "The program installed on {name} did not match the one Brainiac built, so it was removed."
            ))
            .with_details(format!(
                "expected {digest}  brainiac-runner\n{name}: {}",
                installed.trim()
            )));
        }
        // The unit is a constant. It is written on the host, not assembled
        // from the host name.
        write_remote_file(
            &target,
            "/etc/systemd/system/brainiac-runner.service",
            UNIT.as_bytes(),
        )
        .await?;
        // A run that started in the moment before this job took the host
        // is still there, and must not be restarted away.
        self.refuse_live(id).await?;
        ssh::run("ssh", &ssh::ssh_args(&target, service_command(restart))).await?;
        job.finish(
            at + INSTALL,
            Some(if restart {
                "Installed, and the service restarted".to_string()
            } else {
                "Installed, and the service started".to_string()
            }),
        );

        job.begin(at + CHECK);
        let token = ssh::run(
            "ssh",
            &ssh::ssh_args(&target, "sudo -n cat /var/lib/brainiac-runner/token"),
        )
        .await?;
        let token = token.trim().to_string();
        if token.is_empty() || token.contains('\n') {
            return Err(AppError::dependency(
                "The host's run controller did not create its token.",
            ));
        }
        self.write_token(id, &token)?;
        let runtime = self.runtime_for(&row, &token, "")?;
        let info = runtime.running().await.ok_or_else(|| {
            AppError::dependency("The host's run controller did not answer after install.")
        })?;
        let installation = info.installation;
        let protocol = info.protocol as i64;
        let build = runner_binary::current_build();
        job.finish(
            at + CHECK,
            Some(format!(
                "Build {} · protocol {protocol}",
                build
                    .as_deref()
                    .map(runner_binary::short_build)
                    .unwrap_or_else(|| "unknown".into())
            )),
        );
        let id = id.to_string();
        self.db
            .call(move |conn| {
                let now = now_rfc3339();
                conn.execute(
                    "UPDATE agent_hosts SET installation = ?2, protocol = ?3, controller_build = ?4,
                        controller_installed_at = ?5, version = version + 1, updated_at = ?5
                     WHERE id = ?1",
                    params![id, installation, protocol, build, now],
                )?;
                Ok(())
            })
            .await?;
        self.host(&row.id).await
    }

    /// Upgrade waits while a run is live, checking every few seconds, and
    /// stops waiting when the job is cancelled.
    async fn wait_for_no_live_run(
        &self,
        id: &str,
        name: &str,
        job: &JobProgress,
        index: usize,
    ) -> AppResult<()> {
        let mut cancel = job.cancel_signal();
        loop {
            match self.refuse_live(id).await {
                Ok(()) => return Ok(()),
                Err(e) if e.code == ErrorCode::Conflict => {
                    job.progress(
                        index,
                        format!("A run is live on {name}. The upgrade goes on when it ends; new runs there wait for it."),
                    );
                }
                Err(e) => return Err(e),
            }
            job.check_cancelled()?;
            tokio::select! {
                _ = tokio::time::sleep(LIVE_POLL) => {}
                _ = cancel.changed() => {}
            }
            job.check_cancelled()?;
        }
    }

    pub async fn remove(&self, id: &str) -> AppResult<()> {
        if id == LOCAL_ID {
            return Err(AppError::validation("This Mac is not removed."));
        }
        self.refuse_live(id).await?;
        let obligations = self.obligations(id).await?;
        let row = self.require(id).await?;
        if row.installation.is_some() {
            let target = self.ssh_target(&row)?;
            ssh::run(
                "ssh",
                &ssh::ssh_args(
                    &target,
                    "sudo -n systemctl disable --now brainiac-runner.service",
                ),
            )
            .await?;
            ssh::run(
                "ssh",
                &ssh::ssh_args(&target, "sudo -n rm -f /usr/local/bin/brainiac-runner"),
            )
            .await?;
            if !keep_state(obligations.kept, obligations.cleanup) {
                ssh::run(
                    "ssh",
                    &ssh::ssh_args(&target, "sudo -n rm -rf /var/lib/brainiac-runner"),
                )
                .await?;
            }
        }
        let machine = row.machine.as_ref().map(|m| m.id.clone());
        let id = id.to_string();
        if keep_state(obligations.kept, obligations.cleanup) {
            // The run host stays, with its machine no longer trusted, until
            // the user adds the machine again and collects or discards.
            let kept = id.clone();
            self.db
                .call(move |conn| {
                    conn.execute(
                        "UPDATE agent_hosts SET state_kept = 1, version = version + 1,
                            updated_at = ?2 WHERE id = ?1",
                        params![kept, now_rfc3339()],
                    )?;
                    Ok(())
                })
                .await?;
            if let Some(machine) = &machine {
                self.machines.withdraw(machine).await?;
            }
        } else {
            let gone = id.clone();
            self.db
                .call(move |conn| {
                    conn.execute(
                        "DELETE FROM agent_hosts WHERE id = ?1 AND kind = 'ssh'",
                        [&gone],
                    )?;
                    Ok(())
                })
                .await?;
            let _ = std::fs::remove_dir_all(self.host_dir(&id));
            // The run host was the machine's only role.
            if let Some(machine) = &machine {
                self.machines.forget(machine).await?;
            }
        }
        Ok(())
    }

    /// Build image on a host, as the two steps of `host_jobs::image`,
    /// numbered from `at` in `job`.
    pub async fn build_image(
        &self,
        id: &str,
        job: &JobProgress,
        at: usize,
    ) -> AppResult<AgentHost> {
        let row = self.require_ready(id).await?;
        job.begin(at + host_jobs::image::BUILD);
        let runtime = self.connected(&row).await?;
        let tag = image::name_for(&image::recipe());
        let built = runtime
            .build_image_remote(DOCKER_SOCKET, &tag, &image::context())
            .await?;
        job.finish(
            at + host_jobs::image::BUILD,
            Some(format!("{tag} · {}", short_id(&built))),
        );
        job.begin(at + host_jobs::image::PROBE);
        let recipe = image::recipe();
        let now = now_rfc3339();
        let host_id = id.to_string();
        self.db
            .call(move |conn| {
                conn.execute(
                    "UPDATE agent_hosts SET image_id = ?2, image_recipe = ?3, image_built_at = ?4,
                        version = version + 1, updated_at = ?4 WHERE id = ?1",
                    params![host_id, built, recipe, now],
                )?;
                Ok(())
            })
            .await?;
        let row = self.require(id).await?;
        let runtime = self.connected(&row).await?;
        let report = runtime.probe_engine(DOCKER_SOCKET, Some(tag)).await?;
        let id = row.id.clone();
        let engine = report.name.clone();
        let loops = report.loop_devices;
        self.db
            .call(move |conn| {
                conn.execute(
                    "UPDATE agent_hosts SET engine_name = ?2, loop_devices = ?3,
                        version = version + 1, updated_at = ?4 WHERE id = ?1",
                    params![id, report.name, report.loop_devices, now_rfc3339()],
                )?;
                Ok(())
            })
            .await?;
        if !loops {
            return Err(AppError::dependency(
                "This host's Docker engine cannot attach loop devices, so it cannot take a run.",
            ));
        }
        job.finish(
            at + host_jobs::image::PROBE,
            Some(format!("{engine} · loop devices work")),
        );
        self.host(&row.id).await
    }

    pub async fn record_test(&self, id: &str, credential_revision: i64) -> AppResult<()> {
        let now = now_rfc3339();
        let id = id.to_string();
        self.db
            .call(move |conn| {
                conn.execute(
                    "UPDATE agent_hosts SET test_passed_at = ?2, test_credential_revision = ?3,
                        test_image_id = image_id, version = version + 1, updated_at = ?2
                     WHERE id = ?1",
                    params![id, now, credential_revision],
                )?;
                Ok(())
            })
            .await
    }

    pub async fn runtime(&self, id: &str) -> AppResult<Arc<RunRuntime>> {
        if id.is_empty() || id == LOCAL_ID {
            return Err(AppError::validation(
                "This Mac uses the local run controller.",
            ));
        }
        let row = self.require_ready(id).await?;
        Ok(Arc::new(self.connected(&row).await?))
    }

    async fn connected(&self, row: &HostRow) -> AppResult<RunRuntime> {
        let token = self.read_token(&row.id)?;
        let installation = row.installation.clone().unwrap_or_default();
        self.runtime_for(row, &token, &installation)
    }

    fn runtime_for(&self, row: &HostRow, token: &str, installation: &str) -> AppResult<RunRuntime> {
        let target = self.ssh_target(row)?;
        Ok(RunRuntime::remote(
            target,
            token.to_string(),
            installation.to_string(),
            super::runtime::forward_socket(&self.host_dir(&row.id), &row.id),
        ))
    }

    fn ssh_target(&self, row: &HostRow) -> AppResult<Target> {
        let machine = row
            .machine
            .as_ref()
            .ok_or_else(|| AppError::validation("This Mac uses the local run controller."))?;
        self.machines.target(machine)
    }

    pub(crate) async fn host(&self, id: &str) -> AppResult<AgentHost> {
        let id = id.to_string();
        self.db
            .call(move |conn| {
                list(conn)?
                    .into_iter()
                    .find(|h| h.id == id)
                    .ok_or_else(|| AppError::not_found("That host is not saved."))
            })
            .await
    }

    async fn require(&self, id: &str) -> AppResult<HostRow> {
        let id = id.to_string();
        self.db
            .call(move |conn| {
                load_host(conn, &id)?.ok_or_else(|| AppError::not_found("That host is not saved."))
            })
            .await
    }

    async fn require_ready(&self, id: &str) -> AppResult<HostRow> {
        let row = self.require(id).await?;
        if !row.approved() || row.installation.is_none() {
            return Err(AppError::validation("Deploy this host before using it."));
        }
        Ok(row)
    }

    async fn refuse_live(&self, id: &str) -> AppResult<()> {
        if self
            .live_tests
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .contains(id)
        {
            return Err(live_conflict());
        }
        let host_id = id.to_string();
        let live = self
            .history
            .call(move |conn| {
                let n: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM agent_runs WHERE host_id = ?1 AND phase != 'ended'",
                    [&host_id],
                    |r| r.get(0),
                )?;
                Ok(n > 0)
            })
            .await?;
        if live {
            return Err(live_conflict());
        }
        let row = self.require(id).await?;
        if row.installation.is_some() {
            let runtime = self.connected(&row).await.map_err(|_| {
                AppError::dependency(
                    "This host's run controller did not answer, so Brainiac cannot tell whether a run is still live.",
                )
            })?;
            let runs = runtime.runs().await?;
            if live_run(&phases_of(&runs)) {
                return Err(live_conflict());
            }
        }
        Ok(())
    }

    async fn obligations(&self, id: &str) -> AppResult<Obligations> {
        let id = id.to_string();
        self.history
            .call(move |conn| {
                let (kept, cleanup): (i64, i64) = conn.query_row(
                    "SELECT COALESCE(SUM(kept), 0), COALESCE(SUM(cleanup_pending IS NOT NULL), 0)
                     FROM agent_runs WHERE host_id = ?1",
                    [&id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?;
                Ok(Obligations {
                    kept: kept > 0,
                    cleanup: cleanup > 0,
                })
            })
            .await
    }

    fn host_dir(&self, id: &str) -> PathBuf {
        self.data_dir.join("agent-runs").join("hosts").join(id)
    }

    fn cache_dir(&self) -> PathBuf {
        self.data_dir.join("agent-runs").join("runner-cache")
    }

    fn write_token(&self, id: &str, token: &str) -> AppResult<()> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let dir = self.host_dir(id);
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("token");
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        writeln!(file, "{token}")?;
        Ok(())
    }

    fn read_token(&self, id: &str) -> AppResult<String> {
        let text = std::fs::read_to_string(self.host_dir(id).join("token"))?;
        let token = text.trim().to_string();
        if token.is_empty() {
            return Err(AppError::dependency(
                "Deploy this host again: its controller token is missing.",
            ));
        }
        Ok(token)
    }
}

struct Obligations {
    kept: bool,
    cleanup: bool,
}

struct HostRow {
    id: String,
    installation: Option<String>,
    /// The machine a remote host runs on; `None` for this Mac.
    machine: Option<Machine>,
}

impl HostRow {
    fn name(&self) -> String {
        self.machine
            .as_ref()
            .map(|m| m.name.clone())
            .unwrap_or_else(|| LOCAL_NAME.to_string())
    }

    /// A remote host is usable while its machine is approved.
    fn approved(&self) -> bool {
        self.machine.as_ref().is_none_or(|m| m.approved)
    }
}

/// Add the SSH user to the controller's group. The name must pass
/// [`ssh::valid_user`], so it is one token and not a shell expression.
fn group_command(user: &str) -> AppResult<String> {
    if !ssh::valid_user(user) {
        return Err(AppError::validation(
            "The SSH user is not a name Brainiac can use.",
        ));
    }
    Ok(format!("sudo -n usermod -aG {SERVICE_GROUP} {user}"))
}

/// Deletes a copy that was not installed. Best effort: it is in `/tmp`.
async fn remove_upload(target: &Target) {
    let _ = ssh::run("ssh", &ssh::ssh_args(target, &format!("rm -f {UPLOAD}"))).await;
}

/// `sha256:4d1e…` → `4d1e…`, short enough for a line of Settings.
fn short_id(id: &str) -> String {
    let id = id.strip_prefix("sha256:").unwrap_or(id);
    format!("{}…", id.chars().take(12).collect::<String>())
}

/// `sha256sum` output matches the digest of the binary Brainiac built.
fn digest_matches(reported: &str, expected: &str) -> bool {
    let reported = reported.split_whitespace().next().unwrap_or("");
    !expected.is_empty() && reported == expected
}

/// First install starts the service. A later install restarts it, so the
/// running process is the binary just checked.
fn service_command(restart: bool) -> &'static str {
    if restart {
        "sudo -n systemctl daemon-reload && sudo -n systemctl enable brainiac-runner.service && sudo -n systemctl restart brainiac-runner.service"
    } else {
        "sudo -n systemctl daemon-reload && sudo -n systemctl enable --now brainiac-runner.service"
    }
}

fn live_conflict() -> AppError {
    AppError::new(
        crate::models::ErrorCode::Conflict,
        "This host has a live run. Stop or collect it first.",
    )
}

/// Held for the whole of a Settings test. `Drop` clears it when that
/// function ends, including when the test fails.
pub(crate) struct TestLease {
    hosts: Arc<AgentHostService>,
    id: String,
}

impl Drop for TestLease {
    fn drop(&mut self) {
        self.hosts
            .live_tests
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&self.id);
    }
}

/// Held for the whole of Deploy or Upgrade. `Drop` clears it when that
/// function ends, including when the install fails.
struct DeployGuard<'a> {
    hosts: &'a AgentHostService,
    id: String,
}

impl Drop for DeployGuard<'_> {
    fn drop(&mut self) {
        self.hosts
            .deploying
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&self.id);
    }
}

fn load_host(conn: &Connection, id: &str) -> AppResult<Option<HostRow>> {
    let row = conn
        .query_row(
            "SELECT id, installation, machine_id FROM agent_hosts WHERE id = ?1",
            [id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .optional()?;
    let Some((id, installation, machine_id)) = row else {
        return Ok(None);
    };
    let machine = match machine_id {
        Some(machine_id) => Some(
            machines::load(conn, &machine_id)?
                .ok_or_else(|| AppError::db("A host's machine is missing."))?,
        ),
        None => None,
    };
    Ok(Some(HostRow {
        id,
        installation,
        machine,
    }))
}

pub fn list(conn: &Connection) -> AppResult<Vec<AgentHost>> {
    let revision: i64 = conn
        .query_row(
            "SELECT credential_revision FROM agent_profiles WHERE id = ?1",
            [super::settings::PROFILE_ID],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(0);
    let mut stmt = conn.prepare(
        "SELECT h.id, h.kind, COALESCE(m.name, ?1), m.ssh_user, m.ssh_host, m.ssh_port,
            m.identity_path, m.fingerprint, h.installation, COALESCE(m.approved, 1),
            h.engine_name, h.loop_devices, h.image_id, h.image_recipe, h.image_built_at,
            h.test_passed_at, h.test_image_id, h.state_kept, h.version,
            h.test_credential_revision, h.controller_build, h.protocol,
            h.controller_installed_at
         FROM agent_hosts h LEFT JOIN machines m ON m.id = h.machine_id
         ORDER BY h.kind, 3",
    )?;
    let recipe = image::recipe();
    let available = runner_binary::current_build();
    let rows = stmt.query_map([LOCAL_NAME], |r| {
        let kind: String = r.get(1)?;
        let user: Option<String> = r.get(3)?;
        let host: Option<String> = r.get(4)?;
        let port: Option<i64> = r.get(5)?;
        let image_id: Option<String> = r.get(12)?;
        let image_recipe: Option<String> = r.get(13)?;
        let built_at: Option<String> = r.get(14)?;
        let test_image: Option<String> = r.get(16)?;
        let test_passed: Option<String> = r.get(15)?;
        let test_revision: Option<i64> = r.get(19)?;
        let controller_build: Option<String> = r.get(20)?;
        let installed = r.get::<_, Option<String>>(8)?.is_some();
        let kind_is_ssh = kind == "ssh";
        let image = match (image_id.clone(), built_at) {
            (Some(id), Some(built_at)) => Some(AgentImage {
                name: image::name_for(image_recipe.as_deref().unwrap_or("")),
                id,
                built_at,
                current: image_recipe.as_deref() == Some(recipe.as_str()),
            }),
            _ => None,
        };
        let emergency = if kind == "ssh" {
            match (user, host, port) {
                (Some(user), Some(host), Some(port)) => Some(format!(
                    "ssh -p {port} {user}@{host} sudo -n -u brainiac /usr/local/bin/brainiac-runner emergency-stop --state /var/lib/brainiac-runner"
                )),
                _ => None,
            }
        } else {
            None
        };
        Ok(AgentHost {
            id: r.get(0)?,
            kind,
            name: r.get(2)?,
            ssh_user: r.get(3)?,
            ssh_host: r.get(4)?,
            ssh_port: r.get::<_, Option<i64>>(5)?.map(|p| p as u16),
            identity_path: r.get(6)?,
            fingerprint: r.get(7)?,
            approved: r.get::<_, i64>(9)? != 0,
            installed,
            engine_name: r.get(10)?,
            loop_devices: r.get::<_, i64>(11)? != 0,
            // A passed test counts only for the credential and image it used.
            test_current: test_passed.is_some()
                && test_revision == Some(revision)
                && test_image.is_some()
                && test_image == image_id,
            image,
            emergency_stop: emergency,
            state_kept: r.get::<_, i64>(17)? != 0,
            controller_build: controller_build.as_deref().map(runner_binary::short_build),
            protocol: r.get::<_, Option<i64>>(21)?.map(|p| p as u32),
            controller_installed_at: r.get(22)?,
            upgrade_available: upgrade_available(installed, controller_build.as_deref(), available.as_deref()),
            available_build: if kind_is_ssh {
                available.as_deref().map(runner_binary::short_build)
            } else {
                None
            },
            version: r.get(18)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Send `bytes` to a fixed path on the host. The path is not user input.
async fn write_remote_file(target: &Target, path: &str, bytes: &[u8]) -> AppResult<()> {
    use tokio::io::AsyncWriteExt;
    let args = ssh::ssh_args(target, &format!("sudo -n tee {path} >/dev/null"));
    let mut child = tokio::process::Command::new("ssh")
        .args(&args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| {
            AppError::dependency("SSH could not be started.").with_details(e.to_string())
        })?;
    if let Some(stdin) = child.stdin.as_mut() {
        stdin.write_all(bytes).await?;
    }
    let out = child.wait_with_output().await?;
    if !out.status.success() {
        return Err(
            AppError::dependency("The host did not accept the service file.")
                .with_details(String::from_utf8_lossy(&out.stderr).to_string()),
        );
    }
    Ok(())
}

/// An installed controller is behind when this Brainiac builds another one.
/// One installed before builds were recorded counts as behind; a Brainiac
/// that cannot build one offers nothing.
fn upgrade_available(installed: bool, build: Option<&str>, available: Option<&str>) -> bool {
    match (installed, available) {
        (true, Some(available)) => build != Some(available),
        _ => false,
    }
}

/// Status phases from a controller, for the upgrade check the service also
/// makes against history. Exposed so a test can pass phases without a host.
pub fn phases_of(runs: &[RunStatus]) -> Vec<Phase> {
    runs.iter().map(|r| r.phase).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upgrade_and_remove_refuse_a_live_run() {
        assert!(live_run(&[Phase::Running]));
        assert!(live_run(&[Phase::Ended, Phase::Stopping]));
        assert!(!live_run(&[Phase::Ended, Phase::Ended]));
        assert!(!live_run(&[]));
    }

    #[test]
    fn remove_keeps_the_state_directory_while_cleanup_is_pending() {
        assert!(keep_state(false, true));
        assert!(keep_state(true, false));
        assert!(!keep_state(false, false));
    }

    #[test]
    fn the_group_command_refuses_a_user_that_is_not_a_name() {
        assert!(group_command("ada")
            .unwrap()
            .contains("usermod -aG brainiac ada"));
        assert!(group_command("ada;id").is_err());
    }

    #[test]
    fn a_digest_mismatch_does_not_count_as_the_built_program() {
        assert!(digest_matches("abc  /tmp/brainiac-runner.upload", "abc"));
        assert!(!digest_matches(
            "def  /usr/local/bin/brainiac-runner",
            "abc"
        ));
        assert!(!digest_matches("abc  file", ""));
    }

    #[test]
    fn an_upgrade_is_offered_for_another_build() {
        assert!(upgrade_available(true, Some("a1"), Some("b2")));
        assert!(upgrade_available(true, None, Some("b2")));
        assert!(!upgrade_available(true, Some("b2"), Some("b2")));
        assert!(!upgrade_available(false, None, Some("b2")));
        assert!(!upgrade_available(true, Some("a1"), None));
    }

    #[test]
    fn an_installed_service_is_restarted() {
        assert!(service_command(true).contains("systemctl restart"));
        assert!(!service_command(false).contains("systemctl restart"));
    }
}
