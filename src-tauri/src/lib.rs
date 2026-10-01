//! Tauri setup: shared state, native menu, background loops, command registration.

pub mod activity;
pub mod commands;
pub mod db;
pub mod fetcher;
pub mod git;
pub mod models;
pub mod watcher;
pub mod workspaces;

use std::sync::Arc;
use std::time::Duration;

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{Emitter, Manager, WindowEvent};

use crate::models::{ChangeOrigin, MenuEvent, RepositoryChangedEvent};
use crate::workspaces::RepositoryService;

/// Event names shared with the frontend.
pub const EVENT_REPOSITORY_CHANGED: &str = "repository_changed";
pub const EVENT_MENU: &str = "menu";

/// Minimum age of an observation before focus/wake triggers a refresh.
const ACTIVATION_MIN_AGE: Duration = Duration::from_secs(10);
/// How often auto-fetch and the morning digest check whether something is due.
const SCHEDULE_TICK: Duration = Duration::from_secs(60);

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    init_tracing();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        // Self-update from GitHub releases; `process` provides the relaunch after install.
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            let handle = app.handle().clone();

            // --- Persistence -------------------------------------------------
            let data_dir = app.path().app_data_dir()?;
            let db_path = data_dir.join("brainiac.sqlite3");
            let db = db::Db::open(&db_path)?;
            tracing::info!(path = %db_path.display(), "database ready");
            let settings = db.call_blocking(|conn| db::load_settings(conn))?;

            // --- Git ---------------------------------------------------------
            let git = tauri::async_runtime::block_on(git::GitService::detect());
            match &git {
                Ok(g) => tracing::info!(version = g.version(), "git detected"),
                Err(e) => tracing::warn!(error = %e, "git unavailable"),
            }

            // --- Core service ------------------------------------------------
            let emit_handle = handle.clone();
            let emitter: workspaces::Emitter = Arc::new(move |event: RepositoryChangedEvent| {
                if let Err(e) = emit_handle.emit(EVENT_REPOSITORY_CHANGED, &event) {
                    tracing::warn!(error = %e, "failed to emit repository_changed");
                }
            });
            let service = Arc::new(RepositoryService::new(db, git, settings.clone(), emitter));
            let notify_handle = handle.clone();
            service.set_notifier(Arc::new(move |title: String, body: String| {
                use tauri_plugin_notification::NotificationExt;
                if let Err(e) = notify_handle.notification().builder().title(title).body(body).show() {
                    tracing::warn!(error = %e, "could not show a notification");
                }
            }));
            app.manage(Arc::clone(&service));

            // --- Watcher and background loops --------------------------------
            let (fs_watcher, rx) = watcher::RepositoryWatcher::new()?;
            for row in service.db().call_blocking(|conn| db::list_repositories(conn))? {
                let root = std::path::PathBuf::from(&row.canonical_root);
                if root.is_dir() {
                    if let Err(e) = fs_watcher.watch(&row.id, &root, std::path::Path::new(&row.git_dir)) {
                        tracing::warn!(repository = %row.display_path, error = %e, "could not watch repository");
                    }
                }
            }
            app.manage(fs_watcher);

            let debounce_service = Arc::clone(&service);
            tauri::async_runtime::spawn(watcher::run_debounce(rx, debounce_service, watcher::DEBOUNCE));

            let startup_service = Arc::clone(&service);
            tauri::async_runtime::spawn(async move {
                let _ = startup_service.request_refresh_all(ChangeOrigin::Wake, Duration::ZERO).await;
            });

            let timer_service = Arc::clone(&service);
            let interval = Duration::from_secs(settings.refresh_interval_seconds.max(10));
            tauri::async_runtime::spawn(async move {
                let mut ticker = tokio::time::interval(interval);
                ticker.tick().await; // first tick fires immediately; skip it
                loop {
                    ticker.tick().await;
                    let _ = timer_service.request_refresh_all(ChangeOrigin::Timer, interval / 2).await;
                }
            });

            // Opt-in auto-fetch and the morning digest (SPEC §7).
            let schedule_service = Arc::clone(&service);
            tauri::async_runtime::spawn(async move {
                let mut ticker = tokio::time::interval(SCHEDULE_TICK);
                loop {
                    ticker.tick().await;
                    schedule_service.auto_fetch_tick().await;
                    schedule_service.digest_tick().await;
                }
            });

            // --- Native menu -------------------------------------------------
            build_menu(app)?;
            let menu_handle = handle.clone();
            app.on_menu_event(move |_app, event| {
                let _ = menu_handle.emit(EVENT_MENU, MenuEvent { id: event.id().0.clone() });
            });

            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::Focused(true) = event {
                let service = window.state::<commands::Service>().inner().clone();
                tauri::async_runtime::spawn(async move {
                    let _ = service.request_refresh_all(ChangeOrigin::Wake, ACTIVATION_MIN_AGE).await;
                });
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_app_snapshot,
            commands::register_repository,
            commands::remove_repository,
            commands::refresh_repository,
            commands::open_repository,
            commands::set_repository_tab,
            commands::list_changes,
            commands::get_diff,
            commands::list_commits,
            commands::get_commit,
            commands::list_refs,
            commands::open_in_editor,
            commands::reveal_in_finder,
            commands::discover_repositories,
            commands::create_workspace,
            commands::update_workspace_membership,
            commands::rename_workspace,
            commands::remove_workspace,
            commands::set_pinned,
            commands::fetch_repository,
            commands::get_workspace_activity,
            commands::get_team_pulse,
            commands::mark_activity_seen,
            commands::update_activity_settings,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Brainiac");
}

fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,brainiac_lib=debug"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .try_init();
}

fn build_menu(app: &tauri::App) -> tauri::Result<()> {
    let open = MenuItem::with_id(
        app,
        "open_repository",
        "Add Repository or Workspace…",
        true,
        Some("CmdOrCtrl+O"),
    )?;
    let refresh = MenuItem::with_id(app, "refresh", "Refresh", true, Some("CmdOrCtrl+R"))?;
    let fetch = MenuItem::with_id(app, "fetch", "Fetch Now", true, None::<&str>)?;
    let palette = MenuItem::with_id(
        app,
        "palette",
        "Switch Repository or Workspace…",
        true,
        Some("CmdOrCtrl+K"),
    )?;
    let settings = MenuItem::with_id(app, "settings", "Settings…", true, Some("CmdOrCtrl+,"))?;
    let check_updates = MenuItem::with_id(
        app,
        "check_updates",
        "Check for Updates…",
        true,
        None::<&str>,
    )?;

    let app_menu = Submenu::with_items(
        app,
        "Brainiac",
        true,
        &[
            &PredefinedMenuItem::about(app, Some("About Brainiac"), None)?,
            &check_updates,
            &PredefinedMenuItem::separator(app)?,
            &settings,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::services(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::hide(app, None)?,
            &PredefinedMenuItem::hide_others(app, None)?,
            &PredefinedMenuItem::show_all(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::quit(app, None)?,
        ],
    )?;
    let file = Submenu::with_items(
        app,
        "File",
        true,
        &[
            &open,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::close_window(app, None)?,
        ],
    )?;
    let edit = Submenu::with_items(
        app,
        "Edit",
        true,
        &[
            &PredefinedMenuItem::undo(app, None)?,
            &PredefinedMenuItem::redo(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::cut(app, None)?,
            &PredefinedMenuItem::copy(app, None)?,
            &PredefinedMenuItem::paste(app, None)?,
            &PredefinedMenuItem::select_all(app, None)?,
        ],
    )?;
    let view = Submenu::with_items(
        app,
        "View",
        true,
        &[
            &refresh,
            &fetch,
            &palette,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::fullscreen(app, None)?,
        ],
    )?;
    let window = Submenu::with_items(
        app,
        "Window",
        true,
        &[
            &PredefinedMenuItem::minimize(app, None)?,
            &PredefinedMenuItem::maximize(app, None)?,
        ],
    )?;
    let menu = Menu::with_items(app, &[&app_menu, &file, &edit, &view, &window])?;
    app.set_menu(menu)?;
    Ok(())
}
