//! Approved remote hosts (SPEC.md, Remote hosts). SSH is the transport.
//! Deploy installs the Linux run controller as a systemd service; the app
//! reconnects to it and does not stop a healthy run by disconnecting.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rusqlite::{params, Connection, OptionalExtension};

use super::controller::protocol::{Phase, RunStatus};
use super::image;
use super::runner_binary;
use super::runtime::RunRuntime;
use super::ssh::{self, Target};
use crate::db::Db;
use crate::models::{
    now_rfc3339, AgentHost, AgentHostPreview, AgentImage, AppError, AppResult,
    ApproveAgentHostRequest,
};

pub const LOCAL_ID: &str = "local";
const DOCKER_SOCKET: &str = "/var/run/docker.sock";

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
    /// Hosts with a Settings test in progress. That run has no history row,
    /// and Upgrade and Remove must still see it.
    live_tests: Mutex<HashSet<String>>,
    /// Hosts whose controller is being installed or restarted. A new run
    /// or test is refused until that finishes.
    deploying: Mutex<HashSet<String>>,
}

impl AgentHostService {
    pub fn new(db: Db, history: Db, data_dir: &Path) -> Self {
        Self {
            db,
            history,
            data_dir: data_dir.to_path_buf(),
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
                    "SELECT id FROM agent_hosts
                     WHERE kind = 'ssh' AND approved != 0 AND installation IS NOT NULL",
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
        let prints = ssh::fingerprints(host, port).await?;
        Ok(AgentHostPreview {
            fingerprint: prints
                .into_iter()
                .next()
                .ok_or_else(|| AppError::dependency("The host did not present an SSH key."))?,
            actions: install_actions(),
        })
    }

    pub async fn approve(&self, request: ApproveAgentHostRequest) -> AppResult<AgentHost> {
        let target = ssh::target(
            &request.user,
            &request.host,
            request.port,
            request.identity_path.as_deref(),
            PathBuf::from("/dev/null"),
        )?;
        let lines = ssh::keyscan(&target.host, target.port).await?;
        let prints = fingerprints_of(&lines)?;
        // Only the key the user was shown. Other types from the same scan
        // stay untrusted, even when they arrived beside the confirmed one.
        let lines = lines_matching(&lines, &prints, &request.fingerprint);
        if lines.is_empty() {
            return Err(AppError::validation(
                "The host key changed since it was shown. Look again before approving it.",
            ));
        }
        let existing = self
            .db
            .call({
                let user = target.user.clone();
                let host = target.host.clone();
                let port = target.port;
                move |conn| find_host(conn, &user, &host, port)
            })
            .await?;
        if let Some(row) = &existing {
            if row.fingerprint.as_deref() != Some(request.fingerprint.as_str())
                && !request.accept_changed_key
            {
                return Err(AppError::validation(
                    "This host's key changed. Approve the new fingerprint to continue.",
                ));
            }
        }
        let id = existing
            .as_ref()
            .map(|r| r.id.clone())
            .unwrap_or_else(|| uuid::Uuid::new_v4().simple().to_string());
        let known = self.host_dir(&id).join("known_hosts");
        ssh::write_known_hosts(&known, &lines)?;
        let name = {
            let trimmed = request.name.trim();
            if trimmed.is_empty() {
                target.host.clone()
            } else {
                trimmed.to_string()
            }
        };
        let now = now_rfc3339();
        let saved = self
            .db
            .call(move |conn| {
                save_host(
                    conn,
                    &SavedHost {
                        id: &id,
                        name: &name,
                        user: &request.user,
                        host: &request.host,
                        port: request.port,
                        identity: request.identity_path.as_deref(),
                        fingerprint: &request.fingerprint,
                        now: &now,
                    },
                )
            })
            .await?;
        Ok(saved)
    }

    pub async fn deploy(&self, id: &str) -> AppResult<AgentHost> {
        let row = self.require(id).await?;
        if !row.approved {
            return Err(AppError::validation(
                "Confirm this host before deploying to it.",
            ));
        }
        self.refuse_live(id).await?;
        // Held until this function returns, so a run cannot start while the
        // binary is built and the service is about to restart.
        let _deploying = self.begin_deploy(id);
        // A service that is already running keeps its old process across
        // `enable --now`. Replacing its program has to restart it.
        let restart = row.installation.is_some();
        let target = self.ssh_target(&row)?;
        ssh::run("ssh", &ssh::ssh_args(&target, "sudo -n true")).await?;
        let uname = ssh::run("ssh", &ssh::ssh_args(&target, "uname -m")).await?;
        let binary = runner_binary::ensure(&self.cache_dir(), &uname).await?;
        let digest = runner_binary::file_digest(&binary)?;
        let user = ssh::group_command(&row.ssh_user.clone().unwrap_or_default())?;
        for command in [
            "id brainiac >/dev/null 2>&1 || sudo -n useradd --system --create-home --shell /usr/sbin/nologin brainiac",
            "getent group docker >/dev/null && sudo -n usermod -aG docker brainiac || true",
            user.as_str(),
            "sudo -n mkdir -p /var/lib/brainiac-runner && sudo -n chown brainiac:brainiac /var/lib/brainiac-runner && sudo -n chmod 750 /var/lib/brainiac-runner",
        ] {
            ssh::run("ssh", &ssh::ssh_args(&target, command)).await?;
        }
        ssh::run(
            "scp",
            &ssh::scp_args(&target, &binary, "/tmp/brainiac-runner.upload"),
        )
        .await?;
        let uploaded = ssh::run(
            "ssh",
            &ssh::ssh_args(&target, "sha256sum /tmp/brainiac-runner.upload"),
        )
        .await?;
        if !digest_matches(&uploaded, &digest) {
            let _ = ssh::run(
                "ssh",
                &ssh::ssh_args(&target, "rm -f /tmp/brainiac-runner.upload"),
            )
            .await;
            return Err(AppError::dependency(
                "The program on the host does not match the one Brainiac built.",
            ));
        }
        ssh::run(
            "ssh",
            &ssh::ssh_args(
                &target,
                "sudo -n install -o root -g root -m 755 /tmp/brainiac-runner.upload /usr/local/bin/brainiac-runner && rm -f /tmp/brainiac-runner.upload",
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
            return Err(AppError::dependency(
                "The program on the host does not match the one Brainiac built.",
            ));
        }
        // The unit is a constant. It is written on the host, not assembled
        // from the host name.
        write_remote_file(
            &target,
            "/etc/systemd/system/brainiac-runner.service",
            UNIT.as_bytes(),
        )
        .await?;
        // The build above can take minutes. A run that started before this
        // install began is still there, and must not be restarted away.
        self.refuse_live(id).await?;
        ssh::run("ssh", &ssh::ssh_args(&target, service_command(restart))).await?;
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
        let id = id.to_string();
        self.db
            .call(move |conn| {
                conn.execute(
                    "UPDATE agent_hosts SET installation = ?2, protocol = ?3, version = version + 1,
                        updated_at = ?4 WHERE id = ?1",
                    params![id, installation, protocol, now_rfc3339()],
                )?;
                Ok(())
            })
            .await?;
        self.host(&row.id).await
    }

    pub async fn upgrade(&self, id: &str) -> AppResult<AgentHost> {
        // Deploy restarts a service that is already installed, which
        // interrupts its sessions. A live run is refused first.
        self.deploy(id).await
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
        let id = id.to_string();
        if keep_state(obligations.kept, obligations.cleanup) {
            let id = id.clone();
            self.db
                .call(move |conn| {
                    conn.execute(
                        "UPDATE agent_hosts SET approved = 0, state_kept = 1, version = version + 1,
                            updated_at = ?2 WHERE id = ?1",
                        params![id, now_rfc3339()],
                    )?;
                    Ok(())
                })
                .await?;
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
        }
        Ok(())
    }

    pub async fn build_image(&self, id: &str) -> AppResult<AgentHost> {
        let row = self.require_ready(id).await?;
        let runtime = self.connected(&row).await?;
        let tag = image::name_for(&image::recipe());
        let built = runtime
            .build_image_remote(DOCKER_SOCKET, &tag, &image::context())
            .await?;
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
        if !report.loop_devices {
            return Err(AppError::dependency(
                "This host's Docker engine cannot attach loop devices, so it cannot take a run.",
            ));
        }
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
        ssh::target(
            row.ssh_user.as_deref().unwrap_or(""),
            row.ssh_host.as_deref().unwrap_or(""),
            row.ssh_port.unwrap_or(0),
            row.identity_path.as_deref(),
            self.host_dir(&row.id).join("known_hosts"),
        )
    }

    async fn host(&self, id: &str) -> AppResult<AgentHost> {
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
        if !row.approved || row.installation.is_none() {
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
    ssh_user: Option<String>,
    ssh_host: Option<String>,
    ssh_port: Option<u16>,
    identity_path: Option<String>,
    fingerprint: Option<String>,
    installation: Option<String>,
    approved: bool,
}

fn fingerprints_of(lines: &[String]) -> AppResult<Vec<String>> {
    lines.iter().map(|l| ssh::fingerprint(l)).collect()
}

/// The `ssh-keyscan` lines whose fingerprint is the one the user confirmed.
fn lines_matching(lines: &[String], prints: &[String], fingerprint: &str) -> Vec<String> {
    lines
        .iter()
        .zip(prints.iter())
        .filter(|(_, print)| ssh::same_fingerprint(fingerprint, print))
        .map(|(line, _)| line.clone())
        .collect()
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

fn find_host(conn: &Connection, user: &str, host: &str, port: u16) -> AppResult<Option<HostRow>> {
    Ok(conn
        .query_row(
            "SELECT id, ssh_user, ssh_host, ssh_port, identity_path, fingerprint, installation, approved
             FROM agent_hosts WHERE kind = 'ssh' AND ssh_user = ?1 AND ssh_host = ?2 AND ssh_port = ?3",
            params![user, host, port],
            row_of,
        )
        .optional()?)
}

fn load_host(conn: &Connection, id: &str) -> AppResult<Option<HostRow>> {
    Ok(conn
        .query_row(
            "SELECT id, ssh_user, ssh_host, ssh_port, identity_path, fingerprint, installation, approved
             FROM agent_hosts WHERE id = ?1",
            [id],
            row_of,
        )
        .optional()?)
}

fn row_of(r: &rusqlite::Row<'_>) -> rusqlite::Result<HostRow> {
    Ok(HostRow {
        id: r.get(0)?,
        ssh_user: r.get(1)?,
        ssh_host: r.get(2)?,
        ssh_port: r.get::<_, Option<i64>>(3)?.map(|p| p as u16),
        identity_path: r.get(4)?,
        fingerprint: r.get(5)?,
        installation: r.get(6)?,
        approved: r.get::<_, i64>(7)? != 0,
    })
}

struct SavedHost<'a> {
    id: &'a str,
    name: &'a str,
    user: &'a str,
    host: &'a str,
    port: u16,
    identity: Option<&'a str>,
    fingerprint: &'a str,
    now: &'a str,
}

fn save_host(conn: &Connection, saved: &SavedHost<'_>) -> AppResult<AgentHost> {
    conn.execute(
        "INSERT INTO agent_hosts (id, kind, name, ssh_user, ssh_host, ssh_port, identity_path,
            fingerprint, approved, created_at, updated_at)
         VALUES (?1, 'ssh', ?2, ?3, ?4, ?5, ?6, ?7, 1, ?8, ?8)
         ON CONFLICT(id) DO UPDATE SET name = ?2, ssh_user = ?3, ssh_host = ?4, ssh_port = ?5,
            identity_path = ?6, fingerprint = ?7, approved = 1, version = version + 1, updated_at = ?8",
        params![
            saved.id,
            saved.name,
            saved.user,
            saved.host,
            saved.port,
            saved.identity,
            saved.fingerprint,
            saved.now
        ],
    )?;
    list(conn)?
        .into_iter()
        .find(|h| h.id == saved.id)
        .ok_or_else(|| AppError::db("The host was not saved."))
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
        "SELECT id, kind, name, ssh_user, ssh_host, ssh_port, identity_path, fingerprint,
            installation, approved, engine_name, loop_devices, image_id, image_recipe,
            image_built_at, test_passed_at, test_image_id, state_kept, version,
            test_credential_revision
         FROM agent_hosts ORDER BY kind, name",
    )?;
    let recipe = image::recipe();
    let rows = stmt.query_map([], |r| {
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
            installed: r.get::<_, Option<String>>(8)?.is_some(),
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
    fn approval_keeps_only_the_confirmed_key() {
        let lines = vec!["ed25519-line".into(), "rsa-line".into()];
        let prints = vec!["SHA256:ed".into(), "SHA256:rsa".into()];
        assert_eq!(
            lines_matching(&lines, &prints, "SHA256:ed"),
            vec!["ed25519-line".to_string()]
        );
        assert!(lines_matching(&lines, &prints, "SHA256:other").is_empty());
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
    fn an_installed_service_is_restarted() {
        assert!(service_command(true).contains("systemctl restart"));
        assert!(!service_command(false).contains("systemctl restart"));
    }
}
