//! Explaining changes (SPEC.md, section 14, Checks): a subject read from a
//! real repository, and an agent's file checked against it. Each test builds
//! its own repository with the system Git.

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use brainiac_lib::explain::check;
use brainiac_lib::explain::subject::{read_files, read_subject, Changed, EMPTY_TREE};
use brainiac_lib::git::GitService;
use serde_json::json;

const TIMEOUT: Duration = Duration::from_secs(30);

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn write(dir: &Path, rel: &str, content: &[u8]) {
    let path = dir.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

/// A repository whose second commit edits, renames, deletes, and adds a
/// binary file and a file with a space in its name. Returns both commits.
fn repository(dir: &Path) -> (String, String) {
    git(dir, &["init", "-q", "-b", "main"]);
    let mut lines: Vec<String> = (1..=20).map(|n| format!("line {n}")).collect();
    write(dir, "src/lib.rs", (lines.join("\n") + "\n").as_bytes());
    write(
        dir,
        "notes/old name.md",
        b"# Notes\n\nUnchanged text for the rename.\nMore of it.\nAnd more.\n",
    );
    write(dir, "gone.txt", b"bye\n");
    git(dir, &["add", "."]);
    git(dir, &["commit", "-q", "-m", "first"]);
    let first = git(dir, &["rev-parse", "HEAD"]);

    lines[4] = "line five, changed".to_string();
    lines.drain(9..11);
    lines.insert(15, "an added line".to_string());
    write(dir, "src/lib.rs", (lines.join("\n") + "\n").as_bytes());
    git(dir, &["mv", "notes/old name.md", "notes/new name.md"]);
    write(
        dir,
        "notes/new name.md",
        b"# Notes\n\nUnchanged text for the rename.\nMore of it.\nAnd more, edited.\n",
    );
    git(dir, &["rm", "-q", "gone.txt"]);
    write(dir, "logo.png", &[0x89, b'P', b'N', b'G', 0, 1, 2, 3]);
    git(dir, &["add", "."]);
    git(dir, &["commit", "-q", "-m", "second"]);
    (first, git(dir, &["rev-parse", "HEAD"]))
}

#[tokio::test]
async fn a_commit_is_read_as_its_changed_files_and_lines() {
    let tmp = tempfile::tempdir().unwrap();
    let (repo, home) = (tmp.path().join("repo"), tmp.path().join("home"));
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    let (first, second) = repository(&repo);
    let git = GitService::detect().await.unwrap();

    let subject = read_subject(&git, &repo, &home, &first, &second, TIMEOUT)
        .await
        .unwrap();
    let paths: Vec<&str> = subject.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(
        paths,
        ["gone.txt", "logo.png", "notes/new name.md", "src/lib.rs"]
    );
    assert!(subject.file("gone.txt").unwrap().deleted);
    assert!(subject.file("logo.png").unwrap().binary);
    let renamed = subject.file("notes/new name.md").unwrap();
    assert_eq!(renamed.old_path.as_deref(), Some("notes/old name.md"));
    assert_eq!(renamed.changed, vec![Changed { start: 5, count: 1 }]);
    assert_eq!(
        subject.file("src/lib.rs").unwrap().changed,
        vec![
            Changed { start: 5, count: 1 },
            Changed { start: 9, count: 0 },
            Changed {
                start: 16,
                count: 1
            },
        ]
    );

    // A root commit is read against the empty tree.
    let root = read_subject(&git, &repo, &home, EMPTY_TREE, &first, TIMEOUT)
        .await
        .unwrap();
    assert_eq!(root.files.len(), 3);
    assert_eq!(
        root.file("src/lib.rs").unwrap().changed,
        vec![Changed {
            start: 1,
            count: 20
        }]
    );

    // Text is read at the tip; binary, missing, and escaping paths are skipped.
    let wanted: Vec<String> = [
        "src/lib.rs",
        "logo.png",
        "gone.txt",
        "../outside",
        "notes/new name.md",
    ]
    .map(String::from)
    .to_vec();
    let files = read_files(&git, &repo, &home, &second, &wanted, TIMEOUT)
        .await
        .unwrap();
    let read: Vec<&str> = files.keys().map(String::as_str).collect();
    assert_eq!(read, ["notes/new name.md", "src/lib.rs"]);
    assert!(files["src/lib.rs"].contains("an added line"));
}

#[tokio::test]
async fn an_explanation_is_checked_against_a_real_commit() {
    let tmp = tempfile::tempdir().unwrap();
    let (repo, home) = (tmp.path().join("repo"), tmp.path().join("home"));
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    let (first, second) = repository(&repo);
    let git = GitService::detect().await.unwrap();

    let tour = ["src/lib.rs", "notes/new name.md", "gone.txt", "logo.png"]
        .map(|p| json!({ "path": p, "role": "" }));
    let file = json!({
        "summary": "Edits the library and its notes.",
        "tour": tour,
        "notes": [
            {
                "path": "src/lib.rs", "new_start": 5, "new_end": 5,
                "text": "Line five changes.",
                "sources": [
                    { "path": "src/lib.rs", "start": 1, "end": 1, "quote": "line five, changed" },
                    { "path": "notes/new name.md", "start": 5, "end": 5, "quote": "And more, edited." },
                ],
            },
            {
                "path": "src/lib.rs", "new_start": 9, "new_end": 9,
                "text": "Two lines go after line 9.",
                "sources": [{ "path": "src/lib.rs", "start": 9, "end": 9, "quote": "line 9" }],
            },
        ],
        "concepts": [],
    });
    let draft = check::parse(&file.to_string()).unwrap();
    let paths = draft.cited_paths();
    let files = read_files(&git, &repo, &home, &second, &paths, TIMEOUT)
        .await
        .unwrap();
    let subject = read_subject(&git, &repo, &home, &first, &second, TIMEOUT)
        .await
        .unwrap();
    let checked = check::check(draft, &subject, &files, &[], check::Attempt::First).unwrap();
    let notes = &checked.explanation.notes;
    assert_eq!(notes.len(), 2);
    assert_eq!(
        (notes[0].sources[0].start, notes[0].sources[0].moved),
        (5, true)
    );
    assert!(!notes[0].sources[1].moved);
    assert_eq!(checked.explanation.checks.moved, 1);

    // The same note on an unchanged line fails, naming the changed ones.
    let mut off = file.clone();
    off["notes"][0]["new_start"] = json!(2);
    off["notes"][0]["new_end"] = json!(3);
    let draft = check::parse(&off.to_string()).unwrap();
    let failed = check::check(draft, &subject, &files, &[], check::Attempt::First).unwrap_err();
    assert_eq!(
        failed.errors,
        ["notes[0] (src/lib.rs 2–3): no line there is changed on the new side. Changed lines in this file: 5, removed after 9, 16."]
    );
}
