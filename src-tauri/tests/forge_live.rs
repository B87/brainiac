//! Checks against the real GitHub and Bitbucket Cloud, with the tokens in the
//! Keychain items `brainiac`/`github` and `brainiac`/`bitbucket`. Skipped
//! unless asked for, since they need the network and the maintainer's tokens:
//!
//! ```sh
//! BRAINIAC_BITBUCKET_EMAIL=you@example.com cargo test --test forge_live -- --ignored
//! ```
//!
//! Tokens are read with the `security` tool (which macOS already trusts for
//! items it made) and are never printed.

use std::process::Command;

use brainiac_lib::credentials::Token;
use brainiac_lib::forge::http::Http;
use brainiac_lib::forge::{bitbucket, github};

fn keychain_token(account: &str) -> Option<Token> {
    let out = Command::new("security")
        .args([
            "find-generic-password",
            "-s",
            "brainiac",
            "-a",
            account,
            "-w",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Token::new(&String::from_utf8(out.stdout).ok()?).ok()
}

#[tokio::test]
#[ignore = "needs the network and a GitHub token in the Keychain"]
async fn github_says_whose_token_it_is() {
    let token = keychain_token("github").expect("no token in brainiac/github");
    let check = github::check_account(&Http::new().unwrap(), github::API, &token)
        .await
        .unwrap();
    println!(
        "GitHub: {} ({:?}), expires {:?}, missing {:?}",
        check.login, check.token_kind, check.expires_at, check.missing
    );
    assert!(!check.login.is_empty());
}

#[tokio::test]
#[ignore = "needs the network, a Bitbucket token in the Keychain, and BRAINIAC_BITBUCKET_EMAIL"]
async fn bitbucket_says_whose_token_it_is_and_its_scopes() {
    let token = keychain_token("bitbucket").expect("no token in brainiac/bitbucket");
    let email = std::env::var("BRAINIAC_BITBUCKET_EMAIL").expect("BRAINIAC_BITBUCKET_EMAIL");
    match bitbucket::check_account(&Http::new().unwrap(), bitbucket::API, &email, &token).await {
        Ok(check) => println!(
            "Bitbucket: {} scopes {:?}, missing {:?}",
            check.login, check.scopes, check.missing
        ),
        Err(e) => panic!("Bitbucket refused: {}", e.message),
    }
}

// --- Reads against real repositories -------------------------------------
//
// ```sh
// BRAINIAC_BITBUCKET_EMAIL=you@example.com \
// BRAINIAC_LIVE_GITHUB_REPO=owner/name BRAINIAC_LIVE_BITBUCKET_REPO=workspace/name \
// cargo test --test forge_live -- --ignored --nocapture
// ```

use std::sync::Arc;

use brainiac_lib::forge::adapter::{ForgeAdapter, Session};
use brainiac_lib::forge::budget::Budget;
use brainiac_lib::forge::{adapter::Client, ForgeRepository, PullRequestRef};
use brainiac_lib::models::{ForgeAccount, ForgeKind};

fn live_repo(var: &str, kind: ForgeKind) -> Option<ForgeRepository> {
    let full = std::env::var(var).ok()?;
    let (owner, name) = full.split_once('/')?;
    ForgeRepository::new(kind, owner, name)
}

async fn live_session(kind: ForgeKind) -> Option<(Session, Client)> {
    let http = Http::new().unwrap();
    let budget = Arc::new(Budget::default());
    let (token, email, check, api) = match kind {
        ForgeKind::Github => {
            let token = keychain_token("github")?;
            let check = github::check_account(&http, github::API, &token)
                .await
                .ok()?;
            (token, None, check, github::API)
        }
        ForgeKind::BitbucketCloud => {
            let token = keychain_token("bitbucket")?;
            let email = std::env::var("BRAINIAC_BITBUCKET_EMAIL").ok()?;
            let check = bitbucket::check_account(&http, bitbucket::API, &email, &token)
                .await
                .ok()?;
            (token, Some(email), check, bitbucket::API)
        }
    };
    let account = ForgeAccount {
        kind,
        login: check.login,
        user_id: check.user_id.clone(),
        display_name: check.display_name,
        email: email.clone(),
        token_kind: check.token_kind,
        expires_at: check.expires_at,
        scopes: check.scopes,
        read_only: false,
        missing: check.missing,
        checked_at: String::new(),
        token_source: brainiac_lib::models::SecretSource::Store,
        credential: brainiac_lib::models::CredentialState {
            needs_approval: false,
            pending: None,
            revision: 1,
        },
    };
    let session = Session {
        token,
        email,
        user_id: check.user_id,
        account,
        credential: None,
    };
    Some((session, Client::new(kind, api.to_string(), http, budget)))
}

#[tokio::test]
#[ignore = "needs the network, the GitHub token, and BRAINIAC_LIVE_GITHUB_REPO"]
async fn github_lists_reads_and_files_a_real_repository() {
    let repo = live_repo("BRAINIAC_LIVE_GITHUB_REPO", ForgeKind::Github).expect("repo env");
    let (session, client) = live_session(ForgeKind::Github).await.expect("session");
    let gh = github::Github::new(client);
    let open = gh.list(&session, &repo, false, None).await.unwrap();
    println!("GitHub open: {}", open.pull_requests.len());
    for pr in &open.pull_requests {
        println!(
            "  {} {:?} {} head={} reviewers={:?} checks={:?} counts={:?} mine={} actions.merge={:?}",
            pr.reference,
            pr.state,
            pr.title,
            pr.head_sha,
            pr.reviewers.iter().map(|r| (&r.user.login, r.state)).collect::<Vec<_>>(),
            pr.checks,
            pr.counts,
            pr.mine,
            pr.actions.merge
        );
    }
    let closed = gh.list(&session, &repo, true, None).await.unwrap();
    println!("GitHub closed in 30 days: {}", closed.pull_requests.len());
    for pr in &closed.pull_requests {
        println!(
            "  {} {:?} closed_at={:?}",
            pr.reference, pr.state, pr.closed_at
        );
    }
    let first = open
        .pull_requests
        .first()
        .or(closed.pull_requests.first())
        .expect("a pull request");
    let reference: PullRequestRef = first.reference.parse().unwrap();
    let got = gh.get(&session, &reference).await.unwrap();
    assert_eq!(got.version, first.version);
    let files = gh.files(&session, &reference).await.unwrap();
    println!("  files: {:?}", files);
    let checks = gh
        .checks(&session, &reference, &got.head_sha)
        .await
        .unwrap();
    println!("  checks: {:?}", checks);
    let missing = PullRequestRef {
        repository: repo.clone(),
        number: 999_999,
    };
    let err = gh.get(&session, &missing).await.unwrap_err();
    println!("  missing: {:?} {}", err.code, err.message);
    print_conversation(&gh.conversation(&session, &reference).await.unwrap());
    let patch = gh.patch(&session, &reference).await.unwrap();
    print_patch(&patch);
}

fn print_conversation(threads: &[brainiac_lib::models::Thread]) {
    println!("  threads: {}", threads.len());
    for t in threads {
        println!(
            "    {} anchor={:?} resolved={} outdated={} comments={}",
            t.id,
            t.anchor
                .as_ref()
                .map(|a| (&a.path, a.side, a.line, a.start_line)),
            t.resolved,
            t.outdated,
            t.comments.len()
        );
        for c in &t.comments {
            println!(
                "      {} by {} ({:?}) mine={} at {} edited={:?}: {} chars, html {} chars",
                c.id,
                c.author.login,
                c.review,
                c.mine,
                c.created_at,
                c.updated_at,
                c.body.len(),
                c.html.len()
            );
        }
    }
}

fn print_patch(patch: &str) {
    let files = brainiac_lib::forge::patch::split_patch(patch);
    println!("  patch: {} bytes, {} files", patch.len(), files.len());
    for f in &files {
        println!(
            "    {} (old {:?}): {} bytes",
            f.path,
            f.old_path,
            f.text.len()
        );
    }
}

#[tokio::test]
#[ignore = "needs the network, the Bitbucket token, BRAINIAC_BITBUCKET_EMAIL, and BRAINIAC_LIVE_BITBUCKET_REPO"]
async fn bitbucket_lists_reads_and_files_a_real_repository() {
    let repo =
        live_repo("BRAINIAC_LIVE_BITBUCKET_REPO", ForgeKind::BitbucketCloud).expect("repo env");
    let (session, client) = live_session(ForgeKind::BitbucketCloud)
        .await
        .expect("session");
    let bb = bitbucket::Bitbucket::new(client);
    let open = bb.list(&session, &repo, false, None).await.unwrap();
    println!("Bitbucket open: {}", open.pull_requests.len());
    for pr in &open.pull_requests {
        println!(
            "  {} {:?} {} head={} reviewers={:?} counts={:?} mine={} url={}",
            pr.reference,
            pr.state,
            pr.title,
            pr.head_sha,
            pr.reviewers
                .iter()
                .map(|r| (&r.user.login, r.state))
                .collect::<Vec<_>>(),
            pr.counts,
            pr.mine,
            pr.web_url
        );
    }
    let closed = bb.list(&session, &repo, true, None).await.unwrap();
    println!(
        "Bitbucket closed in 30 days: {}",
        closed.pull_requests.len()
    );
    for pr in &closed.pull_requests {
        println!(
            "  {} {:?} closed_at={:?}",
            pr.reference, pr.state, pr.closed_at
        );
    }
    let first = open
        .pull_requests
        .first()
        .or(closed.pull_requests.first())
        .expect("a pull request");
    let reference: PullRequestRef = first.reference.parse().unwrap();
    let mut got = bb.get(&session, &reference).await.unwrap();
    assert_eq!(got.version, first.version);
    let (files, threads) = bb.detail(&session, &mut got, &reference).await.unwrap();
    println!("  detail: checks={:?} counts={:?}", got.checks, got.counts);
    println!("  files: {:?}", files);
    print_conversation(&threads);
    let patch = bb.patch(&session, &reference).await.unwrap();
    print_patch(&patch);
    let missing = PullRequestRef {
        repository: repo.clone(),
        number: 999_999,
    };
    let err = bb.get(&session, &missing).await.unwrap_err();
    println!("  missing: {:?} {}", err.code, err.message);
}

// --- Writes against real repositories (SPEC.md, Reviewing) ------------------
//
// These post comments and a review on the first open pull request of each
// repository, so they run only with `BRAINIAC_LIVE_WRITE=1` on top of the
// variables above. Everything they post starts with "Brainiac live test".

use brainiac_lib::forge::adapter::{find_posted, ReviewToSend, SentPart};
use brainiac_lib::models::{DiffContent, DiffSide, ReviewDraft, ReviewVerdict, ThreadAnchor};

fn live_write() -> bool {
    std::env::var("BRAINIAC_LIVE_WRITE").is_ok_and(|v| v == "1")
}

/// The first changed line of the pull request's first file, from the
/// provider's patch: a line a review comment can go on.
fn first_changed_line(patch: &str) -> Option<(String, u32)> {
    let files = brainiac_lib::forge::patch::split_patch(patch);
    for f in &files {
        let content = brainiac_lib::git::parse_unified_diff(
            f.text.as_bytes(),
            false,
            &brainiac_lib::models::DiffLimits {
                max_bytes: 5_000_000,
                max_lines: 50_000,
            },
        );
        if let DiffContent::Text { hunks, .. } = content {
            for h in &hunks {
                for l in &h.lines {
                    if let Some(n) = l.new_no {
                        return Some((f.path.clone(), n as u32));
                    }
                }
            }
        }
    }
    None
}

async fn live_writes<A: ForgeAdapter>(adapter: &A, session: &Session, repo: &ForgeRepository) {
    let stamp = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S");
    let open = adapter.list(session, repo, false, None).await.unwrap();
    let pr = open.pull_requests.first().expect("an open pull request");
    let reference: PullRequestRef = pr.reference.parse().unwrap();
    println!("writing to {} (head {})", pr.reference, pr.head_sha);

    let head = adapter.current_head(session, pr).await.unwrap();
    println!("  branch tip: {head}");
    assert!(head.starts_with(&pr.head_sha) || pr.head_sha.starts_with(&head));

    let body = format!("Brainiac live test: comment at {stamp}");
    let id = adapter.comment(session, &reference, &body).await.unwrap();
    println!("  comment: {id}");

    let patch = adapter.patch(session, &reference).await.unwrap();
    let (path, line) = first_changed_line(&patch).expect("a changed line");
    let draft_body = format!("Brainiac live test: line comment at {stamp}");
    let draft = ReviewDraft {
        id: "live".into(),
        reference: pr.reference.clone(),
        anchor: ThreadAnchor {
            path: path.clone(),
            side: DiffSide::New,
            line: Some(line),
            start_line: None,
            commit: Some(head.clone()),
        },
        body: draft_body.clone(),
        html: String::new(),
        remote_id: None,
        created_at: String::new(),
        updated_at: String::new(),
    };
    let summary = format!("Brainiac live test: review at {stamp}");
    let review = ReviewToSend {
        drafts: std::slice::from_ref(&draft),
        body: &summary,
        summary_sent: false,
        verdict: ReviewVerdict::Comment,
        head_sha: &head,
    };
    let mut parts = Vec::new();
    adapter
        .submit_review(session, pr, &review, &mut |p: SentPart| parts.push(p))
        .await
        .unwrap();
    println!(
        "  review sent on {path}:{line}; parts reported: {}",
        parts.len()
    );

    let threads = adapter.conversation(session, &reference).await.unwrap();
    assert!(
        find_posted(&threads, None, None, &body).is_some(),
        "the comment"
    );
    let posted = find_posted(&threads, Some(&path), None, &draft_body).expect("the line comment");
    let thread = threads
        .iter()
        .find(|t| t.comments.iter().any(|c| c.id == posted))
        .unwrap();
    println!("  thread {} on {:?}", thread.id, thread.anchor);
    let reply = adapter
        .reply(
            session,
            &reference,
            thread,
            &format!("Brainiac live test: reply at {stamp}"),
        )
        .await
        .unwrap();
    println!("  reply: {reply}");
    adapter
        .resolve(session, &reference, thread, true)
        .await
        .unwrap();
    let threads = adapter.conversation(session, &reference).await.unwrap();
    let again = threads.iter().find(|t| t.id == thread.id).unwrap();
    assert!(again.resolved, "resolved");
    assert_eq!(again.comments.len(), 2);
    adapter
        .resolve(session, &reference, thread, false)
        .await
        .unwrap();
    let threads = adapter.conversation(session, &reference).await.unwrap();
    assert!(!threads.iter().find(|t| t.id == thread.id).unwrap().resolved);
    println!("  resolved and reopened");
}

#[tokio::test]
#[ignore = "writes to a real repository: needs BRAINIAC_LIVE_WRITE=1 and the GitHub variables"]
async fn github_takes_a_comment_a_review_a_reply_and_a_resolution() {
    if !live_write() {
        eprintln!("skipped: set BRAINIAC_LIVE_WRITE=1 to write to the repository");
        return;
    }
    let repo = live_repo("BRAINIAC_LIVE_GITHUB_REPO", ForgeKind::Github).expect("repo env");
    let (session, client) = live_session(ForgeKind::Github).await.expect("session");
    live_writes(&github::Github::new(client), &session, &repo).await;
}

#[tokio::test]
#[ignore = "writes to a real repository: needs BRAINIAC_LIVE_WRITE=1 and the Bitbucket variables"]
async fn bitbucket_takes_a_comment_a_review_a_reply_and_a_resolution() {
    if !live_write() {
        eprintln!("skipped: set BRAINIAC_LIVE_WRITE=1 to write to the repository");
        return;
    }
    let repo =
        live_repo("BRAINIAC_LIVE_BITBUCKET_REPO", ForgeKind::BitbucketCloud).expect("repo env");
    let (session, client) = live_session(ForgeKind::BitbucketCloud)
        .await
        .expect("session");
    live_writes(&bitbucket::Bitbucket::new(client), &session, &repo).await;
}

// --- Merges against real repositories (SPEC.md, Merging) ---------------------
//
// Each test makes a branch with one commit and a pull request for it on the
// repository, then merges it from the adapter and checks the branch is gone.
// Everything it makes is named "brainiac-merge-<time>".

use brainiac_lib::models::{MergeMethod, MergeRequest, PullRequestState};

/// Standard base64, for GitHub's contents API; small enough to not need a crate.
fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().fold(0u32, |acc, b| (acc << 8) | *b as u32) << (8 * (3 - chunk.len()));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(TABLE[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn expect_json(response: &brainiac_lib::forge::http::Response, what: &str) -> serde_json::Value {
    assert!(
        matches!(response.status, 200 | 201),
        "{what}: {} {}",
        response.status,
        String::from_utf8_lossy(&response.body)
    );
    serde_json::from_slice(&response.body).expect(what)
}

/// A branch with one new file and a pull request for it, on GitHub.
async fn github_make_pull_request(
    client: &Client,
    session: &Session,
    repo: &ForgeRepository,
    branch: &str,
) -> PullRequestRef {
    let api = format!("{}/repos/{}/{}", client.api, repo.owner, repo.name);
    let headers = [("Accept", "application/vnd.github+json")];
    let repository = expect_json(
        &client.get(session, &api, &headers).await.unwrap(),
        "repository",
    );
    let default_branch = repository["default_branch"].as_str().unwrap().to_string();
    let tip = expect_json(
        &client
            .get(
                session,
                &format!("{api}/git/ref/heads/{default_branch}"),
                &headers,
            )
            .await
            .unwrap(),
        "default branch",
    );
    let sha = tip["object"]["sha"].as_str().unwrap().to_string();
    expect_json(
        &client
            .post_json(
                session,
                &format!("{api}/git/refs"),
                &headers,
                &serde_json::json!({ "ref": format!("refs/heads/{branch}"), "sha": sha }),
            )
            .await
            .unwrap(),
        "branch",
    );
    expect_json(
        &client
            .put_json(
                session,
                &format!("{api}/contents/{branch}.txt"),
                &headers,
                &serde_json::json!({
                    "message": format!("Brainiac live test: {branch}"),
                    "content": base64(format!("made by {branch}\n").as_bytes()),
                    "branch": branch,
                }),
            )
            .await
            .unwrap(),
        "commit",
    );
    let pr = expect_json(
        &client
            .post_json(
                session,
                &format!("{api}/pulls"),
                &headers,
                &serde_json::json!({
                    "title": format!("Brainiac live test: merge {branch}"),
                    "head": branch, "base": default_branch,
                    "body": "Made and merged by Brainiac's live test.",
                }),
            )
            .await
            .unwrap(),
        "pull request",
    );
    PullRequestRef {
        repository: repo.clone(),
        number: pr["number"].as_u64().unwrap(),
    }
}

/// The same on Bitbucket: the branch, then a commit on it through `/src`.
async fn bitbucket_make_pull_request(
    client: &Client,
    session: &Session,
    repo: &ForgeRepository,
    branch: &str,
) -> PullRequestRef {
    let api = format!("{}/repositories/{}/{}", client.api, repo.owner, repo.name);
    let repository = expect_json(&client.get(session, &api, &[]).await.unwrap(), "repository");
    let main = repository["mainbranch"]["name"]
        .as_str()
        .unwrap()
        .to_string();
    let tip = expect_json(
        &client
            .get(session, &format!("{api}/refs/branches/{main}"), &[])
            .await
            .unwrap(),
        "main branch",
    );
    let hash = tip["target"]["hash"].as_str().unwrap().to_string();
    expect_json(
        &client
            .post_json(
                session,
                &format!("{api}/refs/branches"),
                &[],
                &serde_json::json!({ "name": branch, "target": { "hash": hash } }),
            )
            .await
            .unwrap(),
        "branch",
    );
    let file = format!("{branch}.txt");
    let content = format!("made by {branch}\n");
    let message = format!("Brainiac live test: {branch}");
    let committed = client
        .post_form(
            session,
            &format!("{api}/src"),
            &[("branch", branch), ("message", &message), (&file, &content)],
        )
        .await
        .unwrap();
    assert!(
        matches!(committed.status, 200 | 201),
        "commit: {} {}",
        committed.status,
        String::from_utf8_lossy(&committed.body)
    );
    let pr = expect_json(
        &client
            .post_json(
                session,
                &format!("{api}/pullrequests"),
                &[],
                &serde_json::json!({
                    "title": format!("Brainiac live test: merge {branch}"),
                    "source": { "branch": { "name": branch } },
                    "destination": { "branch": { "name": main } },
                    "description": "Made and merged by Brainiac's live test.",
                }),
            )
            .await
            .unwrap(),
        "pull request",
    );
    PullRequestRef {
        repository: repo.clone(),
        number: pr["id"].as_u64().unwrap(),
    }
}

async fn live_merge<A: ForgeAdapter>(
    adapter: &A,
    session: &Session,
    reference: &PullRequestRef,
    branch_gone: impl Fn(&str) -> String,
    client: &Client,
) {
    let pr = adapter.get(session, reference).await.unwrap();
    println!("merging {} (head {})", pr.reference, pr.head_sha);
    assert!(pr.actions.merge.allowed, "{:?}", pr.actions.merge.reason);
    let options = adapter.merge_options(session, &pr).await.unwrap();
    println!(
        "  methods: {:?}, default {:?}, delete branch {} (itself: {})",
        options.methods,
        options.default_method,
        options.delete_branch,
        options.deletes_branch_itself
    );
    assert!(options.can_delete_branch);
    let head = adapter.current_head(session, &pr).await.unwrap();
    let method = if options.methods.contains(&MergeMethod::Squash) {
        MergeMethod::Squash
    } else {
        options.default_method
    };
    let warning = adapter
        .merge(
            session,
            &pr,
            &MergeRequest {
                reference: pr.reference.clone(),
                method,
                commit_title: format!("{} (#{})", pr.title, pr.number),
                commit_message: "Merged by Brainiac's live test.".into(),
                delete_branch: true,
                expected_head_sha: head,
            },
        )
        .await
        .unwrap();
    println!("  merged with {method:?}; warning: {warning:?}");
    assert!(warning.is_none());
    let merged = adapter.get(session, reference).await.unwrap();
    assert_eq!(merged.state, PullRequestState::Merged);
    assert!(!merged.actions.merge.allowed);
    let gone = client
        .get(session, &branch_gone(&pr.source_branch), &[])
        .await
        .unwrap();
    println!("  branch {} answers {}", pr.source_branch, gone.status);
    assert_eq!(gone.status, 404, "the branch was deleted");
}

#[tokio::test]
#[ignore = "writes to a real repository: needs BRAINIAC_LIVE_WRITE=1 and the GitHub variables"]
async fn github_merges_a_pull_request_it_made() {
    if !live_write() {
        eprintln!("skipped: set BRAINIAC_LIVE_WRITE=1 to write to the repository");
        return;
    }
    let repo = live_repo("BRAINIAC_LIVE_GITHUB_REPO", ForgeKind::Github).expect("repo env");
    let (session, client) = live_session(ForgeKind::Github).await.expect("session");
    let branch = format!(
        "brainiac-merge-{}",
        chrono::Utc::now().format("%Y%m%d-%H%M%S")
    );
    let reference = github_make_pull_request(&client, &session, &repo, &branch).await;
    println!("made {reference} on {branch}");
    let api = format!("{}/repos/{}/{}", client.api, repo.owner, repo.name);
    let adapter = github::Github::new(client);
    let client = live_session(ForgeKind::Github).await.expect("session").1;
    live_merge(
        &adapter,
        &session,
        &reference,
        |b| format!("{api}/git/ref/heads/{b}"),
        &client,
    )
    .await;
}

#[tokio::test]
#[ignore = "writes to a real repository: needs BRAINIAC_LIVE_WRITE=1 and the Bitbucket variables"]
async fn bitbucket_merges_a_pull_request_it_made() {
    if !live_write() {
        eprintln!("skipped: set BRAINIAC_LIVE_WRITE=1 to write to the repository");
        return;
    }
    let repo =
        live_repo("BRAINIAC_LIVE_BITBUCKET_REPO", ForgeKind::BitbucketCloud).expect("repo env");
    let (session, client) = live_session(ForgeKind::BitbucketCloud)
        .await
        .expect("session");
    let branch = format!(
        "brainiac-merge-{}",
        chrono::Utc::now().format("%Y%m%d-%H%M%S")
    );
    let reference = bitbucket_make_pull_request(&client, &session, &repo, &branch).await;
    println!("made {reference} on {branch}");
    let api = format!("{}/repositories/{}/{}", client.api, repo.owner, repo.name);
    let adapter = bitbucket::Bitbucket::new(client);
    let client = live_session(ForgeKind::BitbucketCloud)
        .await
        .expect("session")
        .1;
    live_merge(
        &adapter,
        &session,
        &reference,
        |b| format!("{api}/refs/branches/{b}"),
        &client,
    )
    .await;
}
