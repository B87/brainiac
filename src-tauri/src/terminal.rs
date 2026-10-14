//! The terminal (SPEC.md, section 15; docs/architecture.md, Terminal): tabs
//! that each run the user's login shell, or the program named in Settings →
//! Terminal, in a pseudo-terminal of the app's process.
//!
//! The window owns the tabs' order and where they show (the Terminal view
//! or the dock); this service owns their programs. A tab's output goes to
//! the closure its opener passed, in batches (`pty::batch`); its end is the
//! `terminal_exited` event. Nothing a tab shows is stored. A feature: it
//! imports only the core.

pub mod environment;
pub mod pty;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use rusqlite::{params, OptionalExtension};
use tokio::sync::OnceCell;

use crate::db::Db;
use crate::events::{Emitter, FrontendEvent};
use crate::models::{
    AppError, AppResult, BusyTerminal, OpenTerminalRequest, TerminalExitedEvent, TerminalSettings,
    TerminalSettingsView, TerminalTab,
};

use self::pty::{Pty, Spawn};

/// The key of Settings → Terminal in `brainiac.db`'s settings.
const SETTINGS_KEY: &str = "terminal";

/// Its name in the window.
impl FrontendEvent for TerminalExitedEvent {
    fn name(&self) -> &'static str {
        "terminal_exited"
    }
}

/// The running tabs by ID. `Arc<Mutex<…>>`: the map is shared between the
/// service and each tab's waiting thread, which removes its tab when the
/// program ends; the `Mutex` lets one of them change it at a time. Each
/// `Pty` has its own `Mutex` because its terminal handle may be used from
/// one thread at a time only.
type Tabs = Arc<Mutex<HashMap<String, Arc<Mutex<Pty>>>>>;

pub struct TerminalService {
    core: Db,
    emitter: Emitter<TerminalExitedEvent>,
    tabs: Tabs,
    login_shell: PathBuf,
    /// The login shell's environment, read the first time a named program
    /// starts; `None` inside when it could not be read.
    login_environment: OnceCell<Option<Vec<(String, String)>>>,
}

impl TerminalService {
    pub fn new(core: Db, emitter: Emitter<TerminalExitedEvent>) -> Arc<Self> {
        Arc::new(TerminalService {
            core,
            emitter,
            tabs: Arc::default(),
            login_shell: environment::login_shell(),
            login_environment: OnceCell::new(),
        })
    }

    /// The same service with a given login shell, whose environment is
    /// given too (for tests, so no profile runs).
    pub fn with_login_shell(
        core: Db,
        emitter: Emitter<TerminalExitedEvent>,
        shell: PathBuf,
        login_environment: Vec<(String, String)>,
    ) -> Arc<Self> {
        Arc::new(TerminalService {
            core,
            emitter,
            tabs: Arc::default(),
            login_shell: shell,
            login_environment: OnceCell::new_with(Some(Some(login_environment))),
        })
    }

    pub async fn settings(&self) -> AppResult<TerminalSettings> {
        self.core
            .call(|conn| {
                let value: Option<String> = conn
                    .query_row(
                        "SELECT value_json FROM settings WHERE key = ?1",
                        [SETTINGS_KEY],
                        |r| r.get(0),
                    )
                    .optional()?;
                Ok(value
                    .and_then(|v| serde_json::from_str(&v).ok())
                    .unwrap_or_default())
            })
            .await
    }

    pub async fn settings_view(&self) -> AppResult<TerminalSettingsView> {
        Ok(TerminalSettingsView {
            settings: self.settings().await?,
            login_shell: self.login_shell.display().to_string(),
        })
    }

