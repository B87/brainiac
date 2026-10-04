//! `PullRequestService` (SPEC.md, section 10) over a local server standing in
//! for GitHub and Bitbucket: lists per workspace and repository, what is
//! cached and for how long, a repository that fails beside ones that work,
//! and `pr_changed` events. The adapters' mapping of real answers is checked
//! by the opt-in tests in `forge_live.rs`.

use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};

use brainiac_lib::credentials::{CommandRunner, CredentialService, MemoryStore};
use brainiac_lib::db::Db;
use brainiac_lib::forge::http::Http;
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
    /// Answer only after this long: past the client's timeout, a provider
    /// that got the request but whose answer never came.
    delay_ms: u64,
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
    /// The bodies sent to paths containing `path_contains`.
    fn bodies(&self, path_contains: &str) -> Vec<String> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|(p, _)| p.contains(path_contains))
            .map(|(_, b)| b.clone())
            .collect()
    }
}

async fn serve(routes: Vec<Route>) -> (String, Requests) {
    serve_shared(Arc::new(Mutex::new(routes))).await
}

/// A server whose routes a test can replace while it runs, for answers that
/// depend on commits made after the server started.
async fn serve_shared(routes: Arc<Mutex<Vec<Route>>>) -> (String, Requests) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let requests = Requests::default();
    let seen = requests.clone();
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
                let (status, headers, body, delay) = {
                    let routes = routes.lock().unwrap();
                    let route = routes.iter().find(|r| {
                        r.method == method
                            && path.starts_with(r.path)
                            && lower.contains(r.head_contains)
                            && body.contains(r.body_contains)
                    });
                    match route {
                        Some(r) => (r.status, r.headers.clone(), r.body.clone(), r.delay_ms),
                        None => (599, Vec::new(), format!("no route for {method} {path}"), 0),
                    }
                };
                if delay > 0 {
                    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                }
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
        delay_ms: 0,
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
    github_pr_at(number, updated, head, &"b".repeat(40), None)
}

