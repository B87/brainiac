//! Tauri setup: shared state, native menu, background loops, command registration.

pub mod activity;
pub mod backup;
pub mod commands;
pub mod db;
pub mod fetcher;
pub mod forge;
pub mod git;
pub mod index;
pub mod mcp;
pub mod models;
pub mod notes;
pub mod tasks;
pub mod vault;
pub mod watcher;
pub mod workspaces;

use std::sync::Arc;
use std::time::Duration;

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{Emitter, Manager, WindowEvent};

use crate::models::{AppError, ChangeOrigin, MenuEvent, RepositoryChangedEvent};
use crate::notes::{KnowledgeEvent, NoteService};
use crate::tasks::TaskService;
use crate::workspaces::RepositoryService;

/// Event names shared with the frontend.
pub const EVENT_REPOSITORY_CHANGED: &str = "repository_changed";
pub const EVENT_MENU: &str = "menu";
pub const EVENT_NOTE_CHANGED: &str = "note_changed";
pub const EVENT_NOTE_MISSING: &str = "note_missing";
pub const EVENT_TASK_CHANGED: &str = "task_changed";
pub const EVENT_INDEX_STATUS_CHANGED: &str = "index_status_changed";
pub const EVENT_PR_CHANGED: &str = "pr_changed";

/// Minimum age of a vault scan before focus or wake triggers another.
const VAULT_ACTIVATION_MIN_AGE: Duration = Duration::from_secs(30);

