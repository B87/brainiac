//! The Linux `brainiac-runner` binary a run host gets (docs/architecture.md,
//! Remote hosts). Run from a checkout, Brainiac builds it on this Mac in
//! Docker and caches it by a digest of the sources; a released Brainiac
//! downloads the one its release workflow built and checks it against the
//! SHA-256 it was built with. The remote host does not compile it.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::OnceLock;

use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader};
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

/// The program `ensure` found, built, or downloaded.
#[derive(Debug)]
pub struct Built {
    pub path: PathBuf,
    /// It was already in the cache: nothing was compiled or downloaded.
    pub reused: bool,
    /// The SHA-256 the copy on a host must have, hex: the one this Brainiac
    /// was released with, or a checkout's build's own.
    pub sha256: String,
}

/// The repository whose releases hold the controllers: the one the release
/// workflow ran in (GitHub sets `GITHUB_REPOSITORY`), so a fork's app
/// downloads its own.
const REPOSITORY: &str = match option_env!("GITHUB_REPOSITORY") {
    Some(repository) => repository,
    None => "B87/brainiac",
};

/// The SHA-256 of each controller the release workflow built for this
/// version, set when it builds the app (.github/workflows/release.yml).
/// `option_env!` reads them when this file is compiled, so they are part of
/// the signed app and not something a download could change.
const RELEASED: [(&str, Option<&str>); 2] = [
    (
        "linux/amd64",
        option_env!("BRAINIAC_RUNNER_SHA256_LINUX_AMD64"),
    ),
    (
        "linux/arm64",
        option_env!("BRAINIAC_RUNNER_SHA256_LINUX_ARM64"),
    ),
];

