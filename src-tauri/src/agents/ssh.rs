//! SSH as a transport to a remote run controller (SPEC.md, Remote hosts).
//! `ssh` and `scp` are invoked with argument arrays, the way Git is: nothing
//! here is a shell, and a host name is never interpolated into a command
//! string. The host key is whatever the user approved, in an app-owned
//! `known_hosts`; a changed key does not replace it.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use crate::models::{AppError, AppResult};

/// How long SSH may spend connecting. A stuck host ends here, on its own
/// process, so another host is not waiting on it.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);

/// The controller's socket on the host. It is not a TCP port.
pub const REMOTE_SOCKET: &str = "/var/lib/brainiac-runner/runner.sock";
pub const REMOTE_STATE: &str = "/var/lib/brainiac-runner";
pub const REMOTE_BIN: &str = "/usr/local/bin/brainiac-runner";
pub const SERVICE_USER: &str = "brainiac";
pub const SERVICE_NAME: &str = "brainiac-runner.service";

/// Where to connect, and the host key the user already approved.
#[derive(Clone)]
pub struct Target {
    pub user: String,
    pub host: String,
    pub port: u16,
    pub identity: Option<PathBuf>,
    pub known_hosts: PathBuf,
}

/// A user, host, and port safe to pass as SSH arguments.
pub fn target(
    user: &str,
    host: &str,
    port: u16,
    identity: Option<&str>,
    known_hosts: PathBuf,
) -> AppResult<Target> {
    if !valid_user(user) {
        return Err(AppError::validation(
            "The SSH user must be a short name: letters, digits, '_' and '-'.",
        ));
    }
    if !valid_host(host) {
        return Err(AppError::validation(
            "The SSH host must be a hostname or an address, with no spaces.",
        ));
    }
    if port == 0 {
        return Err(AppError::validation(
            "The SSH port must be between 1 and 65535.",
        ));
    }
    let identity = match identity.map(str::trim).filter(|s| !s.is_empty()) {
        None => None,
        Some(path) => {
            let path = PathBuf::from(path);
            if !path.is_absolute()
                || path
                    .components()
                    .any(|c| matches!(c, std::path::Component::ParentDir))
            {
                return Err(AppError::validation(
                    "The identity file must be an absolute path.",
                ));
            }
            Some(path)
        }
    };
    Ok(Target {
        user: user.to_string(),
        host: host.to_string(),
        port,
        identity,
        known_hosts,
    })
}

fn valid_user(user: &str) -> bool {
    let mut chars = user.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    !user.is_empty()
        && user.len() <= 32
        && user
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn valid_host(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 253
        && !host.starts_with('-')
        && !host.starts_with('.')
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == ':')
        && !host.contains("..")
}

/// Options every invocation shares. `known_hosts` is the app's file, not
/// the user's, so an unapproved key is not silently trusted.
pub fn base_args(target: &Target) -> Vec<String> {
    let mut args = vec![
        "-o".into(),
        "BatchMode=yes".into(),
        "-o".into(),
        format!("ConnectTimeout={}", CONNECT_TIMEOUT.as_secs()),
        "-o".into(),
        "StrictHostKeyChecking=yes".into(),
        "-o".into(),
        "GlobalKnownHostsFile=/dev/null".into(),
        "-o".into(),
        // Quoted: OpenSSH splits an `-o` value on spaces, and the data
        // folder is `Application Support`, so an unquoted path is not read.
        format!(
            "UserKnownHostsFile={}",
            config_token(&target.known_hosts.display().to_string())
        ),
        "-o".into(),
        "ControlMaster=no".into(),
        "-p".into(),
        target.port.to_string(),
    ];
    if let Some(identity) = &target.identity {
        args.push("-i".into());
        args.push(identity.display().to_string());
        args.push("-o".into());
        args.push("IdentitiesOnly=yes".into());
    }
    args
}

pub fn destination(target: &Target) -> String {
    format!("{}@{}", target.user, target.host)
}

/// A value inside an `ssh -o` option. OpenSSH splits it on spaces unless
/// the value is quoted.
fn config_token(value: &str) -> String {
    if value.contains([' ', '"', '\\']) {
        format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        value.to_string()
    }
}

/// `ssh` arguments: the options, the destination, then one remote command
/// that the caller built from constants (or from a user name that
/// [`valid_user`] already accepted).
pub fn ssh_args(target: &Target, remote: &str) -> Vec<String> {
    let mut args = base_args(target);
    args.push(destination(target));
    args.push(remote.to_string());
    args
}

