//! Fetching and the workspace activity feed against a local bare remote.
//! A "teammate" clone pushes to the remote; the user's clone is registered in
//! a workspace and fetches. Every fixture is built here; nothing is checked in.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use brainiac_lib::db::Db;
use brainiac_lib::git::GitService;
use brainiac_lib::models::*;
use brainiac_lib::workspaces::RepositoryService;

fn git_as(dir: &Path, author: &str, args: &[&str]) -> String {
    let email = format!("{}@example.com", author.to_lowercase().replace(' ', "."));
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", author)
        .env("GIT_AUTHOR_EMAIL", &email)
        .env("GIT_COMMITTER_NAME", author)
        .env("GIT_COMMITTER_EMAIL", &email)
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

fn git(dir: &Path, args: &[&str]) -> String {
    git_as(dir, "Test", args)
}

fn commit(dir: &Path, author: &str, file: &str, text: &str, message: &str) {
    std::fs::write(dir.join(file), text).unwrap();
    git_as(dir, author, &["add", file]);
    git_as(dir, author, &["commit", "-q", "-m", message]);
}

struct Fixture {
    _tmp: tempfile::TempDir,
    work: PathBuf,
    mate: PathBuf,
    svc: Arc<RepositoryService>,
    repo_id: String,
    workspace_id: String,
}

/// remote.git with one commit on main; `work` (registered, in a workspace) and
/// `mate` are clones of it.
async fn fixture() -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let remote = tmp.path().join("remote.git");
    let seed = tmp.path().join("seed");
    std::fs::create_dir_all(&seed).unwrap();
    git(&seed, &["init", "-q", "-b", "main"]);
    commit(&seed, "Test", "README.md", "hello\n", "initial");
    git(tmp.path(), &["clone", "-q", "--bare", "seed", "remote.git"]);
    git(tmp.path(), &["clone", "-q", "remote.git", "work"]);
    git(tmp.path(), &["clone", "-q", "remote.git", "mate"]);
    let _ = remote;

    let db = Db::open(&tmp.path().join("data").join("brainiac.sqlite3")).unwrap();
    let emitter: brainiac_lib::workspaces::Emitter = Arc::new(|_| {});
    let svc = Arc::new(RepositoryService::new(
        db,
        GitService::detect().await,
        Settings::default(),
        emitter,
    ));
    let work = tmp.path().join("work");
    let workspace = svc
        .create_workspace(CreateWorkspaceRequest {
            name: "Team".into(),
            discovery_mode: DiscoveryMode::Manual,
            discovery_root: None,
            discovery_path: None,
            paths: vec![work.display().to_string()],
        })
        .await
        .unwrap()
        .workspace;
    let repo_id = workspace.members[0].repository_id.clone().unwrap();
    // The first observation takes the baseline.
    svc.refresh(&repo_id, ChangeOrigin::Refresh).await.unwrap();
    Fixture {
        mate: tmp.path().join("mate"),
        work,
        _tmp: tmp,
        svc,
        repo_id,
        workspace_id: workspace.id,
    }
}