/// A pull request at `head` over `base`; `reviewed` is the commit the
/// account's user (octo) approved, when they did.
fn github_pr_at(
    number: u64,
    updated: &str,
    head: &str,
    base: &str,
    reviewed: Option<&str>,
) -> String {
    let mine = reviewed
        .map(|r| format!(r#", {{"state": "APPROVED", "commit": {{"oid": "{r}"}}, "author": {{"login": "octo", "databaseId": 42, "name": "Octo Cat"}}}}"#))
        .unwrap_or_default();
    format!(
        r#"{{
        "number": {number}, "title": "Add parser", "body": "Parses **things**.\n\n<script>x</script>", "isDraft": false, "state": "OPEN",
        "mergedAt": null, "closedAt": null, "createdAt": "2026-10-01T10:00:00Z", "updatedAt": "{updated}",
        "url": "https://github.com/acme/api/pull/{number}",
        "author": {{"login": "ada", "databaseId": 7, "name": "Ada"}},
        "headRefName": "feature", "headRefOid": "{head}", "headRepository": {{"nameWithOwner": "acme/api"}},
        "baseRefName": "main", "baseRefOid": "{base}", "mergeable": "MERGEABLE", "additions": 10, "deletions": 2, "changedFiles": 2,
        "commits": {{"totalCount": 3}}, "comments": {{"totalCount": 1}},
        "reviewThreads": {{"nodes": [{{"isResolved": false}}, {{"isResolved": true}}]}},
        "reviewRequests": {{"nodes": [{{"requestedReviewer": {{"__typename": "User", "login": "octo", "databaseId": 42, "name": "Octo Cat"}}}}]}},
        "latestReviews": {{"nodes": [{{"state": "APPROVED", "commit": {{"oid": "{base}"}}, "author": {{"login": "bob", "databaseId": 9, "name": null}}}}{mine}]}},
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
    {"id": 1, "parent": null, "content": {"raw": "Is this right?"}, "inline": {"path": "login.js", "from": null, "to": 2, "outdated": false, "src_rev": "0123456789ab"}, "resolution": null, "pending": false, "deleted": false, "user": {"uuid": "{kim}", "nickname": "kim", "display_name": "Kim"}, "created_on": "2026-10-02T10:00:00.000000+00:00", "updated_on": "2026-10-02T10:00:00.000000+00:00", "links": {"html": {"href": "https://bitbucket.org/acme-team/web/pull-requests/5#comment-1"}}},
    {"id": 2, "parent": {"id": 1}, "content": {"raw": "Yes: see https://example.com/doc."}, "inline": {"path": "login.js", "from": null, "to": 2}, "resolution": null, "pending": false, "deleted": false, "user": {"uuid": "{jo}", "nickname": "jo", "display_name": "Jo Doe"}, "created_on": "2026-10-02T10:05:00.000000+00:00", "updated_on": "2026-10-02T10:05:00.000000+00:00"},
    {"id": 3, "parent": null, "content": {"raw": "fixed"}, "inline": {"path": "login.js", "from": 1, "to": null, "outdated": true}, "resolution": {"type": "resolution"}, "pending": false, "deleted": false, "user": {"uuid": "{kim}", "nickname": "kim"}, "created_on": "2026-10-01T10:00:00.000000+00:00"},
    {"id": 4, "parent": null, "content": {"raw": "General remark"}, "inline": null, "resolution": null, "pending": false, "deleted": false, "user": {"uuid": "{lee}", "nickname": "lee"}, "created_on": "2026-10-02T11:00:00.000000+00:00"},
    {"id": 5, "parent": null, "content": {"raw": "draft"}, "inline": {"path": "login.js", "to": 9}, "resolution": null, "pending": true, "deleted": false, "user": {"uuid": "{jo}"}, "created_on": "2026-10-02T12:00:00.000000+00:00"}
]}"#;
const GITHUB_FILES_DIFF: &str = "diff --git a/src/parse.rs b/src/parse.rs\n--- a/src/parse.rs\n+++ b/src/parse.rs\n@@ -1 +1 @@\n-fn a() {}\n+fn a() { b() }\n";
const BITBUCKET_DIFF: &str = "diff --git a/login.js b/login.js\nindex 1..2 100644\n--- a/login.js\n+++ b/login.js\n@@ -1,2 +1,2 @@\n-const a = 1;\n+const a = 2;\n export { a };\ndiff --git a/login.test.js b/login.test.js\nnew file mode 100644\n--- /dev/null\n+++ b/login.test.js\n@@ -0,0 +1 @@\n+test();\n";

/// GitHub's conversation: a comment, two reviews (one says nothing), and
/// two review threads, one resolved and outdated.
const GITHUB_CONVERSATION: &str = r#"{"data": {"repository": {"pullRequest": {
    "comments": {"nodes": [
        {"databaseId": 100, "body": "Looks **good**", "createdAt": "2026-10-02T10:00:00Z", "updatedAt": "2026-10-02T10:00:00Z", "url": "https://github.com/acme/api/pull/1#issuecomment-100", "author": {"login": "bob", "databaseId": 9, "name": null}}
    ]},
    "reviews": {"nodes": [
        {"databaseId": 200, "state": "APPROVED", "body": "", "submittedAt": "2026-10-02T11:00:00Z", "url": null, "commit": {"oid": "cccc"}, "author": {"login": "bob", "databaseId": 9, "name": null}},
        {"databaseId": 201, "state": "COMMENTED", "body": "", "submittedAt": "2026-10-02T09:00:00Z", "url": null, "commit": null, "author": {"login": "octo", "databaseId": 42, "name": "Octo Cat"}}
    ]},
    "reviewThreads": {"pageInfo": {"hasNextPage": false, "endCursor": null}, "nodes": [
        {"id": "T1", "isResolved": false, "isOutdated": false, "path": "src/parse.rs", "line": 12, "startLine": null, "diffSide": "RIGHT", "originalLine": 12, "originalStartLine": null,
         "comments": {"nodes": [
            {"databaseId": 300, "body": "Why?", "createdAt": "2026-10-02T08:00:00Z", "updatedAt": "2026-10-02T08:30:00Z", "url": null, "commit": {"oid": "cccc"}, "originalCommit": {"oid": "cccc"}, "author": {"login": "octo", "databaseId": 42, "name": "Octo Cat"}},
            {"databaseId": 301, "body": "Because.", "createdAt": "2026-10-02T08:10:00Z", "updatedAt": "2026-10-02T08:10:00Z", "url": null, "commit": null, "originalCommit": {"oid": "cccc"}, "author": {"login": "ada", "databaseId": 7, "name": "Ada"}}
         ]}},
        {"id": "T2", "isResolved": true, "isOutdated": true, "path": "docs/new.md", "line": null, "startLine": null, "diffSide": "LEFT", "originalLine": 3, "originalStartLine": 1,
         "comments": {"nodes": [
            {"databaseId": 302, "body": "typo", "createdAt": "2026-10-01T12:00:00Z", "updatedAt": "2026-10-01T12:00:00Z", "url": null, "commit": null, "originalCommit": {"oid": "dddd"}, "author": null}
         ]}}
    ]}
}}}}"#;

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
    let core = db.clone();
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
        Arc::new(CredentialService::new(
            Arc::new(MemoryStore::default()),
            CommandRunner::new(data.join("commands")),
        )),
        http.clone(),
        endpoints.clone(),
    ));
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&events);
    let pull_requests = Arc::new(PullRequestService::new(
        Arc::clone(&repositories),
        Arc::clone(&accounts),
        core,
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
                    source: brainiac_lib::models::SecretSource::Store,
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
            body_contains: "isOutdated",
            ..route("POST", "/graphql", GITHUB_CONVERSATION)
        },
        Route {
            body_contains: "\"number\"",
            ..route("POST", "/graphql", github_get(&github_prs[0]))
        },
        route("POST", "/graphql", github_list(&github_prs)),
        route("GET", "/repos/acme/api/pulls/1/files", GITHUB_FILES),
        Route {
            head_contains: "accept: application/vnd.github.diff",
            ..route("GET", "/repos/acme/api/pulls/1", GITHUB_FILES_DIFF)
        },
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
        route(
            "GET",
            "/repositories/acme-team/web/pullrequests/5/diff",
            BITBUCKET_DIFF,
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
        .files("bitbucket.org/acme-team/web#5", None)
        .await
        .unwrap();
    assert_eq!(files.files.len(), 2);
    assert_eq!(files.head_sha, "0123456789ab");
    assert_eq!(requests.count("/diffstat"), 1);
    let files = h
        .pull_requests
        .files("github.com/acme/api#1", None)
        .await
        .unwrap();
    assert_eq!(files.files[1].old_path.as_deref(), Some("docs/old.md"));
    assert_eq!(files.files[1].status, ChangedFileStatus::Renamed);
    h.pull_requests
        .files("github.com/acme/api#1", None)
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

/// A commit's full ID.
fn rev(dir: &Path, name: &str) -> String {
    let out = Command::new("git")
        .args(["rev-parse", name])
        .current_dir(dir)
        .output()
        .unwrap();
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn conversations_and_diffs_come_from_the_providers_or_local_git() {
    let gh = github_pr(1, "2026-10-03T10:00:00Z", "a".repeat(40).as_str());
    let bb = bitbucket_pr(5, "2026-10-03T09:00:00.000000+00:00", "0123456789ab");
    let shared = Arc::new(Mutex::new(routes(vec![gh], vec![bb.clone()])));
    let (base, requests) = serve_shared(Arc::clone(&shared)).await;
    let h = harness(&base).await;
    h.add_accounts().await;
    let (ws, _api_id, _web_id) = h.workspace(h._tmp.path()).await;

    // The GitHub pull request's branch is on the Mac: a commit on `feature`
    // over `main`, which the account's user reviewed at `main`.
    let api = h._tmp.path().join("api");
    let main_sha = rev(&api, "main");
    git(&api, &["checkout", "-q", "-b", "feature"]);
    std::fs::write(api.join("README.md"), "hello\nworld\n").unwrap();
    git(&api, &["commit", "-q", "-am", "say more"]);
    let head_sha = rev(&api, "feature");
    git(&api, &["checkout", "-q", "main"]);
    let gh = github_pr_at(
        1,
        "2026-10-03T10:00:00Z",
        &head_sha,
        &main_sha,
        Some(&main_sha),
    );
    *shared.lock().unwrap() = routes(vec![gh], vec![bb]);

    h.list(&ws.id, 0).await;
    let pr = h
        .pull_requests
        .get("github.com/acme/api#1", 300)
        .await
        .unwrap();
    assert_eq!(pr.base_sha, main_sha);
    assert_eq!(pr.reviewed_sha.as_deref(), Some(main_sha.as_str()));
    assert_eq!(pr.commits_since_review, Some(1));
    assert!(
        pr.description_html.contains("<strong>things</strong>")
            && !pr.description_html.contains("<script"),
        "{}",
        pr.description_html
    );

    // GitHub's conversation: oldest thread first, a review that says
    // nothing left out, a resolved outdated thread kept as such.
    let conversation = h
        .pull_requests
        .conversation("github.com/acme/api#1", 300)
        .await
        .unwrap();
    let ids: Vec<&str> = conversation.threads.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(ids, ["T2", "T1", "comment:100", "review:200"]);
    let t1 = &conversation.threads[1];
    let anchor = t1.anchor.as_ref().unwrap();
    assert_eq!(
        (anchor.path.as_str(), anchor.line, anchor.side),
        ("src/parse.rs", Some(12), DiffSide::New)
    );
    assert_eq!(anchor.commit.as_deref(), Some("cccc"));
    assert!(!t1.resolved && !t1.outdated);
    assert_eq!(t1.comments.len(), 2);
    assert!(t1.comments[0].mine && !t1.comments[1].mine);
    assert_eq!(
        t1.comments[0].updated_at.as_deref(),
        Some("2026-10-02T08:30:00.000Z")
    );
    assert_eq!(t1.comments[1].updated_at, None);
    let t2 = &conversation.threads[0];
    assert!(t2.resolved && t2.outdated);
    assert_eq!(t2.anchor.as_ref().unwrap().line, Some(3));
    assert_eq!(t2.anchor.as_ref().unwrap().side, DiffSide::Old);
    assert_eq!(t2.comments[0].author.login, "ghost");
    assert_eq!(
        conversation.threads[2].comments[0].html.trim(),
        "<p>Looks <strong>good</strong></p>"
    );
    assert_eq!(
        conversation.threads[3].comments[0].review,
        Some(ReviewState::Approved)
    );
    // Cached for its age, read again on request.
    let before = requests.count("/graphql");
    h.pull_requests
        .conversation("github.com/acme/api#1", 300)
        .await
        .unwrap();
    assert_eq!(requests.count("/graphql"), before);
    h.pull_requests
        .conversation("github.com/acme/api#1", 0)
        .await
        .unwrap();
    assert_eq!(requests.count("/graphql"), before + 1);

    // Bitbucket's conversation came with the detail: no new request.
    let before = requests.count("/comments");
    let conversation = h
        .pull_requests
        .conversation("bitbucket.org/acme-team/web#5", 300)
        .await
        .unwrap();
    assert_eq!(requests.count("/comments"), before);
    let ids: Vec<&str> = conversation.threads.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(ids, ["3", "1", "4"], "the pending draft is left out");
    let inline = &conversation.threads[1];
    assert_eq!(inline.comments.len(), 2);
    assert!(inline.comments[1].mine);
    assert!(inline.comments[1]
        .html
        .contains(r#"<a href="https://example.com/doc">"#));
    assert_eq!(inline.anchor.as_ref().unwrap().line, Some(2));
    assert!(conversation.threads[0].resolved && conversation.threads[0].outdated);
    assert_eq!(
        conversation.threads[0].anchor.as_ref().unwrap().side,
        DiffSide::Old
    );

    // GitHub's diff from local Git, since both commits are here.
    let request = |path: &str, since: Option<&str>| PullRequestDiffRequest {
        reference: "github.com/acme/api#1".into(),
        path: path.into(),
        old_path: None,
        since: since.map(str::to_string),
        options: DiffOptions::default(),
    };
    let diff = h
        .pull_requests
        .diff(request("README.md", None))
        .await
        .unwrap();
    assert_eq!(diff.source, DiffSource::Local);
    assert_eq!(
        (diff.base_sha.as_str(), diff.head_sha.as_str()),
        (main_sha.as_str(), head_sha.as_str())
    );
    let DiffContent::Text { hunks, .. } = &diff.diff.content else {
        panic!("text diff");
    };
    assert_eq!(hunks.len(), 1);
    assert!(hunks[0]
        .lines
        .iter()
        .any(|l| l.kind == DiffLineKind::Add && l.text == "world"));
    assert!(matches!(diff.diff.selector, DiffSelector::Range { .. }));
    // Since the review: the same, from the reviewed commit.
    let diff = h
        .pull_requests
        .diff(request("README.md", Some(&main_sha)))
        .await
        .unwrap();
    assert_eq!(diff.base_sha, main_sha);
    let files = h
        .pull_requests
        .files("github.com/acme/api#1", Some(&main_sha))
        .await
        .unwrap();
    assert!(files.partial);
    assert_eq!(files.files.len(), 1);
    assert_eq!(files.files[0].path, "README.md");
    assert_eq!((files.files[0].additions, files.files[0].deletions), (1, 0));
    assert_eq!(
        requests.count("vnd.github"),
        0,
        "nothing asked of GitHub for a local diff"
    );

    // Bitbucket's head is not here: the provider's diff, read once and cut per file.
    let request = |path: &str| PullRequestDiffRequest {
        reference: "bitbucket.org/acme-team/web#5".into(),
        path: path.into(),
        old_path: None,
        since: None,
        options: DiffOptions::default(),
    };
    let diff = h.pull_requests.diff(request("login.js")).await.unwrap();
    assert_eq!(diff.source, DiffSource::Provider);
    let DiffContent::Text { hunks, .. } = &diff.diff.content else {
        panic!("text diff");
    };
    assert_eq!(hunks[0].lines.len(), 3);
    assert_eq!(hunks[0].lines[1].text, "const a = 2;");
    let diff = h
        .pull_requests
        .diff(request("login.test.js"))
        .await
        .unwrap();
    let DiffContent::Text { hunks, .. } = &diff.diff.content else {
        panic!("text diff");
    };
    assert_eq!(hunks[0].lines[0].text, "test();");
    let diff = h.pull_requests.diff(request("elsewhere.js")).await.unwrap();
    assert!(matches!(&diff.diff.content, DiffContent::Text { hunks, .. } if hunks.is_empty()));
    // The diffstat path starts the same way; the diff itself was read once.
    assert_eq!(
        requests.count("/pullrequests/5/diff") - requests.count("/pullrequests/5/diffstat"),
        1
    );
    // Since a commit that is not on the Mac: nothing but local Git can answer.
    let err = h
        .pull_requests
        .diff(PullRequestDiffRequest {
            since: Some("0123456789ab".into()),
            ..request("login.js")
        })
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);
    let err = h
        .pull_requests
        .diff(PullRequestDiffRequest {
            since: Some("main; rm".into()),
            ..request("login.js")
        })
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Validation);
}

// --- Reviewing (SPEC.md, Reviewing) --------------------------------------------

const BITBUCKET_BRANCH: &str =
    r#"{"target": {"hash": "0123456789ab0123456789ab0123456789ab0123"}}"#;

/// The writes both providers take, in front of the reads.
fn write_routes(second_comment_status: u16) -> Vec<Route> {
    vec![
        Route {
            body_contains: "resolveReviewThread",
            ..route(
                "POST",
                "/graphql",
                r#"{"data": {"resolveReviewThread": {"thread": {"isResolved": true}}}}"#,
            )
        },
        Route {
            body_contains: "unresolveReviewThread",
            ..route(
                "POST",
                "/graphql",
                r#"{"data": {"unresolveReviewThread": {"thread": {"isResolved": false}}}}"#,
            )
        },
        Route {
            status: 201,
            ..route(
                "POST",
                "/repos/acme/api/issues/1/comments",
                r#"{"id": 900}"#,
            )
        },
        Route {
            status: 201,
            ..route(
                "POST",
                "/repos/acme/api/pulls/1/comments/300/replies",
                r#"{"id": 901}"#,
            )
        },
        route("POST", "/repos/acme/api/pulls/1/reviews", r#"{"id": 950}"#),
        Route {
            status: 201,
            body_contains: "\"parent\"",
            ..route(
                "POST",
                "/repositories/acme-team/web/pullrequests/5/comments",
                r#"{"id": 11}"#,
            )
        },
        Route {
            status: second_comment_status,
            body_contains: "second",
            ..route(
                "POST",
                "/repositories/acme-team/web/pullrequests/5/comments",
                r#"{"id": 12}"#,
            )
        },
        Route {
            status: 201,
            body_contains: "slow",
            delay_ms: 4_000,
            ..route(
                "POST",
                "/repositories/acme-team/web/pullrequests/5/comments",
                r#"{"id": 13}"#,
            )
        },
        Route {
            status: 201,
            ..route(
                "POST",
                "/repositories/acme-team/web/pullrequests/5/comments",
                r#"{"id": 10}"#,
            )
        },
        route(
            "POST",
            "/repositories/acme-team/web/pullrequests/5/comments/1/resolve",
            "",
        ),
        Route {
            status: 204,
            ..route(
                "DELETE",
                "/repositories/acme-team/web/pullrequests/5/comments/1/resolve",
                "",
            )
        },
        route(
            "POST",
            "/repositories/acme-team/web/pullrequests/5/approve",
            r#"{"approved": true}"#,
        ),
        route(
            "GET",
            "/repositories/acme-team/web/refs/branches/fix-login",
            BITBUCKET_BRANCH,
        ),
    ]
}

fn anchor(path: &str, line: u32, start: Option<u32>, commit: &str) -> ThreadAnchor {
    ThreadAnchor {
        path: path.into(),
        side: DiffSide::New,
        line: Some(line),
        start_line: start,
        commit: Some(commit.into()),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn reviews_are_drafted_locally_and_sent_as_each_provider_takes_them() {
    let head = "a".repeat(40);
    let gh = github_pr(1, "2026-10-03T10:00:00Z", &head);
    let bb = bitbucket_pr(5, "2026-10-03T09:00:00.000000+00:00", "0123456789ab");
    let mut all = write_routes(500);
    all.extend(routes(vec![gh.clone()], vec![bb.clone()]));
    let shared = Arc::new(Mutex::new(all));
    let (base, requests) = serve_shared(Arc::clone(&shared)).await;
    let h = harness(&base).await;
    h.add_accounts().await;
    let (ws, _, _) = h.workspace(h._tmp.path()).await;
    h.list(&ws.id, 0).await;
    let gh_ref = "github.com/acme/api#1";
    let bb_ref = "bitbucket.org/acme-team/web#5";

    // Drafts stay on the Mac: written, changed, listed, deleted.
    let save = |path: &str, line: u32, start: Option<u32>, commit: &str, body: &str| {
        SaveReviewDraftRequest {
            reference: gh_ref.into(),
            id: None,
            anchor: anchor(path, line, start, commit),
            body: body.into(),
        }
    };
    let drafts = h
        .pull_requests
        .save_draft(save("src/parse.rs", 12, None, &head, "Why **this**?"))
        .await
        .unwrap();
    assert_eq!(drafts.drafts.len(), 1);
    assert!(drafts.drafts[0].html.contains("<strong>this</strong>"));
    let first_id = drafts.drafts[0].id.clone();
    let drafts = h
        .pull_requests
        .save_draft(SaveReviewDraftRequest {
            id: Some(first_id.clone()),
            ..save("src/parse.rs", 12, None, &head, "Why this?")
        })
        .await
        .unwrap();
    assert_eq!(drafts.drafts.len(), 1);
    assert_eq!(drafts.drafts[0].body, "Why this?");
    h.pull_requests
        .save_draft(save("docs/new.md", 3, Some(1), &head, "Range"))
        .await
        .unwrap();
    let gone = h
        .pull_requests
        .save_draft(save("docs/new.md", 3, None, &head, ""))
        .await
        .unwrap_err();
    assert_eq!(gone.code, ErrorCode::Validation);
    let extra = h
        .pull_requests
        .save_draft(save("docs/new.md", 4, None, &head, "Gone soon"))
        .await
        .unwrap();
    let drafts = h
        .pull_requests
        .delete_draft(gh_ref, &extra.drafts[2].id)
        .await
        .unwrap();
    assert_eq!(drafts.drafts.len(), 2);
    assert!(drafts.pending.is_none());

    // The head the user looked at is behind: nothing is sent.
    let submit = |verdict: ReviewVerdict, expected: &str| SubmitReviewRequest {
        reference: gh_ref.into(),
        body: "Looks fine".into(),
        verdict,
        expected_head_sha: expected.into(),
    };
    let err = h
        .pull_requests
        .submit_review(submit(ReviewVerdict::Approve, &"b".repeat(40)))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Conflict);
    assert_eq!(requests.count("/pulls/1/reviews"), 0);
    // A draft written on an earlier commit: the same, until the drafts are
    // moved to the head after looking at the new commits.
    h.pull_requests
        .save_draft(save("src/parse.rs", 20, None, &"0".repeat(40), "Old"))
        .await
        .unwrap();
    let err = h
        .pull_requests
        .submit_review(submit(ReviewVerdict::Approve, &head))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Conflict);
    assert!(
        err.message.contains("started this review"),
        "{}",
        err.message
    );
    assert_eq!(requests.count("/pulls/1/reviews"), 0);
    h.pull_requests.move_drafts(gh_ref, &head).await.unwrap();
    let outcome = h
        .pull_requests
        .submit_review(submit(ReviewVerdict::Approve, &head))
        .await
        .unwrap();
    // GitHub takes the review whole, in one request on the head commit.
    let bodies = requests.bodies("/pulls/1/reviews");
    assert_eq!(bodies.len(), 1);
    let sent: serde_json::Value = serde_json::from_str(&bodies[0]).unwrap();
    assert_eq!(sent["commit_id"], head);
    assert_eq!(sent["event"], "APPROVE");
    assert_eq!(sent["body"], "Looks fine");
    let comments = sent["comments"].as_array().unwrap();
    assert_eq!(comments.len(), 3);
    assert_eq!(comments[0]["path"], "src/parse.rs");
    assert_eq!(comments[0]["line"], 12);
    assert_eq!(comments[0]["side"], "RIGHT");
    assert_eq!(comments[1]["start_line"], 1);
    assert_eq!(comments[1]["line"], 3);
    assert!(comments[0].get("start_line").is_none());
    assert!(h
        .pull_requests
        .drafts(gh_ref)
        .await
        .unwrap()
        .drafts
        .is_empty());
    assert_eq!(outcome.pull_request.reference, gh_ref);
    assert!(h
        .events
        .lock()
        .unwrap()
        .iter()
        .any(|e| e.reference == gh_ref && e.origin == PullRequestChangeOrigin::App));

    // Comment, reply, resolve, reopen on GitHub.
    h.pull_requests
        .comment(CommentRequest {
            reference: gh_ref.into(),
            body: "Hello".into(),
        })
        .await
        .unwrap();
    assert!(requests.bodies("/issues/1/comments")[0].contains("Hello"));
    h.pull_requests
        .reply(ReplyRequest {
            reference: gh_ref.into(),
            thread_id: "T1".into(),
            body: "A reply".into(),
        })
        .await
        .unwrap();
    assert!(requests.bodies("/comments/300/replies")[0].contains("A reply"));
    // A reply to a comment on the pull request is another such comment.
    h.pull_requests
        .reply(ReplyRequest {
            reference: gh_ref.into(),
            thread_id: "comment:100".into(),
            body: "Me too".into(),
        })
        .await
        .unwrap();
    assert!(requests.bodies("/issues/1/comments")[1].contains("Me too"));
    h.pull_requests
        .resolve(ResolveThreadRequest {
            reference: gh_ref.into(),
            thread_id: "T1".into(),
            resolved: true,
        })
        .await
        .unwrap();
    assert!(requests
        .bodies("/graphql")
        .iter()
        .any(|b| b.contains("resolveReviewThread") && b.contains("\"T1\"")));
    // T2 is resolved already: nothing to send.
    let before = requests.total();
    let err = h
        .pull_requests
        .resolve(ResolveThreadRequest {
            reference: gh_ref.into(),
            thread_id: "comment:100".into(),
            resolved: true,
        })
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Validation);
    let _ = before;

    // Bitbucket: one request per comment. The second one fails, so the
    // first is recorded as sent and the summary and verdict wait.
    let bb_head = "0123456789ab";
    for body in ["first", "second"] {
        h.pull_requests
            .save_draft(SaveReviewDraftRequest {
                reference: bb_ref.into(),
                id: None,
                anchor: anchor("login.js", 2, None, bb_head),
                body: body.into(),
            })
            .await
            .unwrap();
    }
    let bb_submit = SubmitReviewRequest {
        reference: bb_ref.into(),
        body: "Summary".into(),
        verdict: ReviewVerdict::Approve,
        expected_head_sha: bb_head.into(),
    };
    let err = h
        .pull_requests
        .submit_review(bb_submit.clone())
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::DependencyUnavailable);
    let drafts = h.pull_requests.drafts(bb_ref).await.unwrap();
    assert_eq!(drafts.drafts[0].remote_id.as_deref(), Some("10"));
    assert_eq!(drafts.drafts[1].remote_id, None);
    let pending = drafts.pending.as_ref().unwrap();
    assert_eq!(pending.body, "Summary");
    assert_eq!(pending.verdict, ReviewVerdict::Approve);
    assert!(!pending.summary_sent);
    assert_eq!(requests.count("/pullrequests/5/approve"), 0);
    // Sent again once Bitbucket answers: only what was left, then the
    // summary, then the approval.
    let mut all = write_routes(201);
    all.extend(routes(vec![gh], vec![bb]));
    *shared.lock().unwrap() = all;
    h.pull_requests.submit_review(bb_submit).await.unwrap();
    // Only what was posted: the path is also read after each write.
    let posted: Vec<String> = requests
        .bodies("/pullrequests/5/comments")
        .into_iter()
        .filter(|b| !b.is_empty())
        .collect();
    assert_eq!(
        posted.iter().filter(|b| b.contains("first")).count(),
        1,
        "nothing is posted twice"
    );
    assert_eq!(posted.iter().filter(|b| b.contains("second")).count(), 2);
    assert_eq!(posted.iter().filter(|b| b.contains("Summary")).count(), 1);
    assert!(posted.last().unwrap().contains("Summary"));
    assert!(posted[0].contains("\"to\":2") && posted[0].contains("login.js"));
    assert_eq!(requests.count("/pullrequests/5/approve"), 1);
    assert_eq!(requests.count("/refs/branches/fix-login"), 2);
    let drafts = h.pull_requests.drafts(bb_ref).await.unwrap();
    assert!(drafts.drafts.is_empty() && drafts.pending.is_none());

    // Reply, resolve, reopen on Bitbucket.
    h.pull_requests
        .reply(ReplyRequest {
            reference: bb_ref.into(),
            thread_id: "1".into(),
            body: "Agreed".into(),
        })
        .await
        .unwrap();
    let reply = requests
        .bodies("/pullrequests/5/comments")
        .into_iter()
        .rfind(|b| !b.is_empty())
        .unwrap();
    assert!(reply.contains("\"parent\":{\"id\":1}") && reply.contains("login.js"));
    h.pull_requests
        .resolve(ResolveThreadRequest {
            reference: bb_ref.into(),
            thread_id: "1".into(),
            resolved: true,
        })
        .await
        .unwrap();
    assert_eq!(requests.count("/comments/1/resolve"), 1);
    h.pull_requests
        .resolve(ResolveThreadRequest {
            reference: bb_ref.into(),
            thread_id: "3".into(),
            resolved: false,
        })
        .await
        .unwrap_err();

    // A comment whose answer never came: the conversation is read again,
    // and the comment is found there, so it is not posted twice.
    let with_slow = BITBUCKET_COMMENTS.replacen(
        r#"{"id": 4, "parent": null, "content": {"raw": "General remark"}"#,
        r#"{"id": 13, "parent": null, "content": {"raw": "slow"}, "inline": null, "resolution": null, "pending": false, "deleted": false, "user": {"uuid": "{jo}", "nickname": "jo"}, "created_on": "2026-10-03T12:00:00.000000+00:00"},
    {"id": 4, "parent": null, "content": {"raw": "General remark"}"#,
        1,
    );
    shared
        .lock()
        .unwrap()
        .iter_mut()
        .find(|r| {
            r.path == "/repositories/acme-team/web/pullrequests/5/comments" && r.method == "GET"
        })
        .unwrap()
        .body = with_slow;
    let outcome = h
        .pull_requests
        .comment(CommentRequest {
            reference: bb_ref.into(),
            body: "slow".into(),
        })
        .await
        .unwrap();
    assert!(outcome
        .conversation
        .threads
        .iter()
        .any(|t| t.id == "13" && t.comments[0].mine));
    assert_eq!(
        requests
            .bodies("/pullrequests/5/comments")
            .iter()
            .filter(|b| b.contains("slow"))
            .count(),
        1
    );
}

// --- Merging (SPEC.md, Merging) -----------------------------------------------

const GITHUB_MERGE_SETTINGS: &str = r#"{"data": {"repository": {
    "mergeCommitAllowed": true, "squashMergeAllowed": true, "rebaseMergeAllowed": false, "deleteBranchOnMerge": false
}}}"#;
const BITBUCKET_TARGET_BRANCH: &str = r#"{"merge_strategies": ["merge_commit", "squash", "fast_forward", "rebase_merge"], "default_merge_strategy": "squash"}"#;

