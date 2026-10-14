//! A pseudo-terminal running one program (docs/architecture.md, Terminal).
//!
//! The program is started with an argument array and an environment given in
//! full, never through a shell of Brainiac's own. Three threads serve it:
//! one writes the keys the window sends, in order; one reads what the
//! program prints and hands it on in batches; one waits for the program to
//! end. None of them knows about Tauri: the caller passes closures.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};

use crate::models::{AppError, AppResult};

/// The most output sent to the window at once.
pub const BATCH_BYTES: usize = 64 * 1024;
/// How long output gathers before it is sent: about one frame.
pub const BATCH_WAIT: Duration = Duration::from_millis(16);
/// How long a program has to end after a hangup before it is killed.
pub const HANGUP_GRACE: Duration = Duration::from_secs(2);
/// How long output may still arrive after the program ended, before its
/// end is reported.
const EXIT_SETTLE: Duration = Duration::from_millis(100);

/// What to start.
#[derive(Debug, Clone)]
pub struct Spawn {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    /// The whole environment the program gets; nothing else is inherited.
    pub env: Vec<(String, String)>,
    pub cols: u16,
    pub rows: u16,
}

/// Receives the program's output, one batch at a time, on the reading thread.
pub type Output = Box<dyn FnMut(Vec<u8>) + Send>;
/// Called once when the program ends, with its exit code when it has one.
pub type Exited = Box<dyn FnOnce(Option<i32>) + Send>;

/// A running program in its pseudo-terminal. Dropping it closes the
/// terminal, which hangs up whatever still runs in it; `close` also makes
/// sure it ends.
pub struct Pty {
    master: Box<dyn MasterPty + Send>,
    /// Keys go to the writing thread through this queue, so they reach the
    /// program in the order they were typed even while it is slow to read.
    input: mpsc::Sender<Vec<u8>>,
    pid: Option<u32>,
}

impl Pty {
    pub fn spawn(spec: Spawn, output: Output, exited: Exited) -> AppResult<Pty> {
        let pair = native_pty_system()
            .openpty(size(spec.cols, spec.rows))
            .map_err(|e| pty_error("Brainiac could not open a terminal.", e))?;
        let mut command = CommandBuilder::new(&spec.program);
        command.args(&spec.args);
        command.cwd(&spec.cwd);
        command.env_clear();
        for (key, value) in &spec.env {
            command.env(key, value);
        }
        let mut child = pair.slave.spawn_command(command).map_err(|e| {
            pty_error(
                &format!("Brainiac could not start {}.", spec.program.display()),
                e,
            )
        })?;
        // The child holds its own copy of the terminal's other end. Keeping
        // ours open would keep the terminal open after the child ends, so
        // reading would never see the end.
        drop(pair.slave);
        let pid = child.process_id();
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| pty_error("Brainiac could not read the terminal.", e))?;
        let mut writer = pair
            .master
            .take_writer()
            .map_err(|e| pty_error("Brainiac could not write to the terminal.", e))?;

        let (input, keys) = mpsc::channel::<Vec<u8>>();
        spawn_thread("terminal-input", move || {
            // Ends when the `Pty` is dropped (the queue closes) or the
            // program stops reading for good.
            for bytes in keys {
                if writer
                    .write_all(&bytes)
                    .and_then(|()| writer.flush())
                    .is_err()
                {
                    break;
                }
            }
        })?;

