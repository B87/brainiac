//! Settings → Accounts (SPEC.md, Accounts): a token is checked with one
//! request, kept in the Keychain only once the check passes, and a token that
//! cannot write is saved only as read-only. A local server stands in for
//! GitHub and Bitbucket, and the Keychain is in memory.

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::{Arc, Mutex};

use brainiac_lib::credentials::{CommandRunner, CredentialService, MemoryStore, SecretStore};
use brainiac_lib::db::Db;
use brainiac_lib::forge::adapter::{Client, Session};
use brainiac_lib::forge::budget::Budget;
use brainiac_lib::forge::http::Http;
use brainiac_lib::forge::{AccountService, Endpoints};
use brainiac_lib::models::{
    CredentialPending, ErrorCode, ForgeKind, ForgeTokenKind, SaveForgeAccountOutcome,
    SaveForgeAccountRequest, SecretSource,
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
    store: Arc<MemoryStore>,
    db: Db,
    base: String,
    dir: tempfile::TempDir,
}

fn service(db: &Db, store: &Arc<MemoryStore>, dir: &Path, base: &str) -> AccountService {
    let credentials = Arc::new(CredentialService::new(
        store.clone(),
        CommandRunner::new(dir.join("commands")),
    ));
    let endpoints = Endpoints {
        github: base.to_string(),
        bitbucket: base.to_string(),
    };
    AccountService::new(
        db.clone(),
        credentials,
        Http::insecure_for_tests().unwrap(),
        endpoints,
    )
}

fn harness(base: &str) -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(&dir.path().join("brainiac.sqlite3")).unwrap();
    let store = Arc::new(MemoryStore::default());
    Harness {
        accounts: service(&db, &store, dir.path(), base),
        store,
        db,
        base: base.to_string(),
        dir,
    }
}

impl Harness {
    /// Brainiac quits and starts again: the same files, nothing in memory.
    fn restart(&mut self) {
        self.accounts = service(&self.db, &self.store, self.dir.path(), &self.base);
    }

    /// A command that prints the token in `token.txt` and counts its runs.
    fn token_command(&self, token: &str) -> SecretSource {
        let script = self.dir.path().join("gh");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\necho run >> '{0}/runs'\ncat '{0}/token.txt'\n",
                self.dir.path().display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        self.set_token(token);
        SecretSource::Command {
            program: script.display().to_string(),
            args: vec!["auth".into(), "token".into()],
        }
    }

    fn set_token(&self, token: &str) {
        std::fs::write(self.dir.path().join("token.txt"), format!("{token}\n")).unwrap();
    }

    fn runs(&self) -> usize {
        std::fs::read_to_string(self.dir.path().join("runs"))
            .map(|s| s.lines().count())
            .unwrap_or(0)
    }
}

