//! `PullRequestService` (SPEC.md, section 10) over a local server standing in
//! for GitHub and Bitbucket: lists per workspace and repository, what is
//! cached and for how long, a repository that fails beside ones that work,
//! and `pr_changed` events. The adapters' mapping of real answers is checked
//! by the opt-in tests in `forge_live.rs`.

use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};

use brainiac_lib::db::Db;
use brainiac_lib::forge::http::Http;
use brainiac_lib::forge::keychain::MemoryKeychain;
use brainiac_lib::forge::{AccountService, Endpoints, PullRequestService};
use brainiac_lib::git::GitService;
use brainiac_lib::models::*;
use brainiac_lib::workspaces::RepositoryService;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

// --- A provider that answers by route -------------------------------------

/// A route: method, a path prefix, a header substring (the authorization
/// scheme tells the providers apart, since both share one base URL here), a
/// body substring (for GraphQL), and the answer.
struct Route {
    method: &'static str,
    path: &'static str,
    head_contains: &'static str,
    body_contains: &'static str,
    status: u16,
    headers: Vec<(&'static str, &'static str)>,
    body: String,
}

#[derive(Clone, Default)]
struct Requests(Arc<Mutex<Vec<(String, String)>>>);

impl Requests {
    fn count(&self, path_contains: &str) -> usize {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|(p, _)| p.contains(path_contains))
            .count()
    }
    fn total(&self) -> usize {
        self.0.lock().unwrap().len()
    }
}

async fn serve(routes: Vec<Route>) -> (String, Requests) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let requests = Requests::default();
    let seen = requests.clone();
    let routes = Arc::new(routes);
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            let routes = Arc::clone(&routes);
            let seen = seen.clone();
            tokio::spawn(async move {
                let mut raw = Vec::new();
                let mut buf = [0u8; 4096];
                let (head_end, mut body_len) = loop {
                    let n = socket.read(&mut buf).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    raw.extend_from_slice(&buf[..n]);
                    if let Some(i) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&raw[..i]).to_lowercase();
                        let len = head
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length:"))
                            .and_then(|v| v.trim().parse::<usize>().ok())
                            .unwrap_or(0);
                        break (i + 4, len);
                    }
                };
                while raw.len() < head_end + body_len {
                    let n = socket.read(&mut buf).await.unwrap_or(0);
                    if n == 0 {
                        body_len = raw.len() - head_end;
                        break;
                    }
                    raw.extend_from_slice(&buf[..n]);
                }
                let head = String::from_utf8_lossy(&raw[..head_end]).into_owned();
                let body =
                    String::from_utf8_lossy(&raw[head_end..head_end + body_len]).into_owned();
                let mut first = head.lines().next().unwrap_or("").split(' ');
                let method = first.next().unwrap_or("").to_string();
                let path = first.next().unwrap_or("").to_string();
                seen.0.lock().unwrap().push((path.clone(), body.clone()));
                let lower = head.to_lowercase();
                let route = routes.iter().find(|r| {
                    r.method == method
                        && path.starts_with(r.path)
                        && lower.contains(r.head_contains)
                        && body.contains(r.body_contains)
                });
                let (status, headers, body) = match route {
                    Some(r) => (r.status, r.headers.clone(), r.body.clone()),
                    None => (599, Vec::new(), format!("no route for {method} {path}")),
                };
                let mut response = format!(
                    "HTTP/1.1 {status} X\r\ncontent-length: {}\r\ncontent-type: application/json\r\nconnection: close\r\n",
                    body.len()
                );
                for (name, value) in headers {
                    response.push_str(&format!("{name}: {value}\r\n"));
                }
                response.push_str("\r\n");
                response.push_str(&body);
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.shutdown().await;
            });
        }
    });
    (base, requests)
}

fn route(method: &'static str, path: &'static str, body: impl Into<String>) -> Route {
    Route {
        method,
        path,
        head_contains: "",
        body_contains: "",
        status: 200,
        headers: Vec::new(),
        body: body.into(),
    }
}

// --- Canned answers ----------------------------------------------------------

const GITHUB_USER: &str = r#"{"login":"octo","id":42,"name":"Octo Cat"}"#;
const BITBUCKET_USER: &str = r#"{"username":"jo","display_name":"Jo Doe","uuid":"{jo}"}"#;
const BITBUCKET_SCOPES: (&str, &str) = (
    "x-oauth-scopes",
    "read:user:bitbucket, read:repository:bitbucket, read:pullrequest:bitbucket, write:pullrequest:bitbucket",
);

