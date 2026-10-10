//! Relocating registrations whose folders moved, and Rescan's move suggestions
//! (SPEC.md, Relocating a repository). Every fixture is built here with the
//! system Git; nothing is checked in.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

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

fn commit(dir: &Path, file: &str, message: &str) {
    std::fs::write(dir.join(file), format!("{message}\n")).unwrap();
    git(dir, &["add", file]);
    git(dir, &["commit", "-q", "-m", message]);
}

/// Create `dir` as a repository with one commit (the message makes histories differ).
fn repo(dir: &Path, message: &str) {
    std::fs::create_dir_all(dir).unwrap();
    git(dir, &["init", "-q", "-b", "main"]);
    commit(dir, "README.md", message);
}

fn canonical(p: &Path) -> String {
    p.canonicalize().unwrap().display().to_string()
}

fn s(p: &Path) -> String {
    p.display().to_string()
}

async fn service(tmp: &Path) -> Arc<RepositoryService> {
    let db = Db::open(&tmp.join("data").join("brainiac.sqlite3")).unwrap();
    let emitter: brainiac_lib::events::Emitter<brainiac_lib::models::RepositoryChangedEvent> =
        Arc::new(|_| {});
    Arc::new(RepositoryService::new(
        db,
        GitService::detect().await,
        Settings::default(),
        emitter,
    ))
}

fn request(id: &str, path: &Path) -> RelocateRepositoryRequest {
    RelocateRepositoryRequest {
        repository_id: id.to_string(),
        path: s(path),
        confirmed_root: None,
    }
}

/// A request that accepts the concerns about `root`.
fn confirmed(id: &str, path: &Path, root: &Path) -> RelocateRepositoryRequest {
    RelocateRepositoryRequest {
        confirmed_root: Some(canonical(root)),
        ..request(id, path)
    }
}

/// `remote.git` with one commit; `work` is a registered clone in the manual
/// workspace "Team", with its activity baseline taken; `mate` is a teammate's clone.
struct Team {
    tmp: tempfile::TempDir,
    work: PathBuf,
    mate: PathBuf,
    svc: Arc<RepositoryService>,
    repo_id: String,
    workspace_id: String,
}

impl Team {
    fn path(&self, name: &str) -> PathBuf {
        self.tmp.path().join(name)
    }

    /// The teammate pushes a commit; `dir` fetches it and is refreshed.
    async fn teammate_pushes(&self, dir: &Path, message: &str) {
        commit(&self.mate, &format!("{message}.txt"), message);
        git(&self.mate, &["push", "-q", "origin", "main"]);
        git(dir, &["fetch", "-q", "origin"]);
        self.svc
            .refresh(&self.repo_id, ChangeOrigin::Refresh)
            .await
            .unwrap();
    }

    async fn activity(&self) -> WorkspaceActivity {
        self.svc
            .workspace_activity(&self.workspace_id)
            .await
            .unwrap()
    }

    async fn summary(&self) -> RepositorySummary {
        self.svc
            .list()
            .await
            .unwrap()
            .into_iter()
            .find(|r| r.id == self.repo_id)
            .unwrap()
    }
}

async fn team() -> Team {
    let tmp = tempfile::tempdir().unwrap();
    let seed = tmp.path().join("seed");
    repo(&seed, "initial");
    git(tmp.path(), &["clone", "-q", "--bare", "seed", "remote.git"]);
    git(tmp.path(), &["clone", "-q", "remote.git", "work"]);
    git(tmp.path(), &["clone", "-q", "remote.git", "mate"]);
    let svc = service(tmp.path()).await;
    let work = tmp.path().join("work");
    let workspace = svc
        .create_workspace(CreateWorkspaceRequest {
            name: "Team".into(),
            discovery_mode: DiscoveryMode::Manual,
            discovery_root: None,
            discovery_path: None,
            paths: vec![s(&work)],
        })
        .await
        .unwrap()
        .workspace;
    let repo_id = workspace.members[0].repository_id.clone().unwrap();
    svc.refresh(&repo_id, ChangeOrigin::Refresh).await.unwrap();
    Team {
        mate: tmp.path().join("mate"),
        work,
        tmp,
        svc,
        repo_id,
        workspace_id: workspace.id,
    }
}

