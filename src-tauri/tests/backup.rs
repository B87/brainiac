//! Export, restore, and the upgrade from 0.1.3 (SPEC.md, section 8;
//! docs/architecture.md, Required verification): tasks and associations
//! survive, and search is rebuilt independently.

mod common;

use std::fs;

use brainiac_lib::backup;
use brainiac_lib::db::{self, Db};
use brainiac_lib::models::{RestoreRequest, TaskFields, TaskStatus};
use brainiac_lib::notes::NoteService;
use brainiac_lib::tasks::TaskService;
use common::{note_id_at, Harness};
use rusqlite::Connection;

#[tokio::test(flavor = "multi_thread")]
async fn an_export_restores_on_another_mac_with_tasks_and_links() {
    let h = Harness::new(false).await;
    h.write("Projects/Plan.md", "# Plan\nship the parser\n");
    h.write("img/diagram.png", "png bytes");
    h.scan().await;
    let note = note_id_at(&h, "Projects/Plan.md").await;
    let repo = h.register_repository("parser").await;
    h.notes.link_repository(&note, &repo).await.unwrap();
    let task = h
        .tasks
        .create(TaskFields {
            title: "Review the parser".into(),
            description: "before Friday".into(),
            status: TaskStatus::InProgress,
            planned_date: Some("2026-10-02".into()),
            due_date: Some("2026-10-03".into()),
            note_id: Some(note.clone()),
            repository_id: Some(repo.clone()),
            sorted: false,
        })
        .await
        .unwrap();

    let exports = h.tmp.path().join("exports");
    fs::create_dir_all(&exports).unwrap();
    let result = backup::export(&h.notes, &exports).await.unwrap();
    assert!(result.complete, "{result:?}");
    assert_eq!((result.notes, result.tasks), (1, 1));
    let export = std::path::PathBuf::from(&result.path);
    assert!(export.join("vault/img/diagram.png").exists());
    let tasks_json = fs::read_to_string(export.join("tasks.json")).unwrap();
    assert!(
        tasks_json.contains("Review the parser")
            && tasks_json.contains("git@example.com:team/parser.git")
    );

    // Another Mac: its own data folder, the repository cloned elsewhere.
    let other = tempfile::tempdir().unwrap();
    let data = other.path().join("data");
    let core = Db::open(&data.join(db::CORE_FILE)).unwrap();
    let (notes, _) = Harness::service(&data, core.clone());
    notes.disable_watching();
    let clone_root = other.path().join("code").join("parser-clone");
    fs::create_dir_all(&clone_root).unwrap();
    let clone_id = uuid::Uuid::new_v4().to_string();
    {
        let row = db::RepositoryRow {
            id: clone_id.clone(),
            canonical_root: clone_root.display().to_string(),
            display_path: clone_root.display().to_string(),
            git_dir: clone_root.join(".git").display().to_string(),
            common_git_dir: clone_root.join(".git").display().to_string(),
            created_at: "2026-10-02T00:00:00.000Z".into(),
            last_opened_at: None,
            last_checked_at: None,
            last_tab: None,
            status: None,
            error: None,
            last_fetch_at: None,
            last_fetch_error: None,
            remote_url: None,
        };
        core.call(move |conn| {
            db::insert_repository(conn, &row)?;
            db::set_remote_url(conn, &row.id, Some("git@example.com:team/parser.git"))
        })
        .await
        .unwrap();
    }
    let preview = backup::preview(&export).unwrap();
    assert_eq!((preview.notes, preview.tasks), (1, 1));

    // The original repository's folder is gone on the new Mac.
    fs::remove_dir_all(h.tmp.path().join("repos")).unwrap();
    let restored_vault = other.path().join("Notes");
    let outcome = backup::restore(
        &notes,
        RestoreRequest {
            export_path: export.display().to_string(),
            vault_path: restored_vault.display().to_string(),
            use_existing_vault: false,
        },
    )
    .await
    .unwrap();
    assert_eq!(outcome.matched_repositories, 1);
    assert!(outcome.unmatched_repositories.is_empty());
    assert!(outcome.needs_restart);
    drop((notes, core));

    // The next launch applies it.
    assert!(backup::apply_pending_restore(&data).unwrap());
    let core = Db::open(&data.join(db::CORE_FILE)).unwrap();
    let (notes, _) = Harness::service(&data, core.clone());
    notes.disable_watching();
    notes.start().await.unwrap();
    notes
        .reconcile(brainiac_lib::vault::Scope::Full, false)
        .await
        .unwrap();
    let tasks = TaskService::new(std::sync::Arc::clone(&notes));
    let t = tasks.get(&task.id).await.unwrap();
    assert_eq!(t.status, TaskStatus::InProgress);
    assert_eq!(t.due_date.as_deref(), Some("2026-10-03"));
    assert_eq!(t.note.as_ref().map(|n| n.id.as_str()), Some(note.as_str()));
    let ctx = notes.context(&note).await.unwrap();
    assert_eq!(ctx.repositories.len(), 1);
    assert!(ctx.repositories[0].registered);
    // The restored registration points at the clone; the one made here stays registered.
    let roots: Vec<String> = core
        .call(|conn| {
            Ok(db::list_repositories(conn)?
                .into_iter()
                .map(|r| r.canonical_root)
                .collect())
        })
        .await
        .unwrap();
    assert_eq!(roots, vec![clone_root.display().to_string()]);
    assert_eq!(
        fs::read_to_string(restored_vault.join("Projects/Plan.md")).unwrap(),
        h.read("Projects/Plan.md")
    );
    let hits = brainiac_lib::index::search(
        &notes,
        brainiac_lib::models::SearchRequest {
            query: "parser".into(),
            kinds: None,
            limit: None,
        },
    )
    .await
    .unwrap();
    assert_eq!((hits.notes.total, hits.tasks.total), (1, 1));
    // The data the restore replaced was snapshotted first.
    let backups: Vec<String> = fs::read_dir(data.join("backups"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        backups.iter().any(|n| n.starts_with("pre-restore-")),
        "{backups:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_folder_that_is_not_an_export_is_refused_before_anything_changes() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = tmp.path().join("fake");
    fs::create_dir_all(&fake).unwrap();
    assert!(backup::preview(&fake).is_err());
    fs::write(fake.join("manifest.json"), r#"{"format":1,"app":"Brainiac","app_version":"9","exported_at":"x","schema_version":99,"vault":null,"notes":[],"repositories":[],"tasks":0,"problems":[],"complete":true}"#).unwrap();
    let err = backup::preview(&fake).unwrap_err();
    assert!(err.message.contains("newer version"), "{}", err.message);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_export_into_the_vault_is_refused() {
    let h = Harness::new(false).await;
    h.write("Plan.md", "# Plan\n");
    h.scan().await;
    fs::create_dir_all(h.vault.join("Backups")).unwrap();
    for parent in [h.vault.clone(), h.vault.join("Backups")] {
        let err = backup::export(&h.notes, &parent).await.unwrap_err();
        assert!(err.message.contains("outside the vault"), "{}", err.message);
    }
    let left: Vec<_> = fs::read_dir(h.vault.join("Backups")).unwrap().collect();
    assert!(left.is_empty(), "nothing is written into the vault");
}

/// A database exactly as 0.1.3 left it: the v0.1 schema with data in it.
fn v013_database(path: &std::path::Path) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(include_str!("../migrations/0001_init.sql"))
        .unwrap();
    conn.pragma_update(None, "application_id", db::APPLICATION_ID)
        .unwrap();
    conn.execute_batch(
        "PRAGMA user_version = 1;
         INSERT INTO repositories (id, canonical_root, display_path, git_dir, common_git_dir, created_at)
           VALUES ('r1', '/code/api', '/code/api', '/code/api/.git', '/code/api/.git', '2026-09-01T00:00:00Z');
         INSERT INTO workspaces (id, name, discovery_mode, created_at)
           VALUES ('w1', 'Work', 'manual', '2026-09-01T00:00:00Z');
         INSERT INTO workspace_members (id, workspace_id, display_name, canonical_path, origin, repository_id)
           VALUES ('m1', 'w1', 'api', '/code/api', 'manual', 'r1');
         INSERT INTO pins (entity_type, entity_id, position) VALUES ('repository', 'r1', 1);
         INSERT INTO settings (key, value_json) VALUES ('refresh_interval_seconds', '90');",
    )
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn migration_0002_upgrades_a_0_1_3_database_and_keeps_its_data() {
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let path = data.join(db::CORE_FILE);
    v013_database(&path);

    let core = Db::open(&path).unwrap();
    let (version, repos, members, pins, interval): (u32, i64, i64, i64, String) = core
        .call(|conn| {
            Ok((
                db::schema_version(conn)?,
                conn.query_row("SELECT COUNT(*) FROM repositories", [], |r| r.get(0))?,
                conn.query_row("SELECT COUNT(*) FROM workspace_members", [], |r| r.get(0))?,
                conn.query_row("SELECT COUNT(*) FROM pins", [], |r| r.get(0))?,
                conn.query_row(
                    "SELECT value_json FROM settings WHERE key = 'refresh_interval_seconds'",
                    [],
                    |r| r.get(0),
                )?,
            ))
        })
        .await
        .unwrap();
    assert_eq!(version, db::SCHEMA_VERSION);
    assert_eq!((repos, members, pins, interval.as_str()), (1, 1, 1, "90"));
    let names: Vec<String> = fs::read_dir(data.join("backups"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        names.iter().any(|n| n.starts_with("pre-migration-v1-")),
        "{names:?}"
    );

    // The new tables work on the upgraded file.
    let (notes, _) = Harness::service(&data, core.clone());
    notes.disable_watching();
    let vault = tmp.path().join("vault");
    fs::create_dir_all(&vault).unwrap();
    fs::write(vault.join("a.md"), "hello\n").unwrap();
    notes
        .select_vault(vault.to_str().unwrap(), false)
        .await
        .unwrap();
    notes
        .reconcile(brainiac_lib::vault::Scope::Full, false)
        .await
        .unwrap();
    let tasks = TaskService::new(std::sync::Arc::clone(&notes));
    tasks
        .create(TaskFields {
            title: "after the upgrade".into(),
            description: String::new(),
            status: TaskStatus::Todo,
            planned_date: None,
            due_date: None,
            note_id: None,
            repository_id: Some("r1".into()),
            sorted: false,
        })
        .await
        .unwrap();
    let _: &NoteService = &notes;

    // A 0.1.3 build would refuse the upgraded file instead of misreading it.
    let user_version: u32 = Connection::open(&path)
        .unwrap()
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert!(user_version > 1);
}
