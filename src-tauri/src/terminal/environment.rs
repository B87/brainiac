//! The user's login shell and its environment (docs/architecture.md,
//! Terminal).
//!
//! An app opened from the Finder starts with a bare environment, so a
//! program named in Settings → Terminal would not find `node`, `claude`, or
//! `herdr` on its PATH. The login shell's environment is read once per
//! launch, as a terminal app's first tab would see it, and given to those
//! programs. A plain shell tab is itself a login shell and needs none of it.

use std::ffi::{CStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

/// How long the login shell may take to print its environment.
pub const READ_TIMEOUT: Duration = Duration::from_secs(5);

/// Printed before the environment, so whatever a profile prints first (a
/// greeting, a version manager's notice) is skipped.
const MARKER: &str = "__brainiac_environment__";

/// What a program in a tab is told about the terminal it runs in.
pub const TERMINAL_VARIABLES: &[(&str, &str)] =
    &[("TERM", "xterm-256color"), ("COLORTERM", "truecolor")];

/// Left out of every tab's environment: they describe the terminal Brainiac
/// was started from, if any, not Brainiac's.
const NOT_INHERITED: &[&str] = &["TERM_PROGRAM", "TERM_PROGRAM_VERSION", "TERM_SESSION_ID"];

/// The user's login shell: `$SHELL`, else the one in the user database, else
/// the system's default.
pub fn login_shell() -> PathBuf {
    if let Some(shell) = std::env::var_os("SHELL").filter(|s| !s.is_empty()) {
        return PathBuf::from(shell);
    }
    // `unsafe` because this calls the C library directly: `getpwuid` returns
    // a pointer into a static record, read at once and copied.
    let from_database = unsafe {
        let record = libc::getpwuid(libc::getuid());
        if record.is_null() || (*record).pw_shell.is_null() {
            None
        } else {
            CStr::from_ptr((*record).pw_shell)
                .to_str()
                .ok()
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
        }
    };
    from_database.unwrap_or_else(|| {
        PathBuf::from(if cfg!(target_os = "macos") {
            "/bin/zsh"
        } else {
            "/bin/sh"
        })
    })
}

/// Read the login shell's environment: `shell -l -c 'env -0'`, with a
/// timeout. `None` when the shell fails, takes too long, or prints nothing
/// usable; the caller then uses Brainiac's own environment.
pub async fn read(shell: &Path, timeout: Duration) -> Option<Vec<(String, String)>> {
    let script = format!("printf '%s' {MARKER}; env -0");
    let mut command = tokio::process::Command::new(shell);
    command
        .args(["-l", "-c", &script])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        // A shell still running at the timeout is killed with the future.
        .kill_on_drop(true);
    let output = match tokio::time::timeout(timeout, command.output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => {
            tracing::warn!(shell = %shell.display(), error = %e, "could not start the login shell");
            return None;
        }
        Err(_) => {
            tracing::warn!(shell = %shell.display(), "the login shell took too long to print its environment");
            return None;
        }
    };
    if !output.status.success() {
        tracing::warn!(shell = %shell.display(), status = %output.status, "the login shell failed");
        return None;
    }
    parse(&output.stdout)
}

/// The variables after the marker, each `NAME=value` and ended by a NUL.
pub fn parse(stdout: &[u8]) -> Option<Vec<(String, String)>> {
    let marker = MARKER.as_bytes();
    let start = stdout.windows(marker.len()).position(|w| w == marker)? + marker.len();
    let variables: Vec<(String, String)> = stdout[start..]
        .split(|&b| b == 0)
        .filter_map(|entry| {
            let entry = String::from_utf8_lossy(entry);
            let (name, value) = entry.split_once('=')?;
            // A name never has a newline; one here is a profile's output
            // printed after `env`, or a broken entry.
            (!name.is_empty() && !name.contains('\n'))
                .then(|| (name.to_string(), value.to_string()))
        })
        .collect();
    (!variables.is_empty()).then_some(variables)
}

/// A tab's environment: `base` without the variables of another terminal,
/// with the ones that describe this one.
pub fn for_tab(base: impl IntoIterator<Item = (String, String)>) -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = base
        .into_iter()
        .filter(|(name, _)| {
            !NOT_INHERITED.contains(&name.as_str())
                && !TERMINAL_VARIABLES.iter().any(|(n, _)| n == name)
        })
        .collect();
    env.extend(
        TERMINAL_VARIABLES
            .iter()
            .map(|(n, v)| (n.to_string(), v.to_string())),
    );
    env
}