fn relocated(outcome: RelocationOutcome) -> (RepositorySummary, Vec<String>) {
    match outcome {
        RelocationOutcome::Relocated {
            repository,
            carried,
        } => (*repository, carried),
        other => panic!("expected a relocation, got {other:?}"),
    }
}

#[tokio::test]
async fn a_moved_repository_keeps_its_identity_and_feed() {
    let t = team().await;
    t.svc
        .set_pinned(PinEntityType::Repository, &t.repo_id, true)
        .await
        .unwrap();
    t.svc
        .set_tab(&t.repo_id, RepositoryTab::History)
        .await
        .unwrap();
    t.teammate_pushes(&t.work, "first").await;
    assert_eq!(t.activity().await.unseen, 1);

    let moved = t.path("moved-work");
    std::fs::rename(&t.work, &moved).unwrap();
    assert_eq!(t.summary().await.state, RepositoryState::Missing);

    let relocation = t
        .svc
        .relocate_repository(request(&t.repo_id, &moved))
        .await
        .unwrap();
    assert_eq!(relocation.moved.len(), 1);
    let (repository, carried) = relocated(relocation.outcome);
    assert!(carried.is_empty());
    assert_eq!(repository.id, t.repo_id);
    assert_eq!(repository.display_path, canonical(&moved));
    assert_eq!(repository.state, RepositoryState::Fresh);
    assert_eq!(repository.last_tab, Some(RepositoryTab::History));

    let snapshot = t.svc.snapshot().await.unwrap();
    assert!(snapshot.pins.iter().any(|p| p.entity_id == t.repo_id));
    let ws = t.svc.workspace(&t.workspace_id).await.unwrap();
    assert_eq!(ws.members[0].canonical_path, canonical(&moved));
    assert_eq!(ws.members[0].display_name, "moved-work");
    assert_eq!(ws.members[0].status, MemberStatus::Ok);

    // The feed and its read state moved with the Git directory, and the
    // move itself is not news.
    let feed = t.activity().await;
    assert_eq!(feed.items.len(), 1);
    assert_eq!(feed.unseen, 1);

    // Tracking continues at the new place.
    t.teammate_pushes(&moved, "second").await;
    let feed = t.activity().await;
    assert_eq!(feed.items.len(), 2);
    assert_eq!(feed.freshness.len(), 1);
}

#[tokio::test]
async fn a_fresh_clone_counts_as_the_same_repository() {
    let t = team().await;
    std::fs::remove_dir_all(&t.work).unwrap();
    git(t.tmp.path(), &["clone", "-q", "remote.git", "reclone"]);
    let outcome = t
        .svc
        .relocate_repository(request(&t.repo_id, &t.path("reclone")))
        .await
        .unwrap()
        .outcome;
    let (repository, _) = relocated(outcome);
    assert_eq!(repository.name, "reclone");
}

#[tokio::test]
async fn an_unrelated_repository_needs_confirmation_and_starts_over() {
    let t = team().await;
    t.teammate_pushes(&t.work, "first").await;
    let other = t.path("other");
    repo(&other, "something else");

    let outcome = t
        .svc
        .relocate_repository(request(&t.repo_id, &other))
        .await
        .unwrap()
        .outcome;
    assert_eq!(
        outcome,
        RelocationOutcome::NeedsConfirmation {
            root: canonical(&other),
            concerns: vec![RelocationConcern::UnrelatedHistory],
        }
    );
    // Nothing was written.
    assert_eq!(t.summary().await.display_path, canonical(&t.work));

    let (repository, _) = relocated(
        t.svc
            .relocate_repository(confirmed(&t.repo_id, &other, &other))
            .await
            .unwrap()
            .outcome,
    );
    assert_eq!(repository.display_path, canonical(&other));
    assert!(
        t.activity().await.items.is_empty(),
        "the old feed is dropped"
    );
}

