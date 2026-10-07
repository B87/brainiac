//! The Linux `brainiac-runner` binary, built on this Mac in Docker and
//! cached by a digest of the sources (docs/architecture.md, Remote hosts).
//! The remote host does not compile it.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::OnceLock;

use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::sync::{mpsc, watch};

use crate::models::{AppError, AppResult, ErrorCode};

/// `linux/amd64` or `linux/arm64` for a host's `uname -m`.
pub fn platform(uname: &str) -> AppResult<&'static str> {
    match uname.trim() {
        "x86_64" => Ok("linux/amd64"),
        "aarch64" | "arm64" => Ok("linux/arm64"),
        _ => Err(AppError::dependency(
            "The host's processor is not one Brainiac builds a run controller for.",
        )
        .with_details(uname.trim().to_string())),
    }
}

/// The program `ensure` found or built.
pub struct Built {
    pub path: PathBuf,
    /// It was already in the cache: nothing was compiled.
    pub reused: bool,
}

/// The cached binary for this checkout and platform, building it when
/// missing. Each line the build prints goes to `on_line`; a `true` on
/// `cancel` stops the build and removes its container.
pub async fn ensure(
    cache_dir: &Path,
    uname: &str,
    on_line: &mut (dyn FnMut(&str) + Send),
    cancel: watch::Receiver<bool>,
) -> AppResult<Built> {
    let platform = platform(uname)?;
    let root = source_root()?;
    let digest = current_build().ok_or_else(not_a_checkout)?;
    std::fs::create_dir_all(cache_dir)?;
    let cached = cache_dir.join(format!(
        "brainiac-runner-{}-{}",
        platform.replace('/', "-"),
        &digest[..16]
    ));
    if cached.is_file() {
        return Ok(Built {
            path: cached,
            reused: true,
        });
    }
    build(&root, platform, &cached, on_line, cancel).await?;
    Ok(Built {
        path: cached,
        reused: false,
    })
}

/// The digest of the controller sources this Brainiac would build, hex, or
/// `None` when it does not run from a checkout. Computed once: the sources
/// do not change under a running app.
pub fn current_build() -> Option<String> {
    // `OnceLock` is a value set the first time it is asked for and shared,
    // read only, by every later caller and thread.
    static BUILD: OnceLock<Option<String>> = OnceLock::new();
    BUILD
        .get_or_init(|| {
            let root = source_root().ok()?;
            source_digest(&root).ok()
        })
        .clone()
}

/// How Settings names a build: the start of its digest.
pub fn short_build(build: &str) -> String {
    build.chars().take(7).collect()
}

/// The processor this Mac's Docker runs natively, as `uname -m` names it.
/// Another one is emulated, which makes the first build slow.
pub fn emulated(uname: &str) -> bool {
    let host = match uname.trim() {
        "arm64" => "aarch64",
        other => other,
    };
    host != std::env::consts::ARCH
}

/// SHA-256 of the cached file, hex.
pub fn file_digest(path: &Path) -> AppResult<String> {
    let bytes = std::fs::read(path)?;
    Ok(hex(&Sha256::digest(&bytes)))
}

fn source_root() -> AppResult<PathBuf> {
    let exe = std::env::current_exe().map_err(|e| {
        AppError::dependency("Brainiac could not find its own program.").with_details(e.to_string())
    })?;
    let mut dir = exe.as_path();
    for _ in 0..8 {
        let Some(parent) = dir.parent() else { break };
        dir = parent;
        let manifest = dir.join("src-tauri/Cargo.toml");
        let controller = dir.join("src-tauri/src/agents/controller.rs");
        if manifest.is_file() && controller.is_file() {
            return Ok(dir.join("src-tauri"));
        }
    }
    Err(not_a_checkout())
}

fn not_a_checkout() -> AppError {
    AppError::dependency(
        "The Linux run controller is built from a Brainiac checkout. Install from the repository.",
    )
}

