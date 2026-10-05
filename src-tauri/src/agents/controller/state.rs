//! The controller's folder, `agent-runs/controller/` in the data folder,
//! outside any container and closed to other users:
//!
//! - `token`: what a client must present, mode 600.
//! - `installation`: this installation's ID, on every container's labels.
//! - `controller.lock`: held by the one running controller.
//! - `controller.pid`: the guard watches it.
//! - `runs/<run-id>/run.json`, `journal.jsonl`, `commands.json`: each run's
//!   record, its journal, and its commands' outcomes.

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::protocol::{Activity, Delivery, Outcome, Phase, RunStatus};
use crate::models::RunPermissions;

#[derive(Clone)]
pub struct StateDir {
    root: PathBuf,
}

impl StateDir {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Create the folder, readable only by this user.
    pub fn ensure(&self) -> io::Result<()> {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(self.runs_dir())?;
        fs::set_permissions(&self.root, fs::Permissions::from_mode(0o700))
    }

    pub fn runs_dir(&self) -> PathBuf {
        self.root.join("runs")
    }

    pub fn run_dir(&self, run_id: &str) -> PathBuf {
        self.runs_dir().join(run_id)
    }

    pub fn pid_path(&self) -> PathBuf {
        self.root.join("controller.pid")
    }

    pub fn log_path(&self) -> PathBuf {
        self.root.join("runner.log")
    }

    /// The token, created on first use. Refused when others can read it.
    pub fn token(&self) -> io::Result<String> {
        self.secret_file("token")
    }

    /// The installation's ID, created on first use. A restore does not bring
    /// it back: it lives here, not in the database.
    pub fn installation(&self) -> io::Result<String> {
        self.secret_file("installation")
    }

    fn secret_file(&self, name: &str) -> io::Result<String> {
        let path = self.root.join(name);
        if !path.exists() {
            // Written in full under another name, then linked into place:
            // a crash never leaves an empty file, and of two processes
            // creating it at once, one wins and both read the winner's.
            let tmp = self.root.join(format!("{name}.{}.tmp", std::process::id()));
            let value = uuid::Uuid::new_v4().simple().to_string();
            let mut file = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&tmp)?;
            file.write_all(value.as_bytes())?;
            file.sync_all()?;
            let linked = fs::hard_link(&tmp, &path);
            let _ = fs::remove_file(&tmp);
            match linked {
                Ok(()) => return Ok(value),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e),
            }
        }
        let meta = fs::symlink_metadata(&path)?;
        if !meta.is_file() || meta.permissions().mode() & 0o077 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("{name} is not a private file"),
            ));
        }
        let value = fs::read_to_string(&path)?.trim().to_string();
        if value.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{name} is empty"),
            ));
        }
        Ok(value)
    }

    /// Hold `controller.lock` for as long as the returned file is open. `None`
    /// when another controller holds it.
    pub fn lock(&self) -> io::Result<Option<File>> {
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(self.root.join("controller.lock"))?;
        use std::os::fd::AsRawFd;
        // `unsafe` because every libc call is, to Rust; `flock` only reads the
        // descriptor, which `file` keeps open.
        let locked = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0;
        Ok(locked.then_some(file))
    }

    pub fn write_pid(&self) -> io::Result<()> {
        write_atomic(&self.pid_path(), std::process::id().to_string().as_bytes())
    }

    pub fn controller_pid(&self) -> Option<u32> {
        fs::read_to_string(self.pid_path())
            .ok()?
            .trim()
            .parse()
            .ok()
    }

    /// Every run's ID with a folder here.
    pub fn run_ids(&self) -> Vec<String> {
        let Ok(entries) = fs::read_dir(self.runs_dir()) else {
            return Vec::new();
        };
        let mut ids: Vec<String> = entries
            .filter_map(|e| e.ok())
            .filter(|e| e.path().join("run.json").is_file())
            .filter_map(|e| e.file_name().into_string().ok())
            .collect();
        ids.sort();
        ids
    }

    pub fn load_run(&self, run_id: &str) -> Option<RunRecord> {
        let text = fs::read(self.run_dir(run_id).join("run.json")).ok()?;
        serde_json::from_slice(&text).ok()
    }
}

/// Replace a file's contents in one step: a crash leaves the old or the new.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)?;
    file.write_all(bytes)?;
    file.sync_data()?;
    fs::rename(tmp, path)
}

/// What the controller keeps about a run, written before each side effect
/// and after each result, so a new controller knows what an old one did.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunRecord {
    pub run_id: String,
    pub attempt: u32,
    pub engine_socket: String,
    pub image: String,
    pub permissions: RunPermissions,
    /// The commit the run started from, for the snapshot's parent.
    #[serde(default)]
    pub start_commit: String,
    /// The run's input bundle, given to the collector again.
    #[serde(default)]
    pub bundle: PathBuf,
    #[serde(default = "default_workspace_gib")]
    pub workspace_gib: u32,
    pub phase: Phase,
    pub outcome: Option<Outcome>,
    pub stop_confirmed: bool,
    /// The container or the volume may exist on the engine.
    pub kept: bool,
    pub container_id: Option<String>,
    pub volume: String,
    pub session_id: Option<String>,
    pub turn: u32,
    pub accepted_at: String,
    pub deadline_at: String,
    pub time_limit_secs: u64,
    pub ended_at: Option<String>,
    pub expired_asleep: bool,
    pub error: Option<String>,
}

