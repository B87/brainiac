//! Workspace discovery and membership against real temporary folders.
//! Every fixture is built here with the system Git; nothing is checked in.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use brainiac_lib::db::Db;
use brainiac_lib::git::GitService;
use brainiac_lib::models::*;
use brainiac_lib::workspaces::RepositoryService;

fn git(dir: &Path, args: &[&str]) {
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
}

/// Create `dir` as a repository with one commit.
fn repo(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    git(dir, &["init", "-q", "-b", "main"]);
    std::fs::write(dir.join("README.md"), "hello\n").unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-q", "-m", "initial"]);
}

fn plain(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("notes.txt"), "plain\n").unwrap();
}

fn canonical(p: &Path) -> String {
    p.canonicalize().unwrap().display().to_string()
}

fn s(p: &Path) -> String {
    p.display().to_string()
}

async fn service(tmp: &Path) -> Arc<RepositoryService> {
    let db = Db::open(&tmp.join("data").join("brainiac.sqlite3")).unwrap();
    let emitter: brainiac_lib::workspaces::Emitter = Arc::new(|_| {});
    Arc::new(RepositoryService::new(
        db,
        GitService::detect().await,
        Settings::default(),
        emitter,
    ))
}

/// `product/` is a repository whose `services/` folder holds two independent
/// repositories, a plain folder, and a hidden repository.
fn product_layout(tmp: &Path) -> PathBuf {
    let product = tmp.join("product");
    repo(&product);
    repo(&product.join("services").join("search"));
    repo(&product.join("services").join("billing"));
    plain(&product.join("services").join("docs"));
    repo(&product.join("services").join(".hidden"));
    plain(&product.join("assets"));
    product
}

/// `code/` is a plain folder holding two repositories and a plain folder.
fn code_layout(tmp: &Path) -> PathBuf {
    let code = tmp.join("code");
    repo(&code.join("web"));
    repo(&code.join("api"));
    plain(&code.join("scratch"));
    std::fs::write(code.join("todo.txt"), "a file, not a folder\n").unwrap();
    code
}

fn names(preview: &WorkspacePreview) -> Vec<(&str, PreviewStatus)> {
    preview
        .entries
        .iter()
        .map(|e| (e.display_name.as_str(), e.status))
        .collect()
}

#[tokio::test]
async fn discovery_in_a_repository_with_a_discovery_subfolder() {
    let tmp = tempfile::tempdir().unwrap();
    let product = product_layout(tmp.path());
    let svc = service(tmp.path()).await;

    let preview = svc
        .discover_repositories(&s(&product), Some("services"))
        .await
        .unwrap();
    assert_eq!(preview.name, "product");
    assert_eq!(preview.discovery_mode, DiscoveryMode::Discovered);
    assert_eq!(
        names(&preview),
        vec![
            ("product", PreviewStatus::Ok),
            ("billing", PreviewStatus::Ok),
            ("docs", PreviewStatus::NotGit),
            ("search", PreviewStatus::Ok),
        ]
    );
    let root = &preview.entries[0];
    assert_eq!(
        root.repository_root.as_deref(),
        Some(canonical(&product).as_str())
    );
    assert!(root.existing_repository_id.is_none());
    // A plain folder inside the root inherits the root's Git context; it is not a repository.
    let docs = &preview.entries[2];
    assert!(docs.repository_root.is_none());
    assert!(docs.message.as_deref().unwrap().contains("belongs to"));

    // Without a discovery path the root's own children are scanned: `services`
    // and `assets` are plain folders inside the root.
    let preview = svc.discover_repositories(&s(&product), None).await.unwrap();
    assert_eq!(
        names(&preview),
        vec![
            ("product", PreviewStatus::Ok),
            ("assets", PreviewStatus::NotGit),
            ("services", PreviewStatus::NotGit),
        ]
    );

    // Registered repositories are recognized.
    let billing = svc
        .register(&product.join("services").join("billing"))
        .await
        .unwrap();
    let preview = svc
        .discover_repositories(&s(&product), Some("./services/"))
        .await
        .unwrap();
    assert_eq!(
        preview.entries[1].existing_repository_id.as_deref(),
        Some(billing.id.as_str())
    );

    // A missing discovery folder is reported, and the root stays usable.
    let preview = svc
        .discover_repositories(&s(&product), Some("packages"))
        .await
        .unwrap();
    assert_eq!(
        names(&preview),
        vec![
            ("product", PreviewStatus::Ok),
            ("packages", PreviewStatus::Missing)
        ]
    );
}

