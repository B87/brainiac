//! Settings → Accounts (SPEC.md, Accounts): a token is checked with one
//! request, kept in the Keychain only once the check passes, and a token that
//! cannot write is saved only as read-only. A local server stands in for
//! GitHub and Bitbucket, and the Keychain is in memory.

use std::sync::{Arc, Mutex};

use brainiac_lib::db::Db;
use brainiac_lib::forge::http::Http;
use brainiac_lib::forge::keychain::{Keychain, MemoryKeychain, Token};
use brainiac_lib::forge::{AccountService, Endpoints};
use brainiac_lib::models::{
    ErrorCode, ForgeKind, ForgeTokenKind, SaveForgeAccountOutcome, SaveForgeAccountRequest,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// A canned answer: status, headers, body.
type Answer = (u16, Vec<(&'static str, &'static str)>, &'static str);

/// Serves `answers` in order, one per connection, and records each request's head.
async fn serve(answers: Vec<Answer>) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&requests);
    tokio::spawn(async move {
        for (status, headers, body) in answers {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut head = Vec::new();
            let mut buf = [0u8; 1024];
            while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = socket.read(&mut buf).await.unwrap();
                if n == 0 {
                    break;
                }
                head.extend_from_slice(&buf[..n]);
            }
            seen.lock()
                .unwrap()
                .push(String::from_utf8_lossy(&head).to_lowercase());
            let mut response = format!(
                "HTTP/1.1 {status} X\r\ncontent-length: {}\r\ncontent-type: application/json\r\nconnection: close\r\n",
                body.len()
            );
            for (name, value) in headers {
                response.push_str(&format!("{name}: {value}\r\n"));
            }
            response.push_str("\r\n");
            response.push_str(body);
            socket.write_all(response.as_bytes()).await.unwrap();
            socket.shutdown().await.ok();
        }
    });
    (base, requests)
}

struct Harness {
    accounts: AccountService,
    keychain: Arc<MemoryKeychain>,
    _dir: tempfile::TempDir,
}

fn harness(base: &str) -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(&dir.path().join("brainiac.sqlite3")).unwrap();
    let keychain = Arc::new(MemoryKeychain::default());
    let endpoints = Endpoints {
        github: base.to_string(),
        bitbucket: base.to_string(),
    };
    Harness {
        accounts: AccountService::new(
            db,
            keychain.clone(),
            Http::insecure_for_tests().unwrap(),
            endpoints,
        ),
        keychain,
        _dir: dir,
    }
}

fn request(kind: ForgeKind, token: Option<&str>, email: Option<&str>) -> SaveForgeAccountRequest {
    SaveForgeAccountRequest {
        kind,
        token: token.map(str::to_string),
        email: email.map(str::to_string),
        read_only: false,
    }
}

const GITHUB_USER: &str = r#"{"login":"octo","id":42,"name":"Octo Cat"}"#;
const BITBUCKET_USER: &str =
    r#"{"username":"jo","display_name":"Jo Doe","uuid":"{0a1b}","account_id":"557058:x"}"#;
const ALL_SCOPES: &str = "read:user:bitbucket, read:repository:bitbucket, read:pullrequest:bitbucket, write:pullrequest:bitbucket";

