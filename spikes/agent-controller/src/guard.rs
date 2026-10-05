//! Stops labeled containers when the controller process is gone. It never
//! removes them: a stopped container and its volume are the run's work.
//! It is a separate process because a controller that has been `kill -9`'d
//! cannot run its own cleanup. The workload cannot see the emergency socket:
//! it is mode 600 on the host and is not mounted into the container.

use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

use crate::engine::Engine;
use crate::state::{pid_alive, StateDir};

pub async fn serve(state: StateDir) -> anyhow::Result<()> {
    state.ensure()?;
    let engine = Engine::connect()?;
    let sock = state.sock_path();
    prepare_socket(&sock)?;
    let listener = UnixListener::bind(&sock)?;
    fs::set_permissions(&sock, fs::Permissions::from_mode(0o600))?;
    eprintln!("guard watching {}", state.pid_path().display());
    loop {
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(500)) => {
                if controller_dead(&state) {
                    stop(&engine, &state, "controller dead").await;
                }
            }
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                let engine = engine.clone();
                let state = StateDir::new(state.root());
                tokio::spawn(async move {
                    let _ = emergency(stream, &engine, &state).await;
                });
            }
        }
    }
}

pub async fn emergency_stop(state: StateDir) -> anyhow::Result<()> {
    let mut stream = UnixStream::connect(state.sock_path()).await?;
    stream.write_all(b"stop\n").await?;
    let mut response = String::new();
    BufReader::new(stream).read_line(&mut response).await?;
    if response.trim() != "stopped" {
        anyhow::bail!("emergency stop was not acknowledged");
    }
    println!("emergency stop acknowledged");
    Ok(())
}

fn prepare_socket(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

fn controller_dead(state: &StateDir) -> bool {
    match state.controller_pid() {
        Some(pid) => !pid_alive(pid),
        // No pid file means the controller is not running. Leftover
        // containers still belong to it and must not keep running.
        None => true,
    }
}

async fn stop(engine: &Engine, state: &StateDir, reason: &str) {
    match engine.stop_labeled().await {
        Ok(0) => {}
        Ok(count) => append_log(state, &format!("stopped {count} ({reason})")),
        Err(err) => append_log(state, &format!("stop failed ({reason}): {err}")),
    }
}

fn append_log(state: &StateDir, line: &str) {
    use std::io::Write;
    if let Ok(mut file) = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(state.guard_log())
    {
        let _ = writeln!(file, "{line}");
    }
    eprintln!("{line}");
}

async fn emergency(stream: UnixStream, engine: &Engine, state: &StateDir) -> anyhow::Result<()> {
    let (read, mut write) = stream.into_split();
    let mut line = String::new();
    BufReader::new(read).read_line(&mut line).await?;
    if line.trim() != "stop" {
        write.write_all(b"rejected\n").await?;
        return Ok(());
    }
    stop(engine, state, "emergency stop").await;
    write.write_all(b"stopped\n").await?;
    Ok(())
}
