//! Notes on a temporary vault: the save contract, reconciliation and note
//! identity, the trash, links between notes, and links to repositories
//! (SPEC.md, section 5; docs/architecture.md, Required verification).

mod common;

use std::fs;

use brainiac_lib::models::{
    CreateNoteRequest, ErrorCode, NoteChangeOrigin, NoteTextState, RenameNoteRequest,
    RevisionReason, SaveNoteRequest,
};
use brainiac_lib::notes::KnowledgeEvent;
use brainiac_lib::vault::Scope;
use common::{note_id_at, Harness};

#[tokio::test(flavor = "multi_thread")]
async fn a_save_writes_only_the_text_and_keeps_the_previous_version() {
    let h = Harness::new(false).await;
    h.write("Plan.md", "# Plan\r\n\r\nFirst draft");
    h.scan().await;
    let id = note_id_at(&h, "Plan.md").await;

    let content = h.notes.read(&id).await.unwrap();
    assert_eq!(content.text.as_deref(), Some("# Plan\r\n\r\nFirst draft"));
    let saved = h
        .notes
        .save(SaveNoteRequest {
            note_id: id.clone(),
            expected_version: content.version.clone(),
            text: "# Plan\r\n\r\nSecond draft".into(),
        })
        .await
        .unwrap();
    assert!(!saved.search_pending);
    assert_eq!(h.read("Plan.md"), "# Plan\r\n\r\nSecond draft");
    assert_ne!(saved.version, content.version);
    // Saving never writes the note's ID or anything else Brainiac did not get as text.
    assert!(!h.read("Plan.md").contains("brainiac_id"));
    assert_eq!(h.leftover_files(), Vec::<String>::new());

    let revisions = h.notes.revisions(&id).await.unwrap();
    assert_eq!(revisions.len(), 1);
    assert_eq!(revisions[0].reason, RevisionReason::AppSave);
    assert_eq!(
        h.notes.revision_text(&revisions[0].id).await.unwrap(),
        "# Plan\r\n\r\nFirst draft"
    );

    // Autosaves within ten minutes count as one version.
    h.notes
        .save(SaveNoteRequest {
            note_id: id.clone(),
            expected_version: saved.version.clone(),
            text: "# Plan\r\n\r\nThird draft".into(),
        })
        .await
        .unwrap();
    assert_eq!(h.notes.revisions(&id).await.unwrap().len(), 1);
    assert!(h.saw(|e| matches!(e, KnowledgeEvent::NoteChanged(c) if c.note_id == id && c.origin == NoteChangeOrigin::App)));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_save_never_overwrites_a_version_it_has_not_seen() {
    let h = Harness::new(false).await;
    h.write("Note.md", "original\n");
    h.scan().await;
    let id = note_id_at(&h, "Note.md").await;
    let content = h.notes.read(&id).await.unwrap();

    h.write("Note.md", "changed in another editor\n");
    let err = h
        .notes
        .save(SaveNoteRequest {
            note_id: id.clone(),
            expected_version: content.version.clone(),
            text: "my edit\n".into(),
        })
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Conflict);
    assert_eq!(h.read("Note.md"), "changed in another editor\n");
    // The draft is kept for Compare…, Reload from Disk, and Save Draft as Copy.
    let again = h.notes.read(&id).await.unwrap();
    let draft = again.draft.expect("draft kept");
    assert_eq!(draft.text, "my edit\n");
    assert_eq!(draft.base_version, content.version);

    let copy = h.notes.save_copy(&id, draft.text.clone()).await.unwrap();
    assert_eq!(copy.relative_path, "Note (copy).md");
    assert_eq!(h.read("Note (copy).md"), "my edit\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_outside_change_is_reindexed_and_kept_as_a_revision() {
    let h = Harness::new(false).await;
    h.write("Log.md", "before the pull\n");
    h.scan().await;
    let id = note_id_at(&h, "Log.md").await;

    // A scan with nothing changed reads no note.
    let stats = h.notes.reconcile(Scope::Full, false).await.unwrap();
    assert_eq!(stats.read, 0, "{stats:?}");

    h.write("Log.md", "after the pull\n");
    let stats = h.notes.reconcile(Scope::Full, false).await.unwrap();
    assert_eq!((stats.read, stats.changed), (1, 1), "{stats:?}");
    let revisions = h.notes.revisions(&id).await.unwrap();
    assert_eq!(revisions[0].reason, RevisionReason::ExternalChange);
    assert_eq!(
        h.notes.revision_text(&revisions[0].id).await.unwrap(),
        "before the pull\n"
    );
    assert_eq!(h.search_notes("pull").await, vec!["Log.md"]);
    assert!(h.search_notes("before").await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_folder_renamed_outside_keeps_its_notes_and_links() {
    let h = Harness::new(false).await;
    h.write(
        "Projects/Alpha/Plan.md",
        "# Plan\nSee [[Spec]] and [notes](Notes.md).\n",
    );
    h.write("Projects/Alpha/Spec.md", "# Spec\n");
    h.write("Projects/Alpha/Notes.md", "# Notes\nunique text one\n");
    h.write("Index.md", "[plan](Projects/Alpha/Plan.md)\n");
    h.scan().await;
    let plan = note_id_at(&h, "Projects/Alpha/Plan.md").await;
    let spec = note_id_at(&h, "Projects/Alpha/Spec.md").await;
    assert_eq!(h.notes.context(&spec).await.unwrap().backlinks.len(), 1);

    fs::rename(
        h.vault.join("Projects/Alpha"),
        h.vault.join("Projects/Beta"),
    )
    .unwrap();
    let stats = h.notes.reconcile(Scope::Full, false).await.unwrap();
    assert_eq!(
        (stats.moved, stats.added, stats.missing),
        (3, 0, 0),
        "{stats:?}"
    );
    assert_eq!(note_id_at(&h, "Projects/Beta/Plan.md").await, plan);
    assert_eq!(note_id_at(&h, "Projects/Beta/Spec.md").await, spec);
    // Links inside the folder resolve again; the one into it is now unresolved.
    let ctx = h.notes.context(&spec).await.unwrap();
    assert_eq!(ctx.backlinks.len(), 1);
    let index_id = note_id_at(&h, "Index.md").await;
    let ctx = h.notes.context(&index_id).await.unwrap();
    assert_eq!(ctx.unresolved.len(), 1);
    assert_eq!(ctx.unresolved[0].suggested_path, "Projects/Alpha/Plan.md");
}

#[tokio::test(flavor = "multi_thread")]
async fn identity_follows_brainiac_id_then_unique_content() {
    let h = Harness::new(false).await;
    h.write("a.md", "---\nbrainiac_id: fixed-id\n---\nfirst\n");
    h.write("b.md", "same text\n");
    h.write("c.md", "same text\n");
    h.write("d.md", "unique\n");
    h.scan().await;
    let (a, b, d) = (
        note_id_at(&h, "a.md").await,
        note_id_at(&h, "b.md").await,
        note_id_at(&h, "d.md").await,
    );

    // Moved and edited: found by brainiac_id.
    fs::remove_file(h.vault.join("a.md")).unwrap();
    h.write("moved/a2.md", "---\nbrainiac_id: fixed-id\n---\nedited\n");
    // Two notes with the same text moved: identical content alone proves nothing.
    fs::rename(h.vault.join("b.md"), h.vault.join("b2.md")).unwrap();
    fs::rename(h.vault.join("c.md"), h.vault.join("c2.md")).unwrap();
    // Moved unchanged with unique content: same note.
    fs::rename(h.vault.join("d.md"), h.vault.join("d2.md")).unwrap();
    h.notes.reconcile(Scope::Full, false).await.unwrap();

    assert_eq!(note_id_at(&h, "moved/a2.md").await, a);
    assert_eq!(note_id_at(&h, "d2.md").await, d);
    assert_ne!(note_id_at(&h, "b2.md").await, b);
    let missing = h.notes.summary(&b).await.unwrap();
    assert!(missing.missing && !missing.trashed);
    assert!(h.saw(|e| matches!(e, KnowledgeEvent::NoteMissing(m) if m.note_id == b)));
    // The text of a note deleted outside is kept for Restore as New File.
    assert_eq!(
        h.notes.read(&b).await.unwrap().text.as_deref(),
        Some("same text\n")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_copied_note_is_a_conflict_not_a_merge() {
    let h = Harness::new(false).await;
    h.write("one.md", "---\nbrainiac_id: shared\n---\nx\n");
    h.scan().await;
    fs::copy(h.vault.join("one.md"), h.vault.join("two.md")).unwrap();
    h.notes.reconcile(Scope::Full, false).await.unwrap();
    let one = h
        .notes
        .summary(&note_id_at(&h, "one.md").await)
        .await
        .unwrap();
    let two = h
        .notes
        .summary(&note_id_at(&h, "two.md").await)
        .await
        .unwrap();
    assert_ne!(one.id, two.id);
    assert!(one.id_conflict && two.id_conflict);
}

#[tokio::test(flavor = "multi_thread")]
async fn large_and_non_utf8_notes_are_found_by_name() {
    let h = Harness::new(false).await;
    let big = "word ".repeat(1_100_000);
    h.write("Huge log.md", &big);
    fs::write(h.vault.join("Latin.md"), [0x63, 0x61, 0x66, 0xe9]).unwrap();
    h.write("attachment.png", "png");
    h.scan().await;
    let huge = h
        .notes
        .summary(&note_id_at(&h, "Huge log.md").await)
        .await
        .unwrap();
    assert_eq!(huge.text_state, NoteTextState::TooLarge);
    let latin = h
        .notes
        .summary(&note_id_at(&h, "Latin.md").await)
        .await
        .unwrap();
    assert_eq!(latin.text_state, NoteTextState::NotUtf8);
    assert_eq!(h.search_notes("huge").await, vec!["Huge log.md"]);
    assert!(h.search_notes("word").await.is_empty());
    assert_eq!(h.search_notes("latin").await, vec!["Latin.md"]);
    let listing = h.notes.list_folder(None).await.unwrap();
    let names: Vec<&str> = listing.entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["attachment.png", "Huge log.md", "Latin.md"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unreadable_vault_is_unavailable_not_deleted() {
    let h = Harness::new(false).await;
    h.write("keep.md", "x\n");
    h.scan().await;
    let id = note_id_at(&h, "keep.md").await;
    let moved = h.tmp.path().join("unmounted");
    fs::rename(&h.vault, &moved).unwrap();
    let stats = h.notes.reconcile(Scope::Full, false).await.unwrap();
    assert!(stats.unavailable);
    assert!(!h.notes.summary(&id).await.unwrap().missing);
    fs::rename(&moved, &h.vault).unwrap();
    h.notes.reconcile(Scope::Full, false).await.unwrap();
    assert!(!h.notes.summary(&id).await.unwrap().missing);
}

#[tokio::test(flavor = "multi_thread")]
async fn notes_are_created_renamed_trashed_and_restored() {
    let h = Harness::new(false).await;
    h.write("Index.md", "See [[Draft]] and [d](Ideas/Draft.md).\n");
    h.scan().await;
    let note = h
        .notes
        .create(CreateNoteRequest {
            folder: Some("Ideas".into()),
            title: Some("Draft".into()),
            repository_id: None,
        })
        .await
        .unwrap();
    assert_eq!(note.relative_path, "Ideas/Draft.md");
    assert!(h
        .read("Ideas/Draft.md")
        .starts_with(&format!("---\nbrainiac_id: {}\n---\n", note.id)));
    let index = note_id_at(&h, "Index.md").await;
    assert_eq!(
        h.notes.context(&note.id).await.unwrap().backlinks[0]
            .note
            .id,
        index
    );

    let preview = h
        .notes
        .preview_rename(&note.id, "Ideas/Final.md")
        .await
        .unwrap();
    assert_eq!(preview.linking_notes.len(), 1);
    let renamed = h
        .notes
        .rename(RenameNoteRequest {
            note_id: note.id.clone(),
            new_path: "Ideas/Final.md".into(),
            update_links: true,
        })
        .await
        .unwrap();
    assert_eq!(renamed.updated, vec!["Index.md"]);
    assert_eq!(
        h.read("Index.md"),
        "See [[Final]] and [d](Ideas/Final.md).\n"
    );
    assert_eq!(h.notes.context(&note.id).await.unwrap().backlinks.len(), 1);

    h.notes.trash(&note.id).await.unwrap();
    assert!(!h.vault.join("Ideas/Final.md").exists());
    assert_eq!(h.search_notes("final").await, vec!["Index.md"]);
    assert_eq!(h.notes.list_trash().await.unwrap().len(), 1);
    // Nothing of the trash is in the vault.
    assert_eq!(h.leftover_files(), Vec::<String>::new());

    h.write("Ideas/Final.md", "someone else\n");
    h.notes.reconcile(Scope::Full, false).await.unwrap();
    let err = h.notes.restore(&note.id, false).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Conflict);
    let restored = h.notes.restore(&note.id, true).await.unwrap();
    assert!(!restored.missing && !restored.trashed);
    assert!(h.read("Ideas/Final.md").contains(&note.id));
}

#[tokio::test(flavor = "multi_thread")]
async fn linking_a_repository_writes_the_note_id_once() {
    let h = Harness::new(false).await;
    h.write("Deploy.md", "# Deploy\nWe deploy billing on Fridays.\n");
    h.scan().await;
    let id = note_id_at(&h, "Deploy.md").await;
    let repo = h.register_repository("billing").await;

    let ctx = h.notes.context(&id).await.unwrap();
    assert_eq!(ctx.suggestions.len(), 1);
    h.notes.link_repository(&id, &repo).await.unwrap();
    let text = h.read("Deploy.md");
    assert_eq!(
        text,
        format!("---\nbrainiac_id: {id}\n---\n# Deploy\nWe deploy billing on Fridays.\n")
    );
    let ctx = h.notes.context(&id).await.unwrap();
    assert_eq!(ctx.repositories.len(), 1);
    assert!(ctx.suggestions.is_empty());
    let tab = h.notes.repository_notes(&repo).await.unwrap();
    assert_eq!(tab.notes.len(), 1);

    // A rename outside keeps the link through brainiac_id even with edits.
    fs::remove_file(h.vault.join("Deploy.md")).unwrap();
    h.write("Ops/Deploy steps.md", &format!("{text}More.\n"));
    h.notes.reconcile(Scope::Full, false).await.unwrap();
    assert_eq!(note_id_at(&h, "Ops/Deploy steps.md").await, id);
    assert_eq!(h.notes.context(&id).await.unwrap().repositories.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn notes_of_another_vault_are_missing_until_it_is_chosen_again() {
    let h = Harness::new(false).await;
    h.write("First.md", "first vault\n");
    h.scan().await;
    let id = note_id_at(&h, "First.md").await;
    let other = h.tmp.path().join("other-vault");
    fs::create_dir_all(&other).unwrap();
    fs::write(
        other.join("First.md"),
        "a different note with the same name\n",
    )
    .unwrap();

    h.notes
        .select_vault(other.to_str().unwrap(), false)
        .await
        .unwrap();
    h.scan().await;
    let summary = h.notes.summary(&id).await.unwrap();
    assert!(
        summary.missing,
        "a note of the previous vault counts as missing"
    );
    // It is never read from the new vault's folder.
    assert!(h.notes.read(&id).await.unwrap().text.is_none());

    h.notes
        .select_vault(h.vault.to_str().unwrap(), false)
        .await
        .unwrap();
    h.scan().await;
    assert!(!h.notes.summary(&id).await.unwrap().missing);
    assert_eq!(
        h.notes.read(&id).await.unwrap().text.as_deref(),
        Some("first vault\n")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_missing_note_is_restored_as_a_new_file_or_relinked() {
    let h = Harness::new(false).await;
    h.write("Gone.md", "precious text\n");
    h.write("Other.md", "other\n");
    h.scan().await;
    let gone = note_id_at(&h, "Gone.md").await;
    let task = h
        .tasks
        .create(brainiac_lib::models::TaskFields {
            title: "keep me".into(),
            description: String::new(),
            status: brainiac_lib::models::TaskStatus::Todo,
            planned_date: None,
            due_date: None,
            note_id: Some(gone.clone()),
            repository_id: None,
            sorted: false,
        })
        .await
        .unwrap();

    fs::remove_file(h.vault.join("Gone.md")).unwrap();
    h.scan().await;
    assert!(h.tasks.get(&task.id).await.unwrap().note.unwrap().missing);
    let restored = h
        .notes
        .recreate(&gone, "precious text\n".into())
        .await
        .unwrap();
    assert_eq!(restored.relative_path, "Gone.md");
    assert!(!h.tasks.get(&task.id).await.unwrap().note.unwrap().missing);

    // Relink: the note's file was renamed beyond recognition.
    fs::remove_file(h.vault.join("Gone.md")).unwrap();
    h.write("Renamed.md", "rewritten entirely\n");
    h.scan().await;
    let relinked = h.notes.relink(&gone, "Renamed.md").await.unwrap();
    assert_eq!(relinked.id, gone);
    assert_eq!(note_id_at(&h, "Renamed.md").await, gone);
    // A note with its own context cannot be swallowed by a relink.
    let other = note_id_at(&h, "Other.md").await;
    h.notes
        .link_repository(&other, &h.register_repository("svc").await)
        .await
        .unwrap();
    fs::remove_file(h.vault.join("Renamed.md")).unwrap();
    h.scan().await;
    let err = h.notes.relink(&gone, "Other.md").await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Conflict);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_note_created_in_a_folder_spelled_differently_is_recorded_once() {
    let h = Harness::new(false).await;
    h.write("Projects/Existing.md", "x\n");
    h.scan().await;
    let note = h
        .notes
        .create(CreateNoteRequest {
            folder: Some("projects".into()),
            title: Some("Roadmap".into()),
            repository_id: None,
        })
        .await
        .unwrap();
    assert_eq!(note.relative_path, "Projects/Roadmap.md");
    h.scan().await;
    assert_eq!(note_id_at(&h, "Projects/Roadmap.md").await, note.id);
    assert!(!h.notes.summary(&note.id).await.unwrap().missing);
}

#[tokio::test(flavor = "multi_thread")]
async fn brainiac_id_wins_over_a_path_reused_by_another_note() {
    let h = Harness::new(false).await;
    h.write("a.md", "---\nbrainiac_id: original\n---\nmine\n");
    h.write("Template.md", "---\nbrainiac_id: template\n---\ntemplate\n");
    h.scan().await;
    let original = note_id_at(&h, "a.md").await;
    fs::rename(h.vault.join("a.md"), h.vault.join("b.md")).unwrap();
    fs::copy(h.vault.join("Template.md"), h.vault.join("a.md")).unwrap();
    h.scan().await;
    assert_eq!(note_id_at(&h, "b.md").await, original);
    assert_ne!(note_id_at(&h, "a.md").await, original);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_note_that_comes_back_to_its_path_keeps_its_id() {
    let h = Harness::new(false).await;
    h.write("n.md", "no id here\n");
    h.scan().await;
    let id = note_id_at(&h, "n.md").await;
    fs::remove_file(h.vault.join("n.md")).unwrap();
    h.scan().await;
    assert!(h.notes.summary(&id).await.unwrap().missing);
    h.write("n.md", "no id here, edited\n");
    h.scan().await;
    assert_eq!(note_id_at(&h, "n.md").await, id);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unreadable_folder_does_not_count_as_deleted() {
    use std::os::unix::fs::PermissionsExt;
    let h = Harness::new(false).await;
    h.write("sub/n.md", "inside\n");
    h.scan().await;
    let id = note_id_at(&h, "sub/n.md").await;
    fs::set_permissions(h.vault.join("sub"), fs::Permissions::from_mode(0o000)).unwrap();
    let stats = h.notes.reconcile(Scope::Full, false).await.unwrap();
    fs::set_permissions(h.vault.join("sub"), fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(stats.missing, 0);
    assert!(!h.notes.summary(&id).await.unwrap().missing);
}

#[tokio::test(flavor = "multi_thread")]
async fn rebuilding_the_index_keeps_every_link_between_notes() {
    let h = Harness::new(false).await;
    // More notes than one indexing batch, each linking to the next.
    for i in 0..450 {
        h.write(
            &format!("n{i:03}.md"),
            &format!("# Note {i}\nsee [[n{:03}]]\n", (i + 1) % 450),
        );
    }
    h.scan().await;
    let target = note_id_at(&h, "n000.md").await;
    assert_eq!(h.notes.context(&target).await.unwrap().backlinks.len(), 1);
    h.notes.rebuild_search().await.unwrap();
    let mut resolved = 0;
    for i in [0, 199, 200, 201, 399, 449] {
        let id = note_id_at(&h, &format!("n{i:03}.md")).await;
        resolved += h.notes.context(&id).await.unwrap().backlinks.len();
    }
    assert_eq!(resolved, 6);
}