/// `scp` uses `-P` for the port. The remote path is a constant.
pub fn scp_args(target: &Target, local: &Path, remote_path: &str) -> Vec<String> {
    let mut args = base_args(target);
    // `base_args` puts `-p` for the port, which `scp` reads as "preserve
    // times". Replace it.
    if let Some(i) = args.iter().position(|a| a == "-p") {
        args[i] = "-P".into();
    }
    args.push(local.display().to_string());
    args.push(format!("{}:{remote_path}", destination(target)));
    args
}

/// A stream-local forward from `local_socket` on this Mac to the
/// controller's socket on the host. `-N` does not run a remote command.
pub fn forward_args(target: &Target, local_socket: &Path) -> Vec<String> {
    let mut args = vec![
        "-N".into(),
        "-o".into(),
        "ExitOnForwardFailure=yes".into(),
        "-o".into(),
        "StreamLocalBindUnlink=yes".into(),
        "-L".into(),
        format!("{}:{REMOTE_SOCKET}", local_socket.display()),
    ];
    args.extend(base_args(target));
    args.push(destination(target));
    args
}

/// `ssh-keyscan` lines for this host, comments dropped. This does not write
/// `known_hosts` and does not trust the key.
pub async fn keyscan(host: &str, port: u16) -> AppResult<Vec<String>> {
    if !valid_host(host) || port == 0 {
        return Err(AppError::validation(
            "That is not a host Brainiac can look up.",
        ));
    }
    let output = run(
        "ssh-keyscan",
        &[
            "-T".into(),
            CONNECT_TIMEOUT.as_secs().to_string(),
            "-p".into(),
            port.to_string(),
            host.to_string(),
        ],
    )
    .await?;
    let lines: Vec<String> = output
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter(|l| l.split_whitespace().count() >= 3)
        .map(str::to_string)
        .collect();
    if lines.is_empty() {
        return Err(AppError::dependency(
            "The host did not present an SSH key. Check the address and that SSH is running.",
        )
        .with_details(output));
    }
    Ok(lines)
}

