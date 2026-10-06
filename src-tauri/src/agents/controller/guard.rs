//! `RunGuard`: `brainiac runner guard`, started beside the controller. A
//! controller killed outright cannot stop its own containers, so the guard
//! watches its process and, once it is gone, stops every live run's
//! containers — and keeps them, with their volumes: they hold the work.
//!
//! The guard takes the controller's lock before it stops anything. A new
//! controller that already holds the lock owns those runs (it interrupts
//! them itself), so the guard never stops a run a newer controller started.

use std::time::Duration;

use super::docker::Workloads;
use super::protocol::Phase;
use super::state::{pid_alive, StateDir};

const WATCH_EVERY: Duration = Duration::from_millis(500);

/// Start the guard for this process, in its own process group so it is not
/// stopped with the controller's.
pub fn spawn(state: &StateDir) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    use std::os::unix::process::CommandExt;
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(state.log_path())?;
    let (exe, mut args) = super::runner_command()?;
    args.push("guard".into());
    let mut child = std::process::Command::new(exe)
        .args(args)
        .arg("--state")
        .arg(state.root())
        .arg("--pid")
        .arg(std::process::id().to_string())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(log)
        .process_group(0)
        .spawn()?;
    // Waited for on a thread of its own, so it does not linger as a
    // zombie once it exits.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// Watch the controller with this process ID until it is gone, then stop
/// what it left running.
///
/// The controller holds its lock for as long as it lives, and the system
/// releases it when the process ends, however it ends. So the guard's test
/// is taking the lock, not the process ID, which another process may reuse.
pub async fn watch<W: Workloads>(state: StateDir, pid: u32, workloads: W) {
    // A SIGTERM (the Mac logs out) does not end the guard: the controller
    // gets one too and stops its runs; the guard waits for it to be gone.
    let mut terminate =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).ok();
    let emergency = bind_emergency(&state).ok();
    loop {
        let incoming = async {
            match emergency.as_ref() {
                Some(listener) => listener.accept().await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            _ = tokio::time::sleep(WATCH_EVERY) => {}
            _ = async {
                match terminate.as_mut() {
                    Some(signal) => { signal.recv().await; }
                    None => std::future::pending::<()>().await,
                }
            } => {}
            accepted = incoming => {
                if let Ok((stream, _)) = accepted {
                    if emergency_request(&state, stream).await {
                        let stopped = sweep(&state, &workloads).await;
                        eprintln!("brainiac runner guard: emergency stop halted the containers of {stopped} run(s)");
                    }
                }
            }
        }
        match state.lock() {
            Ok(Some(_lock)) => {
                let stopped = sweep(&state, &workloads).await;
                if stopped > 0 {
                    eprintln!("brainiac runner guard: stopped the containers of {stopped} run(s) the run controller left");
                }
                return;
            }
            // Its controller is gone and a newer one holds the lock: the
            // newer one stops what was left, with its own guard.
            _ if !pid_alive(pid) => return,
            _ => {}
        }
    }
}

/// Listen for emergency stop. The socket is private to this user; the
/// command runs as the service user.
fn bind_emergency(state: &StateDir) -> std::io::Result<tokio::net::UnixListener> {
    use std::os::unix::fs::PermissionsExt;
    let path = state.emergency_socket();
    if path.exists() {
        let _ = std::fs::remove_file(&path);
    }
    let listener = tokio::net::UnixListener::bind(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

/// Hello with the installation token, then EmergencyStop. Anything else is ignored.
async fn emergency_request(state: &StateDir, stream: tokio::net::UnixStream) -> bool {
    use super::protocol::{Request, Response, MAX_LINE_BYTES};
    let Ok(token) = state.token() else {
        return false;
    };
    let (read, mut write) = stream.into_split();
    let mut reader = tokio::io::BufReader::new(read);
    let Ok(Some(line)) = super::read_line(&mut reader, MAX_LINE_BYTES).await else {
        return false;
    };
    let Ok(Request::Hello {
        token: given,
        protocol: _,
    }) = serde_json::from_slice(&line)
    else {
        return false;
    };
    if !super::same_token(&token, &given) {
        return false;
    }
    let _ = super::write_line(
        &mut write,
        &Response::Welcome {
            protocol: super::protocol::PROTOCOL,
            installation: state.installation().unwrap_or_default(),
            build: String::new(),
            pid: std::process::id(),
        },
    )
    .await;
    let Ok(Some(line)) = super::read_line(&mut reader, MAX_LINE_BYTES).await else {
        return false;
    };
    matches!(
        serde_json::from_slice::<Request>(&line),
        Ok(Request::EmergencyStop)
    )
}

/// `brainiac-runner emergency-stop`: present the token and ask the guard to
/// stop containers. It does not remove them.
pub async fn emergency_stop(state: &StateDir) -> crate::models::AppResult<()> {
    use super::protocol::{Request, Response, MAX_LINE_BYTES, PROTOCOL};
    let token = state.token()?;
    let stream = tokio::net::UnixStream::connect(state.emergency_socket()).await?;
    let (read, mut write) = stream.into_split();
    let mut reader = tokio::io::BufReader::new(read);
    super::write_line(
        &mut write,
        &Request::Hello {
            token,
            protocol: PROTOCOL,
        },
    )
    .await?;
    let line = super::read_line(&mut reader, MAX_LINE_BYTES)
        .await?
        .ok_or_else(|| crate::models::AppError::dependency("The guard did not answer."))?;
    if !matches!(
        serde_json::from_slice::<Response>(&line),
        Ok(Response::Welcome { .. })
    ) {
        return Err(crate::models::AppError::dependency(
            "The guard refused the emergency stop.",
        ));
    }
    super::write_line(&mut write, &Request::EmergencyStop).await?;
    Ok(())
}

/// Stop the containers of every run not recorded as ended. Returns how many
/// runs the engine confirmed stopped.
pub async fn sweep<W: Workloads>(state: &StateDir, workloads: &W) -> usize {
    let Ok(installation) = state.installation() else {
        return 0;
    };
    let mut stopped = 0;
    for id in state.run_ids() {
        let Some(record) = state.load_run(&id) else {
            continue;
        };
        if record.phase == Phase::Ended {
            continue;
        }
        if workloads
            .stop(&record.engine_socket, &installation, &id)
            .await
            .is_ok()
        {
            stopped += 1;
        }
    }
    stopped
}
