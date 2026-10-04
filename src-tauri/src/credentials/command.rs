//! Secrets printed by a program the user chose (SPEC.md, Secrets: Commands):
//! `gh auth token`, `op read`, `bw get password`.
//!
//! The program is started by its absolute path with each argument as its
//! own string, never through a shell. Standard input is closed and no
//! terminal is attached; the program runs in its own process group in a
//! folder Brainiac owns, so a timeout or an overflow stops it and anything it
//! started. Neither output stream ever reaches an error, a log, or a file:
//! errors name the program's file name, its exit status, or the limit hit.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt};

use super::{SecretBytes, MAX_SECRET_BYTES};
use crate::models::{AppError, AppResult, ErrorCode};

/// How long a command may take, from start until its output is closed and it has exited.
pub const DEADLINE: Duration = Duration::from_secs(60);
/// How long a stopped command is given to exit before Brainiac stops waiting.
const CLEANUP_GRACE: Duration = Duration::from_secs(2);
/// Folders a program is looked for in besides the launch `PATH`: an app
/// opened from Finder gets a minimal one without Homebrew's.
const EXTRA_DIRS: [&str; 2] = ["/opt/homebrew/bin", "/usr/local/bin"];

/// Runs secret commands. `Clone` so a read can run on its own task.
#[derive(Debug, Clone)]
pub struct CommandRunner {
    /// The working folder: Brainiac's own, outside every repository and the vault.
    work_dir: PathBuf,
    deadline: Duration,
}

impl CommandRunner {
    pub fn new(work_dir: PathBuf) -> Self {
        CommandRunner {
            work_dir,
            deadline: DEADLINE,
        }
    }

    /// A shorter deadline, for tests.
    pub fn with_deadline(mut self, deadline: Duration) -> Self {
        self.deadline = deadline;
        self
    }

    /// Run `program` with `args` and return what it printed, less exactly
    /// one final line break.
    pub async fn run(&self, program: &str, args: &[String]) -> AppResult<SecretBytes> {
        let name = program_name(program);
        std::fs::create_dir_all(&self.work_dir)?;
        let mut command = tokio::process::Command::new(program);
        command
            .args(args)
            .current_dir(&self.work_dir)
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // Its own process group, so stopping it also stops what it started.
            .process_group(0)
            .kill_on_drop(true);
        let mut child = command.spawn().map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => AppError::not_found(format!(
                "The program {name} is no longer where it was chosen. Choose it again."
            )),
            std::io::ErrorKind::PermissionDenied => AppError::new(
                ErrorCode::PermissionDenied,
                format!("The program {name} cannot be run: it is not executable."),
            ),
            _ => AppError::io(format!("The program {name} could not be started.")),
        })?;
        // Stops the whole group if this future ends early: a timeout, an
        // overflow, or the caller giving up.
        let mut group = GroupGuard(child.id());
        let stdout = child.stdout.take().expect("stdout is piped");
        let stderr = child.stderr.take().expect("stderr is piped");

        let ran = tokio::time::timeout(self.deadline, async {
            // `try_join!` drains both streams at once and stops at the first error.
            let (out, ()) = tokio::try_join!(
                read_bounded(stdout, &name, "printed"),
                discard_bounded(stderr, &name),
            )?;
            let status = child
                .wait()
                .await
                .map_err(|_| AppError::io(format!("Brainiac lost track of the program {name}.")))?;
            Ok::<(Vec<u8>, ExitStatus), AppError>((out, status))
        })
        .await;

        let (mut out, status) = match ran {
            Ok(Ok(done)) => done,
            Ok(Err(e)) => {
                group.stop();
                let _ = tokio::time::timeout(CLEANUP_GRACE, child.wait()).await;
                return Err(e);
            }
            Err(_) => {
                group.stop();
                let _ = tokio::time::timeout(CLEANUP_GRACE, child.wait()).await;
                return Err(AppError::new(
                    ErrorCode::Timeout,
                    format!(
                        "The command {name} did not finish within {} seconds. Run it in Terminal to see whether it waits for a sign-in.",
                        self.deadline.as_secs()
                    ),
                ));
            }
        };
        group.disarm();
        if !status.success() {
            out.iter_mut().for_each(|b| *b = 0);
            let how = match status.code() {
                Some(code) => format!("exited with status {code}"),
                None => "was stopped".to_string(),
            };
            return Err(AppError::io(format!(
                "The command {name} {how}. Unlock or sign in to it in Terminal, then try again."
            )));
        }
        if out.ends_with(b"\r\n") {
            out.truncate(out.len() - 2);
        } else if out.ends_with(b"\n") {
            out.truncate(out.len() - 1);
        }
        if out.is_empty() {
            return Err(AppError::validation(format!(
                "The command {name} printed nothing."
            )));
        }
        Ok(SecretBytes::new(out))
    }
}