#[tokio::test]
async fn baseline_then_fast_forward_with_overlap() {
    let f = fixture().await;
    let feed = f.svc.workspace_activity(&f.workspace_id).await.unwrap();
    assert!(feed.items.is_empty(), "the baseline emits nothing");

    // The user edits README.md; a teammate changes it too and adds a file.
    std::fs::write(f.work.join("README.md"), "hello\nmine\n").unwrap();
    commit(
        &f.mate,
        "Alex Kim",
        "README.md",
        "hello\ntheirs\n",
        "Edit readme",
    );
    commit(&f.mate, "Alex Kim", "NEW.md", "new\n", "Add notes");
    git(&f.mate, &["push", "-q", "origin", "main"]);

    let local_main = git(&f.work, &["rev-parse", "main"]);
    let index_before = std::fs::read(f.work.join(".git").join("index")).unwrap();
    let result = f.svc.fetch(&f.repo_id, false).await.unwrap();
    assert_eq!(result.remote, "origin");
    assert_eq!(result.moved, vec!["origin/main".to_string()]);

    // Fetching never touches the local branch, the index, or working files.
    assert_eq!(git(&f.work, &["rev-parse", "main"]), local_main);
    assert_eq!(
        std::fs::read(f.work.join(".git").join("index")).unwrap(),
        index_before
    );
    assert_eq!(
        std::fs::read_to_string(f.work.join("README.md")).unwrap(),
        "hello\nmine\n"
    );

    let feed = f.svc.workspace_activity(&f.workspace_id).await.unwrap();
    assert_eq!(feed.unseen, 1);
    let item = &feed.items[0];
    assert_eq!(item.kind, ActivityKind::Advanced);
    assert_eq!(item.ref_name, "origin/main");
    assert_eq!(item.detail.total_commits, 2);
    assert_eq!(item.detail.authors, vec!["Alex Kim".to_string()]);
    assert_eq!(item.detail.commits[0].subject, "Add notes");
    assert_eq!(item.detail.conflict_paths, vec!["README.md".to_string()]);
    // main is behind origin/main by two commits.
    let drift = item.detail.drift.as_ref().expect("drift");
    assert_eq!((drift.branch.as_str(), drift.behind), ("main", 2));
    assert!(feed.freshness[0].last_fetch_at.is_some());
    let pulse = f.svc.team_pulse(&f.workspace_id).await.unwrap();
    assert_eq!(pulse.commits, 3);
    // Unchanged refs reuse the cached pulse.
    assert_eq!(f.svc.team_pulse(&f.workspace_id).await.unwrap(), pulse);

    let summary = f.svc.list().await.unwrap();
    assert!(summary[0].last_fetch_at.is_some());
    let ws = f.svc.workspace(&f.workspace_id).await.unwrap();
    assert_eq!(ws.unseen_activity, 1);

    f.svc
        .mark_activity_seen(&f.workspace_id, None)
        .await
        .unwrap();
    let feed = f.svc.workspace_activity(&f.workspace_id).await.unwrap();
    assert_eq!(feed.unseen, 0);
    assert!(feed.items[0].seen);
}

#[tokio::test]
async fn force_push_and_tags() {
    let f = fixture().await;
    commit(&f.mate, "Jo Martin", "a.txt", "a\n", "First try");
    git(&f.mate, &["push", "-q", "origin", "main"]);
    f.svc.fetch(&f.repo_id, false).await.unwrap();

    // Rewrite the pushed commit and tag the result.
    git(&f.mate, &["reset", "-q", "--hard", "HEAD~1"]);
    commit(&f.mate, "Jo Martin", "a.txt", "b\n", "Second try");
    git(&f.mate, &["push", "-q", "--force", "origin", "main"]);
    git_as(
        &f.mate,
        "Jo Martin",
        &["tag", "-a", "v1.0", "-m", "Release 1.0"],
    );
    git(&f.mate, &["push", "-q", "origin", "v1.0"]);
    let result = f.svc.fetch(&f.repo_id, false).await.unwrap();
    assert!(result.moved.contains(&"origin/main".to_string()));
    assert!(result.moved.contains(&"v1.0".to_string()));

    let feed = f.svc.workspace_activity(&f.workspace_id).await.unwrap();
    let rewritten = feed
        .items
        .iter()
        .find(|i| i.kind == ActivityKind::Rewritten)
        .expect("rewrite event");
    assert_eq!(
        (rewritten.detail.replaced, rewritten.detail.total_commits),
        (1, 1)
    );
    let tagged = feed
        .items
        .iter()
        .find(|i| i.kind == ActivityKind::Tagged)
        .expect("tag event");
    assert_eq!(tagged.ref_name, "v1.0");
    assert_eq!(tagged.detail.authors, vec!["Jo Martin".to_string()]);
    assert_eq!(f.svc.team_pulse(&f.workspace_id).await.unwrap().releases, 1);
}