fn github_pr(number: u64, updated: &str, head: &str) -> String {
    format!(
        r#"{{
        "number": {number}, "title": "Add parser", "body": "Parses things.", "isDraft": false, "state": "OPEN",
        "mergedAt": null, "closedAt": null, "createdAt": "2026-10-01T10:00:00Z", "updatedAt": "{updated}",
        "url": "https://github.com/acme/api/pull/{number}",
        "author": {{"login": "ada", "databaseId": 7, "name": "Ada"}},
        "headRefName": "feature", "headRefOid": "{head}", "headRepository": {{"nameWithOwner": "acme/api"}},
        "baseRefName": "main", "mergeable": "MERGEABLE", "additions": 10, "deletions": 2, "changedFiles": 2,
        "commits": {{"totalCount": 3}}, "comments": {{"totalCount": 1}},
        "reviewThreads": {{"nodes": [{{"isResolved": false}}, {{"isResolved": true}}]}},
        "reviewRequests": {{"nodes": [{{"requestedReviewer": {{"__typename": "User", "login": "octo", "databaseId": 42, "name": "Octo Cat"}}}}]}},
        "latestReviews": {{"nodes": [{{"state": "APPROVED", "author": {{"login": "bob", "databaseId": 9, "name": null}}}}]}},
        "lastCommit": {{"nodes": [{{"commit": {{"statusCheckRollup": {{"contexts": {{"nodes": [
            {{"__typename": "CheckRun", "name": "build", "status": "COMPLETED", "conclusion": "SUCCESS", "detailsUrl": "https://ci/1", "title": null}},
            {{"__typename": "StatusContext", "context": "lint", "state": "PENDING", "targetUrl": null, "description": "running"}}
        ]}}}}}}}}]}}
    }}"#
    )
}

fn github_list(prs: &[String]) -> String {
    format!(
        r#"{{"data": {{"repository": {{"viewerPermission": "WRITE", "pullRequests": {{
            "pageInfo": {{"hasNextPage": false, "endCursor": null}}, "nodes": [{}]}}}}}}}}"#,
        prs.join(",")
    )
}

fn github_get(pr: &str) -> String {
    format!(r#"{{"data": {{"repository": {{"viewerPermission": "WRITE", "pullRequest": {pr}}}}}}}"#)
}

const GITHUB_FILES: &str = r#"[
    {"filename": "src/parse.rs", "status": "modified", "additions": 8, "deletions": 2, "patch": "@@"},
    {"filename": "docs/new.md", "previous_filename": "docs/old.md", "status": "renamed", "additions": 2, "deletions": 0, "patch": "@@"}
]"#;

fn bitbucket_pr(id: u64, updated: &str, head: &str) -> String {
    format!(
        r#"{{
        "id": {id}, "title": "Fix login", "state": "OPEN", "draft": false,
        "author": {{"uuid": "{{jo}}", "nickname": "jo", "display_name": "Jo Doe"}},
        "source": {{"branch": {{"name": "fix-login"}}, "commit": {{"hash": "{head}"}}, "repository": {{"full_name": "acme-team/web"}}}},
        "destination": {{"branch": {{"name": "main"}}}},
        "reviewers": [{{"uuid": "{{kim}}", "nickname": "kim", "display_name": "Kim"}}],
        "participants": [
            {{"user": {{"uuid": "{{kim}}", "nickname": "kim", "display_name": "Kim"}}, "role": "REVIEWER", "approved": false, "state": null, "participated_on": null}},
            {{"user": {{"uuid": "{{lee}}", "nickname": "lee", "display_name": "Lee"}}, "role": "PARTICIPANT", "approved": true, "state": "approved", "participated_on": "2026-10-02T10:00:00+00:00"}}
        ],
        "comment_count": 3, "created_on": "2026-10-01T09:00:00.000000+00:00", "updated_on": "{updated}",
        "closed_on": null, "links": {{"html": {{"href": "https://bitbucket.org/acme-team/web/pull-requests/{id}"}}}},
        "summary": {{"raw": "Fixes the login form."}}
    }}"#
    )
}