/// Stops a process group unless disarmed.
struct GroupGuard(Option<u32>);

impl GroupGuard {
    fn stop(&mut self) {
        if let Some(pid) = self.0.take() {
            // SAFETY: `killpg` only sends a signal; the group is the child's
            // own (`process_group(0)` made its ID the child's).
            unsafe {
                libc::killpg(pid as libc::pid_t, libc::SIGKILL);
            }
        }
    }

    fn disarm(&mut self) {
        self.0 = None;
    }
}

impl Drop for GroupGuard {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Read a stream whole, failing as soon as it passes the limit, without
/// keeping a cut-off value.
async fn read_bounded(
    mut stream: impl AsyncRead + Unpin,
    name: &str,
    verb: &str,
) -> AppResult<Vec<u8>> {
    let mut out = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = stream
            .read(&mut buf)
            .await
            .map_err(|_| AppError::io(format!("Brainiac could not read what {name} {verb}.")))?;
        if n == 0 {
            buf.iter_mut().for_each(|b| *b = 0);
            return Ok(out);
        }
        if out.len() + n > MAX_SECRET_BYTES {
            out.iter_mut().for_each(|b| *b = 0);
            buf.iter_mut().for_each(|b| *b = 0);
            return Err(AppError::validation(format!(
                "The command {name} {verb} more than {} KiB. It must print only the secret.",
                MAX_SECRET_BYTES / 1024
            )));
        }
        out.extend_from_slice(&buf[..n]);
    }
}

/// Drain standard error without keeping it; a program that writes more
/// than the limit is stopped.
async fn discard_bounded(mut stream: impl AsyncRead + Unpin, name: &str) -> AppResult<()> {
    let mut total = 0usize;
    let mut buf = [0u8; 8192];
    loop {
        let n = stream.read(&mut buf).await.map_err(|_| {
            AppError::io(format!("Brainiac could not read the messages of {name}."))
        })?;
        buf.iter_mut().for_each(|b| *b = 0);
        if n == 0 {
            return Ok(());
        }
        total += n;
        if total > MAX_SECRET_BYTES {
            return Err(AppError::validation(format!(
                "The command {name} wrote more than {} KiB of messages, so it was stopped.",
                MAX_SECRET_BYTES / 1024
            )));
        }
    }
}

/// The file name of a program, safe to show in an error.
pub fn program_name(program: &str) -> String {
    Path::new(program)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "the program".to_string())
}