#[tokio::test]
async fn auto_fetch_watched_branches_only() {
    let f = fixture().await;
    git(&f.mate, &["checkout", "-q", "-b", "topic"]);
    commit(&f.mate, "Sam Rivera", "t.txt", "t\n", "Topic work");
    git(&f.mate, &["push", "-q", "origin", "topic"]);
    git(&f.mate, &["checkout", "-q", "main"]);
    commit(&f.mate, "Sam Rivera", "m.txt", "m\n", "Main work");
    git(&f.mate, &["push", "-q", "origin", "main"]);

    let settings = ActivitySettings {
        auto_fetch: true,
        ..ActivitySettings::default()
    };
    let ws = f
        .svc
        .update_activity_settings(&f.workspace_id, settings)
        .await
        .unwrap();
    assert!(ws.activity.auto_fetch);

    let result = f.svc.fetch(&f.repo_id, true).await.unwrap();
    assert_eq!(result.moved, vec!["origin/main".to_string()]);
    let topic = Command::new("git")
        .args([
            "rev-parse",
            "--verify",
            "--quiet",
            "refs/remotes/origin/topic",
        ])
        .current_dir(&f.work)
        .output()
        .unwrap();
    assert!(
        !topic.status.success(),
        "unwatched branches are not fetched"
    );
}

#[tokio::test]
async fn busy_repository_is_skipped() {
    let f = fixture().await;
    let lock = f.work.join(".git").join("index.lock");
    std::fs::write(&lock, "").unwrap();
    let err = f.svc.fetch(&f.repo_id, false).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Conflict);
    std::fs::remove_file(lock).unwrap();
    f.svc.fetch(&f.repo_id, false).await.unwrap();
}

#[tokio::test]
async fn drift_from_the_forked_branch() {
    let f = fixture().await;
    git(&f.work, &["checkout", "-q", "-b", "feature/limits"]);
    commit(&f.work, "Test", "mine.txt", "x\n", "My work");
    f.svc
        .refresh(&f.repo_id, ChangeOrigin::Refresh)
        .await
        .unwrap();
    commit(&f.mate, "Alex Kim", "theirs.txt", "y\n", "Their work");
    git(&f.mate, &["push", "-q", "origin", "main"]);
    f.svc.fetch(&f.repo_id, false).await.unwrap();

    let feed = f.svc.workspace_activity(&f.workspace_id).await.unwrap();
    let drift = feed.items[0].detail.drift.as_ref().expect("drift");
    assert_eq!(drift.branch, "feature/limits");
    assert_eq!(drift.base, "origin/main");
    assert_eq!(drift.behind, 1);
}

