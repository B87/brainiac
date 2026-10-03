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
