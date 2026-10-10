//! Integration tests against real temporary repositories.
//! Each test builds its own fixture with the system Git; nothing is checked in.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use brainiac_lib::db::Db;
use brainiac_lib::git::GitService;
use brainiac_lib::models::*;
use brainiac_lib::workspaces::RepositoryService;

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

fn write(dir: &Path, rel: &str, content: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

/// A repository with two commits and a clean tree.
fn fixture_repo(dir: &Path) {
    git(dir, &["init", "-q", "-b", "main"]);
    write(dir, "a.txt", "one\ntwo\nthree\n");
    write(dir, "b.txt", "b\n");
    git(dir, &["add", "."]);
    git(dir, &["commit", "-q", "-m", "first commit"]);
    write(dir, "a.txt", "one\ntwo\nthree\nfour\n");
    git(dir, &["commit", "-q", "-am", "second commit: add four"]);
}

async fn service_for(
    tmp: &Path,
) -> (
    Arc<RepositoryService>,
    Arc<std::sync::Mutex<Vec<RepositoryChangedEvent>>>,
) {
    let db = Db::open(&tmp.join("data").join("brainiac.sqlite3")).unwrap();
    let git = GitService::detect().await;
    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = Arc::clone(&events);
    let emitter: brainiac_lib::events::Emitter<brainiac_lib::models::RepositoryChangedEvent> =
        Arc::new(move |e| sink.lock().unwrap().push(e.clone()));
    (
        Arc::new(RepositoryService::new(
            db,
            git,
            Settings::default(),
            emitter,
        )),
        events,
    )
}

#[tokio::test]
async fn detects_git_and_resolves_normal_repo_and_linked_worktree() {
    let git_service = GitService::detect().await.expect("git available");
    assert!(git_service.version().starts_with("git version"));

    let tmp = tempfile::tempdir().unwrap();
    let main = tmp.path().join("main");
    std::fs::create_dir(&main).unwrap();
    fixture_repo(&main);

    let resolved = git_service
        .resolve(main.join("sub-dir-does-not-exist").parent().unwrap())
        .await
        .unwrap();
    assert_eq!(resolved.root, main.canonicalize().unwrap());
    assert!(!resolved.is_linked_worktree());

    let wt = tmp.path().join("wt");
    git(
        &main,
        &[
            "worktree",
            "add",
            "-q",
            wt.to_str().unwrap(),
            "-b",
            "feature",
        ],
    );
    let resolved_wt = git_service.resolve(&wt).await.unwrap();
    assert_eq!(resolved_wt.root, wt.canonicalize().unwrap());
    assert!(resolved_wt.is_linked_worktree());
    assert_eq!(
        resolved_wt.common_git_dir,
        main.canonicalize().unwrap().join(".git")
    );

    // A plain folder is rejected with a VALIDATION error.
    let plain = tmp.path().join("plain");
    std::fs::create_dir(&plain).unwrap();
    let err = git_service.resolve(&plain).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Validation);
}