/// Minimum age of an observation before focus/wake triggers a refresh.
const ACTIVATION_MIN_AGE: Duration = Duration::from_secs(10);
/// How often auto-fetch and the morning digest check whether something is due.
const SCHEDULE_TICK: Duration = Duration::from_secs(60);

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let context = tauri::generate_context!();
    // `brainiac mcp` is the agents' stdio helper, not the app (SPEC.md, section 9).
    // It must start before logging, because its stdout carries the protocol.
    if std::env::args().nth(1).as_deref() == Some("mcp") {
        std::process::exit(mcp::helper::run(&context.config().identifier));
    }
    init_tracing();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        // Self-update from GitHub releases; `process` provides the relaunch after install.
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_notification::init())
        // Vault images for Live Preview: `vault://localhost/<path in the vault>`
        // serves image files inside the active vault and nothing else.
        .register_asynchronous_uri_scheme_protocol("vault", |ctx, request, responder| {
            let app = ctx.app_handle().clone();
            tauri::async_runtime::spawn(async move {
                responder.respond(vault_image(&app, request.uri().path()));
            });
        })
        .setup(|app| {
            let handle = app.handle().clone();

            // --- Persistence -------------------------------------------------
            let data_dir = app.path().app_data_dir()?;
            let db_path = data_dir.join(db::CORE_FILE);
            // A restore staged before the last quit replaces the data now,
            // before anything opens it (SPEC.md, Backup and restore).
            if let Err(e) = backup::apply_pending_restore(&data_dir) {
                tracing::error!(error = %e, details = ?e.details, "cannot apply the restore");
                show_startup_error(app, &e);
                return Ok(());
            }
            let db = match db::Db::open(&db_path) {
                Ok(db) => db,
                Err(e) => {
                    tracing::error!(error = %e, details = ?e.details, "cannot open the database");
                    show_startup_error(app, &e);
                    return Ok(());
                }
            };
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
            let service = Arc::new(RepositoryService::new(db.clone(), git, settings.clone(), emitter));
            let notify_handle = handle.clone();
            service.set_notifier(Arc::new(move |title: String, body: String| {
                use tauri_plugin_notification::NotificationExt;
                if let Err(e) = notify_handle.notification().builder().title(title).body(body).show() {
                    tracing::warn!(error = %e, "could not show a notification");
                }
            }));
            app.manage(Arc::clone(&service));

            // --- Pull request accounts (v0.3) -------------------------------
            let accounts = forge::AccountService::new(
                db.clone(),
                Arc::new(forge::keychain::MacKeychain),
                forge::http::Http::new()?,
                forge::Endpoints::production(),
            );
            let accounts = Arc::new(accounts);
            app.manage(Arc::clone(&accounts));
            let forge_cache = match forge::PullRequestService::open_cache(&data_dir) {
                Ok(db) => db,
                Err(e) => {
                    tracing::error!(error = %e, details = ?e.details, "cannot open the pull request cache");
                    show_startup_error(app, &e);
                    return Ok(());
                }
            };
            let pr_handle = handle.clone();
            let pr_emitter: forge::PullRequestEmitter = Arc::new(move |event| {
                if let Err(e) = pr_handle.emit(EVENT_PR_CHANGED, &event) {
                    tracing::warn!(error = %e, "failed to emit pr_changed");
                }
            });
            let pull_requests = Arc::new(forge::PullRequestService::new(
                Arc::clone(&service),
                accounts,
                db.clone(),
                forge_cache,
                forge::http::Http::new()?,
                forge::Endpoints::production(),
                pr_emitter,
            ));
            app.manage(Arc::clone(&pull_requests));
            // A fetch that moves a pull request's branch refreshes it at once
            // (SPEC.md, Staying up to date), off the fetch's own task.
            let moved_handle = Arc::clone(&pull_requests);
            service.set_fetch_listener(Arc::new(move |result| {
                let pull_requests = Arc::clone(&moved_handle);
                tauri::async_runtime::spawn(async move {
                    pull_requests.branches_moved(&result).await;
                });
            }));
            // Workspace lists every five minutes while Brainiac is open (SPEC.md, Staying up to date).
            tauri::async_runtime::spawn(async move {
                let mut ticker = tokio::time::interval(SCHEDULE_TICK);
                loop {
                    ticker.tick().await;
                    pull_requests.sync_tick().await;
                }
            });

            // --- Notes, tasks, and search (v0.2) ----------------------------
            let stores = match notes::Stores::open(&data_dir, db) {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!(error = %e, details = ?e.details, "cannot open the notes databases");
                    show_startup_error(app, &e);
                    return Ok(());
                }
            };
            let knowledge_handle = handle.clone();
            let knowledge_emitter: notes::KnowledgeEmitter = Arc::new(move |event: KnowledgeEvent| {
                let sent = match &event {
                    KnowledgeEvent::NoteChanged(e) => knowledge_handle.emit(EVENT_NOTE_CHANGED, e),
                    KnowledgeEvent::NoteMissing(e) => knowledge_handle.emit(EVENT_NOTE_MISSING, e),
                    KnowledgeEvent::TaskChanged(e) => knowledge_handle.emit(EVENT_TASK_CHANGED, e),
                    KnowledgeEvent::IndexStatus(e) => knowledge_handle.emit(EVENT_INDEX_STATUS_CHANGED, e),
                };
                if let Err(e) = sent {
                    tracing::warn!(error = %e, "failed to emit a notes event");
                }
            });
            let notes = NoteService::new(stores, data_dir.clone(), knowledge_emitter);
            notes.set_write_note_ids(settings.write_note_ids);
            app.manage(Arc::clone(&notes));
            let tasks = Arc::new(TaskService::new(Arc::clone(&notes)));
            app.manage(Arc::clone(&tasks));

            // --- Agent access (v0.2.x) --------------------------------------
            let agent = mcp::AgentServer::new(
                Arc::clone(&notes),
                tasks,
                Arc::clone(&service),
                settings.agent_access,
            );
            let socket = mcp::socket_path(&data_dir, &app.config().identifier);
            let listening = Arc::clone(&agent);
            tauri::async_runtime::spawn(async move {
                match listening.bind(&socket).await {
                    Ok(listener) => {
                        tracing::info!(path = %socket.display(), "agent socket ready");
                        listening.accept(listener).await;
                    }
                    Err(e) => tracing::warn!(error = %e, details = ?e.details, "agent access unavailable"),
                }
            });
            app.manage(agent);
            // Know the vault before the window asks for it; its scan runs in the background.
            if let Err(e) = tauri::async_runtime::block_on(notes.start()) {
                tracing::warn!(error = %e, "could not open the vault");
            }

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

            // Opt-in auto-fetch and the morning digest (SPEC.md, Workspaces and repositories).
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
                // Absent only when startup failed and the error dialog is showing.
                let Some(service) = window.try_state::<commands::Service>() else {
                    return;
                };
                let service = service.inner().clone();
                tauri::async_runtime::spawn(async move {
                    let _ = service.request_refresh_all(ChangeOrigin::Wake, ACTIVATION_MIN_AGE).await;
                });
                if let Some(notes) = window.try_state::<commands::Notes>() {
                    let notes = notes.inner().clone();
                    tauri::async_runtime::spawn(async move {
                        notes.request_full_reconcile(VAULT_ACTIVATION_MIN_AGE);
                    });
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_app_snapshot,
            commands::register_repository,
            commands::remove_repository,
            commands::relocate_repository,
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
            commands::rescan_workspace,
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
            commands::update_settings,
            commands::get_agent_access_status,
            commands::get_vault_state,
            commands::select_vault,
            commands::list_folder,
            commands::get_note_lists,
            commands::read_note,
            commands::mark_note_opened,
            commands::save_note,
            commands::create_note,
            commands::preview_rename,
            commands::rename_note,
            commands::follow_note_title,
            commands::trash_note,
            commands::list_trash,
            commands::restore_note,
            commands::recreate_note,
            commands::relink_note,
            commands::list_revisions,
            commands::read_revision,
            commands::restore_revision,
            commands::save_draft,
            commands::discard_draft,
            commands::save_draft_as_copy,
            commands::open_vault_file,
            commands::reveal_vault_path,
            commands::get_note_context,
            commands::resolve_link,
            commands::get_repository_notes,
            commands::link_repository,
            commands::unlink_repository,
            commands::dismiss_suggestion,
            commands::reconnect_repository,
            commands::list_tasks,
            commands::get_task,
            commands::get_today,
            commands::create_task,
            commands::update_task,
            commands::delete_task,
            commands::search,
            commands::rebuild_search,
            commands::export_backup,
            commands::preview_restore,
            commands::restore_backup,
            commands::list_forge_accounts,
            commands::save_forge_account,
            commands::remove_forge_account,
            commands::set_repository_forge,
            commands::update_workspace_pull_requests,
            commands::list_pull_requests,
            commands::get_pull_request,
            commands::list_pull_request_files,
            commands::get_pull_request_checks,
            commands::get_pull_request_conversation,
            commands::get_pull_request_diff,
            commands::get_pull_request_repository,
            commands::list_review_drafts,
            commands::save_review_draft,
            commands::delete_review_draft,
            commands::move_review_drafts,
            commands::comment_on_pull_request,
            commands::reply_to_thread,
            commands::resolve_thread,
            commands::submit_review,
            commands::get_merge_options,
            commands::merge_pull_request,
            commands::get_review_counts,
        ])
        .build(context)
        .expect("error while running Brainiac")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                if let Some(agent) = app.try_state::<commands::Agent>() {
                    agent.close();
                }
            }
        });
}