fn merge_routes(github_merge_status: u16) -> Vec<Route> {
    vec![
        Route {
            body_contains: "mergeCommitAllowed",
            ..route("POST", "/graphql", GITHUB_MERGE_SETTINGS)
        },
        Route {
            status: github_merge_status,
            ..route(
                "PUT",
                "/repos/acme/api/pulls/1/merge",
                r#"{"sha": "m", "merged": true, "message": "Head branch was modified. Review and try the merge again."}"#,
            )
        },
        Route {
            status: 204,
            ..route("DELETE", "/repos/acme/api/git/refs/heads/feature", "")
        },
        route(
            "GET",
            "/repositories/acme-team/web/refs/branches/main",
            BITBUCKET_TARGET_BRANCH,
        ),
        route(
            "GET",
            "/repositories/acme-team/web/refs/branches/fix-login",
            BITBUCKET_BRANCH,
        ),
        route(
            "GET",
            "/repositories/acme-team/web/pullrequests/5?fields=close_source_branch",
            r#"{"close_source_branch": true}"#,
        ),
        route(
            "GET",
            "/repositories/acme-team/web/pullrequests/5/merge/task-status/7",
            r#"{"task_status": "SUCCESS", "merge_result": {}}"#,
        ),
    ]
}

#[tokio::test(flavor = "multi_thread")]
async fn merges_are_confirmed_against_the_branch_tip_on_both_providers() {
    let head = "a".repeat(40);
    let gh = github_pr(1, "2026-10-03T10:00:00Z", &head);
    let bb = bitbucket_pr(5, "2026-10-03T09:00:00.000000+00:00", "0123456789ab");
    let mut all = merge_routes(200);
    all.extend(routes(vec![gh.clone()], vec![bb.clone()]));
    let shared = Arc::new(Mutex::new(all));
    let (base, requests) = serve_shared(Arc::clone(&shared)).await;
    // Bitbucket names the merge task in `Location`, an absolute URL.
    let task: &'static str = Box::leak(
        format!("{base}/repositories/acme-team/web/pullrequests/5/merge/task-status/7")
            .into_boxed_str(),
    );
    shared.lock().unwrap().insert(
        0,
        Route {
            status: 202,
            headers: vec![("location", task)],
            ..route(
                "POST",
                "/repositories/acme-team/web/pullrequests/5/merge",
                "",
            )
        },
    );
    let h = harness(&base).await;
    h.add_accounts().await;
    let (ws, _, _) = h.workspace(h._tmp.path()).await;
    h.list(&ws.id, 0).await;
    let gh_ref = "github.com/acme/api#1";
    let bb_ref = "bitbucket.org/acme-team/web#5";

    // What the confirmation offers comes from the repository's settings.
    let options = h.pull_requests.merge_options(gh_ref).await.unwrap();
    assert_eq!(
        options.methods,
        vec![MergeMethod::MergeCommit, MergeMethod::Squash]
    );
    assert_eq!(options.default_method, MergeMethod::MergeCommit);
    assert!(options.can_delete_branch && !options.delete_branch && !options.deletes_branch_itself);
    let options = h.pull_requests.merge_options(bb_ref).await.unwrap();
    assert_eq!(
        options.methods,
        vec![
            MergeMethod::MergeCommit,
            MergeMethod::Squash,
            MergeMethod::FastForward
        ],
        "a strategy without a counterpart on GitHub is not offered"
    );
    assert_eq!(options.default_method, MergeMethod::Squash);
    assert!(options.delete_branch && !options.deletes_branch_itself);

    // The head the user looked at is behind: nothing is merged.
    let merge = |reference: &str, expected: &str| MergeRequest {
        reference: reference.into(),
        method: MergeMethod::Squash,
        commit_title: "Add parser (#1)".into(),
        commit_message: "Parses things.".into(),
        delete_branch: true,
        expected_head_sha: expected.into(),
    };
    let err = h
        .pull_requests
        .merge(merge(gh_ref, &"b".repeat(40)))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Conflict);
    assert_eq!(requests.count("/pulls/1/merge"), 0);
    let err = h
        .pull_requests
        .merge(merge(gh_ref, "nonsense"))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Validation);

    // GitHub: one PUT naming the full head commit and the method, then the
    // branch is deleted as asked.
    let outcome = h
        .pull_requests
        .merge(merge(gh_ref, &head[..12]))
        .await
        .unwrap();
    assert!(outcome.warning.is_none());
    let bodies = requests.bodies("/pulls/1/merge");
    assert_eq!(bodies.len(), 1);
    let sent: serde_json::Value = serde_json::from_str(&bodies[0]).unwrap();
    assert_eq!(sent["sha"], head);
    assert_eq!(sent["merge_method"], "squash");
    assert_eq!(sent["commit_title"], "Add parser (#1)");
    assert_eq!(sent["commit_message"], "Parses things.");
    assert_eq!(requests.count("/git/refs/heads/feature"), 1);
    assert!(h
        .events
        .lock()
        .unwrap()
        .iter()
        .any(|e| e.reference == gh_ref && e.origin == PullRequestChangeOrigin::App));
    // A rebase takes no message; a branch in a fork is left alone.
    h.pull_requests
        .merge(MergeRequest {
            method: MergeMethod::Rebase,
            ..merge(gh_ref, &head)
        })
        .await
        .unwrap();
    let sent: serde_json::Value =
        serde_json::from_str(requests.bodies("/pulls/1/merge").last().unwrap()).unwrap();
    assert_eq!(sent["merge_method"], "rebase");
    assert!(sent.get("commit_title").is_none() && sent.get("commit_message").is_none());
    // GitHub refused: the branch moved between the check and the merge
    // (409), or protection stands in the way (405).
    for (status, code) in [(409, ErrorCode::Conflict), (405, ErrorCode::Validation)] {
        let mut all = merge_routes(status);
        all.extend(routes(vec![gh.clone()], vec![bb.clone()]));
        *shared.lock().unwrap() = all;
        let err = h
            .pull_requests
            .merge(merge(gh_ref, &head))
            .await
            .unwrap_err();
        assert_eq!(err.code, code, "{}", err.message);
    }
    // A refusal for a missing permission turns merging off for the account.
    {
        let mut all = merge_routes(403);
        all[1].body = r#"{"message": "Resource not accessible by personal access token"}"#.into();
        all.extend(routes(vec![gh.clone()], vec![bb.clone()]));
        *shared.lock().unwrap() = all;
    }
    let err = h
        .pull_requests
        .merge(merge(gh_ref, &head))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::PermissionDenied);
    assert!(
        err.message.contains("Contents: Read and write"),
        "{}",
        err.message
    );
    let pr = h.pull_requests.get(gh_ref, 0).await.unwrap();
    assert!(!pr.actions.merge.allowed && pr.actions.comment.allowed);
    let err = h
        .pull_requests
        .merge(merge(gh_ref, &head))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::PermissionDenied);

    // Bitbucket: the branch's tip is compared first, then the merge task
    // is followed to its end. The message is one text.
    let err = h
        .pull_requests
        .merge(merge(bb_ref, "9999999999"))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Conflict);
    assert_eq!(requests.count("/pullrequests/5/merge"), 0);
    let mut all = merge_routes(200);
    all.extend(routes(vec![gh.clone()], vec![bb.clone()]));
    all.insert(
        0,
        Route {
            status: 202,
            headers: vec![("location", task)],
            ..route(
                "POST",
                "/repositories/acme-team/web/pullrequests/5/merge",
                "",
            )
        },
    );
    *shared.lock().unwrap() = all;
    let outcome = h
        .pull_requests
        .merge(merge(bb_ref, "0123456789ab"))
        .await
        .unwrap();
    assert!(outcome.warning.is_none());
    let bodies: Vec<String> = requests
        .bodies("/pullrequests/5/merge?async=true")
        .into_iter()
        .filter(|b| !b.is_empty())
        .collect();
    assert_eq!(bodies.len(), 1);
    let sent: serde_json::Value = serde_json::from_str(&bodies[0]).unwrap();
    assert_eq!(sent["merge_strategy"], "squash");
    assert_eq!(sent["close_source_branch"], true);
    assert_eq!(sent["message"], "Add parser (#1)\n\nParses things.");
    assert_eq!(requests.count("/merge/task-status/7"), 1);

    // An answer that never came: the pull request is read again, and
    // merged counts as merged.
    let mut all = merge_routes(200);
    all.extend(routes(vec![gh.clone()], vec![bb.clone()]));
    all.insert(
        0,
        Route {
            status: 202,
            delay_ms: 4_000,
            ..route(
                "POST",
                "/repositories/acme-team/web/pullrequests/5/merge",
                "",
            )
        },
    );
    *shared.lock().unwrap() = all;
    let merged = bb
        .replace(r#""state": "OPEN""#, r#""state": "MERGED""#)
        .replace(
            r#""closed_on": null"#,
            r#""closed_on": "2026-10-03T12:00:00.000000+00:00""#,
        );
    let swap = Arc::clone(&shared);
    tokio::spawn(async move {
        // Bitbucket merged while Brainiac was still waiting for its answer.
        tokio::time::sleep(std::time::Duration::from_millis(1_000)).await;
        swap.lock()
            .unwrap()
            .iter_mut()
            .find(|r| r.path == "/repositories/acme-team/web/pullrequests/5" && r.method == "GET")
            .unwrap()
            .body = merged;
    });
    let outcome = h
        .pull_requests
        .merge(merge(bb_ref, "0123456789ab"))
        .await
        .unwrap();
    assert_eq!(outcome.pull_request.state, PullRequestState::Merged);
}

