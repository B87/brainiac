//! Opening the database safely: refusing files Brainiac cannot use, marking
//! its own, and never leaving a partial backup behind.

use std::path::Path;
use std::time::Duration;

use brainiac_lib::db::{Db, APPLICATION_ID, SCHEMA_VERSION};
use rusqlite::Connection;

/// A second connection to the file, as another process or an older copy of the app would have.
fn raw(path: &Path) -> Connection {
    let conn = Connection::open(path).unwrap();
    conn.busy_timeout(Duration::from_secs(5)).unwrap();
    conn
}

fn application_id(path: &Path) -> i32 {
    raw(path)
        .query_row("PRAGMA application_id", [], |r| r.get(0))
        .unwrap()
}

fn user_version(path: &Path) -> u32 {
    raw(path)
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap()
}

fn open_error(path: &Path) -> brainiac_lib::models::AppError {
    match Db::open(path) {
        Ok(_) => panic!("{} should not open", path.display()),
        Err(e) => e,
    }
}

fn backup_names(path: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(path.parent().unwrap().join("backups"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[tokio::test]
async fn a_new_database_and_its_backups_are_marked_as_brainiac() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("data").join("brainiac.sqlite3");
    let db = Db::open(&path).unwrap();
    assert_eq!(application_id(&path), APPLICATION_ID);

    let names = backup_names(&path);
    assert_eq!(names.len(), 1, "{names:?}");
    assert!(names[0].starts_with("daily-") && names[0].ends_with(".sqlite3"));

    let copy = db
        .backup_to(tmp.path().join("export").join("copy.sqlite3"))
        .await
        .unwrap();
    assert_eq!(application_id(&copy), APPLICATION_ID);
    let export_dir: Vec<_> = std::fs::read_dir(copy.parent().unwrap()).unwrap().collect();
    assert_eq!(
        export_dir.len(),
        1,
        "no temporary file is left next to the copy"
    );
}

#[test]
fn a_database_from_a_newer_version_is_refused_and_left_untouched() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("brainiac.sqlite3");
    drop(Db::open(&path).unwrap());
    let newer = SCHEMA_VERSION + 1;
    raw(&path)
        .pragma_update(None, "user_version", newer)
        .unwrap();

    let err = open_error(&path);
    assert!(
        err.message.contains("newer version of Brainiac"),
        "{}",
        err.message
    );
    assert!(
        err.message.contains("backups"),
        "names where the snapshots are"
    );
    let details = err.details.unwrap();
    assert!(details.contains(&newer.to_string()) && details.contains(&SCHEMA_VERSION.to_string()));
    assert_eq!(user_version(&path), newer, "no migration ran");
}

#[test]
fn a_database_from_another_program_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("other.sqlite3");
    let other = raw(&path);
    other.pragma_update(None, "application_id", 42).unwrap();
    other
        .execute("CREATE TABLE notes (id INTEGER PRIMARY KEY)", [])
        .unwrap();
    drop(other);

    let err = open_error(&path);
    assert!(
        err.message.contains("is not a Brainiac database"),
        "{}",
        err.message
    );
    assert_eq!(user_version(&path), 0, "no migration ran");
}

#[test]
fn a_database_from_before_the_marker_is_adopted() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("brainiac.sqlite3");
    drop(Db::open(&path).unwrap());
    raw(&path).pragma_update(None, "application_id", 0).unwrap();

    drop(Db::open(&path).unwrap());
    assert_eq!(application_id(&path), APPLICATION_ID);
}

#[test]
fn an_interrupted_backup_is_cleaned_up_and_never_counts_as_one() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("brainiac.sqlite3");
    let backups = tmp.path().join("backups");
    std::fs::create_dir_all(&backups).unwrap();
    let today = chrono::Local::now().format("%Y-%m-%d");
    // What a crash halfway through today's backup leaves behind.
    std::fs::write(
        backups.join(format!(".daily-{today}.sqlite3.partial")),
        b"half",
    )
    .unwrap();

    drop(Db::open(&path).unwrap());
    assert_eq!(backup_names(&path), vec![format!("daily-{today}.sqlite3")]);
}