fn bitbucket_list(prs: &[String]) -> String {
    format!(r#"{{"values": [{}]}}"#, prs.join(","))
}

const BITBUCKET_DIFFSTAT: &str = r#"{"values": [
    {"status": "modified", "lines_added": 4, "lines_removed": 1, "old": {"path": "login.js"}, "new": {"path": "login.js"}},
    {"status": "added", "lines_added": 20, "lines_removed": 0, "old": null, "new": {"path": "login.test.js"}}
]}"#;
const BITBUCKET_STATUSES: &str = r#"{"values": [
    {"key": "ci", "name": "Pipeline", "state": "FAILED", "description": "2 tests failed", "url": "https://ci/2"}
]}"#;
const BITBUCKET_COMMENTS: &str = r#"{"values": [
    {"id": 1, "parent": null, "inline": {"path": "login.js"}, "resolution": null, "pending": false, "deleted": false},
    {"id": 2, "parent": {"id": 1}, "inline": {"path": "login.js"}, "resolution": null, "pending": false, "deleted": false},
    {"id": 3, "parent": null, "inline": {"path": "login.js"}, "resolution": {"type": "resolution"}, "pending": false, "deleted": false},
    {"id": 4, "parent": null, "inline": null, "resolution": null, "pending": false, "deleted": false},
    {"id": 5, "parent": null, "inline": {"path": "login.js"}, "resolution": null, "pending": true, "deleted": false}
]}"#;

// --- The harness ---------------------------------------------------------------

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
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn repo(dir: &Path, origin: &str) {
    std::fs::create_dir_all(dir).unwrap();
    git(dir, &["init", "-q", "-b", "main"]);
    std::fs::write(dir.join("README.md"), "hello\n").unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-q", "-m", "initial"]);
    git(dir, &["remote", "add", "origin", origin]);
}

struct Harness {
    repositories: Arc<RepositoryService>,
    accounts: Arc<AccountService>,
    pull_requests: Arc<PullRequestService>,
    events: Arc<Mutex<Vec<PullRequestChangedEvent>>>,
    _tmp: tempfile::TempDir,
}

async fn harness(base: &str) -> Harness {
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let db = Db::open(&data.join("brainiac.sqlite3")).unwrap();
    let emitter: brainiac_lib::workspaces::Emitter = Arc::new(|_| {});
    let repositories = Arc::new(RepositoryService::new(
        db.clone(),
        GitService::detect().await,
        Settings::default(),
        emitter,
    ));
    let endpoints = Endpoints {
        github: base.to_string(),
        bitbucket: base.to_string(),
    };
    let http = Http::insecure_for_tests().unwrap();
    let accounts = Arc::new(AccountService::new(
        db,
        Arc::new(MemoryKeychain::default()),
        http.clone(),
        endpoints.clone(),
    ));
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&events);
    let pull_requests = Arc::new(PullRequestService::new(
        Arc::clone(&repositories),
        Arc::clone(&accounts),
        PullRequestService::open_cache(&data).unwrap(),
        http,
        endpoints,
        Arc::new(move |e| sink.lock().unwrap().push(e)),
    ));
    Harness {
        repositories,
        accounts,
        pull_requests,
        events,
        _tmp: tmp,
    }
}

impl Harness {
    async fn add_accounts(&self) {
        for (kind, email) in [
            (ForgeKind::Github, None),
            (
                ForgeKind::BitbucketCloud,
                Some("jo@example.com".to_string()),
            ),
        ] {
            let outcome = self
                .accounts
                .save(SaveForgeAccountRequest {
                    kind,
                    token: Some("secret".into()),
                    email,
                    read_only: false,
                })
                .await
                .unwrap();
            assert!(matches!(outcome, SaveForgeAccountOutcome::Saved { .. }));
        }
    }

    /// A workspace of a GitHub and a Bitbucket repository, pull requests on.
    async fn workspace(&self, root: &Path) -> (Workspace, String, String) {
        let api = root.join("api");
        repo(&api, "git@github.com:acme/api.git");
        let web = root.join("web");
        repo(&web, "https://bitbucket.org/acme-team/web.git");
        // Registering first stores each origin URL now; a workspace's own
        // registrations observe it in the background.
        self.repositories.register(&api).await.unwrap();
        self.repositories.register(&web).await.unwrap();
        let ws = self
            .repositories
            .create_workspace(CreateWorkspaceRequest {
                name: "Acme".into(),
                discovery_mode: DiscoveryMode::Manual,
                discovery_root: None,
                discovery_path: None,
                paths: vec![api.display().to_string(), web.display().to_string()],
            })
            .await
            .unwrap()
            .workspace;
        let ws = self
            .repositories
            .set_pull_requests(&ws.id, true)
            .await
            .unwrap();
        let id_of = |name: &str| {
            ws.members
                .iter()
                .find(|m| m.display_name == name)
                .and_then(|m| m.repository_id.clone())
                .unwrap()
        };
        let (api_id, web_id) = (id_of("api"), id_of("web"));
        (ws, api_id, web_id)
    }