#[tokio::test]
async fn discovery_in_a_plain_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let code = code_layout(tmp.path());
    let outside = tmp.path().join("outside");
    repo(&outside);
    std::os::unix::fs::symlink(&outside, code.join("linked")).unwrap();
    let svc = service(tmp.path()).await;

    let preview = svc.discover_repositories(&s(&code), None).await.unwrap();
    assert_eq!(preview.name, "code");
    assert_eq!(
        names(&preview),
        vec![
            ("api", PreviewStatus::Ok),
            ("linked", PreviewStatus::Unsupported),
            ("scratch", PreviewStatus::NotGit),
            ("web", PreviewStatus::Ok),
        ]
    );
    assert_eq!(
        preview.entries[2].message.as_deref(),
        Some("Not a Git repository.")
    );

    // A plain discovery subfolder with no repositories inside.
    let preview = svc
        .discover_repositories(&s(&code), Some("scratch"))
        .await
        .unwrap();
    assert!(preview.entries.is_empty());
}

#[tokio::test]
async fn discovery_rejects_escaping_paths_and_missing_folders() {
    let tmp = tempfile::tempdir().unwrap();
    let code = code_layout(tmp.path());
    let svc = service(tmp.path()).await;

    for bad in ["..", "../code", "/tmp", "web/../.."] {
        let err = svc
            .discover_repositories(&s(&code), Some(bad))
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::Validation, "{bad}");
    }
    // A symbolic link that leaves the folder is rejected as well.
    std::os::unix::fs::symlink(tmp.path(), code.join("up")).unwrap();
    let err = svc
        .discover_repositories(&s(&code), Some("up"))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Validation);

    let err = svc
        .discover_repositories(&s(&tmp.path().join("nope")), None)
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);
}

#[tokio::test]
async fn creating_and_updating_workspaces_reuses_registrations() {
    let tmp = tempfile::tempdir().unwrap();
    let product = product_layout(tmp.path());
    let services = product.join("services");
    let svc = service(tmp.path()).await;

    let request = CreateWorkspaceRequest {
        name: "  Product  ".into(),
        discovery_mode: DiscoveryMode::Discovered,
        discovery_root: Some(s(&product)),
        discovery_path: Some("services".into()),
        paths: vec![
            s(&services.join("search")),
            s(&product),
            s(&services.join("billing")),
            s(&services.join("billing")),
        ],
    };
    let created = svc.create_workspace(request.clone()).await.unwrap();
    let ws = created.workspace;
    assert_eq!(ws.name, "Product");
    assert_eq!(created.new_repositories.len(), 3);
    assert_eq!(
        ws.discovery_root.as_deref(),
        Some(canonical(&product).as_str())
    );
    assert_eq!(ws.discovery_path.as_deref(), Some("services"));
    let member_names: Vec<_> = ws.members.iter().map(|m| m.display_name.as_str()).collect();
    assert_eq!(member_names, vec!["product", "billing", "search"]);
    assert_eq!(ws.members[0].repository_id, ws.root_repository_id);
    assert!(ws.root_repository_id.is_some());
    assert!(ws
        .members
        .iter()
        .all(|m| m.origin == MemberOrigin::Discovered && m.status == MemberStatus::Ok));

    // A second workspace over the same folders registers nothing new.
    let again = svc.create_workspace(request).await.unwrap();
    assert!(again.new_repositories.is_empty());
    assert_eq!(svc.rows().await.unwrap().len(), 3);

    // Add a manual repository from elsewhere and a plain folder; drop billing.
    let elsewhere = tmp.path().join("elsewhere");
    repo(&elsewhere);
    let billing_path = canonical(&services.join("billing"));
    let billing_id = ws
        .members
        .iter()
        .find(|m| m.canonical_path == billing_path)
        .and_then(|m| m.repository_id.clone())
        .unwrap();
    let update = UpdateWorkspaceMembershipRequest {
        workspace_id: ws.id.clone(),
        add: vec![s(&elsewhere), s(&services.join("docs"))],
        remove: vec![billing_path.clone()],
    };
    let updated = svc
        .update_workspace_membership(update.clone())
        .await
        .unwrap();
    assert_eq!(updated.new_repositories.len(), 1);
    let ws2 = updated.workspace;
    let summary: Vec<_> = ws2
        .members
        .iter()
        .map(|m| (m.display_name.as_str(), m.origin, m.status))
        .collect();
    assert_eq!(
        summary,
        vec![
            ("product", MemberOrigin::Discovered, MemberStatus::Ok),
            // `docs` is inside the discovery folder, so it counts as discovered.
            ("docs", MemberOrigin::Discovered, MemberStatus::NotGit),
            ("elsewhere", MemberOrigin::Manual, MemberStatus::Ok),
            ("search", MemberOrigin::Discovered, MemberStatus::Ok),
        ]
    );
    // Removing a member keeps the repository registered.
    assert!(svc.row(&billing_id).await.is_ok());

    // Repeating the same update changes nothing and registers nothing.
    let repeated = svc.update_workspace_membership(update).await.unwrap();
    assert!(repeated.new_repositories.is_empty());
    assert_eq!(repeated.workspace.members.len(), 4);
    assert_eq!(svc.rows().await.unwrap().len(), 4);

    // Removing the root member clears the root; adding it back restores it.
    let root_path = canonical(&product);
    let no_root = svc
        .update_workspace_membership(UpdateWorkspaceMembershipRequest {
            workspace_id: ws.id.clone(),
            add: vec![],
            remove: vec![root_path.clone()],
        })
        .await
        .unwrap()
        .workspace;
    assert!(no_root.root_repository_id.is_none());
    let restored = svc
        .update_workspace_membership(UpdateWorkspaceMembershipRequest {
            workspace_id: ws.id.clone(),
            add: vec![root_path],
            remove: vec![],
        })
        .await
        .unwrap()
        .workspace;
    assert_eq!(restored.root_repository_id, ws.root_repository_id);
    assert_eq!(restored.members[0].display_name, "product");

    // Missing paths are rejected before anything is written.
    let err = svc
        .update_workspace_membership(UpdateWorkspaceMembershipRequest {
            workspace_id: ws.id.clone(),
            add: vec![s(&tmp.path().join("gone"))],
            remove: vec![],
        })
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);
}