/// The SHA-256 this Brainiac was released with for `platform`'s controller.
fn released_digest(platform: &str) -> Option<&'static str> {
    RELEASED
        .iter()
        .find(|(p, _)| *p == platform)
        .and_then(|(_, digest)| *digest)
        .filter(|d| d.len() == 64 && d.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// This Brainiac downloads the controller its release built, rather than
/// building one from a checkout.
pub fn from_release() -> bool {
    source_root().is_err() && RELEASED.iter().any(|(p, _)| released_digest(p).is_some())
}

/// The URL of this version's controller for `platform` on its release.
fn release_url(platform: &str) -> String {
    format!(
        "https://github.com/{REPOSITORY}/releases/download/v{}/{}",
        env!("CARGO_PKG_VERSION"),
        asset_name(platform)
    )
}

/// The release asset of `platform`'s controller, as the release workflow
/// names it.
fn asset_name(platform: &str) -> String {
    format!("brainiac-runner-{}", platform.replace('/', "-"))
}

/// The cached binary for this Brainiac and platform. From a checkout it is
/// built when missing, each line the build prints going to `on_line`; a
/// released Brainiac downloads it instead. A `true` on `cancel` stops either
/// and removes what it left.
pub async fn ensure(
    cache_dir: &Path,
    uname: &str,
    on_line: &mut (dyn FnMut(&str) + Send),
    cancel: watch::Receiver<bool>,
) -> AppResult<Built> {
    let platform = platform(uname)?;
    std::fs::create_dir_all(cache_dir)?;
    let root = match source_root() {
        Ok(root) => root,
        Err(_) if from_release() => {
            let digest = released_digest(platform).ok_or_else(|| {
                AppError::dependency(format!(
                    "This release of Brainiac has no run controller for {platform}."
                ))
            })?;
            let cached = cache_dir.join(format!(
                "{}-v{}",
                asset_name(platform),
                env!("CARGO_PKG_VERSION")
            ));
            return download(&release_url(platform), digest, &cached, on_line, cancel).await;
        }
        Err(e) => return Err(e),
    };
    let digest = current_build().ok_or_else(no_controller)?;
    let cached = cache_dir.join(format!(
        "brainiac-runner-{}-{}",
        platform.replace('/', "-"),
        &digest[..16]
    ));
    let reused = cached.is_file();
    if !reused {
        build(&root, platform, &cached, on_line, cancel).await?;
    }
    Ok(Built {
        sha256: file_digest(&cached)?,
        path: cached,
        reused,
    })
}

/// The controller this Brainiac installs: from a checkout, the digest of
/// the controller sources, hex; from a release, its version (`v0.7.1`).
/// `None` when it has neither. Computed once: the sources do not change
/// under a running app.
pub fn current_build() -> Option<String> {
    // `OnceLock` is a value set the first time it is asked for and shared,
    // read only, by every later caller and thread.
    static BUILD: OnceLock<Option<String>> = OnceLock::new();
    BUILD
        .get_or_init(|| match source_root() {
            Ok(root) => source_digest(&root).ok(),
            Err(_) => from_release().then(|| format!("v{}", env!("CARGO_PKG_VERSION"))),
        })
        .clone()
}

/// How Settings names a build: a release's version whole, otherwise the
/// start of its digest.
pub fn short_build(build: &str) -> String {
    if build.starts_with('v') {
        return build.to_string();
    }
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
    Err(no_controller())
}

fn no_controller() -> AppError {
    AppError::dependency(
        "This Brainiac has no Linux run controller: it was neither released with one nor run from a checkout. Install a released Brainiac, or run it from the repository.",
    )
}

fn cancelled() -> AppError {
    AppError::new(ErrorCode::Cancelled, "Cancelled.")
}

/// Resolves once `cancel` is `true`; never, once its sender is dropped,
/// because nothing can cancel any more.
async fn until_cancelled(cancel: &mut watch::Receiver<bool>) {
    if cancel.wait_for(|c| *c).await.is_err() {
        std::future::pending::<()>().await;
    }
}

/// How long a download may go without any bytes arriving.
const DOWNLOAD_STALL: std::time::Duration = std::time::Duration::from_secs(60);

/// Far above any controller (about 15 MB): a download past it is stopped.
const DOWNLOAD_LIMIT: u64 = 256 * 1024 * 1024;

/// Downloads `url` to `dest` unless a file with `sha256` is already there,
/// and keeps it only when its SHA-256 is `sha256`. A `true` on `cancel`
/// stops it and removes the part written.
async fn download(
    url: &str,
    sha256: &str,
    dest: &Path,
    on_line: &mut (dyn FnMut(&str) + Send),
    mut cancel: watch::Receiver<bool>,
) -> AppResult<Built> {
    let sha256 = sha256.to_ascii_lowercase();
    if dest.is_file() {
        if file_digest(dest)? == sha256 {
            return Ok(Built {
                path: dest.to_path_buf(),
                reused: true,
                sha256,
            });
        }
        // Not the program this Brainiac was released with: fetch it again.
        // Another host's install may have removed it a moment before.
        if let Err(e) = std::fs::remove_file(dest) {
            if e.kind() != std::io::ErrorKind::NotFound {
                return Err(e.into());
            }
        }
    }
    let unreachable = |e: reqwest::Error| {
        AppError::dependency("Brainiac could not download the run controller.")
            .with_details(format!("{url}\n{e}"))
    };
    let client = reqwest::Client::builder()
        .user_agent(concat!("Brainiac/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(std::time::Duration::from_secs(15))
        .read_timeout(DOWNLOAD_STALL)
        .build()
        .map_err(unreachable)?;
    on_line(&format!("GET {url}"));
    let response = tokio::select! {
        response = client.get(url).send() => response.map_err(unreachable)?,
        _ = until_cancelled(&mut cancel) => return Err(cancelled()),
    };
    if !response.status().is_success() {
        return Err(AppError::dependency(format!(
            "The run controller for this Brainiac could not be downloaded ({}).",
            response.status()
        ))
        .with_details(format!(
            "{url}\nA release is downloadable only once it is published."
        )));
    }
    let parent = dest.parent().unwrap_or(Path::new("/tmp"));
    let part = parent.join(format!("download-{}", uuid::Uuid::new_v4().simple()));
    let (got, size) = match receive(response, &part, &mut cancel).await {
        Ok(received) => received,
        Err(mut e) => {
            let _ = std::fs::remove_file(&part);
            e.details = Some(match e.details {
                Some(details) => format!("{url}\n{details}"),
                None => url.to_string(),
            });
            return Err(e);
        }
    };
    if got != sha256 {
        let _ = std::fs::remove_file(&part);
        return Err(AppError::dependency(
            "The downloaded run controller is not the one this Brainiac was released with. It was deleted; nothing was installed.",
        )
        .with_details(format!("{url}\nexpected SHA-256 {sha256}\ndownloaded {got}")));
    }
    // A rename onto an existing file replaces it in one step, so a host
    // copying the cached program meanwhile reads the whole of one of them.
    if let Err(e) = std::fs::rename(&part, dest) {
        let _ = std::fs::remove_file(&part);
        return Err(e.into());
    }
    on_line(&format!(
        "{:.1} MB, SHA-256 {}",
        size as f64 / 1_000_000.0,
        &got[..12]
    ));
    Ok(Built {
        path: dest.to_path_buf(),
        reused: false,
        sha256,
    })
}

/// Writes `response`'s body to `part`, returning its SHA-256 and size. The
/// caller removes `part` when this fails.
async fn receive(
    mut response: reqwest::Response,
    part: &Path,
    cancel: &mut watch::Receiver<bool>,
) -> AppResult<(String, u64)> {
    let mut file = tokio::fs::File::create(part).await?;
    let mut hasher = Sha256::new();
    let mut size = 0u64;
    loop {
        let chunk = tokio::select! {
            chunk = response.chunk() => chunk.map_err(|e| {
                AppError::dependency("Brainiac could not download the run controller.")
                    .with_details(e.to_string())
            })?,
            _ = until_cancelled(cancel) => return Err(cancelled()),
        };
        let Some(bytes) = chunk else { break };
        size += bytes.len() as u64;
        if size > DOWNLOAD_LIMIT {
            return Err(AppError::dependency(
                "The run controller's download was far larger than any controller, so it was stopped.",
            ));
        }
        hasher.update(&bytes);
        file.write_all(&bytes).await?;
    }
    file.flush().await?;
    Ok((hex(&hasher.finalize()), size))
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
    let script = "cargo build --locked --release --no-default-features --features runner --bin brainiac-runner --target-dir /tmp/brainiac-target && cp /tmp/brainiac-target/release/brainiac-runner /out/brainiac-runner";
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

    #[test]
    fn a_released_build_is_named_by_its_whole_version() {
        assert_eq!(short_build("v0.10.12"), "v0.10.12");
    }

    #[test]
    fn a_release_asset_is_named_by_its_platform() {
        assert_eq!(asset_name("linux/amd64"), "brainiac-runner-linux-amd64");
        assert!(release_url("linux/arm64").ends_with(&format!(
            "/v{}/brainiac-runner-linux-arm64",
            env!("CARGO_PKG_VERSION")
        )));
    }

    /// Answers each of `answers` (status, body) to one connection, in order.
    async fn serve(answers: Vec<(u16, &'static [u8])>) -> String {
        serve_with(
            answers
                .into_iter()
                .map(|(s, b)| (s, vec![], b, false))
                .collect(),
        )
        .await
    }

    /// One answer: status, extra headers, body, and whether to stall after
    /// the head instead of sending the body.
    type Answer = (u16, Vec<String>, &'static [u8], bool);

    async fn serve_with(answers: Vec<Answer>) -> String {
        use tokio::io::AsyncReadExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            for (status, headers, body, stall) in answers {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut head = Vec::new();
                let mut buf = [0u8; 1024];
                while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                    let n = socket.read(&mut buf).await.unwrap();
                    if n == 0 {
                        break;
                    }
                    head.extend_from_slice(&buf[..n]);
                }
                let mut response = format!(
                    "HTTP/1.1 {status} X\r\ncontent-length: {}\r\nconnection: close\r\n",
                    body.len()
                );
                for header in headers {
                    response.push_str(&format!("{header}\r\n"));
                }
                response.push_str("\r\n");
                socket.write_all(response.as_bytes()).await.unwrap();
                if stall {
                    socket.write_all(&body[..1]).await.unwrap();
                    // Holds the connection open with the body unfinished.
                    tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                    continue;
                }
                socket.write_all(body).await.unwrap();
            }
        });
        base
    }

    fn sha(bytes: &[u8]) -> String {
        hex(&Sha256::digest(bytes))
    }

    #[tokio::test]
    async fn a_download_is_kept_only_when_it_is_the_released_program() {
        let program: &[u8] = b"\x7fELF the released controller";
        let base = serve(vec![(200, program), (200, b"\x7fELF something else")]).await;
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("brainiac-runner-linux-amd64-v1");
        let (_cancel, signal) = watch::channel(false);
        let built = download(
            &format!("{base}/a"),
            &sha(program),
            &dest,
            &mut |_| {},
            signal,
        )
        .await
        .unwrap();
        assert!(!built.reused);
        assert_eq!(std::fs::read(&dest).unwrap(), program);

        // The next host reuses it without a request.
        let (_cancel, signal) = watch::channel(false);
        let again = download(
            &format!("{base}/a"),
            &sha(program),
            &dest,
            &mut |_| {},
            signal,
        )
        .await
        .unwrap();
        assert!(again.reused);

        // A program with another digest is deleted, and nothing is kept.
        let other = dir.path().join("brainiac-runner-linux-arm64-v1");
        let (_cancel, signal) = watch::channel(false);
        let error = download(
            &format!("{base}/b"),
            &sha(program),
            &other,
            &mut |_| {},
            signal,
        )
        .await
        .unwrap_err();
        assert!(error
            .message
            .contains("not the one this Brainiac was released with"));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[tokio::test]
    async fn a_cached_program_that_does_not_match_is_downloaded_again() {
        let program: &[u8] = b"\x7fELF the released controller";
        let base = serve(vec![(200, program)]).await;
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("brainiac-runner-linux-amd64-v1");
        std::fs::write(&dest, b"truncated").unwrap();
        let (_cancel, signal) = watch::channel(false);
        let built = download(
            &format!("{base}/a"),
            &sha(program),
            &dest,
            &mut |_| {},
            signal,
        )
        .await
        .unwrap();
        assert!(!built.reused);
        assert_eq!(std::fs::read(&dest).unwrap(), program);
    }

    #[tokio::test]
    async fn a_release_without_the_program_says_so() {
        let base = serve(vec![(404, b"Not Found")]).await;
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("brainiac-runner-linux-amd64-v1");
        let (_cancel, signal) = watch::channel(false);
        let error = download(&format!("{base}/a"), &sha(b"x"), &dest, &mut |_| {}, signal)
            .await
            .unwrap_err();
        assert!(error.message.contains("could not be downloaded (404"));
        assert!(error.details.unwrap().contains("published"));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn a_download_follows_the_release_redirect() {
        // GitHub answers a release asset with a redirect to its storage.
        let program: &[u8] = b"\x7fELF the released controller";
        let storage = serve(vec![(200, program)]).await;
        let release = serve_with(vec![(
            302,
            vec![format!("location: {storage}/object")],
            b"",
            false,
        )])
        .await;
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("brainiac-runner-linux-amd64-v1");
        let (_cancel, signal) = watch::channel(false);
        let built = download(
            &format!("{release}/a"),
            &sha(program),
            &dest,
            &mut |_| {},
            signal,
        )
        .await
        .unwrap();
        assert_eq!(built.sha256, sha(program));
        assert_eq!(std::fs::read(&dest).unwrap(), program);
    }

    #[tokio::test]
    async fn a_cancelled_download_stops_at_once_and_leaves_nothing() {
        let base = serve_with(vec![(200, vec![], b"\x7fELF a long body", true)]).await;
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("brainiac-runner-linux-amd64-v1");
        let (cancel, signal) = watch::channel(false);
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            let _ = cancel.send(true);
        });
        let started = std::time::Instant::now();
        let error = download(&format!("{base}/a"), &sha(b"x"), &dest, &mut |_| {}, signal)
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Cancelled);
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}