    async fn list(&self, workspace_id: &str, max_age: u64) -> PullRequestList {
        self.pull_requests
            .list(ListPullRequestsRequest {
                workspace_id: Some(workspace_id.into()),
                repository_id: None,
                closed: false,
                max_age_seconds: max_age,
            })
            .await
            .unwrap()
    }
}

fn routes(github_prs: Vec<String>, bitbucket_prs: Vec<String>) -> Vec<Route> {
    vec![
        Route {
            head_contains: "authorization: bearer",
            headers: vec![(
                "github-authentication-token-expiration",
                "2027-10-02 22:00:00 UTC",
            )],
            ..route("GET", "/user", GITHUB_USER)
        },
        Route {
            head_contains: "authorization: basic",
            headers: vec![BITBUCKET_SCOPES],
            ..route("GET", "/user", BITBUCKET_USER)
        },
        Route {
            body_contains: "\"number\"",
            ..route("POST", "/graphql", github_get(&github_prs[0]))
        },
        route("POST", "/graphql", github_list(&github_prs)),
        route("GET", "/repos/acme/api/pulls/1/files", GITHUB_FILES),
        route(
            "GET",
            "/repositories/acme-team/web/pullrequests?state=OPEN",
            bitbucket_list(&bitbucket_prs),
        ),
        route(
            "GET",
            "/repositories/acme-team/web/pullrequests/5/diffstat",
            BITBUCKET_DIFFSTAT,
        ),
        route(
            "GET",
            "/repositories/acme-team/web/pullrequests/5/comments",
            BITBUCKET_COMMENTS,
        ),
        route("GET", "/repositories/acme-team/web/pullrequests/5", {
            // `get` returns the same pull request (the route above wins for its sub-paths).
            bitbucket_prs[0].clone()
        }),
        route(
            "GET",
            "/repositories/acme-team/web/commit/",
            BITBUCKET_STATUSES,
        ),
    ]
}