#[tokio::test]
async fn a_subfolder_stands_for_its_working_tree() {
    let t = team().await;
    let moved = t.path("moved-work");
    std::fs::rename(&t.work, &moved).unwrap();
    std::fs::create_dir_all(moved.join("src")).unwrap();
    let outcome = t
        .svc
        .relocate_repository(request(&t.repo_id, &moved.join("src")))
        .await
        .unwrap()
        .outcome;
    assert_eq!(
        outcome,
        RelocationOutcome::NeedsConfirmation {
            root: canonical(&moved),
            concerns: vec![RelocationConcern::InsideRepository],
        }
    );
}

#[tokio::test]
async fn refusals() {
    let t = team().await;
    // A folder registered as another repository.
    let summary = t.svc.register(&t.mate).await.unwrap();
    let err = t
        .svc
        .relocate_repository(confirmed(&t.repo_id, &t.mate, &t.mate))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Conflict);
    assert_ne!(summary.id, t.repo_id);

    // A plain folder.
    let plain = t.path("plain");
    std::fs::create_dir_all(&plain).unwrap();
    let err = t
        .svc
        .relocate_repository(request(&t.repo_id, &plain))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Validation);
}

#[tokio::test]
async fn a_linked_worktree_follows_its_moved_git_directory() {
    let t = team().await;
    let wt = t.path("work-wt");
    git(
        &t.work,
        &["worktree", "add", "-q", wt.to_str().unwrap(), "-b", "wt"],
    );
    let ws = t
        .svc
        .update_workspace_membership(UpdateWorkspaceMembershipRequest {
            workspace_id: t.workspace_id.clone(),
            add: vec![s(&wt)],
            remove: vec![],
        })
        .await
        .unwrap()
        .workspace;
    let wt_id = ws
        .members
        .iter()
        .filter_map(|m| m.repository_id.clone())
        .find(|id| *id != t.repo_id)
        .unwrap();
    t.teammate_pushes(&t.work, "first").await;

    let moved = t.path("moved-work");
    std::fs::rename(&t.work, &moved).unwrap();
    let relocation = t
        .svc
        .relocate_repository(request(&t.repo_id, &moved))
        .await
        .unwrap();
    assert_eq!(
        relocation.moved.len(),
        2,
        "the main checkout and its worktree"
    );
    let wt_row = t.svc.row(&wt_id).await.unwrap();
    assert_eq!(wt_row.common_git_dir, canonical(&moved.join(".git")));
    assert!(wt_row.git_dir.starts_with(&wt_row.common_git_dir));

    // Once Git's own pointers are repaired, the worktree works again and
    // shares the moved feed.
    git(&moved, &["worktree", "repair", wt.to_str().unwrap()]);
    let summary = t.svc.refresh(&wt_id, ChangeOrigin::Refresh).await.unwrap();
    assert!(summary.error.is_none(), "{:?}", summary.error);
    let feed = t.activity().await;
    assert_eq!(feed.freshness.len(), 1, "one Git directory");
    assert_eq!(feed.items.len(), 1);
}

/// `product/` is a repository whose `services/` folder holds `search` and
/// `billing` and a plain `docs` folder, all tracked in a discovered workspace.
async fn product(tmp: &Path, svc: &Arc<RepositoryService>) -> (PathBuf, Workspace) {
    let product = tmp.join("product");
    repo(&product, "product");
    repo(&product.join("services").join("search"), "search");
    repo(&product.join("services").join("billing"), "billing");
    std::fs::create_dir_all(product.join("services").join("docs")).unwrap();
    let services = product.join("services");
    let ws = svc
        .create_workspace(CreateWorkspaceRequest {
            name: "Product".into(),
            discovery_mode: DiscoveryMode::Discovered,
            discovery_root: Some(s(&product)),
            discovery_path: Some("services".into()),
            paths: vec![
                s(&product),
                s(&services.join("search")),
                s(&services.join("billing")),
                s(&services.join("docs")),
            ],
        })
        .await
        .unwrap()
        .workspace;
    for id in ws.members.iter().filter_map(|m| m.repository_id.as_ref()) {
        svc.refresh(id, ChangeOrigin::Refresh).await.unwrap();
    }
    (product, ws)
}

