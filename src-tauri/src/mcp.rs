//! Agent access (SPEC.md, section 9; docs/architecture.md, Agent access):
//! the MCP server agents reach through `brainiac mcp` and a Unix socket.
//!
//! The app owns the socket while it runs, whatever the access mode; the mode
//! decides which tools a connection lists and may call.

pub mod helper;
pub mod tools;

use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use rmcp::ServiceExt;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::UnixListener;
use tokio::sync::watch;

use crate::models::{AgentAccess, AgentAccessStatus, AppError, AppResult};
use crate::notes::NoteService;
use crate::tasks::TaskService;
use crate::workspaces::RepositoryService;

/// The socket's name in the app's data folder.
pub const SOCKET_NAME: &str = "mcp.sock";
/// Overrides the socket path, for tests and development.
pub const SOCKET_ENV: &str = "BRAINIAC_MCP_SOCKET";
/// macOS limits a Unix socket path to 104 bytes, including the final NUL.
pub(crate) const MAX_SOCKET_PATH: usize = 103;

/// The user ID this process runs as.
pub fn current_uid() -> u32 {
    // `unsafe` because every libc call is, to Rust; `getuid` cannot fail.
    unsafe { libc::getuid() }
}

/// Where the app with this identifier keeps its data, as Tauri's
/// `app_data_dir` resolves it on macOS.
pub fn data_dir(identifier: &str) -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(
        PathBuf::from(home)
            .join("Library/Application Support")
            .join(identifier),
    )
}

/// The socket path: `mcp.sock` in the data folder, or, when that path is too
/// long for a socket, one in `/tmp/brainiac-<uid>/`.
pub fn socket_path(data_dir: &Path, identifier: &str) -> PathBuf {
    let preferred = data_dir.join(SOCKET_NAME);
    if preferred.as_os_str().len() <= MAX_SOCKET_PATH {
        preferred
    } else {
        fallback_dir().join(format!("{identifier}.sock"))
    }
}

pub(crate) fn fallback_dir() -> PathBuf {
    PathBuf::from(format!("/tmp/brainiac-{}", current_uid()))
}

/// Create the socket's folder when it is the `/tmp` fallback, and refuse a
/// folder another user owns or, in `/tmp`, one others can open.
pub(crate) fn prepare_socket_dir(socket: &Path) -> AppResult<()> {
    let dir = socket
        .parent()
        .ok_or_else(|| AppError::validation("The agent socket needs a folder."))?;
    let shared_tmp = dir == fallback_dir();
    if shared_tmp {
        use std::os::unix::fs::DirBuilderExt;
        match std::fs::DirBuilder::new().mode(0o700).create(dir) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.into()),
        }
    } else {
        std::fs::create_dir_all(dir)?;
    }
    let meta = std::fs::symlink_metadata(dir)?;
    let private = meta.mode() & 0o077 == 0;
    if !meta.is_dir() || meta.uid() != current_uid() || (shared_tmp && !private) {
        return Err(AppError::new(
            crate::models::ErrorCode::PermissionDenied,
            "The folder for the agent socket belongs to someone else.",
        )
        .with_details(dir.display().to_string()));
    }
    Ok(())
}

/// The socket is ours to replace: a socket file this user owns.
pub(crate) fn owned_socket(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|m| m.file_type().is_socket() && m.uid() == current_uid())
        .unwrap_or(false)
}

/// The server: shared by every connection, holding the access mode.
pub struct AgentServer {
    pub(crate) notes: Arc<NoteService>,
    pub(crate) tasks: Arc<TaskService>,
    pub(crate) repositories: Arc<RepositoryService>,
    /// A `watch` channel: connections see each change and tell their agent
    /// that the tool list changed.
    access: watch::Sender<AgentAccess>,
    connections: AtomicUsize,
    /// The socket this server bound, removed on `close`.
    socket: Mutex<Option<PathBuf>>,
    /// Why the socket could not be opened, shown in Settings.
    problem: Mutex<Option<String>>,
}

impl AgentServer {
    pub fn new(
        notes: Arc<NoteService>,
        tasks: Arc<TaskService>,
        repositories: Arc<RepositoryService>,
        access: AgentAccess,
    ) -> Arc<Self> {
        Arc::new(Self {
            notes,
            tasks,
            repositories,
            access: watch::Sender::new(access),
            connections: AtomicUsize::new(0),
            socket: Mutex::new(None),
            problem: Mutex::new(None),
        })
    }

