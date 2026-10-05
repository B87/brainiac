//! Files the controller and the guard share on the engine host.
//! Nothing here is a path on the Mac.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const PORT: u16 = 47321;
pub const DEFAULT_STATE: &str = "/var/lib/brainiac-spike";
pub const LABEL_KEY: &str = "brainiac.spike";
pub const LABEL_VALUE: &str = "1";
/// Names the run a container or workspace volume belongs to.
pub const RUN_LABEL: &str = "brainiac.spike.run";
pub const IMAGE: &str = "brainiac-spike-stub:local";
pub const CLAUDE_IMAGE: &str = "brainiac-spike-claude:local";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunRecord {
    pub run_id: String,
    pub container_id: String,
    pub tty: bool,
    pub log_driver: String,
    pub live: bool,
    pub interrupted: bool,
    /// The host deadline stopped this run. Absent on records written before
    /// that field existed.
    #[serde(default)]
    pub expired: bool,
    /// The run's workspace volume. It stays after the run ends, is stopped,
    /// or is interrupted, until an explicit discard clears this field.
    #[serde(default)]
    pub volume: Option<String>,
}

impl RunRecord {
    /// The run has ended and its stopped container and volume are kept.
    pub fn kept(&self) -> bool {
        !self.live && self.volume.is_some()
    }
}

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

    pub fn ensure(&self) -> io::Result<()> {
        fs::create_dir_all(&self.root)
    }

    pub fn token(&self) -> io::Result<String> {
        let path = self.root.join("token");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path)?.permissions().mode();
            if mode & 0o077 != 0 {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "token file is readable by other users",
                ));
            }
        }
        let text = fs::read_to_string(path)?;
        let token = text.trim().to_string();
        if token.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "token file is empty",
            ));
        }
        Ok(token)
    }

    pub fn pid_path(&self) -> PathBuf {
        self.root.join("controller.pid")
    }

    pub fn write_pid(&self) -> io::Result<()> {
        fs::write(self.pid_path(), std::process::id().to_string())
    }

    pub fn remove_pid(&self) -> io::Result<()> {
        match fs::remove_file(self.pid_path()) {
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
            other => other,
        }
    }

    pub fn controller_pid(&self) -> Option<u32> {
        fs::read_to_string(self.pid_path())
            .ok()
            .and_then(|text| text.trim().parse().ok())
    }

    pub fn journal_path(&self) -> PathBuf {
        self.root.join("journal.jsonl")
    }

    pub fn ledger_path(&self) -> PathBuf {
        self.root.join("ledger.json")
    }

    pub fn run_path(&self) -> PathBuf {
        self.root.join("run.json")
    }

    pub fn sock_path(&self) -> PathBuf {
        self.root.join("emergency.sock")
    }

    pub fn guard_log(&self) -> PathBuf {
        self.root.join("guard.log")
    }

    pub fn load_run(&self) -> Option<RunRecord> {
        let text = fs::read_to_string(self.run_path()).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub fn save_run(&self, run: &RunRecord) -> io::Result<()> {
        let text = serde_json::to_string(run).expect("run record serializes");
        let tmp = self.run_path().with_extension("tmp");
        fs::write(&tmp, text)?;
        fs::rename(tmp, self.run_path())
    }
}

/// `kill -0` checks that the process exists without signaling it.
pub fn pid_alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

pub fn state_from_args() -> PathBuf {
    let mut args = std::env::args().skip(2);
    let mut state = PathBuf::from(DEFAULT_STATE);
    while let Some(arg) = args.next() {
        if arg == "--state" {
            let Some(path) = args.next() else {
                eprintln!("--state needs a path");
                std::process::exit(2);
            };
            state = PathBuf::from(path);
        }
    }
    state
}