#[tokio::test]
async fn moving_a_workspace_root_carries_what_was_inside_it() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = service(tmp.path()).await;
    let (product, ws) = product(tmp.path(), &svc).await;
    let root_id = ws.root_repository_id.clone().unwrap();

    let renamed = tmp.path().join("product-2");
    std::fs::rename(&product, &renamed).unwrap();
    let relocation = svc
        .relocate_repository(request(&root_id, &renamed))
        .await
        .unwrap();
    assert_eq!(relocation.moved.len(), 3);
    let (_, mut carried) = relocated(relocation.outcome);
    carried.sort();
    assert_eq!(carried, vec!["billing", "search"]);

    let ws = svc.workspace(&ws.id).await.unwrap();
    assert_eq!(ws.discovery_root, Some(canonical(&renamed)));
    let members: Vec<_> = ws
        .members
        .iter()
        .map(|m| (m.display_name.as_str(), m.origin, m.status))
        .collect();
    assert_eq!(
        members,
        vec![
            ("product-2", MemberOrigin::Discovered, MemberStatus::Ok),
            ("billing", MemberOrigin::Discovered, MemberStatus::Ok),
            ("docs", MemberOrigin::Discovered, MemberStatus::NotGit),
            ("search", MemberOrigin::Discovered, MemberStatus::Ok),
        ]
    );
    assert!(ws
        .members
        .iter()
        .all(|m| m.canonical_path.starts_with(&canonical(&renamed))));
    // Rescan works from the new folder and finds nothing new.
    let rescan = svc.rescan_workspace(&ws.id).await.unwrap();
    assert!(rescan.repositories.is_empty());
    assert!(rescan.moves.is_empty());
}

#[tokio::test]
async fn a_member_moved_out_of_the_discovery_folder_becomes_manual() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = service(tmp.path()).await;
    let (product, ws) = product(tmp.path(), &svc).await;
    let search = product.join("services").join("search");
    let search_id = ws
        .members
        .iter()
        .find(|m| m.display_name == "search")
        .and_then(|m| m.repository_id.clone())
        .unwrap();
    let outside = tmp.path().join("search");
    std::fs::rename(&search, &outside).unwrap();
    relocated(
        svc.relocate_repository(request(&search_id, &outside))
            .await
            .unwrap()
            .outcome,
    );
    let ws = svc.workspace(&ws.id).await.unwrap();
    let member = ws
        .members
        .iter()
        .find(|m| m.repository_id.as_deref() == Some(search_id.as_str()))
        .unwrap();
    assert_eq!(member.origin, MemberOrigin::Manual);
    assert_eq!(member.canonical_path, canonical(&outside));
}

/// `code/` is a plain folder with `web` and `api`, tracked in a discovered workspace.
async fn code(tmp: &Path, svc: &Arc<RepositoryService>) -> (PathBuf, Workspace) {
    let code = tmp.join("code");
    repo(&code.join("web"), "web");
    repo(&code.join("api"), "api");
    let ws = svc
        .create_workspace(CreateWorkspaceRequest {
            name: "Code".into(),
            discovery_mode: DiscoveryMode::Discovered,
            discovery_root: Some(s(&code)),
            discovery_path: None,
            paths: vec![s(&code.join("web")), s(&code.join("api"))],
        })
        .await
        .unwrap()
        .workspace;
    // Record each HEAD, which is the identity evidence of a repository without remotes.
    for id in ws.members.iter().filter_map(|m| m.repository_id.as_ref()) {
        svc.refresh(id, ChangeOrigin::Refresh).await.unwrap();
    }
    (code, ws)
}