#[tokio::test]
async fn changes_carry_line_counts_and_both_diff() {
    let f = fixture().await;
    std::fs::write(f.work.join("README.md"), "hello\nstaged\n").unwrap();
    git(&f.work, &["add", "README.md"]);
    std::fs::write(f.work.join("README.md"), "hello\nstaged\nunstaged\nmore\n").unwrap();
    let changes = f.svc.changes(&f.repo_id).await.unwrap();
    let staged = changes
        .entries
        .iter()
        .find(|e| e.group == ChangeGroup::Staged)
        .unwrap();
    assert_eq!((staged.additions, staged.deletions), (Some(1), Some(0)));
    let unstaged = changes
        .entries
        .iter()
        .find(|e| e.group == ChangeGroup::Unstaged)
        .unwrap();
    assert_eq!((unstaged.additions, unstaged.deletions), (Some(2), Some(0)));

    let both = f
        .svc
        .diff(
            &f.repo_id,
            DiffSelector::WorktreeVsHead {
                path: "README.md".into(),
            },
            DiffOptions::default(),
        )
        .await
        .unwrap();
    match both.content {
        DiffContent::Text { hunks, .. } => {
            let added = hunks[0]
                .lines
                .iter()
                .filter(|l| l.kind == DiffLineKind::Add)
                .count();
            assert_eq!(added, 3);
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[tokio::test]
async fn history_by_author_and_excluding_a_base() {
    let f = fixture().await;
    git(&f.work, &["checkout", "-q", "-b", "topic"]);
    commit(&f.work, "Alex Kim", "a.txt", "a\n", "Alex change");
    commit(&f.work, "Jo Martin", "b.txt", "b\n", "Jo change");
    let page = |author: Option<&str>, exclude: Option<&str>| ListCommitsRequest {
        repository_id: f.repo_id.clone(),
        ref_name: Some("topic".into()),
        filter: None,
        cursor: None,
        limit: None,
        author: author.map(str::to_string),
        exclude: exclude.map(str::to_string),
    };
    let by_alex = f.svc.commits(page(Some("alex"), None)).await.unwrap();
    assert_eq!(by_alex.items.len(), 1);
    assert_eq!(by_alex.items[0].subject, "Alex change");
    let only_topic = f.svc.commits(page(None, Some("main"))).await.unwrap();
    assert_eq!(only_topic.items.len(), 2);

    let refs = f.svc.refs(&f.repo_id).await.unwrap();
    assert_eq!(refs.base.as_deref(), Some("origin/main"));
    let topic = refs.refs.iter().find(|r| r.name == "topic").unwrap();
    assert_eq!((topic.base_ahead, topic.base_behind), (Some(2), Some(0)));
}

fn kinds(items: &[ActivityItem], kind: ActivityKind) -> usize {
    items.iter().filter(|i| i.kind == kind).count()
}

fn rev_exists(dir: &Path, rev: &str) -> bool {
    Command::new("git")
        .args(["rev-parse", "--verify", "--quiet", rev])
        .current_dir(dir)
        .output()
        .unwrap()
        .status
        .success()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_observations_record_one_event() {
    let f = fixture().await;
    commit(&f.mate, "Alex Kim", "x.txt", "x\n", "Work");
    git(&f.mate, &["push", "-q", "origin", "main"]);
    git(&f.work, &["fetch", "-q", "origin"]);
    let (a, b) = tokio::join!(
        f.svc.refresh(&f.repo_id, ChangeOrigin::Watcher),
        f.svc.refresh(&f.repo_id, ChangeOrigin::Fetch)
    );
    a.unwrap();
    b.unwrap();
    let feed = f.svc.workspace_activity(&f.workspace_id).await.unwrap();
    assert_eq!(kinds(&feed.items, ActivityKind::Advanced), 1);
}

#[tokio::test]
async fn auto_fetch_survives_a_branch_deleted_on_the_remote() {
    let f = fixture().await;
    git(&f.mate, &["push", "-q", "origin", "main:master"]);
    f.svc.fetch(&f.repo_id, false).await.unwrap();
    git(&f.mate, &["push", "-q", "origin", "--delete", "master"]);
    commit(&f.mate, "Alex Kim", "x.txt", "x\n", "Work");
    git(&f.mate, &["push", "-q", "origin", "main"]);
    let settings = ActivitySettings {
        auto_fetch: true,
        ..ActivitySettings::default()
    };
    f.svc
        .update_activity_settings(&f.workspace_id, settings)
        .await
        .unwrap();
    let result = f.svc.fetch(&f.repo_id, true).await.unwrap();
    assert_eq!(result.moved, vec!["origin/main".to_string()]);
}

#[tokio::test]
async fn fetch_never_prunes_or_touches_local_branches() {
    let f = fixture().await;
    git(&f.mate, &["push", "-q", "origin", "main:topic"]);
    f.svc.fetch(&f.repo_id, false).await.unwrap();
    git(&f.mate, &["push", "-q", "origin", "--delete", "topic"]);
    git(&f.work, &["config", "remote.origin.prune", "true"]);
    git(&f.work, &["config", "remote.origin.pruneTags", "true"]);
    // A mirror refspec would write local branches; it must be ignored.
    git(
        &f.work,
        &[
            "config",
            "--add",
            "remote.origin.fetch",
            "+refs/heads/*:refs/heads/*",
        ],
    );
    git(&f.work, &["checkout", "-q", "--detach"]);
    commit(&f.mate, "Alex Kim", "x.txt", "x\n", "Work");
    git(&f.mate, &["push", "-q", "origin", "main"]);
    let local_main = git(&f.work, &["rev-parse", "refs/heads/main"]);

    f.svc.fetch(&f.repo_id, false).await.unwrap();
    assert!(rev_exists(&f.work, "refs/remotes/origin/topic"), "pruned");
    assert_eq!(git(&f.work, &["rev-parse", "refs/heads/main"]), local_main);
    assert_eq!(
        git(&f.work, &["rev-parse", "refs/remotes/origin/main"]),
        git(&f.mate, &["rev-parse", "HEAD"])
    );
}

#[tokio::test]
async fn each_workspace_sees_only_what_it_watches() {
    let f = fixture().await;
    let other = f
        .svc
        .create_workspace(CreateWorkspaceRequest {
            name: "Releases".into(),
            discovery_mode: DiscoveryMode::Manual,
            discovery_root: None,
            discovery_path: None,
            paths: vec![f.work.display().to_string()],
        })
        .await
        .unwrap()
        .workspace;
    f.svc
        .update_activity_settings(
            &other.id,
            ActivitySettings {
                watched_branches: vec!["release/*".into()],
                watched_tags: vec![],
                ..ActivitySettings::default()
            },
        )
        .await
        .unwrap();
    git(&f.mate, &["push", "-q", "origin", "main:release/1.0"]);
    commit(&f.mate, "Alex Kim", "x.txt", "x\n", "Work");
    git(&f.mate, &["push", "-q", "origin", "main"]);
    f.svc.fetch(&f.repo_id, false).await.unwrap();

    let team = f.svc.workspace_activity(&f.workspace_id).await.unwrap();
    let names: Vec<_> = team.items.iter().map(|i| i.ref_name.as_str()).collect();
    assert_eq!(names, vec!["origin/main"]);
    let releases = f.svc.workspace_activity(&other.id).await.unwrap();
    assert_eq!(releases.items.len(), 1);
    assert_eq!(releases.items[0].kind, ActivityKind::Created);
    assert_eq!(
        releases.items[0].full_ref,
        "refs/remotes/origin/release/1.0"
    );

    // Marking everything seen in one workspace leaves the other's news unread.
    f.svc
        .mark_activity_seen(&f.workspace_id, None)
        .await
        .unwrap();
    let ws = f.svc.workspaces().await.unwrap();
    let unseen = |id: &str| ws.iter().find(|w| w.id == id).unwrap().unseen_activity;
    assert_eq!((unseen(&f.workspace_id), unseen(&other.id)), (0, 1));
}

#[tokio::test]
async fn a_missing_old_commit_does_not_wedge_the_feed() {
    let f = fixture().await;
    commit(&f.mate, "Jo", "a.txt", "a\n", "First");
    git(&f.mate, &["push", "-q", "origin", "main"]);
    f.svc.fetch(&f.repo_id, false).await.unwrap();
    // Rewritten and garbage-collected while Brainiac was not looking.
    git(&f.mate, &["reset", "-q", "--hard", "HEAD~1"]);
    commit(&f.mate, "Jo", "a.txt", "b\n", "Second");
    git(&f.mate, &["push", "-q", "--force", "origin", "main"]);
    git(&f.work, &["fetch", "-q", "origin"]);
    git(
        &f.work,
        &[
            "reflog",
            "expire",
            "--expire=now",
            "--expire-unreachable=now",
            "--all",
        ],
    );
    git(&f.work, &["gc", "-q", "--prune=now"]);
    f.svc
        .refresh(&f.repo_id, ChangeOrigin::Refresh)
        .await
        .unwrap();
    commit(&f.mate, "Jo", "c.txt", "c\n", "Third");
    git(&f.mate, &["push", "-q", "origin", "main"]);
    git(&f.work, &["fetch", "-q", "origin"]);
    f.svc
        .refresh(&f.repo_id, ChangeOrigin::Refresh)
        .await
        .unwrap();

    let feed = f.svc.workspace_activity(&f.workspace_id).await.unwrap();
    assert_eq!(kinds(&feed.items, ActivityKind::Rewritten), 1);
    assert_eq!(kinds(&feed.items, ActivityKind::Advanced), 2);
}

#[tokio::test]
async fn patterns_git_cannot_use_are_rejected() {
    let f = fixture().await;
    for bad in ["release/[0-9]*", "v?", "a*b*"] {
        let err = f
            .svc
            .update_activity_settings(
                &f.workspace_id,
                ActivitySettings {
                    watched_branches: vec!["main".into(), bad.into()],
                    auto_fetch: true,
                    ..ActivitySettings::default()
                },
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::Validation, "{bad}");
    }
}

#[tokio::test]
async fn a_linked_worktree_shares_one_feed() {
    let f = fixture().await;
    let wt = f.work.parent().unwrap().join("work-wt");
    git(
        &f.work,
        &["worktree", "add", "-q", wt.to_str().unwrap(), "-b", "wt"],
    );
    let ws = f
        .svc
        .update_workspace_membership(UpdateWorkspaceMembershipRequest {
            workspace_id: f.workspace_id.clone(),
            add: vec![wt.display().to_string()],
            remove: vec![],
        })
        .await
        .unwrap()
        .workspace;
    let ids: Vec<String> = ws
        .members
        .iter()
        .filter_map(|m| m.repository_id.clone())
        .collect();
    assert_eq!(ids.len(), 2);
    for id in &ids {
        f.svc.refresh(id, ChangeOrigin::Refresh).await.unwrap();
    }
    commit(&f.mate, "Alex", "x.txt", "x\n", "Work");
    git(&f.mate, &["push", "-q", "origin", "main"]);
    git(&f.work, &["fetch", "-q", "origin"]);
    for id in &ids {
        f.svc.refresh(id, ChangeOrigin::Refresh).await.unwrap();
    }
    let feed = f.svc.workspace_activity(&f.workspace_id).await.unwrap();
    assert_eq!(kinds(&feed.items, ActivityKind::Advanced), 1);
    assert_eq!(feed.freshness.len(), 1, "one Git directory");
    assert_eq!(
        f.svc
            .workspace(&f.workspace_id)
            .await
            .unwrap()
            .unseen_activity,
        1
    );
}

#[tokio::test]
async fn joining_another_workspace_is_not_news() {
    let f = fixture().await;
    git(&f.mate, &["push", "-q", "origin", "main:release/1.0"]);
    git(&f.mate, &["tag", "nightly-1"]);
    git(&f.mate, &["push", "-q", "origin", "nightly-1"]);
    f.svc.fetch(&f.repo_id, false).await.unwrap();
    // The new workspace watches refs that already exist.
    let other = f
        .svc
        .create_workspace(CreateWorkspaceRequest {
            name: "Releases".into(),
            discovery_mode: DiscoveryMode::Manual,
            discovery_root: None,
            discovery_path: None,
            paths: vec![f.work.display().to_string()],
        })
        .await
        .unwrap()
        .workspace;
    f.svc
        .update_activity_settings(
            &other.id,
            ActivitySettings {
                watched_branches: vec!["release/*".into()],
                watched_tags: vec!["nightly-*".into()],
                ..ActivitySettings::default()
            },
        )
        .await
        .unwrap();
    f.svc
        .refresh(&f.repo_id, ChangeOrigin::Refresh)
        .await
        .unwrap();
    let feed = f.svc.workspace_activity(&other.id).await.unwrap();
    assert!(feed.items.is_empty(), "{:?}", feed.items);
}

#[tokio::test]
async fn a_burst_of_tags_is_capped() {
    let f = fixture().await;
    for i in 0..30 {
        git(&f.mate, &["tag", &format!("v1.{i}")]);
    }
    git(&f.mate, &["push", "-q", "origin", "--tags"]);
    f.svc.fetch(&f.repo_id, false).await.unwrap();
    let feed = f.svc.workspace_activity(&f.workspace_id).await.unwrap();
    assert_eq!(kinds(&feed.items, ActivityKind::Tagged), 20);
    // The rest joined the baseline: nothing more arrives on the next pass.
    f.svc.fetch(&f.repo_id, false).await.unwrap();
    let feed = f.svc.workspace_activity(&f.workspace_id).await.unwrap();
    assert_eq!(kinds(&feed.items, ActivityKind::Tagged), 20);
}

#[tokio::test]
async fn a_leftover_lock_file_is_reported() {
    let f = fixture().await;
    let lock = f.work.join(".git").join("packed-refs.lock");
    std::fs::write(&lock, "").unwrap();
    let old = Command::new("touch")
        .args(["-t", "200001010000"])
        .arg(&lock)
        .status()
        .unwrap();
    assert!(old.success());
    let err = f.svc.fetch(&f.repo_id, false).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Io);
    assert!(err.message.contains("leftover"), "{}", err.message);
    let summary = f.svc.list().await.unwrap();
    assert!(summary[0].fetch_error.is_some(), "the error is recorded");
}