fn is_executable(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// Where a program is: an absolute path as given, or a short name looked
/// up in the launch `PATH`, then in Homebrew's folders. The path found is
/// shown to the user and saved; it is never looked up again at use.
pub fn find_program(name: &str) -> AppResult<PathBuf> {
    let name = name.trim();
    if name.is_empty() {
        return Err(AppError::validation("Enter a program."));
    }
    let given = Path::new(name);
    if given.is_absolute() {
        return if is_executable(given) {
            Ok(given.to_path_buf())
        } else {
            Err(AppError::not_found(format!(
                "There is no program at {name}."
            )))
        };
    }
    if name.contains('/') {
        return Err(AppError::validation(
            "Enter a program's name, such as gh, or its full path.",
        ));
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    let dirs = std::env::split_paths(&path).chain(EXTRA_DIRS.iter().map(PathBuf::from));
    for dir in dirs {
        let candidate = dir.join(name);
        if dir.is_absolute() && is_executable(&candidate) {
            return Ok(candidate);
        }
    }
    Err(AppError::not_found(format!(
        "{name} was found neither in Brainiac's PATH nor in {}. Choose the program with its full path.",
        EXTRA_DIRS.join(" or ")
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn script(dir: &Path, name: &str, body: &str) -> String {
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path.display().to_string()
    }

    fn runner(dir: &Path) -> CommandRunner {
        CommandRunner::new(dir.join("work")).with_deadline(Duration::from_secs(3))
    }

    #[tokio::test]
    async fn arguments_arrive_exactly_and_one_final_line_break_is_removed() {
        let dir = tempfile::tempdir().unwrap();
        let echo = script(dir.path(), "echo-args", r#"printf '%s|' "$@"; printf '\n'"#);
        let args = vec![
            "two words".to_string(),
            "it's \"quoted\"".to_string(),
            "$HOME".to_string(),
            "*".to_string(),
        ];
        let out = runner(dir.path()).run(&echo, &args).await.unwrap();
        assert_eq!(
            out.expose(),
            b"two words|it's \"quoted\"|$HOME|*|".as_slice()
        );

        let crlf = script(dir.path(), "crlf", r"printf ' a\nb \r\n'");
        let out = runner(dir.path()).run(&crlf, &[]).await.unwrap();
        assert_eq!(out.expose(), b" a\nb ".as_slice());
        let two = script(dir.path(), "two", r"printf 'x\n\n'");
        let out = runner(dir.path()).run(&two, &[]).await.unwrap();
        assert_eq!(out.expose(), b"x\n".as_slice());

        // It runs in Brainiac's folder, with stdin closed and Git's prompt off.
        let env = script(
            dir.path(),
            "env",
            r#"read line || echo "pwd=$(pwd) prompt=$GIT_TERMINAL_PROMPT""#,
        );
        let out = runner(dir.path()).run(&env, &[]).await.unwrap();
        let text = String::from_utf8(out.expose().to_vec()).unwrap();
        assert!(text.ends_with("/work prompt=0"), "{text}");
    }

    #[tokio::test]
    async fn failures_never_show_what_the_program_printed() {
        let dir = tempfile::tempdir().unwrap();
        let r = runner(dir.path());
        let fails = script(
            dir.path(),
            "fails",
            "echo hunter2-out; echo hunter2-err >&2; exit 3",
        );
        let err = r
            .run(&fails, &["--secret=hunter2".into()])
            .await
            .unwrap_err();
        assert_eq!(
            err.message,
            "The command fails exited with status 3. Unlock or sign in to it in Terminal, then try again."
        );
        assert!(!format!("{err:?}").contains("hunter2"), "{err:?}");

        let stderr_only = script(dir.path(), "quiet", "echo hunter2 >&2");
        let err = r.run(&stderr_only, &[]).await.unwrap_err();
        assert_eq!(err.message, "The command quiet printed nothing.");
        assert!(!format!("{err:?}").contains("hunter2"));

        let empty = script(dir.path(), "empty", "printf '\\n'");
        assert!(r.run(&empty, &[]).await.is_err());

        let missing = dir.path().join("gone").display().to_string();
        let err = r.run(&missing, &[]).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::NotFound);
    }

    #[tokio::test]
    async fn floods_and_hangs_stop_within_the_deadline() {
        let dir = tempfile::tempdir().unwrap();
        let r = runner(dir.path());
        let flood = script(dir.path(), "flood", "yes hunter2");
        let err = r.run(&flood, &[]).await.unwrap_err();
        assert!(err.message.contains("printed more than 64 KiB"), "{err:?}");
        let flood_err = script(dir.path(), "flood-err", "yes hunter2 >&2");
        let err = r.run(&flood_err, &[]).await.unwrap_err();
        assert!(err.message.contains("messages"), "{err:?}");
        assert!(!format!("{err:?}").contains("hunter2"));

        let quick =
            CommandRunner::new(dir.path().join("work")).with_deadline(Duration::from_secs(1));
        let started = std::time::Instant::now();
        let hangs = script(dir.path(), "hangs", "sleep 30");
        let err = quick.run(&hangs, &[]).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::Timeout);

        // A child that keeps the output open after its parent exits is stopped too.
        let marker = dir.path().join("child-alive");
        let orphan = script(
            dir.path(),
            "orphan",
            &format!(
                "(sleep 2; touch '{}') & echo token; exit 0",
                marker.display()
            ),
        );
        let err = quick.run(&orphan, &[]).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::Timeout);
        assert!(started.elapsed() < Duration::from_secs(6));
        tokio::time::sleep(Duration::from_millis(2500)).await;
        assert!(!marker.exists(), "the child kept running");
    }

    #[test]
    fn programs_are_found_by_name_or_full_path() {
        let found = find_program("sh").unwrap();
        assert!(found.is_absolute() && found.ends_with("sh"), "{found:?}");
        assert_eq!(find_program("/bin/sh").unwrap(), PathBuf::from("/bin/sh"));
        assert!(find_program("/nonexistent/op").is_err());
        assert!(find_program("bin/sh").is_err());
        assert!(find_program("brainiac-no-such-program").is_err());
    }
}