#[tokio::test]
async fn status_reports_staged_unstaged_and_untracked_separately() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    fixture_repo(&repo);
    let g = GitService::detect().await.unwrap();

    let clean = g.status(&repo).await.unwrap();
    assert_eq!(clean.head.kind, HeadKind::Branch);
    assert_eq!(clean.head.branch.as_deref(), Some("main"));
    assert_eq!(clean.counts, ChangeCounts::default());
    // The service adds the commit time only when HEAD moved.
    let head = clean.head.commit_id.clone().unwrap();
    assert!(g.commit_time(&repo, &head).await.is_some());

    // Edit without touching .git (the case plain .git watching misses).
    write(&repo, "a.txt", "changed\n");
    // Stage a change to b.txt, then edit it again: both staged and unstaged.
    write(&repo, "b.txt", "staged\n");
    git(&repo, &["add", "b.txt"]);
    write(&repo, "b.txt", "staged then edited\n");
    write(&repo, "new.txt", "untracked\n");

    let s = g.status(&repo).await.unwrap();
    assert_eq!(s.counts.staged, 1);
    assert_eq!(s.counts.unstaged, 2);
    assert_eq!(s.counts.untracked, 1);
    assert_eq!(s.counts.unique_paths, 3);
    let b: Vec<_> = s
        .entries
        .iter()
        .filter(|e| e.path == "b.txt")
        .map(|e| e.group)
        .collect();
    assert!(b.contains(&ChangeGroup::Staged) && b.contains(&ChangeGroup::Unstaged));

    // A new folder lists its files; a nested repository stays one entry.
    write(&repo, "fresh/one.txt", "1\n");
    write(&repo, "fresh/deeper/two.txt", "2\n");
    let nested = repo.join("nested");
    std::fs::create_dir(&nested).unwrap();
    fixture_repo(&nested);
    let s = g.status(&repo).await.unwrap();
    let mut untracked: Vec<_> = s
        .entries
        .iter()
        .filter(|e| e.group == ChangeGroup::Untracked)
        .map(|e| e.path.as_str())
        .collect();
    untracked.sort_unstable();
    assert_eq!(
        untracked,
        [
            "fresh/deeper/two.txt",
            "fresh/one.txt",
            "nested/",
            "new.txt"
        ]
    );
}

#[tokio::test]
async fn empty_repository_has_unborn_head_and_empty_history() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    write(&repo, "x.txt", "x\n");
    git(&repo, &["add", "x.txt"]);
    let g = GitService::detect().await.unwrap();
    let s = g.status(&repo).await.unwrap();
    assert_eq!(s.head.kind, HeadKind::Unborn);
    assert_eq!(s.counts.staged, 1);
    assert_eq!(g.resolve_commit(&repo, "HEAD").await.unwrap(), None);
    // Diff against an unborn HEAD still works for the index.
    let limits = DiffLimits {
        max_bytes: 1 << 20,
        max_lines: 10_000,
    };
    let d = g
        .diff(
            &repo,
            &DiffSelector::IndexVsHead {
                path: "x.txt".into(),
            },
            &limits,
            DiffOptions::default(),
        )
        .await
        .unwrap();
    assert!(matches!(d, DiffContent::Text { ref hunks, .. } if hunks.len() == 1));
}

