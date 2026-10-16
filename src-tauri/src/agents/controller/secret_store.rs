//! Secrets kept on a run host, for runs it starts without the Mac
//! (docs/design/scheduled-runs.md, Credentials on the host). Each secret is
//! one file, `secrets/<NAME>.cred` in the controller's state folder, sealed
//! by `systemd-creds --user`: only this user on this machine can open it. A
//! value is written by the app, read back only by the controller when a run
//! starts, and never listed or returned to a client. There is no plain-file
//! fallback: a host without `systemd-creds --user` takes no secrets, and says why.
//!
//! The program is a field so a test can stand in for it with a script;
//! production uses the one on `PATH`.

use std::fs;
use std::io;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::protocol::{valid_secret_name, MAX_EXTRA_VALUE, MIN_EXTRA_VALUE};
use super::state::write_atomic;
use crate::models::{AppError, AppResult};

/// Sealing or opening a value must finish in this long.
const SEAL_TIMEOUT: Duration = Duration::from_secs(15);
/// What `systemd-creds` may print: a sealed 16 KiB value is a little larger.
const MAX_SEALED: usize = 64 * 1024;

/// A secret as a client may see it: its name and when it was set. Never its value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretInfo {
    pub name: String,
    /// Seconds since the Unix epoch.
    pub set_at: u64,
}

/// Model credentials are sealed like secrets under this prefix, and a run
/// cannot name one (`list` hides them and `check_host_secret_names` refuses).
pub const MODEL_PREFIX: &str = "MODELKEY_";

pub struct SecretStore {
    dir: PathBuf,
    program: PathBuf,
}

impl SecretStore {
    /// `state` is the controller's folder; `program` is `systemd-creds`.
    pub fn new(state: &Path, program: impl Into<PathBuf>) -> Self {
        Self {
            dir: state.join("secrets"),
            program: program.into(),
        }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.join(format!("{name}.cred"))
    }

    /// Seal a value and keep it under `name`, replacing any earlier one.
    pub async fn put(&self, name: &str, value: &str) -> AppResult<()> {
        if !valid_secret_name(name) {
            return Err(AppError::validation(
                "A secret's name must be capitals, digits and underscores, and not one the agent or the container uses.",
            ));
        }
        if !(MIN_EXTRA_VALUE..=MAX_EXTRA_VALUE).contains(&value.len()) || value.contains('\0') {
            return Err(AppError::validation(
                "A secret's value is the wrong size, or holds a NUL character.",
            ));
        }
        let sealed = self.run("encrypt", name, value.as_bytes()).await?;
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&self.dir)
            .and_then(|()| fs::set_permissions(&self.dir, fs::Permissions::from_mode(0o700)))
            .and_then(|()| write_atomic(&self.path(name), &sealed))
            .map_err(|e| {
                AppError::io("The secret could not be saved on this host.")
                    .with_details(e.to_string())
            })
    }

    /// The sealed name a profile's model credential is kept under.
    fn model_name(profile_id: &str) -> String {
        let tail: String = profile_id
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_uppercase()
                } else {
                    '_'
                }
            })
            .collect();
        format!("{MODEL_PREFIX}{tail}")
    }

    /// Seal a profile's model credential for schedules to start runs with.
    pub async fn put_model(&self, profile_id: &str, value: &str) -> AppResult<()> {
        self.put(&Self::model_name(profile_id), value).await
    }

    /// Open it again when a schedule fires.
    pub async fn get_model(&self, profile_id: &str) -> AppResult<String> {
        self.get(&Self::model_name(profile_id)).await
    }

    /// Whether this host holds that profile's model credential.
    pub fn has_model(&self, profile_id: &str) -> bool {
        self.path(&Self::model_name(profile_id)).is_file()
    }

    /// Forget a secret. Removing one that is not there is not an error.
    pub fn delete(&self, name: &str) -> AppResult<()> {
        if !valid_secret_name(name) {
            return Err(AppError::validation("That is not a secret's name."));
        }
        match fs::remove_file(self.path(name)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(
                AppError::io("The secret could not be removed from this host.")
                    .with_details(e.to_string()),
            ),
        }
    }

    /// The secrets set, by name.
    pub fn list(&self) -> Vec<SecretInfo> {
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let mut secrets: Vec<SecretInfo> = entries
            .flatten()
            .filter_map(|entry| {
                let file = entry.file_name().into_string().ok()?;
                let name = file.strip_suffix(".cred")?;
                // A model credential kept for a schedule is not a secret a run names.
                if !valid_secret_name(name) || name.starts_with(MODEL_PREFIX) {
                    return None;
                }
                let set_at = entry
                    .metadata()
                    .ok()?
                    .modified()
                    .ok()?
                    .duration_since(UNIX_EPOCH)
                    .ok()?
                    .as_secs();
                Some(SecretInfo {
                    name: name.to_string(),
                    set_at,
                })
            })
            .collect();
        secrets.sort_by(|a, b| a.name.cmp(&b.name));
        secrets
    }

    /// Open a secret for a run that names it. The only place a value leaves
    /// the store.
    pub async fn get(&self, name: &str) -> AppResult<String> {
        if !valid_secret_name(name) {
            return Err(AppError::validation("That is not a secret's name."));
        }
        let sealed = match fs::read(self.path(name)) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Err(AppError::not_found(format!(
                    "This host does not have the secret {name}. Share it from Settings."
                )))
            }
            Err(e) => {
                return Err(AppError::io("A secret could not be read on this host.")
                    .with_details(e.to_string()))
            }
        };
        let opened = self.run("decrypt", name, &sealed).await?;
        String::from_utf8(opened)
            .map_err(|_| AppError::io(format!("The secret {name} on this host is not text.")))
    }

    /// `systemd-creds <verb> --user --name=<name> - -`: the input on stdin and
    /// the answer on stdout. Neither is ever put in an error. The program
    /// runs with an argument array, is killed if it takes too long, and its
    /// stderr is discarded.
    async fn run(&self, verb: &str, name: &str, input: &[u8]) -> AppResult<Vec<u8>> {
        let failed = |why: &str| {
            AppError::dependency(format!(
                "This host cannot keep secrets: `systemd-creds --user` did not work ({why}). It needs systemd 256 or newer."
            ))
        };
        let mut child = tokio::process::Command::new(&self.program)
            .args([verb, "--user", &format!("--name={name}"), "-", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|_| failed("it could not start"))?;
        let mut stdin = child.stdin.take().ok_or_else(|| failed("no input"))?;
        let mut stdout = child.stdout.take().ok_or_else(|| failed("no output"))?;
        let work = async {
            // Write and read together, so a large value cannot fill a pipe both ways.
            // Moving `stdin` in closes it when the write ends: the program
            // reads to the end of its input, and would wait forever otherwise.
            let write = async move {
                stdin.write_all(input).await?;
                drop(stdin);
                Ok::<_, io::Error>(())
            };
            let read = async {
                let mut out = Vec::new();
                (&mut stdout)
                    .take(MAX_SEALED as u64 + 1)
                    .read_to_end(&mut out)
                    .await?;
                Ok::<_, io::Error>(out)
            };
            let (written, out) = tokio::join!(write, read);
            written?;
            let out = out?;
            let status = child.wait().await?;
            Ok::<_, io::Error>((status, out))
        };
        let (status, out) = tokio::time::timeout(SEAL_TIMEOUT, work)
            .await
            .map_err(|_| failed("it took too long"))?
            .map_err(|_| failed("it stopped early"))?;
        if !status.success() || out.len() > MAX_SEALED {
            return Err(failed("it refused the value"));
        }
        Ok(out)
    }
}
