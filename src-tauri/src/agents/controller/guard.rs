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
    let mut child = std::process::Command::new(std::env::current_exe()?)
        .arg("runner")
        .arg("guard")
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
    loop {
        tokio::select! {
            _ = tokio::time::sleep(WATCH_EVERY) => {}
            _ = async {
                match terminate.as_mut() {
                    Some(signal) => { signal.recv().await; }
                    None => std::future::pending::<()>().await,
                }
            } => {}
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