/// Image types Live Preview shows from the vault.
const VAULT_IMAGE_TYPES: &[(&str, &str)] = &[
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("gif", "image/gif"),
    ("webp", "image/webp"),
    ("svg", "image/svg+xml"),
    ("avif", "image/avif"),
    ("bmp", "image/bmp"),
];

/// Serve an image file of the active vault for `vault://localhost/<path>`.
/// Anything else, including paths that leave the vault, is a 404.
fn vault_image(app: &tauri::AppHandle, uri_path: &str) -> tauri::http::Response<Vec<u8>> {
    let not_found = || {
        tauri::http::Response::builder()
            .status(404)
            .body(Vec::new())
            .expect("static response")
    };
    let Some(notes) = app.try_state::<commands::Notes>() else {
        return not_found();
    };
    let relative = percent_decode_path(uri_path.trim_start_matches('/'));
    let Some(mime) = relative
        .rsplit('.')
        .next()
        .and_then(|ext| {
            VAULT_IMAGE_TYPES
                .iter()
                .find(|(e, _)| e.eq_ignore_ascii_case(ext))
        })
        .map(|(_, m)| *m)
    else {
        return not_found();
    };
    match notes
        .absolute(&relative)
        .and_then(|p| Ok(std::fs::read(p)?))
    {
        Ok(bytes) => tauri::http::Response::builder()
            .status(200)
            .header("Content-Type", mime)
            // An SVG may not run scripts or load anything.
            .header(
                "Content-Security-Policy",
                "default-src 'none'; style-src 'unsafe-inline'",
            )
            .body(bytes)
            .unwrap_or_else(|_| not_found()),
        Err(_) => not_found(),
    }
}