#[tokio::test]
async fn history_pages_are_anchored_and_diffs_are_bounded() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    fixture_repo(&repo);
    for i in 0..5 {
        write(&repo, "c.txt", &format!("{i}\n"));
        git(&repo, &["add", "c.txt"]);
        git(
            &repo,
            &["commit", "-q", "-m", &format!("commit number {i}")],
        );
    }
    let (service, _events) = service_for(tmp.path()).await;
    let summary = service.register(&repo).await.unwrap();
    assert_eq!(summary.state, RepositoryState::Fresh);

    let page1 = service
        .commits(ListCommitsRequest {
            repository_id: summary.id.clone(),
            ref_name: None,
            filter: None,
            cursor: None,
            limit: Some(3),
            author: None,
            exclude: None,
        })
        .await
        .unwrap();
    assert_eq!(page1.items.len(), 3);
    assert_eq!(page1.items[0].subject, "commit number 4");
    let cursor = page1.next_cursor.clone().expect("more pages");

    // A new commit must not shift the in-progress traversal.
    write(&repo, "d.txt", "d\n");
    git(&repo, &["add", "d.txt"]);
    git(&repo, &["commit", "-q", "-m", "late commit"]);

    let page2 = service
        .commits(ListCommitsRequest {
            repository_id: summary.id.clone(),
            ref_name: None,
            filter: None,
            cursor: Some(cursor),
            limit: Some(3),
            author: None,
            exclude: None,
        })
        .await
        .unwrap();
    assert_eq!(page2.anchor_commit_id, page1.anchor_commit_id);
    assert_eq!(page2.items[0].subject, "commit number 1");
    let page3 = service
        .commits(ListCommitsRequest {
            repository_id: summary.id.clone(),
            ref_name: None,
            filter: None,
            cursor: page2.next_cursor.clone(),
            limit: Some(3),
            author: None,
            exclude: None,
        })
        .await
        .unwrap();
    assert_eq!(page3.items.last().unwrap().subject, "first commit");
    assert!(page3.next_cursor.is_none());

    // Filtering by message and by hash prefix.
    let filtered = service
        .commits(ListCommitsRequest {
            repository_id: summary.id.clone(),
            ref_name: None,
            filter: Some("number 2".into()),
            cursor: None,
            limit: None,
            author: None,
            exclude: None,
        })
        .await
        .unwrap();
    assert_eq!(filtered.items.len(), 1);
    let by_hash = service
        .commits(ListCommitsRequest {
            repository_id: summary.id.clone(),
            ref_name: None,
            filter: Some(page1.items[1].short_id.clone()),
            cursor: None,
            limit: None,
            author: None,
            exclude: None,
        })
        .await
        .unwrap();
    assert_eq!(by_hash.items.len(), 1);
    assert_eq!(by_hash.items[0].id, page1.items[1].id);

    // Commit diff for a normal commit, the root commit, and a bounded worktree diff.
    let all: Vec<&CommitSummary> = [&page1, &page2, &page3]
        .iter()
        .flat_map(|p| p.items.iter())
        .collect();
    let second = all
        .iter()
        .find(|c| c.subject.starts_with("second"))
        .unwrap();
    let d = service
        .diff(
            &summary.id,
            DiffSelector::Commit {
                commit_id: second.id.clone(),
                path: "a.txt".into(),
                old_path: None,
                parent_index: 0,
            },
            DiffOptions::default(),
        )
        .await
        .unwrap();
    match d.content {
        DiffContent::Text { hunks, .. } => {
            assert_eq!(
                hunks[0]
                    .lines
                    .iter()
                    .filter(|l| l.kind == DiffLineKind::Add)
                    .count(),
                1
            );
        }
        other => panic!("{other:?}"),
    }
    let root = page3.items.last().unwrap();
    assert!(root.parent_ids.is_empty());
    let d = service
        .diff(
            &summary.id,
            DiffSelector::Commit {
                commit_id: root.id.clone(),
                path: "a.txt".into(),
                old_path: None,
                parent_index: 0,
            },
            DiffOptions::default(),
        )
        .await
        .unwrap();
    assert!(matches!(d.content, DiffContent::Text { ref hunks, .. } if hunks[0].lines.len() == 3));

    let big: String = (0..20_000).map(|i| format!("line {i}\n")).collect();
    write(&repo, "a.txt", &big);
    let d = service
        .diff(
            &summary.id,
            DiffSelector::WorktreeVsIndex {
                path: "a.txt".into(),
            },
            DiffOptions::default(),
        )
        .await
        .unwrap();
    match d.content {
        DiffContent::Text {
            hunks, truncated, ..
        } => {
            assert!(truncated);
            let shown: usize = hunks.iter().map(|h| h.lines.len()).sum();
            assert!(shown <= 10_000);
        }
        other => panic!("{other:?}"),
    }
    let d = service
        .diff(
            &summary.id,
            DiffSelector::Commit {
                commit_id: "--output=/tmp/x".into(),
                path: "a.txt".into(),
                old_path: None,
                parent_index: 0,
            },
            DiffOptions::default(),
        )
        .await;
    assert_eq!(d.unwrap_err().code, ErrorCode::Validation);
    let d = service
        .diff(
            &summary.id,
            DiffSelector::WorktreeVsIndex {
                path: "../escape".into(),
            },
            DiffOptions::default(),
        )
        .await;
    assert_eq!(d.unwrap_err().code, ErrorCode::Validation);
}

