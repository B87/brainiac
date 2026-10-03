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

use brainiac_lib::forge::http::Http;
use brainiac_lib::forge::keychain::Token;
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
    };
    let session = Session {
        token,
        email,
        user_id: check.user_id,
        account,
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
    let files = bb.detail(&session, &mut got, &reference).await.unwrap();
    println!("  detail: checks={:?} counts={:?}", got.checks, got.counts);
    println!("  files: {:?}", files);
    let missing = PullRequestRef {
        repository: repo.clone(),
        number: 999_999,
    };
    let err = bb.get(&session, &missing).await.unwrap_err();
    println!("  missing: {:?} {}", err.code, err.message);
}