fn default_workspace_gib() -> u32 {
    20
}

impl RunRecord {
    pub fn save(&self, dir: &Path) -> io::Result<()> {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
        let bytes = serde_json::to_vec_pretty(self).map_err(io::Error::other)?;
        write_atomic(&dir.join("run.json"), &bytes)
    }

    /// The run as the protocol shows it, without what only a live run knows.
    pub fn status(&self, cursor: u64) -> RunStatus {
        RunStatus {
            run_id: self.run_id.clone(),
            attempt: self.attempt,
            phase: self.phase,
            activity: match self.phase {
                Phase::Preparing => Activity::Preparing,
                Phase::Running => Activity::Idle,
                Phase::Stopping => Activity::Stopping,
                Phase::Ended => Activity::Ended,
            },
            turn: self.turn,
            outcome: self.outcome,
            stop_confirmed: self.stop_confirmed,
            kept: self.kept,
            permissions: Vec::new(),
            session_id: self.session_id.clone(),
            accepted_at: self.accepted_at.clone(),
            deadline_at: self.deadline_at.clone(),
            ended_at: self.ended_at.clone(),
            expired_asleep: self.expired_asleep,
            cursor,
            error: self.error.clone(),
        }
    }
}

/// Command IDs and their outcomes. The intent is stored before anything is
/// written to the agent and the outcome before the client is told, so a
/// repeated command gets the first answer and is never written twice; an
/// intent with no outcome is uncertain and is never retried.
pub struct Ledger {
    path: PathBuf,
    commands: BTreeMap<String, Option<Delivery>>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Begin {
    /// Not seen before: it may be written to the agent once.
    Fresh,
    /// Seen before, with this outcome.
    Seen(Delivery),
}

impl Ledger {
    pub fn open(run_dir: &Path) -> io::Result<Self> {
        let path = run_dir.join("commands.json");
        let commands = match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(io::Error::other)?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => return Err(e),
        };
        Ok(Self { path, commands })
    }

    pub fn outcome(&self, id: &str) -> Option<Delivery> {
        self.commands
            .get(id)
            .map(|outcome| outcome.unwrap_or(Delivery::Uncertain))
    }

    pub fn begin(&mut self, id: &str) -> io::Result<Begin> {
        if let Some(outcome) = self.outcome(id) {
            return Ok(Begin::Seen(outcome));
        }
        self.commands.insert(id.to_string(), None);
        self.store()?;
        Ok(Begin::Fresh)
    }

    pub fn finish(&mut self, id: &str, outcome: Delivery) -> io::Result<()> {
        self.commands.insert(id.to_string(), Some(outcome));
        self.store()
    }

    /// Forget a command that was refused before anything was written: the
    /// same ID may be tried again once the run allows it.
    pub fn forget(&mut self, id: &str) -> io::Result<()> {
        self.commands.remove(id);
        self.store()
    }

    fn store(&self) -> io::Result<()> {
        let bytes = serde_json::to_vec(&self.commands).map_err(io::Error::other)?;
        write_atomic(&self.path, &bytes)
    }
}

/// `true` while a process with this ID exists.
pub fn pid_alive(pid: u32) -> bool {
    let Ok(pid) = i32::try_from(pid) else {
        return false;
    };
    // `kill` with signal 0 only checks that the process exists. EPERM means
    // it exists under another user, which is still alive.
    // `unsafe` because every libc call is, to Rust.
    let result = unsafe { libc::kill(pid, 0) };
    result == 0 || io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_token_is_created_once_and_private() {
        let dir = tempfile::tempdir().unwrap();
        let state = StateDir::new(dir.path().join("controller"));
        state.ensure().unwrap();
        let token = state.token().unwrap();
        assert_eq!(state.token().unwrap(), token);
        let mode = fs::metadata(dir.path().join("controller/token"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        fs::set_permissions(
            dir.path().join("controller/token"),
            fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        assert!(state.token().is_err());
    }

    #[test]
    fn a_command_is_fresh_once_and_uncertain_without_an_outcome() {
        let dir = tempfile::tempdir().unwrap();
        let mut ledger = Ledger::open(dir.path()).unwrap();
        assert_eq!(ledger.begin("c1").unwrap(), Begin::Fresh);
        assert_eq!(ledger.begin("c2").unwrap(), Begin::Fresh);
        ledger.finish("c1", Delivery::Delivered).unwrap();
        drop(ledger);
        let mut ledger = Ledger::open(dir.path()).unwrap();
        assert_eq!(
            ledger.begin("c1").unwrap(),
            Begin::Seen(Delivery::Delivered)
        );
        assert_eq!(
            ledger.begin("c2").unwrap(),
            Begin::Seen(Delivery::Uncertain)
        );
    }

    #[test]
    fn one_controller_holds_the_lock() {
        let dir = tempfile::tempdir().unwrap();
        let state = StateDir::new(dir.path());
        state.ensure().unwrap();
        let held = state.lock().unwrap();
        assert!(held.is_some());
        // flock is per open file, so a second open in this process is refused too.
        assert!(state.lock().unwrap().is_none());
        drop(held);
        assert!(state.lock().unwrap().is_some());
    }

    #[test]
    fn this_process_is_alive_and_a_finished_one_is_not() {
        assert!(pid_alive(std::process::id()));
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();
        assert!(!pid_alive(pid));
    }
}