#[tokio::test]
async fn commit_details_list_files_and_compare_with_the_chosen_parent() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    write(&repo, "a.txt", "one\ntwo\n");
    write(&repo, "old.txt", "alpha\nbeta\ngamma\ndelta\nepsilon\n");
    write(&repo, "gone.txt", "bye\n");
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "root"]);
    let root_id = git(&repo, &["rev-parse", "HEAD"]);

    // A commit that modifies, renames (with an edit), deletes, and adds a binary.
    write(&repo, "a.txt", "one\nTWO\nthree\n");
    git(&repo, &["mv", "old.txt", "new.txt"]);
    write(
        &repo,
        "new.txt",
        "alpha\nbeta\ngamma\ndelta\nepsilon\nzeta\n",
    );
    git(&repo, &["rm", "-q", "gone.txt"]);
    std::fs::write(repo.join("bin.dat"), [0u8, 1, 2, 0, 3]).unwrap();
    git(&repo, &["add", "-A"]);
    git(
        &repo,
        &[
            "commit",
            "-q",
            "-m",
            "mixed changes",
            "-m",
            "Body line one.\n\nBody line two.",
        ],
    );
    let mixed_id = git(&repo, &["rev-parse", "HEAD"]);

    // A merge commit: `feature` adds f.txt while `main` adds m.txt.
    git(&repo, &["checkout", "-q", "-b", "feature"]);
    write(&repo, "f.txt", "feature\n");
    git(&repo, &["add", "f.txt"]);
    git(&repo, &["commit", "-q", "-m", "feature work"]);
    git(&repo, &["checkout", "-q", "main"]);
    write(&repo, "m.txt", "main\n");
    git(&repo, &["add", "m.txt"]);
    git(&repo, &["commit", "-q", "-m", "main work"]);
    git(
        &repo,
        &["merge", "-q", "--no-ff", "-m", "merge feature", "feature"],
    );
    let merge_id = git(&repo, &["rev-parse", "HEAD"]);

    let (service, _events) = service_for(tmp.path()).await;
    let summary = service.register(&repo).await.unwrap();
    let id = summary.id.clone();

    let d = service.commit(&id, &mixed_id, None).await.unwrap();
    assert_eq!(d.repository_id, id);
    assert_eq!(d.summary.subject, "mixed changes");
    assert_eq!(d.body, "Body line one.\n\nBody line two.");
    assert_eq!(d.committer_name, "Test");
    assert_eq!(d.compared_parent_index, 0);
    let find = |path: &str| d.files.iter().find(|f| f.path == path).unwrap();
    assert_eq!(find("a.txt").kind, ChangeKind::Modified);
    assert_eq!(
        (find("a.txt").additions, find("a.txt").deletions),
        (Some(2), Some(1))
    );
    assert_eq!(find("new.txt").kind, ChangeKind::Renamed);
    assert_eq!(find("new.txt").old_path.as_deref(), Some("old.txt"));
    assert_eq!(find("new.txt").additions, Some(1));
    assert_eq!(find("gone.txt").kind, ChangeKind::Deleted);
    assert!(find("bin.dat").is_binary);
    assert_eq!(d.files.len(), 4);

    // The renamed file's patch pairs both paths instead of showing a full add.
    let patch = service
        .diff(
            &id,
            DiffSelector::Commit {
                commit_id: mixed_id.clone(),
                path: "new.txt".into(),
                old_path: Some("old.txt".into()),
                parent_index: 0,
            },
            DiffOptions::default(),
        )
        .await
        .unwrap();
    match patch.content {
        DiffContent::Text {
            old_path,
            new_path,
            hunks,
            ..
        } => {
            assert_eq!(old_path.as_deref(), Some("old.txt"));
            assert_eq!(new_path.as_deref(), Some("new.txt"));
            let adds: Vec<_> = hunks
                .iter()
                .flat_map(|h| h.lines.iter())
                .filter(|l| l.kind == DiffLineKind::Add)
                .collect();
            assert_eq!(adds.len(), 1);
            assert_eq!(adds[0].text, "zeta");
        }
        other => panic!("{other:?}"),
    }
    let deleted = service
        .diff(
            &id,
            DiffSelector::Commit {
                commit_id: mixed_id.clone(),
                path: "gone.txt".into(),
                old_path: None,
                parent_index: 0,
            },
            DiffOptions::default(),
        )
        .await
        .unwrap();
    assert!(
        matches!(deleted.content, DiffContent::Text { ref hunks, ref new_path, .. } if new_path.is_none() && hunks[0].lines[0].kind == DiffLineKind::Delete)
    );

    // The root commit compares with the empty tree.
    let root = service.commit(&id, &root_id, None).await.unwrap();
    assert!(root.summary.parent_ids.is_empty());
    assert_eq!(root.files.len(), 3);
    assert!(root.files.iter().all(|f| f.kind == ChangeKind::Added));
    assert_eq!(
        service
            .commit(&id, &root_id, Some(1))
            .await
            .unwrap_err()
            .code,
        ErrorCode::Validation
    );

    // A merge compares with its first parent by default and with the second on request.
    let first = service.commit(&id, &merge_id, None).await.unwrap();
    assert_eq!(first.summary.parent_ids.len(), 2);
    let paths = |d: &CommitDetail| d.files.iter().map(|f| f.path.clone()).collect::<Vec<_>>();
    assert_eq!(paths(&first), vec!["f.txt"]);
    let second = service.commit(&id, &merge_id, Some(1)).await.unwrap();
    assert_eq!(second.compared_parent_index, 1);
    assert_eq!(paths(&second), vec!["m.txt"]);
    let patch = service
        .diff(
            &id,
            DiffSelector::Commit {
                commit_id: merge_id.clone(),
                path: "m.txt".into(),
                old_path: None,
                parent_index: 1,
            },
            DiffOptions::default(),
        )
        .await
        .unwrap();
    assert!(matches!(patch.content, DiffContent::Text { ref hunks, .. } if hunks.len() == 1));
    assert_eq!(
        service
            .commit(&id, &merge_id, Some(2))
            .await
            .unwrap_err()
            .code,
        ErrorCode::Validation
    );
    assert_eq!(
        service
            .commit(&id, "--output=x", None)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Validation
    );
}

