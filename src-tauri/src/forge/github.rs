//! GitHub (github.com), REST and, for review threads, GraphQL
//! (docs/architecture.md, Pull requests — v0.3).

use chrono::{DateTime, NaiveDateTime, Utc};
use serde::Deserialize;

use super::accounts::AccountCheck;
use super::http::{unexpected, Auth, Http, Response};
use super::keychain::Token;
use crate::models::{AppError, AppResult, ErrorCode, ForgeTokenKind};

pub const API: &str = "https://api.github.com";
const PROVIDER: &str = "GitHub";
const API_VERSION: (&str, &str) = ("X-GitHub-Api-Version", "2022-11-28");
const ACCEPT: (&str, &str) = ("Accept", "application/vnd.github+json");

/// The scope a GitHub personal access token (classic) needs for private
/// repositories' pull requests, reviews, and merges.
const CLASSIC_SCOPE: &str = "repo";

#[derive(Deserialize)]
struct User {
    login: String,
    id: u64,
    name: Option<String>,
}

/// Check a token with `GET /user`: who it belongs to and when it expires.
/// GitHub does not reveal a fine-grained token's permissions, so a missing
/// one is found when GitHub first refuses an action (SPEC.md, Accounts).
pub async fn check_account(http: &Http, api: &str, token: &Token) -> AppResult<AccountCheck> {
    let response = http
        .get(
            PROVIDER,
            &format!("{api}/user"),
            Auth::Bearer(token),
            &[API_VERSION, ACCEPT],
        )
        .await?;
    interpret_user(&response, token)
}

fn interpret_user(response: &Response, token: &Token) -> AppResult<AccountCheck> {
    match response.status {
        200 => {}
        401 => {
            return Err(AppError::new(
                ErrorCode::PermissionDenied,
                "GitHub did not accept this token. It may have expired, been revoked, or been copied only in part.",
            ))
        }
        403 if response.header("x-ratelimit-remaining") == Some("0") => {
            return Err(AppError::dependency(
                "GitHub has no requests left for this token right now. Try again later.",
            ))
        }
        403 => {
            return Err(AppError::new(
                ErrorCode::PermissionDenied,
                "GitHub refused to say who this token belongs to.",
            )
            .with_details(String::from_utf8_lossy(&response.body).into_owned()))
        }
        _ => return Err(unexpected(PROVIDER, response)),
    }
    let user: User = response.json(PROVIDER)?;
    let token_kind = token_kind(token);
    // Classic tokens list their scopes; fine-grained ones send an empty header.
    let scopes = (token_kind == ForgeTokenKind::Classic).then(|| {
        response
            .header("x-oauth-scopes")
            .map(split_list)
            .unwrap_or_default()
    });
    let missing = match &scopes {
        Some(scopes) if !scopes.iter().any(|s| s == CLASSIC_SCOPE) => {
            vec![CLASSIC_SCOPE.to_string()]
        }
        _ => Vec::new(),
    };
    Ok(AccountCheck {
        login: user.login,
        user_id: user.id.to_string(),
        display_name: user.name.filter(|n| !n.trim().is_empty()),
        token_kind,
        expires_at: response
            .header("github-authentication-token-expiration")
            .and_then(parse_expiry),
        scopes,
        missing,
    })
}

/// GitHub's token prefixes say what kind of token it is.
fn token_kind(token: &Token) -> ForgeTokenKind {
    let t = token.expose();
    if t.starts_with("github_pat_") {
        ForgeTokenKind::FineGrained
    } else if t.starts_with("ghp_") {
        ForgeTokenKind::Classic
    } else {
        ForgeTokenKind::Other
    }
}

/// `2027-10-02 22:00:00 UTC` (or with a numeric offset) as an RFC 3339 instant.
fn parse_expiry(text: &str) -> Option<String> {
    let text = text.trim();
    let instant = match text.strip_suffix(" UTC") {
        Some(naive) => NaiveDateTime::parse_from_str(naive, "%Y-%m-%d %H:%M:%S")
            .ok()?
            .and_utc(),
        None => DateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S %z")
            .ok()?
            .with_timezone(&Utc),
    };
    Some(instant.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
}

/// A comma-separated header value as a list.
pub(crate) fn split_list(text: &str) -> Vec<String> {
    text.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_expiry_is_read_in_both_forms() {
        assert_eq!(
            parse_expiry("2027-10-02 22:00:00 UTC").as_deref(),
            Some("2027-10-02T22:00:00.000Z")
        );
        assert_eq!(
            parse_expiry("2027-10-02 22:00:00 +0200").as_deref(),
            Some("2027-10-02T20:00:00.000Z")
        );
        assert_eq!(parse_expiry("never"), None);
    }

    #[test]
    fn token_kinds_come_from_their_prefix() {
        let kind = |t: &str| token_kind(&Token::new(t).unwrap());
        assert_eq!(kind("github_pat_11AB"), ForgeTokenKind::FineGrained);
        assert_eq!(kind("ghp_abc"), ForgeTokenKind::Classic);
        assert_eq!(kind("gho_abc"), ForgeTokenKind::Other);
    }
}