#[tokio::test]
async fn rescan_suggests_a_renamed_member() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = service(tmp.path()).await;
    let (code, ws) = code(tmp.path(), &svc).await;
    let api_id = ws
        .members
        .iter()
        .find(|m| m.display_name == "api")
        .and_then(|m| m.repository_id.clone())
        .unwrap();
    std::fs::rename(code.join("api"), code.join("api-v2")).unwrap();
    repo(&code.join("new"), "new");
    std::fs::create_dir_all(code.join("scratch")).unwrap();

    let rescan = svc.rescan_workspace(&ws.id).await.unwrap();
    assert_eq!(
        rescan.moves,
        vec![SuggestedMove {
            repository_id: api_id.clone(),
            name: "api".into(),
            from: canonical(&code).to_string() + "/api",
            to: canonical(&code.join("api-v2")),
        }]
    );
    let names: Vec<_> = rescan
        .repositories
        .iter()
        .map(|e| e.display_name.as_str())
        .collect();
    assert_eq!(names, vec!["new"]);
    assert_eq!(rescan.skipped, 1);

    // Accepting the suggestion is an ordinary relocation.
    relocated(
        svc.relocate_repository(request(&api_id, &code.join("api-v2")))
            .await
            .unwrap()
            .outcome,
    );
    let rescan = svc.rescan_workspace(&ws.id).await.unwrap();
    assert!(rescan.moves.is_empty());
    let ws = svc.workspace(&ws.id).await.unwrap();
    assert!(ws.members.iter().all(|m| m.status == MemberStatus::Ok));
}

#[tokio::test]
async fn rescan_suggests_nothing_when_the_match_is_ambiguous() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = service(tmp.path()).await;
    let (code, ws) = code(tmp.path(), &svc).await;
    std::fs::rename(code.join("api"), code.join("api-v2")).unwrap();
    git(&code, &["clone", "-q", "api-v2", "api-copy"]);

    let rescan = svc.rescan_workspace(&ws.id).await.unwrap();
    assert!(rescan.moves.is_empty());
    let mut names: Vec<_> = rescan
        .repositories
        .iter()
        .map(|e| e.display_name.as_str())
        .collect();
    names.sort();
    assert_eq!(names, vec!["api-copy", "api-v2"]);
}

#[tokio::test]
async fn rescan_needs_a_discovered_workspace() {
    let t = team().await;
    let err = t.svc.rescan_workspace(&t.workspace_id).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Validation);
}

// Regressions from review.

#[tokio::test]
async fn switching_to_a_clone_leaves_the_old_worktrees_alone() {
    let t = team().await;
    let wt = t.path("work-wt");
    git(
        &t.work,
        &["worktree", "add", "-q", wt.to_str().unwrap(), "-b", "wt"],
    );
    let wt_id = t.svc.register(&wt).await.unwrap().id;
    let before = t.svc.row(&wt_id).await.unwrap();
    git(t.tmp.path(), &["clone", "-q", "remote.git", "reclone"]);

    // The main checkout switches to a clone while its folder still exists.
    let relocation = t
        .svc
        .relocate_repository(request(&t.repo_id, &t.path("reclone")))
        .await
        .unwrap();
    assert_eq!(relocation.moved.len(), 1);
    assert_eq!(t.svc.row(&wt_id).await.unwrap(), before);

    // The worktree switches to a clone: the main checkout is untouched.
    let main_before = t.svc.row(&t.repo_id).await.unwrap();
    git(t.tmp.path(), &["clone", "-q", "remote.git", "reclone-2"]);
    t.svc
        .relocate_repository(request(&wt_id, &t.path("reclone-2")))
        .await
        .unwrap();
    assert_eq!(t.svc.row(&t.repo_id).await.unwrap(), main_before);
}

#[tokio::test]
async fn concurrent_relocations_keep_the_feed() {
    let t = team().await;
    t.teammate_pushes(&t.work, "first").await;
    let moved = t.path("moved-work");
    std::fs::rename(&t.work, &moved).unwrap();
    let (a, b) = tokio::join!(
        t.svc.relocate_repository(request(&t.repo_id, &moved)),
        t.svc.relocate_repository(request(&t.repo_id, &moved)),
    );
    assert!(a.is_ok() || b.is_ok());
    for r in [a, b] {
        if let Err(e) = r {
            assert_eq!(e.code, ErrorCode::Conflict);
        }
    }
    assert_eq!(t.activity().await.items.len(), 1);
}