        let (chunks, received) = mpsc::channel::<Vec<u8>>();
        spawn_thread("terminal-read", move || {
            let mut buffer = vec![0u8; 16 * 1024];
            loop {
                match reader.read(&mut buffer) {
                    // The end, or the terminal closed (Linux reports EIO).
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if chunks.send(buffer[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                }
            }
        })?;
        spawn_thread("terminal-output", move || batch(received, output))?;

        spawn_thread("terminal-wait", move || {
            let code = child.wait().ok().map(|status| status.exit_code() as i32);
            // A program's last lines may still be on their way; a program
            // it left running in the background may keep the terminal open
            // for ever, so its end is not waited for.
            thread::sleep(EXIT_SETTLE);
            exited(code);
        })?;

        Ok(Pty {
            master: pair.master,
            input,
            pid,
        })
    }

    /// Send keys or pasted text to the program.
    pub fn write(&self, bytes: Vec<u8>) {
        // Fails only once the program is gone, when nothing reads anyway.
        let _ = self.input.send(bytes);
    }

    pub fn resize(&self, cols: u16, rows: u16) -> AppResult<()> {
        self.master
            .resize(size(cols, rows))
            .map_err(|e| pty_error("Brainiac could not resize the terminal.", e))
    }

    /// The program the user is running in front of the tab's own program,
    /// such as `pnpm` in a shell: its name, or `None` when the tab's program
    /// is in front (a shell waiting at its prompt).
    pub fn foreground(&self) -> Option<String> {
        let front = self.foreground_group()?;
        if Some(front) == self.pid {
            return None;
        }
        Some(program_name(front).unwrap_or_else(|| "A program".to_string()))
    }

    /// Hang up the tab's program and whatever runs in front of it, and kill
    /// them if they are still there after `HANGUP_GRACE`, as closing a
    /// terminal app's tab does.
    pub fn close(&self) {
        let groups: Vec<u32> = [self.pid, self.foreground_group()]
            .into_iter()
            .flatten()
            .collect();
        signal_groups(&groups, libc::SIGHUP);
        let _ = spawn_thread("terminal-close", move || {
            thread::sleep(HANGUP_GRACE);
            signal_groups(&groups, libc::SIGKILL);
        });
    }

    fn foreground_group(&self) -> Option<u32> {
        self.master
            .process_group_leader()
            .and_then(|pid| u32::try_from(pid).ok())
    }
}

/// Gather output into batches: what arrives within `BATCH_WAIT` of the first
/// chunk, up to `BATCH_BYTES`, goes to the window as one message, so a flood
/// of output is a few messages a frame rather than one per read.
pub fn batch(chunks: mpsc::Receiver<Vec<u8>>, mut output: impl FnMut(Vec<u8>)) {
    while let Ok(first) = chunks.recv() {
        let deadline = Instant::now() + BATCH_WAIT;
        let mut gathered = first;
        let mut ended = false;
        while gathered.len() < BATCH_BYTES {
            match chunks.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(more) => gathered.extend_from_slice(&more),
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => {
                    ended = true;
                    break;
                }
            }
        }
        for piece in gathered.chunks(BATCH_BYTES) {
            output(piece.to_vec());
        }
        if ended {
            return;
        }
    }
}

/// Send a signal to each process group that still exists.
fn signal_groups(groups: &[u32], signal: libc::c_int) {
    for &group in groups {
        let Ok(group) = libc::pid_t::try_from(group) else {
            continue;
        };
        // `unsafe` because this calls the C library directly; `killpg`
        // reads nothing of ours, and a group that is gone only returns an
        // error, which is ignored.
        unsafe {
            if libc::killpg(group, 0) == 0 {
                libc::killpg(group, signal);
            }
        }
    }
}