#[tokio::test(flavor = "multi_thread")]
async fn a_github_token_is_checked_then_kept_in_the_keychain() {
    let (base, requests) = serve(vec![(
        200,
        vec![(
            "github-authentication-token-expiration",
            "2027-10-02 22:00:00 UTC",
        )],
        GITHUB_USER,
    )])
    .await;
    let h = harness(&base);

    let outcome = h
        .accounts
        .save(request(ForgeKind::Github, Some(" github_pat_abc \n"), None))
        .await
        .unwrap();
    let SaveForgeAccountOutcome::Saved { account } = outcome else {
        panic!("not saved: {outcome:?}");
    };
    assert_eq!(account.login, "octo");
    assert_eq!(account.display_name.as_deref(), Some("Octo Cat"));
    assert_eq!(account.token_kind, ForgeTokenKind::FineGrained);
    assert_eq!(
        account.expires_at.as_deref(),
        Some("2027-10-02T22:00:00.000Z")
    );
    assert_eq!(account.scopes, None);
    assert!(!account.read_only);
    assert_eq!(
        h.keychain.get(ForgeKind::Github).unwrap(),
        Some(Token::new("github_pat_abc").unwrap())
    );

    let head = requests.lock().unwrap()[0].clone();
    assert!(head.starts_with("get /user "), "{head}");
    assert!(
        head.contains("authorization: bearer github_pat_abc\r\n"),
        "{head}"
    );
    assert!(head.contains("user-agent: brainiac/"), "{head}");

    let slots = h.accounts.list().await.unwrap();
    assert_eq!(slots[0].kind, ForgeKind::Github);
    assert_eq!(slots[0].account.as_ref().unwrap().login, "octo");
    assert_eq!(slots[1].kind, ForgeKind::BitbucketCloud);
    assert!(slots[1].account.is_none());
    assert!(!slots[1].keychain_token);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_refused_token_is_neither_kept_nor_saved() {
    let (base, _) = serve(vec![(401, vec![], "")]).await;
    let h = harness(&base);
    let error = h
        .accounts
        .save(request(
            ForgeKind::BitbucketCloud,
            Some("ATATT3x"),
            Some("jo@example.com"),
        ))
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::PermissionDenied);
    assert!(
        error.message.contains("190 characters"),
        "{}",
        error.message
    );
    assert!(!h.keychain.contains(ForgeKind::BitbucketCloud).unwrap());
    assert!(h
        .accounts
        .account(ForgeKind::BitbucketCloud)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_bitbucket_token_that_cannot_write_is_saved_only_as_read_only() {
    let read_only = "read:user:bitbucket, read:repository:bitbucket, read:pullrequest:bitbucket";
    let (base, requests) = serve(vec![
        (200, vec![("x-oauth-scopes", read_only)], BITBUCKET_USER),
        (200, vec![("x-oauth-scopes", read_only)], BITBUCKET_USER),
    ])
    .await;
    let h = harness(&base);
    let mut req = request(
        ForgeKind::BitbucketCloud,
        Some("ATATT3x"),
        Some(" jo@example.com "),
    );

    let outcome = h.accounts.save(req.clone()).await.unwrap();
    assert_eq!(
        outcome,
        SaveForgeAccountOutcome::ReadOnly {
            login: "jo".into(),
            missing: vec!["write:pullrequest:bitbucket".into()],
        }
    );
    assert!(!h.keychain.contains(ForgeKind::BitbucketCloud).unwrap());

    req.read_only = true;
    let SaveForgeAccountOutcome::Saved { account } = h.accounts.save(req).await.unwrap() else {
        panic!("not saved");
    };
    assert!(account.read_only);
    assert_eq!(account.email.as_deref(), Some("jo@example.com"));
    assert_eq!(account.token_kind, ForgeTokenKind::ApiToken);
    assert_eq!(account.missing, vec!["write:pullrequest:bitbucket"]);
    assert_eq!(account.scopes.as_ref().map(Vec::len), Some(3));
    assert!(h.keychain.contains(ForgeKind::BitbucketCloud).unwrap());

    // HTTP Basic with the email: base64("jo@example.com:ATATT3x").
    let head = requests.lock().unwrap()[0].clone();
    assert!(
        head.contains("authorization: basic am9azxhhbxbszs5jb206qvrbvfqzea==\r\n"),
        "{head}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_token_without_a_read_scope_names_it() {
    let (base, _) = serve(vec![(
        403,
        vec![(
            "x-oauth-scopes",
            "read:repository:bitbucket, write:pullrequest:bitbucket",
        )],
        r#"{"type":"error"}"#,
    )])
    .await;
    let h = harness(&base);
    let error = h
        .accounts
        .save(request(
            ForgeKind::BitbucketCloud,
            Some("ATATT3x"),
            Some("jo@example.com"),
        ))
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::PermissionDenied);
    assert!(
        error.message.contains("read:user:bitbucket"),
        "{}",
        error.message
    );
    // A write scope covers its read scope.
    assert!(
        !error.message.contains("read:pullrequest"),
        "{}",
        error.message
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_token_already_in_the_keychain_is_found_and_used() {
    let (base, _) = serve(vec![(
        200,
        vec![("x-oauth-scopes", ALL_SCOPES)],
        BITBUCKET_USER,
    )])
    .await;
    let h = harness(&base);
    h.keychain
        .set(ForgeKind::BitbucketCloud, &Token::new("ATATT3x").unwrap())
        .unwrap();
    let slots = h.accounts.list().await.unwrap();
    assert!(slots[1].keychain_token);

    // Bitbucket still needs the email.
    let error = h
        .accounts
        .save(request(ForgeKind::BitbucketCloud, None, None))
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Validation);

    let outcome = h
        .accounts
        .save(request(
            ForgeKind::BitbucketCloud,
            None,
            Some("jo@example.com"),
        ))
        .await
        .unwrap();
    let SaveForgeAccountOutcome::Saved { account } = outcome else {
        panic!("not saved: {outcome:?}");
    };
    assert!(!account.read_only);
    assert!(account.missing.is_empty());

    // Without a token anywhere there is nothing to check.
    let error = h
        .accounts
        .save(request(ForgeKind::Github, None, None))
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::NotFound);

    let slots = h.accounts.remove(ForgeKind::BitbucketCloud).await.unwrap();
    assert!(slots[1].account.is_none());
    assert!(!slots[1].keychain_token);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unreachable_provider_is_a_dependency_error() {
    // Bind a port, then close it, so nothing answers there.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let h = harness(&base);
    let error = h
        .accounts
        .save(request(ForgeKind::Github, Some("ghp_abc"), None))
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::DependencyUnavailable);
    assert!(error.retryable);
    assert!(!h.keychain.contains(ForgeKind::Github).unwrap());
}