fn request(kind: ForgeKind, token: Option<&str>, email: Option<&str>) -> SaveForgeAccountRequest {
    SaveForgeAccountRequest {
        kind,
        source: SecretSource::Store,
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
    let SaveForgeAccountOutcome::Saved { account, .. } = outcome else {
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
    assert_eq!(h.store.text("github").as_deref(), Some("github_pat_abc"));

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
    assert_eq!(error.code, ErrorCode::Unauthenticated);
    assert!(
        error.message.contains("190 characters"),
        "{}",
        error.message
    );
    assert!(!h.store.contains("bitbucket").unwrap());
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
    assert!(!h.store.contains("bitbucket").unwrap());

    req.read_only = true;
    let SaveForgeAccountOutcome::Saved { account, .. } = h.accounts.save(req).await.unwrap() else {
        panic!("not saved");
    };
    assert!(account.read_only);
    assert_eq!(account.email.as_deref(), Some("jo@example.com"));
    assert_eq!(account.token_kind, ForgeTokenKind::ApiToken);
    assert_eq!(account.missing, vec!["write:pullrequest:bitbucket"]);
    assert_eq!(account.scopes.as_ref().map(Vec::len), Some(3));
    assert!(h.store.contains("bitbucket").unwrap());

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
    h.store.insert("bitbucket", "ATATT3x");
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
    let SaveForgeAccountOutcome::Saved { account, .. } = outcome else {
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
    assert!(!h.store.contains("github").unwrap());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_token_from_a_command_is_checked_and_never_switches_the_account() {
    let mona = r#"{"login":"mona","id":43,"name":"Mona"}"#;
    let (base, requests) = serve(vec![
        (200, vec![], GITHUB_USER),
        (200, vec![], GITHUB_USER),
        (200, vec![], mona),
        (401, vec![], ""),
    ])
    .await;
    let h = harness(&base);
    let source = h.token_command("gho_first");
    let outcome = h
        .accounts
        .save(SaveForgeAccountRequest {
            source: source.clone(),
            ..request(ForgeKind::Github, None, None)
        })
        .await
        .unwrap();
    let SaveForgeAccountOutcome::Saved { account, warning } = outcome else {
        panic!("not saved: {outcome:?}");
    };
    assert_eq!((account.login.as_str(), warning), ("octo", None));
    assert_eq!(account.token_source, source);
    assert!(
        !h.store.contains("github").unwrap(),
        "nothing goes to the Keychain"
    );
    assert!(requests.lock().unwrap()[0].contains("authorization: bearer gho_first"));

    // The token checked by the save is used without another read or check.
    let (token, _) = h.accounts.token(ForgeKind::Github).await.unwrap();
    assert_eq!(token.expose(), "gho_first");
    assert_eq!((h.runs(), requests.lock().unwrap().len()), (1, 1));

    // Refreshed: read and checked again.
    h.accounts.refresh(ForgeKind::Github);
    h.accounts.token(ForgeKind::Github).await.unwrap();
    h.accounts.token(ForgeKind::Github).await.unwrap();
    assert_eq!((h.runs(), requests.lock().unwrap().len()), (2, 2));

    // The tool switched users: refused, and the account stays octo's.
    h.accounts.refresh(ForgeKind::Github);
    h.set_token("gho_mona");
    let err = h.accounts.token(ForgeKind::Github).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::PermissionDenied);
    assert!(err.message.contains("belongs to mona"), "{}", err.message);
    let still = h
        .accounts
        .account(ForgeKind::Github)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (still.login.as_str(), still.user_id.as_str()),
        ("octo", "42")
    );

    // Revoked: the refused token is forgotten, so the next use reads again.
    h.accounts.refresh(ForgeKind::Github);
    h.set_token("gho_revoked");
    let err = h.accounts.token(ForgeKind::Github).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Unauthenticated);
    let runs = h.runs();
    let _ = h.accounts.token(ForgeKind::Github).await;
    assert_eq!(h.runs(), runs + 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_move_to_a_command_deletes_the_item_and_a_restored_source_waits_for_approval() {
    let (base, _) = serve(vec![
        (200, vec![], GITHUB_USER),
        (200, vec![], GITHUB_USER),
        (200, vec![], GITHUB_USER),
    ])
    .await;
    let mut h = harness(&base);
    h.accounts
        .save(request(ForgeKind::Github, Some("github_pat_abc"), None))
        .await
        .unwrap();
    assert!(h.store.contains("github").unwrap());
    let source = h.token_command("gho_first");
    let outcome = h
        .accounts
        .save(SaveForgeAccountRequest {
            source,
            ..request(ForgeKind::Github, None, None)
        })
        .await
        .unwrap();
    let SaveForgeAccountOutcome::Saved { account, warning } = outcome else {
        panic!("not saved: {outcome:?}");
    };
    assert_eq!(warning, None);
    assert_eq!(account.credential.pending, None);
    assert!(!h.store.contains("github").unwrap());

    // What a restore does to every account.
    h.db.call(|conn| Ok(conn.execute("UPDATE forge_accounts SET source_approved = 0", [])?))
        .await
        .unwrap();
    h.restart();
    let runs = h.runs();
    let err = h.accounts.token(ForgeKind::Github).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::PermissionDenied);
    assert_eq!(h.runs(), runs, "nothing ran before the source was allowed");
    let entries = h.accounts.secret_entries().await.unwrap();
    assert!(entries[0].state.needs_approval);
    assert_eq!(
        entries[0].destination,
        format!("{} as octo", base.trim_start_matches("http://"))
    );

    h.accounts
        .approve(ForgeKind::Github, entries[0].state.revision)
        .await
        .unwrap();
    let (token, _) = h.accounts.token(ForgeKind::Github).await.unwrap();
    assert_eq!(token.expose(), "gho_first");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_token_read_again_after_its_expiry_is_checked_again() {
    let mona = r#"{"login":"mona","id":43,"name":"Mona"}"#;
    let (base, requests) = serve(vec![
        (
            200,
            vec![(
                "github-authentication-token-expiration",
                "2020-01-01 00:00:00 UTC",
            )],
            GITHUB_USER,
        ),
        (200, vec![], mona),
    ])
    .await;
    let h = harness(&base);
    let source = h.token_command("gho_first");
    h.accounts
        .save(SaveForgeAccountRequest {
            source,
            ..request(ForgeKind::Github, None, None)
        })
        .await
        .unwrap();
    // The saved expiry has passed: the token is read again, and the new
    // read is checked like any other, so the switch is caught.
    h.set_token("gho_mona");
    let err = h.accounts.token(ForgeKind::Github).await.unwrap_err();
    assert!(err.message.contains("belongs to mona"), "{}", err.message);
    assert_eq!(requests.lock().unwrap().len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_item_left_by_a_failed_cleanup_is_never_taken_up_again() {
    let (base, _) = serve(vec![(200, vec![], GITHUB_USER), (200, vec![], GITHUB_USER)]).await;
    let h = harness(&base);
    h.accounts
        .save(request(ForgeKind::Github, Some("github_pat_old"), None))
        .await
        .unwrap();
    h.store.fail_deletes(true);
    let source = h.token_command("gho_first");
    let outcome = h
        .accounts
        .save(SaveForgeAccountRequest {
            source,
            ..request(ForgeKind::Github, None, None)
        })
        .await
        .unwrap();
    let SaveForgeAccountOutcome::Saved { account, warning } = outcome else {
        panic!("not saved: {outcome:?}");
    };
    assert!(warning.unwrap().contains("could not be deleted"));
    assert_eq!(account.credential.pending, Some(CredentialPending::Cleanup));

    // Back to the Keychain without pasting: refused, not the old token.
    let err = h
        .accounts
        .save(request(ForgeKind::Github, None, None))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Validation);
    let err = h
        .accounts
        .test(request(ForgeKind::Github, None, None))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Validation);
    assert_eq!(h.store.text("github").as_deref(), Some("github_pat_old"));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_interrupted_account_save_stays_blocked_until_saved_again() {
    let (base, _) = serve(vec![
        (200, vec![], GITHUB_USER),
        (200, vec![], GITHUB_USER),
        (200, vec![], GITHUB_USER),
    ])
    .await;
    let mut h = harness(&base);
    h.accounts
        .save(request(ForgeKind::Github, Some("github_pat_one"), None))
        .await
        .unwrap();
    h.store.fail_writes(true);
    let err = h
        .accounts
        .save(request(ForgeKind::Github, Some("github_pat_two"), None))
        .await
        .unwrap_err();
    assert!(
        err.message.contains("until its token is saved again"),
        "{err:?}"
    );
    h.restart();
    let account = h
        .accounts
        .account(ForgeKind::Github)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(account.credential.pending, Some(CredentialPending::Save));
    assert_eq!(
        h.accounts.token(ForgeKind::Github).await.unwrap_err().code,
        ErrorCode::PermissionDenied
    );
    // Checking the item again is not enough: what it holds is unknown.
    let err = h
        .accounts
        .save(request(ForgeKind::Github, None, None))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Validation);
    h.store.fail_writes(false);
    h.accounts
        .save(request(ForgeKind::Github, Some("github_pat_three"), None))
        .await
        .unwrap();
    let (token, _) = h.accounts.token(ForgeKind::Github).await.unwrap();
    assert_eq!(token.expose(), "github_pat_three");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_401_from_any_request_forgets_the_token_it_used() {
    let (base, _) = serve(vec![(200, vec![], GITHUB_USER), (401, vec![], "")]).await;
    let h = harness(&base);
    h.accounts
        .save(request(ForgeKind::Github, Some("github_pat_abc"), None))
        .await
        .unwrap();
    let (token, credential) = h.accounts.token(ForgeKind::Github).await.unwrap();
    let account = h
        .accounts
        .account(ForgeKind::Github)
        .await
        .unwrap()
        .unwrap();
    let session = Session {
        token,
        email: None,
        user_id: account.user_id.clone(),
        account,
        credential: Some(credential.clone()),
    };
    let client = Client::new(
        ForgeKind::Github,
        base.clone(),
        Http::insecure_for_tests().unwrap(),
        Arc::new(Budget::default()),
    );
    let response = client
        .get(&session, &format!("{base}/repos/acme/api"), &[])
        .await
        .unwrap();
    assert_eq!(response.status, 401);
    assert!(!credential.is_current());
}