#[tokio::test]
async fn renaming_a_plain_discovery_folder_moves_the_workspace() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = service(tmp.path()).await;
    let (code, ws) = code(tmp.path(), &svc).await;
    let web_id = ws
        .members
        .iter()
        .find(|m| m.display_name == "web")
        .and_then(|m| m.repository_id.clone())
        .unwrap();
    let renamed = tmp.path().join("code-2");
    std::fs::rename(&code, &renamed).unwrap();

    let (_, carried) = relocated(
        svc.relocate_repository(request(&web_id, &renamed.join("web")))
            .await
            .unwrap()
            .outcome,
    );
    assert_eq!(carried, vec!["api"]);
    let ws = svc.workspace(&ws.id).await.unwrap();
    assert_eq!(ws.discovery_root, Some(canonical(&renamed)));
    assert!(ws
        .members
        .iter()
        .all(|m| m.status == MemberStatus::Ok && m.origin == MemberOrigin::Discovered));
    assert!(svc.rescan_workspace(&ws.id).await.is_ok());
}

#[tokio::test]
async fn a_root_relocated_away_from_its_folder_stops_being_the_root() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = service(tmp.path()).await;
    let (product, ws) = product(tmp.path(), &svc).await;
    let root_id = ws.root_repository_id.clone().unwrap();
    let clone = tmp.path().join("product-clone");
    git(
        tmp.path(),
        &["clone", "-q", product.to_str().unwrap(), "product-clone"],
    );
    relocated(
        svc.relocate_repository(request(&root_id, &clone))
            .await
            .unwrap()
            .outcome,
    );
    let ws = svc.workspace(&ws.id).await.unwrap();
    assert_eq!(ws.root_repository_id, None);
    assert_eq!(ws.discovery_root, Some(canonical(&product)));
    let member = ws
        .members
        .iter()
        .find(|m| m.repository_id.as_deref() == Some(root_id.as_str()))
        .unwrap();
    assert_eq!(member.origin, MemberOrigin::Manual);
}

#[tokio::test]
async fn moving_back_and_locating_watches_again() {
    let t = team().await;
    let relocation = t
        .svc
        .relocate_repository(request(&t.repo_id, &t.work))
        .await
        .unwrap();
    assert_eq!(relocation.moved.len(), 1, "returned for re-watching");
}

#[tokio::test]
async fn a_confirmation_only_covers_the_root_it_was_given_for() {
    let t = team().await;
    let other = t.path("other");
    repo(&other, "something else");
    let third = t.path("third");
    repo(&third, "third");
    let outcome = t
        .svc
        .relocate_repository(confirmed(&t.repo_id, &third, &other))
        .await
        .unwrap()
        .outcome;
    assert!(matches!(
        outcome,
        RelocationOutcome::NeedsConfirmation { .. }
    ));
}

#[tokio::test]
async fn checking_identity_never_downloads_into_a_partial_clone() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    repo(&src, "one");
    git(&src, &["config", "uploadpack.allowFilter", "true"]);
    let url = format!("file://{}", canonical(&src));
    git(
        tmp.path(),
        &["clone", "-q", "--filter=blob:none", &url, "partial"],
    );
    commit(&src, "two.txt", "two");
    let missing = git(&src, &["rev-parse", "HEAD"]);
    let partial = tmp.path().join("partial");

    let service = GitService::detect().await.unwrap();
    let found = service
        .contains_any(&partial, std::slice::from_ref(&missing))
        .await
        .unwrap();
    assert_ne!(found, Some(true));
    let out = Command::new("git")
        .args(["cat-file", "-e", &missing])
        .current_dir(&partial)
        .env("GIT_NO_LAZY_FETCH", "1")
        .output()
        .unwrap();
    assert!(!out.status.success(), "the commit was downloaded");
}
