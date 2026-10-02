//! The vault watcher on a real temporary vault (FSEvents on macOS): each
//! change ends with the index matching the vault and moved notes keeping
//! their IDs (docs/architecture.md, Required verification).

mod common;

use std::fs;
use std::process::Command;

use common::{note_id_at, wait_for, Harness};

/// The live note at `rel` once the watcher has caught up, or a timeout.
async fn eventually_at(h: &Harness, rel: &str) -> String {
    let vault = h.notes.vault().unwrap().id;
    let rel = rel.to_string();
    let core = h.core.clone();
    let found = std::sync::Arc::new(std::sync::Mutex::new(None::<String>));
    let slot = std::sync::Arc::clone(&found);
    wait_for(&format!("a note at {rel}"), || {
        let (core, vault, rel, slot) = (core.clone(), vault.clone(), rel.clone(), slot.clone());
        async move {
            let id: Option<String> = core
                .call(move |conn| {
                    Ok(conn
                        .query_row(
                            "SELECT id FROM notes WHERE vault_id = ?1 AND relative_path = ?2 AND missing_at IS NULL",
                            [&vault, &rel],
                            |r| r.get(0),
                        )
                        .ok())
                })
                .await
                .unwrap();
            let done = id.is_some();
            *slot.lock().unwrap() = id;
            done
        }
    })
    .await;
    let id = found.lock().unwrap().clone().unwrap();
    id
}

async fn live_count(h: &Harness) -> i64 {
    let vault = h.notes.vault().unwrap().id;
    h.core
        .call(move |conn| {
            Ok(conn.query_row(
                "SELECT COUNT(*) FROM notes WHERE vault_id = ?1 AND missing_at IS NULL",
                [&vault],
                |r| r.get(0),
            )?)
        })
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn an_edit_in_place_and_a_temporary_file_renamed_over_are_seen() {
    let h = Harness::new(true).await;
    h.write("Note.md", "first words\n");
    let id = eventually_at(&h, "Note.md").await;

    h.write("Note.md", "edited in place zebra\n");
    wait_for("the edit in search", || async {
        h.search_notes("zebra").await == vec!["Note.md"]
    })
    .await;

    // How many editors save: write a temporary file, then rename it over the note.
    fs::write(
        h.vault.join("Note.md.tmp-save"),
        "saved atomically giraffe\n",
    )
    .unwrap();
    fs::rename(h.vault.join("Note.md.tmp-save"), h.vault.join("Note.md")).unwrap();
    wait_for("the atomic save in search", || async {
        h.search_notes("giraffe").await == vec!["Note.md"]
    })
    .await;
    assert_eq!(note_id_at(&h, "Note.md").await, id);
    assert_eq!(live_count(&h).await, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_case_only_rename_and_a_folder_rename_keep_identities() {
    let h = Harness::new(true).await;
    h.write("Plan.md", "plan text\n");
    h.write("Area/One.md", "one\n");
    h.write("Area/Two.md", "two\n");
    let plan = eventually_at(&h, "Plan.md").await;
    let one = eventually_at(&h, "Area/One.md").await;
    eventually_at(&h, "Area/Two.md").await;

    fs::rename(h.vault.join("Plan.md"), h.vault.join("plan.md")).unwrap();
    assert_eq!(eventually_at(&h, "plan.md").await, plan);

    fs::rename(h.vault.join("Area"), h.vault.join("Renamed Area")).unwrap();
    assert_eq!(eventually_at(&h, "Renamed Area/One.md").await, one);
    eventually_at(&h, "Renamed Area/Two.md").await;
    wait_for("no duplicates", || async { live_count(&h).await == 3 }).await;
}

fn git(dir: &std::path::Path, args: &[&str]) {
    let out = Command::new("git")
        .args([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_git_checkout_that_renames_notes_keeps_their_ids() {
    let h = Harness::new(true).await;
    for i in 0..30 {
        h.write(
            &format!("notes/n{i}.md"),
            &format!("note number {i} with its own text\n"),
        );
    }
    git(&h.vault, &["init", "-q", "-b", "main"]);
    git(&h.vault, &["add", "."]);
    git(&h.vault, &["commit", "-q", "-m", "notes"]);
    git(&h.vault, &["checkout", "-q", "-b", "reorganized"]);
    git(&h.vault, &["mv", "notes", "archive"]);
    git(&h.vault, &["commit", "-q", "-m", "move"]);
    git(&h.vault, &["checkout", "-q", "main"]);
    let mut ids = Vec::new();
    for i in 0..30 {
        ids.push(eventually_at(&h, &format!("notes/n{i}.md")).await);
    }

    // Git deletes every old path before writing the new ones.
    git(&h.vault, &["checkout", "-q", "reorganized"]);
    for (i, id) in ids.iter().enumerate() {
        assert_eq!(
            &eventually_at(&h, &format!("archive/n{i}.md")).await,
            id,
            "note {i}"
        );
    }
    wait_for("no duplicates", || async { live_count(&h).await == 30 }).await;
    // The vault's Git state is untouched: nothing Brainiac wrote is in it.
    let status = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(&h.vault)
        .output()
        .unwrap();
    assert!(
        status.stdout.is_empty(),
        "{}",
        String::from_utf8_lossy(&status.stdout)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_vault_opened_through_a_symlink_still_receives_events() {
    let tmp = tempfile::tempdir().unwrap();
    let real = tmp.path().join("real-vault");
    fs::create_dir_all(&real).unwrap();
    let link = tmp.path().join("linked-vault");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let h = Harness::with_vault(tmp, link, true).await;
    fs::write(real.join("Linked.md"), "through the link\n").unwrap();
    eventually_at(&h, "Linked.md").await;
}