fn source_digest(root: &Path) -> AppResult<String> {
    let mut files = Vec::new();
    walk(&root.join("src"), &mut files)?;
    for name in ["Cargo.toml", "Cargo.lock", "build.rs"] {
        let path = root.join(name);
        if path.is_file() {
            files.push(path);
        }
    }
    let toolchain = root.join("../rust-toolchain.toml");
    if toolchain.is_file() {
        files.push(toolchain);
    }
    files.sort();
    let mut hasher = Sha256::new();
    for path in files {
        let bytes = std::fs::read(&path)?;
        hasher.update(path.display().to_string().as_bytes());
        hasher.update(&bytes);
    }
    Ok(hex(&hasher.finalize()))
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> AppResult<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            walk(&path, out)?;
        } else {
            out.push(path);
        }
    }
    Ok(())
}

async fn build(
    root: &Path,
    platform: &str,
    dest: &Path,
    on_line: &mut (dyn FnMut(&str) + Send),
    mut cancel: watch::Receiver<bool>,
) -> AppResult<()> {
    let repo = root
        .parent()
        .ok_or_else(|| AppError::dependency("The checkout is incomplete."))?;
    let parent = dest.parent().unwrap_or(Path::new("/tmp"));
    // Each build writes into its own folder, so two hosts building at once
    // do not overwrite each other's program.
    let out = parent.join(format!("build-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&out)?;
    let name = format!("brainiac-runner-build-{}", uuid::Uuid::new_v4().simple());
    let arch = platform.rsplit('/').next().unwrap_or("native");
    let script = "cargo build --locked --release --no-default-features --bin brainiac-runner --target-dir /tmp/brainiac-target && cp /tmp/brainiac-target/release/brainiac-runner /out/brainiac-runner";
    // A login shell (`bash -l`) resets PATH and drops the image's cargo.
    // The registry and the target folder are named volumes on this Mac's
    // engine, so the next build after a Brainiac update compiles only what
    // changed. Removing them only makes that build slow again.
    let mut child = tokio::process::Command::new("docker")
        .args([
            "run",
            "--rm",
            "--name",
            &name,
            "--platform",
            platform,
            "-v",
            &format!("{}:/src", repo.display()),
            "-v",
            &format!("{}:/out", out.display()),
            "-v",
            "brainiac-runner-cargo:/usr/local/cargo/registry",
            "-v",
            &format!("brainiac-runner-target-{arch}:/tmp/brainiac-target"),
            "-w",
            "/src/src-tauri",
            "rust:1.88",
            "bash",
            "-c",
            script,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| {
            AppError::dependency("Docker could not build the run controller.")
                .with_details(e.to_string())
        })?;
    // Both streams feed one channel, so lines arrive in about the order
    // they were printed while this function waits on one thing at a time.
    let (lines_tx, mut lines) = mpsc::unbounded_channel::<String>();
    for stream in [
        child
            .stdout
            .take()
            .map(|s| Box::new(s) as Box<dyn AsyncRead + Send + Unpin>),
        child
            .stderr
            .take()
            .map(|s| Box::new(s) as Box<dyn AsyncRead + Send + Unpin>),
    ]
    .into_iter()
    .flatten()
    {
        let tx = lines_tx.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stream).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
    }
    drop(lines_tx);
    let mut tail: VecDeque<String> = VecDeque::new();
    let mut cancelled = *cancel.borrow();
    while !cancelled {
        tokio::select! {
            line = lines.recv() => match line {
                Some(line) => {
                    on_line(&line);
                    tail.push_back(line);
                    if tail.len() > TAIL_LINES {
                        tail.pop_front();
                    }
                }
                None => break,
            },
            changed = cancel.changed() => {
                // A dropped sender cannot cancel any more; stop listening.
                if changed.is_err() {
                    let _ = lines_until_end(&mut lines, on_line, &mut tail).await;
                    break;
                }
                cancelled = *cancel.borrow();
            }
        }
    }
    if cancelled {
        // Killing the `docker` client leaves its container running.
        let _ = tokio::process::Command::new("docker")
            .args(["rm", "-f", &name])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;
        let _ = child.kill().await;
        let _ = std::fs::remove_dir_all(&out);
        return Err(AppError::new(ErrorCode::Cancelled, "Cancelled."));
    }
    let status = child.wait().await.map_err(|e| {
        AppError::dependency("Docker could not build the run controller.")
            .with_details(e.to_string())
    })?;
    let built = out.join("brainiac-runner");
    if !status.success() {
        let _ = std::fs::remove_dir_all(&out);
        let log: Vec<String> = tail.into_iter().collect();
        return Err(
            AppError::dependency("The Linux run controller did not build.")
                .with_details(log_tail(&log.join("\n"))),
        );
    }
    if !built.is_file() {
        let _ = std::fs::remove_dir_all(&out);
        return Err(AppError::dependency(
            "The Linux run controller was not written.",
        ));
    }
    std::fs::rename(&built, dest)?;
    let _ = std::fs::remove_dir_all(&out);
    Ok(())
}

async fn lines_until_end(
    lines: &mut mpsc::UnboundedReceiver<String>,
    on_line: &mut (dyn FnMut(&str) + Send),
    tail: &mut VecDeque<String>,
) {
    while let Some(line) = lines.recv().await {
        on_line(&line);
        tail.push_back(line);
        if tail.len() > TAIL_LINES {
            tail.pop_front();
        }
    }
}

/// Lines kept for the error of a failed build.
const TAIL_LINES: usize = 40;

/// A crate Cargo starts compiling, from one line of its output.
pub fn compiled_crate(line: &str) -> Option<&str> {
    line.trim_start()
        .strip_prefix("Compiling ")
        .and_then(|rest| rest.split_whitespace().next())
}

/// The end of a failed build log. The whole log is too long to keep.
fn log_tail(log: &str) -> String {
    let lines: Vec<&str> = log.lines().collect();
    let start = lines.len().saturating_sub(40);
    lines[start..].join("\n")
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0xf) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_build_log_keeps_its_ending() {
        let log = (0..50)
            .map(|n| format!("line {n}"))
            .collect::<Vec<_>>()
            .join("\n");
        let tail = log_tail(&log);
        assert!(tail.contains("line 49"));
        assert!(!tail.contains("line 0"));
    }

    #[test]
    fn a_compiling_line_names_its_crate() {
        assert_eq!(
            compiled_crate("   Compiling tokio-util v0.7.16"),
            Some("tokio-util")
        );
        assert_eq!(compiled_crate("    Finished `release` profile"), None);
    }

    /// Builds the controller in this Mac's Docker, as Install does. Slow
    /// and needs an engine: `cargo test --lib builds_the_controller -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn builds_the_controller_in_docker_and_streams_its_output() {
        let cache = tempfile::tempdir().unwrap();
        let (_cancel, signal) = watch::channel(false);
        let mut compiled = 0;
        let mut on_line = |line: &str| {
            if compiled_crate(line).is_some() {
                compiled += 1;
            }
        };
        let built = ensure(cache.path(), std::env::consts::ARCH, &mut on_line, signal)
            .await
            .unwrap();
        assert!(!built.reused && built.path.is_file());
        let (_cancel, signal) = watch::channel(false);
        let again = ensure(cache.path(), std::env::consts::ARCH, &mut |_| {}, signal)
            .await
            .unwrap();
        assert!(again.reused);
        // Only the program is left beside the cache, not a build folder.
        assert_eq!(std::fs::read_dir(cache.path()).unwrap().count(), 1);
        println!("{compiled} crates compiled");
    }

    #[test]
    fn a_build_is_named_by_the_start_of_its_digest() {
        assert_eq!(short_build("7c2e51a9f0"), "7c2e51a");
    }
}