/// `SHA256:…` for one `ssh-keyscan` line, from `ssh-keygen -lf`.
pub fn fingerprint(line: &str) -> AppResult<String> {
    let dir = std::env::temp_dir().join(format!("brainiac-key-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("key");
    let result = (|| {
        std::fs::write(&path, format!("{line}\n"))?;
        let output = std::process::Command::new("ssh-keygen")
            .args(["-lf", path.to_str().unwrap_or("")])
            .output()?;
        if !output.status.success() {
            return Err(AppError::dependency("The host key could not be read."));
        }
        let text = String::from_utf8_lossy(&output.stdout);
        text.split_whitespace()
            .find(|w| w.starts_with("SHA256:"))
            .map(str::to_string)
            .ok_or_else(|| AppError::dependency("The host key has no fingerprint."))
    })();
    let _ = std::fs::remove_dir_all(&dir);
    result
}

/// The fingerprints of every key the host presents now.
pub async fn fingerprints(host: &str, port: u16) -> AppResult<Vec<String>> {
    let mut out = Vec::new();
    for line in keyscan(host, port).await? {
        out.push(fingerprint(&line)?);
    }
    Ok(out)
}

pub fn same_fingerprint(approved: &str, presented: &str) -> bool {
    approved.trim() == presented.trim() && approved.trim().starts_with("SHA256:")
}

/// Write the approved key lines. Callers do this only after the user
/// confirms the fingerprint.
pub fn write_known_hosts(path: &Path, lines: &[String]) -> AppResult<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    use std::io::Write;
    for line in lines {
        if line.contains('\n') || line.split_whitespace().count() < 3 {
            return Err(AppError::validation("The host key line is not usable."));
        }
        writeln!(file, "{line}")?;
    }
    Ok(())
}

pub struct CmdOut {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
}

pub async fn run(program: &str, args: &[String]) -> AppResult<String> {
    let out = output(program, args).await?;
    if out.status != 0 {
        let detail = if out.stderr.trim().is_empty() {
            out.stdout.trim().to_string()
        } else {
            out.stderr.trim().to_string()
        };
        return Err(AppError::dependency(format!("{program} failed.")).with_details(detail));
    }
    Ok(out.stdout)
}

pub async fn output(program: &str, args: &[String]) -> AppResult<CmdOut> {
    let program = program.to_string();
    let args = args.to_vec();
    let joined = tokio::task::spawn_blocking(move || {
        std::process::Command::new(&program)
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
    })
    .await
    .map_err(|e| AppError::io("A command could not be waited for.").with_details(e.to_string()))?
    .map_err(|e| {
        AppError::dependency("A command could not be started.").with_details(e.to_string())
    })?;
    Ok(CmdOut {
        status: joined.status.code().unwrap_or(1),
        stdout: String::from_utf8_lossy(&joined.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&joined.stderr).into_owned(),
    })
}

/// Add the SSH user to the controller's group. The name was checked by
/// [`valid_user`], so it is one token and not a shell expression.
pub fn group_command(user: &str) -> AppResult<String> {
    if !valid_user(user) {
        return Err(AppError::validation(
            "The SSH user is not a name Brainiac can use.",
        ));
    }
    Ok(format!("sudo -n usermod -aG {SERVICE_USER} {user}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Target {
        Target {
            user: "ada".into(),
            host: "runner.example".into(),
            port: 22,
            identity: None,
            known_hosts: PathBuf::from("/tmp/brainiac-known-hosts"),
        }
    }

    #[test]
    fn ssh_args_are_an_array_and_pin_the_host_key() {
        let args = ssh_args(&sample(), "uname -m");
        assert!(args.contains(&"BatchMode=yes".into()));
        assert!(args.contains(&"StrictHostKeyChecking=yes".into()));
        assert!(args.iter().any(|a| a.contains("UserKnownHostsFile=")));
        assert_eq!(args[args.len() - 2], "ada@runner.example");
        assert_eq!(args.last().map(String::as_str), Some("uname -m"));
        assert!(!args.iter().any(|a| a.contains(';')));
    }

    #[test]
    fn a_host_with_a_space_or_a_shell_character_is_refused() {
        assert!(target("ada", "bad host", 22, None, PathBuf::from("/t")).is_err());
        assert!(target("ada", "host;rm", 22, None, PathBuf::from("/t")).is_err());
        assert!(target(
            "ada lovelace",
            "runner.example",
            22,
            None,
            PathBuf::from("/t")
        )
        .is_err());
        assert!(target("-ada", "runner.example", 22, None, PathBuf::from("/t")).is_err());
        assert!(target("ada", "runner.example", 0, None, PathBuf::from("/t")).is_err());
        assert!(target(
            "ada",
            "runner.example",
            22,
            Some("key"),
            PathBuf::from("/t")
        )
        .is_err());
    }

    #[test]
    fn scp_uses_the_port_flag_scp_understands() {
        let args = scp_args(
            &sample(),
            Path::new("/tmp/brainiac-runner"),
            "/tmp/brainiac-runner.upload",
        );
        assert!(args.contains(&"-P".into()));
        assert!(!args.contains(&"-p".into()));
        assert!(args
            .last()
            .unwrap()
            .ends_with(":/tmp/brainiac-runner.upload"));
    }

    #[test]
    fn the_forward_does_not_open_a_remote_shell() {
        let args = forward_args(&sample(), Path::new("/tmp/forward.sock"));
        assert!(args.contains(&"-N".into()));
        assert!(args.iter().any(|a| a.contains(REMOTE_SOCKET)));
        assert!(!args.iter().any(|a| a == "uname -m"));
    }

    #[test]
    fn a_changed_fingerprint_does_not_match() {
        assert!(same_fingerprint("SHA256:abc", "SHA256:abc"));
        assert!(!same_fingerprint("SHA256:abc", "SHA256:def"));
        assert!(!same_fingerprint("abc", "abc"));
    }

    #[test]
    fn a_known_hosts_path_with_a_space_is_quoted() {
        let mut host = sample();
        host.known_hosts = PathBuf::from("/Users/ada/Application Support/known_hosts");
        let args = base_args(&host);
        assert!(args
            .iter()
            .any(|a| { a == "UserKnownHostsFile=\"/Users/ada/Application Support/known_hosts\"" }));
    }

    #[test]
    fn the_group_command_refuses_a_user_that_is_not_a_name() {
        assert!(group_command("ada")
            .unwrap()
            .contains("usermod -aG brainiac ada"));
        assert!(group_command("ada;id").is_err());
    }
}