#[tokio::test]
async fn refs_list_branches_remotes_and_tags_and_scope_history() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    fixture_repo(&repo);
    git(&repo, &["tag", "-a", "v1.0", "-m", "first release"]);
    git(&repo, &["tag", "light"]);
    git(&repo, &["branch", "feature", "HEAD~1"]);
    // A remote-tracking ref and its symbolic HEAD, without any network access.
    git(
        &repo,
        &[
            "config",
            "remote.origin.url",
            "https://example.com/project.git",
        ],
    );
    git(
        &repo,
        &[
            "config",
            "remote.origin.fetch",
            "+refs/heads/*:refs/remotes/origin/*",
        ],
    );
    git(&repo, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
    git(
        &repo,
        &[
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/main",
        ],
    );
    git(&repo, &["config", "branch.main.remote", "origin"]);
    git(&repo, &["config", "branch.main.merge", "refs/heads/main"]);
    let head = git(&repo, &["rev-parse", "HEAD"]);
    let first = git(&repo, &["rev-parse", "HEAD~1"]);

    let (service, _events) = service_for(tmp.path()).await;
    let summary = service.register(&repo).await.unwrap();
    let refs = service.refs(&summary.id).await.unwrap();
    assert_eq!(refs.repository_id, summary.id);
    let names: Vec<&str> = refs.refs.iter().map(|r| r.full_name.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "refs/heads/feature",
            "refs/heads/main",
            "refs/remotes/origin/main",
            "refs/tags/light",
            "refs/tags/v1.0",
        ]
    );
    let by = |full: &str| refs.refs.iter().find(|r| r.full_name == full).unwrap();
    let main = by("refs/heads/main");
    assert!(main.is_head);
    assert_eq!(main.kind, RefKind::LocalBranch);
    assert_eq!(main.upstream.as_deref(), Some("origin/main"));
    assert!(!by("refs/heads/feature").is_head);
    assert_eq!(by("refs/remotes/origin/main").name, "origin/main");
    assert_eq!(by("refs/remotes/origin/main").kind, RefKind::RemoteBranch);
    // The annotated tag is peeled to the commit it marks.
    assert_eq!(by("refs/tags/v1.0").target_id, head);
    assert_eq!(by("refs/tags/v1.0").kind, RefKind::Tag);

    // Selecting a ref scopes history to it without checking it out.
    let page = service
        .commits(ListCommitsRequest {
            repository_id: summary.id.clone(),
            ref_name: Some("refs/heads/feature".into()),
            filter: None,
            cursor: None,
            limit: None,
            author: None,
            exclude: None,
        })
        .await
        .unwrap();
    assert_eq!(page.anchor_commit_id, first);
    assert_eq!(page.items.len(), 1);
    assert_eq!(git(&repo, &["symbolic-ref", "--short", "HEAD"]), "main");
}