fn percent_decode_path(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Some(v) = std::str::from_utf8(&b[i + 1..i + 3])
                .ok()
                .and_then(|h| u8::from_str_radix(h, 16).ok())
            {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Tell the user why Brainiac cannot start and quit when they dismiss it.
/// The main window stays hidden: without the database no command can run.
fn show_startup_error(app: &tauri::App, error: &AppError) {
    use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
    let text = match &error.details {
        Some(details) => format!("{}\n\n{}", error.message, details),
        None => error.message.clone(),
    };
    let handle = app.handle().clone();
    app.dialog()
        .message(text)
        .title("Brainiac can't open its data")
        .kind(MessageDialogKind::Error)
        .show(move |_| handle.exit(1));
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
        "Search or Switch…",
        true,
        Some("CmdOrCtrl+K"),
    )?;
    let settings = MenuItem::with_id(app, "settings", "Settings…", true, Some("CmdOrCtrl+,"))?;
    let new_note = MenuItem::with_id(app, "new_note", "New Note", true, Some("CmdOrCtrl+N"))?;
    let new_task = MenuItem::with_id(app, "new_task", "New Task", true, Some("CmdOrCtrl+Shift+N"))?;
    let save = MenuItem::with_id(app, "save", "Save", true, Some("CmdOrCtrl+S"))?;
    let export = MenuItem::with_id(app, "export", "Export…", true, None::<&str>)?;
    let restore = MenuItem::with_id(app, "restore", "Restore from Export…", true, None::<&str>)?;
    let toggle_source = MenuItem::with_id(
        app,
        "toggle_source",
        "Switch Live Preview / Source",
        true,
        Some("CmdOrCtrl+Shift+E"),
    )?;
    let toggle_context = MenuItem::with_id(
        app,
        "toggle_context",
        "Show or Hide Context Panel",
        true,
        Some("Alt+CmdOrCtrl+0"),
    )?;
    let show_today = MenuItem::with_id(app, "show_today", "Today", true, None::<&str>)?;
    let show_tasks = MenuItem::with_id(app, "show_tasks", "Tasks", true, None::<&str>)?;
    let show_notes = MenuItem::with_id(app, "show_notes", "Notes", true, None::<&str>)?;
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
            &new_note,
            &new_task,
            &open,
            &PredefinedMenuItem::separator(app)?,
            &save,
            &PredefinedMenuItem::separator(app)?,
            &export,
            &restore,
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
            &show_today,
            &show_tasks,
            &show_notes,
            &PredefinedMenuItem::separator(app)?,
            &refresh,
            &fetch,
            &palette,
            &PredefinedMenuItem::separator(app)?,
            &toggle_source,
            &toggle_context,
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