/// Brainiac's own environment, as text.
pub fn own() -> Vec<(String, String)> {
    std::env::vars_os()
        .filter_map(|(n, v)| Some((n.into_string().ok()?, v.into_string().ok()?)))
        .collect()
}

/// Where `program` is: itself when it names a path, else the first match on
/// the `PATH` in `env`.
pub fn find_program(program: &str, env: &[(String, String)]) -> Option<PathBuf> {
    if program.contains('/') {
        let path = PathBuf::from(program);
        return is_executable(&path).then_some(path);
    }
    let path: OsString = env
        .iter()
        .find(|(n, _)| n == "PATH")
        .map(|(_, v)| v.into())?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(program))
        .find(|candidate| is_executable(candidate))
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    /// A stand-in login shell: a script that ignores `-l -c …` and does
    /// what the test needs.
    fn fake_shell(dir: &Path, body: &str) -> PathBuf {
        let path = dir.join("shell");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn value<'a>(env: &'a [(String, String)], name: &str) -> Option<&'a str> {
        env.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }

    #[tokio::test]
    async fn a_real_shell_prints_its_environment() {
        let env = read(Path::new("/bin/sh"), READ_TIMEOUT).await.unwrap();
        assert!(value(&env, "PATH").is_some());
    }

    #[tokio::test]
    async fn a_noisy_profile_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let shell = fake_shell(
            dir.path(),
            &format!("echo 'Welcome back'; printf '%s' {MARKER}; printf 'PATH=/opt/tools/bin\\0GREETING=a=b\\0'"),
        );
        let env = read(&shell, READ_TIMEOUT).await.unwrap();
        assert_eq!(value(&env, "PATH"), Some("/opt/tools/bin"));
        // Only the first `=` separates the name.
        assert_eq!(value(&env, "GREETING"), Some("a=b"));
        assert_eq!(value(&env, "Welcome back"), None);
    }

    #[tokio::test]
    async fn a_slow_shell_gives_up_at_the_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let shell = fake_shell(dir.path(), "sleep 10");
        let started = std::time::Instant::now();
        assert_eq!(read(&shell, Duration::from_millis(200)).await, None);
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn a_failing_shell_gives_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let shell = fake_shell(dir.path(), &format!("printf '%s' {MARKER}; exit 1"));
        assert_eq!(read(&shell, READ_TIMEOUT).await, None);
        assert_eq!(read(&dir.path().join("missing"), READ_TIMEOUT).await, None);
    }

    /// fish takes the same `-l -c`, with `printf` and `env` as in sh. Run
    /// only where fish is installed.
    #[tokio::test]
    async fn fish_prints_its_environment_when_installed() {
        let fish = [
            "/usr/bin/fish",
            "/opt/homebrew/bin/fish",
            "/usr/local/bin/fish",
        ]
        .into_iter()
        .map(Path::new)
        .find(|p| p.exists());
        let Some(fish) = fish else {
            return;
        };
        let env = read(fish, READ_TIMEOUT).await.unwrap();
        assert!(value(&env, "PATH").is_some());
    }

    #[test]
    fn no_marker_or_no_variables_is_nothing() {
        assert_eq!(parse(b"PATH=/bin\0"), None);
        assert_eq!(parse(MARKER.as_bytes()), None);
    }

    #[test]
    fn a_tab_says_which_terminal_it_is_and_drops_another_terminals() {
        let env = for_tab([
            ("TERM".to_string(), "dumb".to_string()),
            ("TERM_PROGRAM".to_string(), "OtherTerminal".to_string()),
            ("HOME".to_string(), "/home/someone".to_string()),
        ]);
        assert_eq!(value(&env, "TERM"), Some("xterm-256color"));
        assert_eq!(value(&env, "COLORTERM"), Some("truecolor"));
        assert_eq!(value(&env, "TERM_PROGRAM"), None);
        assert_eq!(value(&env, "HOME"), Some("/home/someone"));
        assert_eq!(env.iter().filter(|(n, _)| n == "TERM").count(), 1);
    }

    #[test]
    fn a_program_is_found_on_the_given_path() {
        let dir = tempfile::tempdir().unwrap();
        let tool = fake_shell(dir.path(), "true");
        let env = vec![(
            "PATH".to_string(),
            format!("/nonexistent:{}", dir.path().display()),
        )];
        assert_eq!(find_program("shell", &env), Some(tool.clone()));
        assert_eq!(find_program(tool.to_str().unwrap(), &[]), Some(tool));
        assert_eq!(find_program("not-there", &env), None);
    }
}