#[tokio::test]
async fn registration_persists_across_reopen_and_refresh_emits_events() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    fixture_repo(&repo);

    let id = {
        let (service, events) = service_for(tmp.path()).await;
        let summary = service.register(&repo).await.unwrap();
        assert_eq!(summary.counts.unwrap(), ChangeCounts::default());
        assert_eq!(events.lock().unwrap().len(), 1);
        assert_eq!(events.lock().unwrap()[0].origin, ChangeOrigin::Registration);
        // Registering the same path again does not duplicate it.
        let again = service.register(&repo).await.unwrap();
        assert_eq!(again.id, summary.id);
        assert_eq!(service.list().await.unwrap().len(), 1);
        summary.id
    };

    // Reopen the database (simulating an app restart).
    let (service, events) = service_for(tmp.path()).await;
    let snapshot = service.snapshot().await.unwrap();
    assert_eq!(snapshot.repositories.len(), 1);
    assert_eq!(snapshot.repositories[0].id, id);
    assert_eq!(snapshot.recent_repository_ids, vec![id.clone()]);
    assert!(snapshot.git.available);

    write(&repo, "a.txt", "dirty\n");
    let refreshed = service.refresh(&id, ChangeOrigin::Watcher).await.unwrap();
    assert_eq!(refreshed.counts.unwrap().unstaged, 1);
    let ev = events.lock().unwrap();
    assert_eq!(ev.len(), 1);
    assert_eq!(ev[0].origin, ChangeOrigin::Watcher);
    assert_eq!(ev[0].snapshot_version, snapshot.snapshot_version + 1);

    // Inspection never modified the repository.
    // Exactly one entry, unstaged-only ("1 .M ..."): nothing was staged or committed.
    let porcelain = git(&repo, &["status", "--porcelain=v2"]);
    assert_eq!(porcelain.lines().count(), 1);
    assert!(
        porcelain.starts_with("1 .M "),
        "unexpected status: {porcelain}"
    );
}

#[tokio::test]
async fn loading_changes_reports_a_change_only_when_status_differs() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    fixture_repo(&repo);
    let (service, events) = service_for(tmp.path()).await;
    let summary = service.register(&repo).await.unwrap();
    let last_changed = || events.lock().unwrap().last().map(|e| e.changed);

    // The view reloads on changed events and itself calls `changes`, so an
    // unchanged repository must not report a change or the two would loop.
    // An observation under a second old is reused without a new event.
    let settle = || tokio::time::sleep(std::time::Duration::from_millis(1100));
    let count = || events.lock().unwrap().len();
    let before = count();
    service.changes(&summary.id).await.unwrap();
    assert_eq!(count(), before, "a fresh observation is reused");
    settle().await;
    service.changes(&summary.id).await.unwrap();
    assert_eq!(last_changed(), Some(false));

    write(&repo, "a.txt", "edited\n");
    settle().await;
    service.changes(&summary.id).await.unwrap();
    assert_eq!(last_changed(), Some(true));
    settle().await;
    service.changes(&summary.id).await.unwrap();
    assert_eq!(last_changed(), Some(false));

    // Timer polls follow the same rule; manual and watcher refreshes always report a change.
    service
        .refresh(&summary.id, ChangeOrigin::Timer)
        .await
        .unwrap();
    assert_eq!(last_changed(), Some(false));
    service
        .refresh(&summary.id, ChangeOrigin::Refresh)
        .await
        .unwrap();
    assert_eq!(last_changed(), Some(true));
}