/// A process's name: `/proc` on Linux, `ps` elsewhere (macOS).
pub fn program_name(pid: u32) -> Option<String> {
    if let Ok(name) = std::fs::read_to_string(format!("/proc/{pid}/comm")) {
        let name = name.trim();
        if !name.is_empty() {
            return Some(name.to_string());
        }
    }
    let out = std::process::Command::new("ps")
        .args(["-o", "comm=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let path = text.trim();
    // macOS gives the full path; a login shell's name starts with `-`.
    let name = path.rsplit('/').next()?.trim_start_matches('-');
    (!name.is_empty()).then(|| name.to_string())
}

fn size(cols: u16, rows: u16) -> PtySize {
    PtySize {
        rows: rows.max(1),
        cols: cols.max(1),
        pixel_width: 0,
        pixel_height: 0,
    }
}

fn spawn_thread(name: &str, f: impl FnOnce() + Send + 'static) -> AppResult<()> {
    thread::Builder::new()
        .name(name.to_string())
        .spawn(f)
        .map(|_| ())
        .map_err(|e| {
            AppError::io("Brainiac could not start the terminal.").with_details(e.to_string())
        })
}

fn pty_error(message: &str, error: impl std::fmt::Display) -> AppError {
    AppError::io(message).with_details(error.to_string())
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    /// What a test program printed, and how it ended.
    #[derive(Default)]
    struct Seen {
        output: Vec<u8>,
        batches: usize,
        exit: Option<Option<i32>>,
    }

    fn start(script: &str) -> (Pty, Arc<Mutex<Seen>>) {
        let seen = Arc::new(Mutex::new(Seen::default()));
        let (out, end) = (Arc::clone(&seen), Arc::clone(&seen));
        let pty = Pty::spawn(
            Spawn {
                program: "/bin/sh".into(),
                args: vec!["-c".into(), script.into()],
                cwd: std::env::temp_dir(),
                env: vec![("PATH".into(), "/usr/bin:/bin".into())],
                cols: 80,
                rows: 24,
            },
            Box::new(move |bytes| {
                let mut seen = out.lock().unwrap();
                seen.output.extend(bytes);
                seen.batches += 1;
            }),
            Box::new(move |code| end.lock().unwrap().exit = Some(code)),
        )
        .unwrap();
        (pty, seen)
    }

    /// Wait up to five seconds for `done` to hold.
    fn until(seen: &Arc<Mutex<Seen>>, done: impl Fn(&Seen) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if done(&seen.lock().unwrap()) {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        let seen = seen.lock().unwrap();
        panic!(
            "timed out; output so far: {:?}, exit: {:?}",
            String::from_utf8_lossy(&seen.output),
            seen.exit
        );
    }

    fn text(seen: &Arc<Mutex<Seen>>) -> String {
        String::from_utf8_lossy(&seen.lock().unwrap().output).into_owned()
    }

    #[test]
    fn a_program_prints_and_ends_with_its_code() {
        let (_pty, seen) = start("printf 'hello from the tab'; exit 3");
        until(&seen, |s| s.exit.is_some());
        assert!(text(&seen).contains("hello from the tab"));
        assert_eq!(seen.lock().unwrap().exit, Some(Some(3)));
    }

    #[test]
    fn keys_reach_the_program_in_order() {
        let (pty, seen) = start("read a; read b; printf 'got %s then %s' \"$a\" \"$b\"");
        pty.write(b"first\r".to_vec());
        pty.write(b"second\r".to_vec());
        until(&seen, |s| s.exit.is_some());
        assert!(
            text(&seen).contains("got first then second"),
            "{}",
            text(&seen)
        );
    }

    #[test]
    fn the_program_sees_its_size_change() {
        let (pty, seen) = start("read _; stty size");
        pty.resize(132, 40).unwrap();
        pty.write(b"\r".to_vec());
        until(&seen, |s| s.exit.is_some());
        assert!(text(&seen).contains("40 132"), "{}", text(&seen));
    }

    #[test]
    fn a_flood_of_output_arrives_whole_in_few_batches() {
        // 1,000,000 bytes: far more than one batch.
        let (_pty, seen) = start("head -c 1000000 /dev/zero | tr '\\0' 'x'");
        until(&seen, |s| s.exit.is_some());
        let seen = seen.lock().unwrap();
        let xs = seen.output.iter().filter(|&&b| b == b'x').count();
        assert_eq!(xs, 1_000_000);
        assert!(seen.batches < 200, "{} batches", seen.batches);
    }

    #[test]
    fn the_shell_at_its_prompt_is_not_busy_and_a_program_in_front_is() {
        // An interactive shell runs `sleep` in its own process group, in front.
        let (pty, seen) = start("exec /bin/sh -i");
        thread::sleep(Duration::from_millis(300));
        assert_eq!(pty.foreground(), None);
        pty.write(b"sleep 30\r".to_vec());
        let deadline = Instant::now() + Duration::from_secs(5);
        while pty.foreground().is_none() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(pty.foreground().as_deref(), Some("sleep"));
        pty.close();
        until(&seen, |s| s.exit.is_some());
    }

    #[test]
    fn closing_ends_a_program_that_ignores_the_hangup() {
        let (pty, seen) = start("trap '' HUP; printf ready; sleep 30");
        until(&seen, |s| s.output.ends_with(b"ready"));
        let closed = Instant::now();
        pty.close();
        until(&seen, |s| s.exit.is_some());
        assert!(closed.elapsed() >= HANGUP_GRACE - Duration::from_millis(100));
    }

    #[test]
    fn batches_split_at_the_limit_and_end_with_the_input() {
        let (send, receive) = mpsc::channel();
        send.send(vec![1u8; BATCH_BYTES]).unwrap();
        send.send(vec![2u8; 10]).unwrap();
        drop(send);
        let mut batches = Vec::new();
        batch(receive, |b| batches.push(b));
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0].len(), BATCH_BYTES);
        assert_eq!(batches[1], vec![2u8; 10]);
    }

    #[test]
    fn a_program_that_cannot_start_is_an_error() {
        let result = Pty::spawn(
            Spawn {
                program: "/nonexistent/program".into(),
                args: Vec::new(),
                cwd: std::env::temp_dir(),
                env: Vec::new(),
                cols: 80,
                rows: 24,
            },
            Box::new(|_| {}),
            Box::new(|_| {}),
        );
        assert!(result.is_err());
    }
}