    pub fn access(&self) -> AgentAccess {
        *self.access.borrow()
    }

    /// Apply Settings → Agent access to every connection at once.
    pub fn set_access(&self, access: AgentAccess) {
        self.access.send_if_modified(|current| {
            let changed = *current != access;
            *current = access;
            changed
        });
    }

    pub(crate) fn watch_access(&self) -> watch::Receiver<AgentAccess> {
        self.access.subscribe()
    }

    /// Agents connected now.
    pub fn connections(&self) -> usize {
        self.connections.load(Ordering::SeqCst)
    }

    /// What Settings → Agent access shows.
    pub fn status(&self) -> AgentAccessStatus {
        AgentAccessStatus {
            access: self.access(),
            connections: self.connections() as u32,
            executable: std::env::current_exe()
                .and_then(|p| p.canonicalize())
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
            problem: self.problem.lock().expect("problem").clone(),
        }
    }

    /// Bind the socket, replacing a stale one left by a previous run. A
    /// failure is kept for Settings to show.
    pub async fn bind(&self, path: &Path) -> AppResult<UnixListener> {
        let bound = self.bind_inner(path).await;
        *self.problem.lock().expect("problem") = bound.as_ref().err().map(|e| e.message.clone());
        bound
    }

    async fn bind_inner(&self, path: &Path) -> AppResult<UnixListener> {
        prepare_socket_dir(path)?;
        if std::fs::symlink_metadata(path).is_ok() {
            if !owned_socket(path) {
                return Err(AppError::new(
                    crate::models::ErrorCode::PermissionDenied,
                    "Something other than Brainiac's agent socket is in its place.",
                )
                .with_details(path.display().to_string()));
            }
            if tokio::net::UnixStream::connect(path).await.is_ok() {
                return Err(AppError::new(
                    crate::models::ErrorCode::Conflict,
                    "Another Brainiac is already serving agents.",
                ));
            }
            std::fs::remove_file(path)?;
        }
        let listener = UnixListener::bind(path)?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        *self.socket.lock().expect("socket path") = Some(path.to_path_buf());
        Ok(listener)
    }

    /// Accept agents until the app quits. A connection from another user
    /// is closed unanswered.
    pub async fn accept(self: Arc<Self>, listener: UnixListener) {
        let me = current_uid();
        loop {
            let stream = match listener.accept().await {
                Ok((stream, _)) => stream,
                Err(e) => {
                    tracing::warn!(error = %e, "agent socket accept failed");
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                    continue;
                }
            };
            match stream.peer_cred() {
                Ok(cred) if cred.uid() == me => {}
                other => {
                    tracing::warn!(peer = ?other, "refused an agent connection from another user");
                    continue;
                }
            }
            let server = Arc::clone(&self);
            tokio::spawn(async move { server.serve(stream).await });
        }
    }

    /// Serve one agent over a byte stream until it disconnects.
    pub async fn serve<S>(self: &Arc<Self>, stream: S)
    where
        S: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    {
        self.connections.fetch_add(1, Ordering::SeqCst);
        let connection = tools::Connection::new(Arc::clone(self));
        match connection.serve(stream).await {
            Ok(running) => {
                let _ = running.waiting().await;
            }
            Err(e) => tracing::warn!(error = %e, "an agent connection failed to start"),
        }
        self.connections.fetch_sub(1, Ordering::SeqCst);
    }

    /// Remove the socket; the app is quitting.
    pub fn close(&self) {
        if let Some(path) = self.socket.lock().expect("socket path").take() {
            if owned_socket(&path) {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_data_folder_moves_the_socket_to_tmp() {
        let short = socket_path(Path::new("/Users/a/Library/Application Support/x"), "x");
        assert_eq!(
            short,
            Path::new("/Users/a/Library/Application Support/x/mcp.sock")
        );
        let long_dir = format!(
            "/Users/{}/Library/Application Support/dev.brainiac.desktop",
            "u".repeat(60)
        );
        let long = socket_path(Path::new(&long_dir), "dev.brainiac.desktop");
        assert_eq!(
            long,
            PathBuf::from(format!(
                "/tmp/brainiac-{}/dev.brainiac.desktop.sock",
                current_uid()
            ))
        );
    }
}