#[tokio::test]
async fn coalesced_refresh_collapses_bursts() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    fixture_repo(&repo);
    let (service, events) = service_for(tmp.path()).await;
    let summary = service.register(&repo).await.unwrap();
    events.lock().unwrap().clear();

    for _ in 0..25 {
        service.request_refresh(&summary.id, ChangeOrigin::Watcher);
    }
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let n = events.lock().unwrap().len();
    assert!(
        (1..=2).contains(&n),
        "expected 1-2 coalesced refreshes, got {n}"
    );
}

#[tokio::test]
async fn watcher_notices_unstaged_edit_without_git_metadata_change() {
    use brainiac_lib::watcher::{run_debounce, RepositoryWatcher, DEBOUNCE};
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    fixture_repo(&repo);
    let (service, events) = service_for(tmp.path()).await;
    let summary = service.register(&repo).await.unwrap();
    let row = service.row(&summary.id).await.unwrap();
    events.lock().unwrap().clear();

    let (fs_watcher, rx) = RepositoryWatcher::new().unwrap();
    fs_watcher
        .watch(
            &row.id,
            Path::new(&row.canonical_root),
            Path::new(&row.git_dir),
        )
        .unwrap();
    let loop_service = Arc::clone(&service);
    let task = tokio::spawn(run_debounce(rx, loop_service, DEBOUNCE));

    tokio::time::sleep(Duration::from_millis(300)).await;
    write(&repo, "a.txt", "edited by an external editor\n");

    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    loop {
        if events
            .lock()
            .unwrap()
            .iter()
            .any(|e| e.origin == ChangeOrigin::Watcher)
        {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "no watcher-originated refresh within 8s"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let row = service.row(&summary.id).await.unwrap();
    assert_eq!(row.status.unwrap().counts.unstaged, 1);
    task.abort();
    let _ = PathBuf::new();
}

#[test]
fn enum_columns_use_lookup_tables() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Db::open(&tmp.path().join("brainiac.sqlite3")).unwrap();
    db.call_blocking(|c| {
        c.execute(
            "INSERT INTO workspaces (id, name, discovery_mode, created_at) \
             VALUES ('w1', 'Work', 'manual', '2026-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        // An unknown value is rejected by the foreign key, as a CHECK would.
        let bad = c.execute(
            "INSERT INTO workspaces (id, name, discovery_mode, created_at) \
             VALUES ('w2', 'Bad', 'nested', '2026-01-01T00:00:00Z')",
            [],
        );
        assert!(bad.is_err(), "unknown discovery_mode must be rejected");
        // Renaming a value updates the lookup row; the workspace row follows.
        c.execute(
            "UPDATE discovery_modes SET name = 'picked' WHERE name = 'manual'",
            [],
        )
        .unwrap();
        let mode: String = c
            .query_row(
                "SELECT discovery_mode FROM workspaces WHERE id = 'w1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(mode, "picked");
        Ok(())
    })
    .unwrap();
}

#[tokio::test]
async fn database_migrates_backs_up_and_keeps_settings() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("app").join("brainiac.sqlite3");
    let db = Db::open(&path).unwrap();
    assert_eq!(
        db.call_blocking(|c| brainiac_lib::db::schema_version(c))
            .unwrap(),
        brainiac_lib::db::SCHEMA_VERSION
    );
    let mut settings = Settings::default();
    settings.editor.executable = "cursor".into();
    let s2 = settings.clone();
    db.call_blocking(move |c| brainiac_lib::db::save_settings(c, &s2))
        .unwrap();
    let loaded = db
        .call_blocking(|c| brainiac_lib::db::load_settings(c))
        .unwrap();
    assert_eq!(loaded, settings);

    let backups = path.parent().unwrap().join("backups");
    let daily: Vec<_> = std::fs::read_dir(&backups)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(daily.len(), 1, "one daily backup after first open");

    let snapshot = db
        .backup_to(tmp.path().join("snapshot.sqlite3"))
        .await
        .unwrap();
    let copy = Db::open(&snapshot).unwrap();
    assert_eq!(
        copy.call_blocking(|c| brainiac_lib::db::load_settings(c))
            .unwrap(),
        settings
    );
}