#[tokio::test]
async fn snapshot_lists_workspaces_by_name_with_the_root_first() {
    let tmp = tempfile::tempdir().unwrap();
    let product = product_layout(tmp.path());
    let code = code_layout(tmp.path());
    let svc = service(tmp.path()).await;

    let manual = svc
        .create_workspace(CreateWorkspaceRequest {
            name: "beta".into(),
            discovery_mode: DiscoveryMode::Manual,
            discovery_root: None,
            discovery_path: None,
            paths: vec![s(&code.join("web")), s(&code.join("api"))],
        })
        .await
        .unwrap()
        .workspace;
    assert!(manual.root_repository_id.is_none());
    assert!(manual
        .members
        .iter()
        .all(|m| m.origin == MemberOrigin::Manual));

    let discovered = svc
        .create_workspace(CreateWorkspaceRequest {
            name: "Alpha".into(),
            discovery_mode: DiscoveryMode::Discovered,
            discovery_root: Some(s(&product)),
            discovery_path: Some("services".into()),
            paths: vec![
                s(&product.join("services").join("search")),
                s(&product.join("services").join("billing")),
                s(&product),
            ],
        })
        .await
        .unwrap()
        .workspace;

    let snap = svc.snapshot().await.unwrap();
    let ws_names: Vec<_> = snap.workspaces.iter().map(|w| w.name.as_str()).collect();
    assert_eq!(ws_names, vec!["Alpha", "beta"]);
    let alpha = &snap.workspaces[0];
    let alpha_members: Vec<_> = alpha
        .members
        .iter()
        .map(|m| m.display_name.as_str())
        .collect();
    assert_eq!(alpha_members, vec!["product", "billing", "search"]);
    assert_eq!(alpha.members[0].repository_id, alpha.root_repository_id);
    let beta_members: Vec<_> = snap.workspaces[1]
        .members
        .iter()
        .map(|m| m.display_name.as_str())
        .collect();
    assert_eq!(beta_members, vec!["api", "web"]);
    // Joining a workspace does not mark repositories as recently opened.
    assert!(snap.recent_repository_ids.is_empty());

    // A member whose folder disappeared is reported missing.
    std::fs::rename(code.join("web"), tmp.path().join("moved-web")).unwrap();
    let beta = svc.workspace(&manual.id).await.unwrap();
    assert_eq!(beta.members[1].display_name, "web");
    assert_eq!(beta.members[1].status, MemberStatus::Missing);

    // Rename, pin, and remove.
    let renamed = svc.rename_workspace(&manual.id, "Gamma").await.unwrap();
    assert_eq!(renamed.name, "Gamma");
    let err = svc.rename_workspace(&manual.id, "   ").await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Validation);
    svc.set_pinned(PinEntityType::Workspace, &manual.id, true)
        .await
        .unwrap();
    assert_eq!(svc.snapshot().await.unwrap().pins.len(), 1);
    let err = svc
        .set_pinned(PinEntityType::Workspace, "no-such-id", true)
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);
    let before = svc.rows().await.unwrap().len();
    svc.remove_workspace(&manual.id).await.unwrap();
    let snap = svc.snapshot().await.unwrap();
    assert_eq!(snap.workspaces.len(), 1);
    assert!(snap.pins.is_empty());
    assert_eq!(snap.repositories.len(), before);
    let err = svc.remove_workspace(&manual.id).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);

    // Removing a repository registration drops its memberships, not the workspace.
    let root_id = discovered.root_repository_id.clone().unwrap();
    svc.remove(&root_id).await.unwrap();
    let alpha = svc.workspace(&discovered.id).await.unwrap();
    assert!(alpha.root_repository_id.is_none());
    let alpha_members: Vec<_> = alpha
        .members
        .iter()
        .map(|m| m.display_name.as_str())
        .collect();
    assert_eq!(alpha_members, vec!["billing", "search"]);
}