#[tokio::test(flavor = "multi_thread")]
async fn a_workspace_lists_both_providers_and_caches_what_it_read() {
    let gh = github_pr(1, "2026-10-03T10:00:00Z", "a".repeat(40).as_str());
    let bb = bitbucket_pr(5, "2026-10-03T09:00:00.000000+00:00", "0123456789ab");
    let (base, requests) = serve(routes(vec![gh], vec![bb])).await;
    let h = harness(&base).await;
    h.add_accounts().await;
    let (ws, api_id, web_id) = h.workspace(h._tmp.path()).await;

    let list = h.list(&ws.id, 300).await;
    assert!(list.enabled);
    assert!(list.missing_accounts.is_empty());
    assert_eq!(list.groups.len(), 2);
    let api = list
        .groups
        .iter()
        .find(|g| g.repository_id == api_id)
        .unwrap();
    let web = list
        .groups
        .iter()
        .find(|g| g.repository_id == web_id)
        .unwrap();
    assert_eq!(list.groups[0].repository_id, api_id, "member order");
    assert!(api.error.is_none(), "{:?}", api.error);
    assert!(web.error.is_none(), "{:?}", web.error);

    let pr = &api.pull_requests[0];
    assert_eq!(pr.reference, "github.com/acme/api#1");
    assert_eq!(pr.state, PullRequestState::Open);
    assert_eq!(pr.author.login, "ada");
    assert!(!pr.mine);
    assert!(pr.awaiting_my_review);
    let reviewers: Vec<_> = pr
        .reviewers
        .iter()
        .map(|r| (r.user.login.as_str(), r.state, r.is_me))
        .collect();
    assert_eq!(
        reviewers,
        vec![
            ("octo", ReviewState::Requested, true),
            ("bob", ReviewState::Approved, false)
        ]
    );
    assert_eq!(pr.checks.state, Some(CheckState::Pending));
    assert_eq!(
        (pr.checks.total, pr.checks.passed, pr.checks.pending),
        (2, 1, 1)
    );
    assert_eq!(pr.counts.unresolved_threads, Some(1));
    assert_eq!(pr.counts.additions, Some(10));
    assert_eq!(pr.mergeability, Mergeability::Mergeable);
    assert!(pr.actions.merge.allowed);
    assert_eq!(pr.updated_at, "2026-10-03T10:00:00.000Z");

    let pr = &web.pull_requests[0];
    assert_eq!(pr.reference, "bitbucket.org/acme-team/web#5");
    assert!(pr.mine);
    assert_eq!(pr.description, "Fixes the login form.");
    let reviewers: Vec<_> = pr
        .reviewers
        .iter()
        .map(|r| (r.user.login.as_str(), r.state))
        .collect();
    assert_eq!(
        reviewers,
        vec![
            ("kim", ReviewState::Requested),
            ("lee", ReviewState::Approved)
        ]
    );
    // The detail requests filled in what the list leaves out.
    assert_eq!(pr.checks.state, Some(CheckState::Failure));
    assert_eq!(
        pr.counts.unresolved_threads,
        Some(1),
        "root inline, unresolved, not pending"
    );
    assert_eq!(
        (
            pr.counts.additions,
            pr.counts.deletions,
            pr.counts.changed_files
        ),
        (Some(24), Some(1), Some(2))
    );
    assert_eq!(pr.counts.comments, 3);
    assert_eq!(pr.mergeability, Mergeability::Unknown);
    assert_eq!(requests.count("/diffstat"), 1);
    assert_eq!(requests.count("/commit/0123456789ab/statuses"), 1);

    // Two events, one per pull request read.
    let mut refs: Vec<String> = h
        .events
        .lock()
        .unwrap()
        .iter()
        .map(|e| e.reference.clone())
        .collect();
    refs.sort();
    assert_eq!(
        refs,
        vec!["bitbucket.org/acme-team/web#5", "github.com/acme/api#1"]
    );

    // Young enough: nothing is asked again.
    let before = requests.total();
    let again = h.list(&ws.id, 300).await;
    assert_eq!(again.groups[1].pull_requests.len(), 1);
    assert_eq!(requests.total(), before);

    // Asked to read again: the lists are, but Bitbucket's unchanged pull
    // request needs no detail requests, and no event fires.
    h.list(&ws.id, 0).await;
    assert_eq!(requests.count("/graphql"), 2);
    assert_eq!(requests.count("pullrequests?state=OPEN"), 2);
    assert_eq!(requests.count("/diffstat"), 1);
    assert_eq!(h.events.lock().unwrap().len(), 2);

    // Files: Bitbucket's came with the diffstat; GitHub's are read from REST once.
    let files = h
        .pull_requests
        .files("bitbucket.org/acme-team/web#5")
        .await
        .unwrap();
    assert_eq!(files.files.len(), 2);
    assert_eq!(files.head_sha, "0123456789ab");
    assert_eq!(requests.count("/diffstat"), 1);
    let files = h
        .pull_requests
        .files("github.com/acme/api#1")
        .await
        .unwrap();
    assert_eq!(files.files[1].old_path.as_deref(), Some("docs/old.md"));
    assert_eq!(files.files[1].status, ChangedFileStatus::Renamed);
    h.pull_requests
        .files("github.com/acme/api#1")
        .await
        .unwrap();
    assert_eq!(requests.count("/pulls/1/files"), 1);

    // Checks: GitHub's come with the same query; Bitbucket's from the statuses.
    let checks = h
        .pull_requests
        .checks("github.com/acme/api#1", 300)
        .await
        .unwrap();
    assert_eq!(checks.checks.len(), 2);
    assert_eq!(checks.checks[0].url.as_deref(), Some("https://ci/1"));
    let checks = h
        .pull_requests
        .checks("bitbucket.org/acme-team/web#5", 300)
        .await
        .unwrap();
    assert_eq!(checks.checks[0].name, "Pipeline");
    assert_eq!(checks.checks[0].state, CheckState::Failure);

    // One pull request, fresh from the cache or read again on request.
    let before = requests.total();
    let one = h
        .pull_requests
        .get("github.com/acme/api#1", 300)
        .await
        .unwrap();
    assert_eq!(one.title, "Add parser");
    assert_eq!(requests.total(), before);
    h.pull_requests
        .get("github.com/acme/api#1", 0)
        .await
        .unwrap();
    assert_eq!(requests.total(), before + 1);
    let err = h
        .pull_requests
        .get("github.com/acme/api", 0)
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Validation);
    let err = h
        .pull_requests
        .get("github.com/other/repo#1", 0)
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);

    // The budget counts Bitbucket's requests; GitHub's come from its headers.
    let budgets = h.pull_requests.budgets();
    let bb = budgets
        .iter()
        .find(|b| b.kind == ForgeKind::BitbucketCloud)
        .unwrap();
    assert_eq!(bb.used as usize, requests.count("/repositories/"));
    assert_eq!(bb.limit, 1000);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_repository_that_fails_keeps_its_place_beside_the_others() {
    let gh = github_pr(1, "2026-10-03T10:00:00Z", "a".repeat(40).as_str());
    let bb = bitbucket_pr(5, "2026-10-03T09:00:00.000000+00:00", "0123456789ab");
    let mut all = routes(vec![gh], vec![bb]);
    // Bitbucket refuses the repository.
    let list_route = all
        .iter_mut()
        .find(|r| r.path.ends_with("pullrequests?state=OPEN"))
        .unwrap();
    list_route.status = 403;
    list_route.body = r#"{"type":"error","error":{"message":"Access denied"}}"#.into();
    let (base, _requests) = serve(all).await;
    let h = harness(&base).await;
    h.add_accounts().await;
    let (ws, api_id, web_id) = h.workspace(h._tmp.path()).await;

    let list = h.list(&ws.id, 0).await;
    let api = list
        .groups
        .iter()
        .find(|g| g.repository_id == api_id)
        .unwrap();
    let web = list
        .groups
        .iter()
        .find(|g| g.repository_id == web_id)
        .unwrap();
    assert_eq!(api.pull_requests.len(), 1);
    assert!(api.error.is_none());
    assert!(web.pull_requests.is_empty());
    let err = web.error.as_ref().unwrap();
    assert_eq!(err.code, ErrorCode::PermissionDenied);
    assert!(web.fetched_at.is_none());

    // Without a GitHub account, its repository says so and the list names the provider.
    h.accounts.remove(ForgeKind::Github).await.unwrap();
    let list = h.list(&ws.id, 0).await;
    assert_eq!(list.missing_accounts, vec![ForgeKind::Github]);
    let api = list
        .groups
        .iter()
        .find(|g| g.repository_id == api_id)
        .unwrap();
    assert!(api
        .error
        .as_ref()
        .unwrap()
        .message
        .contains("No GitHub account"));
    // What was cached is still shown.
    assert_eq!(api.pull_requests.len(), 1);
    assert!(api.fetched_at.is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_workspace_with_pull_requests_off_asks_nothing() {
    let gh = github_pr(1, "2026-10-03T10:00:00Z", "a".repeat(40).as_str());
    let bb = bitbucket_pr(5, "2026-10-03T09:00:00.000000+00:00", "0123456789ab");
    let (base, requests) = serve(routes(vec![gh], vec![bb])).await;
    let h = harness(&base).await;
    h.add_accounts().await;
    let (ws, api_id, _) = h.workspace(h._tmp.path()).await;
    h.repositories
        .set_pull_requests(&ws.id, false)
        .await
        .unwrap();
    let before = requests.total();

    let list = h.list(&ws.id, 0).await;
    assert!(!list.enabled);
    assert!(list.groups.is_empty());
    assert_eq!(requests.total(), before);

    // One repository: untracked, it names no workspace and lists nothing.
    let one = |closed| ListPullRequestsRequest {
        workspace_id: None,
        repository_id: Some(api_id.clone()),
        closed,
        max_age_seconds: 0,
    };
    let list = h.pull_requests.list(one(false)).await.unwrap();
    assert!(!list.enabled);
    assert!(list.tracked_by.is_empty());
    assert!(list.groups.is_empty());
    let err = h
        .pull_requests
        .get("github.com/acme/api#1", 0)
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);

    h.repositories
        .set_pull_requests(&ws.id, true)
        .await
        .unwrap();
    let list = h.pull_requests.list(one(false)).await.unwrap();
    assert!(list.enabled);
    assert_eq!(list.tracked_by, vec!["Acme"]);
    assert_eq!(list.groups.len(), 1);
    assert_eq!(list.groups[0].pull_requests.len(), 1);

    // Closed ones are a repository-only list.
    let err = h
        .pull_requests
        .list(ListPullRequestsRequest {
            workspace_id: Some(ws.id.clone()),
            repository_id: None,
            closed: true,
            max_age_seconds: 0,
        })
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Validation);
}
