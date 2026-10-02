//! `brainiac mcp`: the stdio helper an agent runs. It forwards bytes between
//! its stdin/stdout and the running app's socket, opening the app first when
//! it is closed. It never parses MCP and never writes anything but the app's
//! replies to stdout, which carries the protocol.

use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::io::AsyncWriteExt;
use tokio::net::UnixStream;

use super::{current_uid, data_dir, socket_path, SOCKET_ENV};

/// How long to wait for the app to open and start serving.
const LAUNCH_WAIT: Duration = Duration::from_secs(15);
const RETRY_EVERY: Duration = Duration::from_millis(250);

/// Run the helper for the app with this identifier; returns the exit code.
pub fn run(identifier: &str) -> i32 {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("brainiac mcp: {e}");
            return 1;
        }
    };
    match runtime.block_on(pipe(identifier)) {
        Ok(()) => 0,
        Err(message) => {
            eprintln!("brainiac mcp: {message}");
            1
        }
    }
}

async fn pipe(identifier: &str) -> Result<(), String> {
    let socket = match std::env::var_os(SOCKET_ENV) {
        Some(path) => PathBuf::from(path),
        None => {
            let dir = data_dir(identifier).ok_or("HOME is not set.")?;
            socket_path(&dir, identifier)
        }
    };
    let stream = match connect(&socket).await {
        Ok(stream) => stream,
        Err(first) => match app_bundle() {
            Some(bundle) => {
                open_in_background(&bundle)?;
                wait_for(&socket).await.ok_or_else(|| {
                    format!(
                        "Brainiac did not start serving agents within {} seconds ({first}).",
                        LAUNCH_WAIT.as_secs()
                    )
                })?
            }
            None => {
                return Err(format!(
                    "Brainiac is not running ({first}). Open it, then reconnect."
                ))
            }
        },
    };

    let (mut from_app, mut to_app) = stream.into_split();
    let mut stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    let upstream = async {
        let copied = tokio::io::copy(&mut stdin, &mut to_app).await;
        let _ = to_app.shutdown().await;
        copied
    };
    let downstream = async {
        let copied = tokio::io::copy(&mut from_app, &mut stdout).await;
        let _ = stdout.flush().await;
        copied
    };
    // Either side closing ends the session: the agent quit, or the app did.
    tokio::select! {
        r = upstream => r.map(|_| ()).map_err(|e| format!("reading from the agent failed: {e}")),
        r = downstream => match r {
            Ok(_) => Err("Brainiac closed the connection (did it quit?).".into()),
            Err(e) => Err(format!("reading from Brainiac failed: {e}")),
        },
    }
}

/// Connect, only to a socket this user owns: another user could otherwise
/// put a socket of theirs in the shared `/tmp` fallback.
async fn connect(socket: &Path) -> Result<UnixStream, String> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    let meta = std::fs::symlink_metadata(socket).map_err(|e| e.to_string())?;
    if !meta.file_type().is_socket() || meta.uid() != current_uid() {
        return Err(format!("{} is not Brainiac's socket", socket.display()));
    }
    UnixStream::connect(socket).await.map_err(|e| e.to_string())
}

async fn wait_for(socket: &Path) -> Option<UnixStream> {
    let deadline = tokio::time::Instant::now() + LAUNCH_WAIT;
    while tokio::time::Instant::now() < deadline {
        tokio::time::sleep(RETRY_EVERY).await;
        if let Ok(stream) = connect(socket).await {
            return Some(stream);
        }
    }
    None
}

/// The `.app` this executable is inside, if any; a development build is not.
fn app_bundle() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    exe.ancestors()
        .find(|p| p.extension().is_some_and(|e| e == "app"))
        .map(Path::to_path_buf)
}

/// Open the app without bringing it to the front.
fn open_in_background(bundle: &Path) -> Result<(), String> {
    let status = std::process::Command::new("/usr/bin/open")
        .arg("-g")
        .arg(bundle)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|e| format!("could not open Brainiac: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("could not open {}", bundle.display()))
    }
}