// --- The sidebar count, and a fetch that moves a branch -----------------------

/// Swap the answer of the first route matching `body_contains`.
fn replace_route(routes: &Arc<Mutex<Vec<Route>>>, body_contains: &str, status: u16, body: String) {
    let mut routes = routes.lock().unwrap();
    let route = routes
        .iter_mut()
        .find(|r| r.body_contains == body_contains)
        .expect("route");
    route.status = status;
    route.body = body;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_fetch_that_moves_a_branch_refreshes_its_pull_request_and_the_count() {
    let gh = github_pr(1, "2026-10-03T10:00:00Z", "a".repeat(40).as_str());
    let bb = bitbucket_pr(5, "2026-10-03T09:00:00.000000+00:00", "0123456789ab");
    let routes = Arc::new(Mutex::new(routes(vec![gh], vec![bb])));
    let (base, requests) = serve_shared(Arc::clone(&routes)).await;
    let h = harness(&base).await;
    h.add_accounts().await;

    // A checkout of a local bare remote, its pull requests tracked on GitHub
    // through the override, as a fork pointed at its upstream would be.
    let tmp = h._tmp.path();
    let seed = tmp.join("seed");
    std::fs::create_dir_all(&seed).unwrap();
    git(&seed, &["init", "-q", "-b", "main"]);
    std::fs::write(seed.join("README.md"), "hello\n").unwrap();
    git(&seed, &["add", "."]);
    git(&seed, &["commit", "-q", "-m", "initial"]);
    git(tmp, &["clone", "-q", "--bare", "seed", "remote.git"]);
    git(tmp, &["clone", "-q", "remote.git", "api"]);
    git(tmp, &["clone", "-q", "remote.git", "mate"]);
    let api = tmp.join("api");
    let api_id = h.repositories.register(&api).await.unwrap().id;
    let ws = h
        .repositories
        .create_workspace(CreateWorkspaceRequest {
            name: "Acme".into(),
            discovery_mode: DiscoveryMode::Manual,
            discovery_root: None,
            discovery_path: None,
            paths: vec![api.display().to_string()],
        })
        .await
        .unwrap()
        .workspace;
    h.repositories
        .set_forge(SetRepositoryForgeRequest {
            repository_id: api_id.clone(),
            forge: Some(ForgeTarget {
                kind: ForgeKind::Github,
                owner: "acme".into(),
                name: "api".into(),
            }),
        })
        .await
        .unwrap();

    // Nothing is counted while the workspace has pull requests off.
    assert!(h.pull_requests.review_counts().await.unwrap().is_empty());
    h.repositories
        .set_pull_requests(&ws.id, true)
        .await
        .unwrap();
    let list = h.list(&ws.id, 300).await;
    assert_eq!(list.groups.len(), 1);
    assert!(list.groups[0].pull_requests[0].awaiting_my_review);
    let counts = h.pull_requests.review_counts().await.unwrap();
    assert_eq!(
        counts,
        vec![ReviewCount {
            workspace_id: ws.id.clone(),
            awaiting: 1
        }]
    );
    // Counted from the cache: no request was made.
    let before = requests.total();
    h.pull_requests.review_counts().await.unwrap();
    assert_eq!(requests.total(), before);

    // A teammate pushes to the pull request's branch; the fetch tells the listener.
    let (tx, rx) = std::sync::mpsc::channel();
    h.repositories
        .set_fetch_listener(Arc::new(move |result| tx.send(result).unwrap()));
    let mate = tmp.join("mate");
    git(&mate, &["checkout", "-q", "-b", "feature"]);
    std::fs::write(mate.join("parser.rs"), "fn parse() {}\n").unwrap();
    git(&mate, &["add", "."]);
    git(&mate, &["commit", "-q", "-m", "parser"]);
    git(&mate, &["push", "-q", "origin", "feature"]);
    let fetched = h.repositories.fetch(&api_id, false).await.unwrap();
    assert_eq!(fetched.moved, vec!["origin/feature"]);
    assert_eq!(rx.try_recv().unwrap().moved, fetched.moved);

    // The provider cannot be reached: the pull request is marked stale, so
    // the next read with any maximum age asks again.
    let reference = "github.com/acme/api#1";
    replace_route(&routes, "\"number\"", 503, "{}".into());
    let before = requests.total();
    let read = h.pull_requests.branches_moved(&fetched).await;
    assert!(read.is_empty());
    assert_eq!(requests.total(), before + 1);
    let moved = github_pr(1, "2026-10-03T11:00:00Z", "c".repeat(40).as_str());
    replace_route(&routes, "\"number\"", 200, github_get(&moved));
    let events = h.events.lock().unwrap().len();
    let pr = h.pull_requests.get(reference, 300).await.unwrap();
    assert_eq!(pr.head_sha, "c".repeat(40));
    assert_eq!(requests.total(), before + 2);
    assert_eq!(h.events.lock().unwrap().len(), events + 1);

    // Reached: read at once, and fresh again afterwards.
    let later = github_pr(1, "2026-10-03T12:00:00Z", "d".repeat(40).as_str());
    replace_route(&routes, "\"number\"", 200, github_get(&later));
    let read = h.pull_requests.branches_moved(&fetched).await;
    assert_eq!(read, vec![reference.to_string()]);
    let before = requests.total();
    let pr = h.pull_requests.get(reference, 300).await.unwrap();
    assert_eq!(pr.head_sha, "d".repeat(40));
    assert_eq!(requests.total(), before);

    // Other refs leave the pull requests alone: another branch, a tag, a
    // branch of another remote.
    for moved in ["origin/other", "v1.0", "upstream/feature"] {
        let other = FetchResult {
            moved: vec![moved.into()],
            ..fetched.clone()
        };
        assert!(
            h.pull_requests.branches_moved(&other).await.is_empty(),
            "{moved}"
        );
    }
    assert_eq!(requests.total(), before);

    // Off again: the workspace drops out of the counts.
    h.repositories
        .set_pull_requests(&ws.id, false)
        .await
        .unwrap();
    assert!(h.pull_requests.review_counts().await.unwrap().is_empty());
}