    /// Save Settings → Terminal. Tabs already open keep their program;
    /// turning the terminal off leaves them open until they are closed.
    pub async fn save_settings(&self, mut settings: TerminalSettings) -> AppResult<()> {
        settings.program = settings.program.trim().to_string();
        if settings.program.is_empty() {
            settings.args.clear();
        }
        let json = serde_json::to_string(&settings)?;
        self.core
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO settings (key, value_json, version) VALUES (?1, ?2, 1)
                     ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, version = version + 1",
                    params![SETTINGS_KEY, json],
                )?;
                Ok(())
            })
            .await
    }

    /// Open a tab in `request.cwd`, running the program Settings names.
    pub async fn open(
        &self,
        request: OpenTerminalRequest,
        output: pty::Output,
    ) -> AppResult<TerminalTab> {
        let settings = self.settings().await?;
        if !settings.enabled {
            return Err(AppError::validation(
                "The terminal is turned off in Settings → Terminal.",
            ));
        }
        let cwd = if request.cwd.is_empty() {
            home_folder()
        } else {
            PathBuf::from(&request.cwd)
        };
        if !cwd.is_dir() {
            return Err(AppError::not_found(format!(
                "{} is not there any more.",
                cwd.display()
            )));
        }
        let (program, args, env) = self.program(&settings).await?;
        let id = uuid::Uuid::new_v4().to_string();
        let name = program
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| program.display().to_string());

        let tabs = Arc::clone(&self.tabs);
        let emitter = Arc::clone(&self.emitter);
        let ended = id.clone();
        let exited: pty::Exited = Box::new(move |code| {
            if let Ok(mut tabs) = tabs.lock() {
                tabs.remove(&ended);
            }
            emitter(&TerminalExitedEvent { id: ended, code });
        });
        let pty = Pty::spawn(
            Spawn {
                program,
                args,
                cwd: cwd.clone(),
                env,
                cols: request.cols,
                rows: request.rows,
            },
            output,
            exited,
        )?;
        self.lock().insert(id.clone(), Arc::new(Mutex::new(pty)));
        Ok(TerminalTab {
            id,
            program: name,
            cwd: cwd.display().to_string(),
        })
    }

    /// The program, its arguments, and its environment for a new tab.
    async fn program(
        &self,
        settings: &TerminalSettings,
    ) -> AppResult<(PathBuf, Vec<String>, Vec<(String, String)>)> {
        if settings.program.is_empty() {
            // The login shell, as a login shell: it reads the user's profile
            // itself, so Brainiac's own environment is enough.
            let env = environment::for_tab(environment::own());
            return Ok((self.login_shell.clone(), vec!["-l".to_string()], env));
        }
        let env = environment::for_tab(
            self.login_environment()
                .await
                .unwrap_or_else(environment::own),
        );
        let program = environment::find_program(&settings.program, &env).ok_or_else(|| {
            AppError::not_found(format!(
                "{} was not found on your shell's PATH. Name it with its full path in Settings → Terminal.",
                settings.program
            ))
        })?;
        Ok((program, settings.args.clone(), env))
    }

    async fn login_environment(&self) -> Option<Vec<(String, String)>> {
        let shell = self.login_shell.clone();
        self.login_environment
            .get_or_init(
                || async move { environment::read(&shell, environment::READ_TIMEOUT).await },
            )
            .await
            .clone()
    }

    pub fn write(&self, id: &str, bytes: Vec<u8>) -> AppResult<()> {
        self.tab(id)?.lock().map_err(poisoned)?.write(bytes);
        Ok(())
    }

    pub fn resize(&self, id: &str, cols: u16, rows: u16) -> AppResult<()> {
        self.tab(id)?.lock().map_err(poisoned)?.resize(cols, rows)
    }

    /// The program running in front of the tab's own, which closing the tab
    /// would end; `None` when the tab's program is in front.
    pub async fn foreground(&self, id: &str) -> AppResult<Option<String>> {
        let tab = self.tab(id)?;
        // Naming a process may run `ps`, which waits on another program.
        tokio::task::spawn_blocking(move || tab.lock().map_err(poisoned).map(|p| p.foreground()))
            .await
            .map_err(|e| {
                AppError::io("Brainiac could not check the terminal.").with_details(e.to_string())
            })?
    }

    /// Every tab with a program in front, for the quit dialog.
    pub async fn busy(&self) -> Vec<BusyTerminal> {
        let tabs: Vec<(String, Arc<Mutex<Pty>>)> = self
            .lock()
            .iter()
            .map(|(id, pty)| (id.clone(), Arc::clone(pty)))
            .collect();
        tokio::task::spawn_blocking(move || {
            tabs.into_iter()
                .filter_map(|(id, pty)| {
                    let program = pty.lock().ok()?.foreground()?;
                    Some(BusyTerminal { id, program })
                })
                .collect()
        })
        .await
        .unwrap_or_default()
    }

    /// Close a tab: its program and whatever runs in front of it end.
    pub fn close(&self, id: &str) {
        let removed = self.lock().remove(id);
        if let Some(pty) = removed {
            if let Ok(pty) = pty.lock() {
                pty.close();
            }
        }
    }

    /// Close every tab, when Brainiac quits.
    pub fn close_all(&self) {
        let all: Vec<Arc<Mutex<Pty>>> = self.lock().drain().map(|(_, pty)| pty).collect();
        for pty in all {
            if let Ok(pty) = pty.lock() {
                pty.close();
            }
        }
    }

    fn tab(&self, id: &str) -> AppResult<Arc<Mutex<Pty>>> {
        self.lock()
            .get(id)
            .cloned()
            .ok_or_else(|| AppError::not_found("That terminal has closed."))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Arc<Mutex<Pty>>>> {
        // A panic while the map was held leaves it as it was; carry on.
        self.tabs.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Where a tab opens when no folder is given: the user's home folder.
fn home_folder() -> PathBuf {
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

fn poisoned<T>(_: std::sync::PoisonError<T>) -> AppError {
    AppError::io("The terminal stopped responding. Close the tab and open another.")
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::{Duration, Instant};

    use super::*;

    struct Fixture {
        _dir: tempfile::TempDir,
        service: Arc<TerminalService>,
        exited: Arc<Mutex<Vec<TerminalExitedEvent>>>,
        folder: PathBuf,
    }

    /// A service whose login shell is `/bin/sh`, with `PATH` set and no
    /// profile read.
    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let core = Db::open(&dir.path().join("brainiac.db")).unwrap();
        let exited = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&exited);
        let service = TerminalService::with_login_shell(
            core,
            Arc::new(move |e: &TerminalExitedEvent| sink.lock().unwrap().push(e.clone())),
            "/bin/sh".into(),
            vec![("PATH".into(), "/usr/bin:/bin".into())],
        );
        let folder = dir.path().join("widgets");
        std::fs::create_dir(&folder).unwrap();
        Fixture {
            _dir: dir,
            service,
            exited,
            folder,
        }
    }

    fn request(folder: &Path) -> OpenTerminalRequest {
        OpenTerminalRequest {
            cwd: folder.display().to_string(),
            cols: 80,
            rows: 24,
        }
    }

    fn collector() -> (pty::Output, Arc<Mutex<Vec<u8>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&seen);
        (
            Box::new(move |bytes: Vec<u8>| sink.lock().unwrap().extend(bytes)),
            seen,
        )
    }

    async fn until(done: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !done() {
            assert!(Instant::now() < deadline, "timed out");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    #[tokio::test]
    async fn settings_start_on_and_are_saved() {
        let f = fixture();
        assert_eq!(
            f.service.settings().await.unwrap(),
            TerminalSettings::default()
        );
        let named = TerminalSettings {
            program: "  herdr ".into(),
            args: vec!["session".into(), "attach".into(), "brainiac".into()],
            ..TerminalSettings::default()
        };
        f.service.save_settings(named).await.unwrap();
        let saved = f.service.settings().await.unwrap();
        assert_eq!(saved.program, "herdr");
        assert_eq!(saved.args.len(), 3);
        // No program, no arguments.
        f.service
            .save_settings(TerminalSettings {
                args: vec!["stray".into()],
                ..TerminalSettings::default()
            })
            .await
            .unwrap();
        assert!(f.service.settings().await.unwrap().args.is_empty());
    }

    #[tokio::test]
    async fn a_tab_runs_the_login_shell_in_its_folder() {
        let f = fixture();
        let (output, seen) = collector();
        let tab = f.service.open(request(&f.folder), output).await.unwrap();
        assert_eq!(tab.program, "sh");
        f.service.write(&tab.id, b"pwd; exit 4\r".to_vec()).unwrap();
        until(|| !f.exited.lock().unwrap().is_empty()).await;
        let text = String::from_utf8_lossy(&seen.lock().unwrap()).into_owned();
        assert!(text.contains("widgets"), "{text}");
        assert_eq!(
            f.exited.lock().unwrap()[0],
            TerminalExitedEvent {
                id: tab.id.clone(),
                code: Some(4)
            }
        );
        // An ended tab is gone.
        assert!(f.service.write(&tab.id, b"x".to_vec()).is_err());
    }

    #[tokio::test]
    async fn a_named_program_runs_with_its_arguments_and_the_terminal_variables() {
        let f = fixture();
        f.service
            .save_settings(TerminalSettings {
                program: "sh".into(),
                args: vec![
                    "-c".into(),
                    "printf '%s:%s' \"$TERM\" \"$1\"".into(),
                    "sh".into(),
                    "an argument".into(),
                ],
                ..TerminalSettings::default()
            })
            .await
            .unwrap();
        let (output, seen) = collector();
        let tab = f.service.open(request(&f.folder), output).await.unwrap();
        assert_eq!(tab.program, "sh");
        until(|| !f.exited.lock().unwrap().is_empty()).await;
        let text = String::from_utf8_lossy(&seen.lock().unwrap()).into_owned();
        assert!(text.contains("xterm-256color:an argument"), "{text}");
    }

    #[tokio::test]
    async fn a_missing_program_folder_or_switched_off_terminal_opens_nothing() {
        let f = fixture();
        let missing = f.folder.join("gone");
        let (output, _) = collector();
        assert!(f.service.open(request(&missing), output).await.is_err());

        f.service
            .save_settings(TerminalSettings {
                program: "no-such-program".into(),
                ..TerminalSettings::default()
            })
            .await
            .unwrap();
        let (output, _) = collector();
        let error = f
            .service
            .open(request(&f.folder), output)
            .await
            .unwrap_err();
        assert!(
            error.message.contains("no-such-program"),
            "{}",
            error.message
        );

        f.service
            .save_settings(TerminalSettings {
                enabled: false,
                ..TerminalSettings::default()
            })
            .await
            .unwrap();
        let (output, _) = collector();
        assert!(f.service.open(request(&f.folder), output).await.is_err());
    }

    #[tokio::test]
    async fn a_busy_tab_is_listed_and_closing_ends_its_program() {
        let f = fixture();
        f.service
            .save_settings(TerminalSettings {
                program: "sh".into(),
                args: vec!["-i".into()],
                ..TerminalSettings::default()
            })
            .await
            .unwrap();
        let (output, _) = collector();
        let tab = f.service.open(request(&f.folder), output).await.unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(f.service.foreground(&tab.id).await.unwrap(), None);
        assert!(f.service.busy().await.is_empty());

        f.service.write(&tab.id, b"sleep 30\r".to_vec()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while f.service.foreground(&tab.id).await.unwrap().is_none() {
            assert!(Instant::now() < deadline, "sleep never came to the front");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(
            f.service.busy().await,
            vec![BusyTerminal {
                id: tab.id.clone(),
                program: "sleep".into()
            }]
        );

        f.service.close(&tab.id);
        until(|| !f.exited.lock().unwrap().is_empty()).await;
        assert!(f.service.busy().await.is_empty());
    }
}
