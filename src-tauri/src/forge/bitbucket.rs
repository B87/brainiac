//! Bitbucket Cloud (bitbucket.org), REST API 2.0 (docs/architecture.md,
//! Pull requests — v0.3).

use serde::Deserialize;

use super::accounts::AccountCheck;
use super::github::split_list;
use super::http::{unexpected, Auth, Http, Response};
use super::keychain::Token;
use crate::models::{AppError, AppResult, ErrorCode, ForgeTokenKind};

pub const API: &str = "https://api.bitbucket.org/2.0";
const PROVIDER: &str = "Bitbucket";

/// Scopes without which Brainiac cannot show pull requests: who you are (to
/// know which pull requests are yours and wait on you), repositories, and
/// pull requests.
pub const READ_SCOPES: [&str; 3] = [
    "read:user:bitbucket",
    "read:repository:bitbucket",
    "read:pullrequest:bitbucket",
];
/// Commenting, reviewing, and merging.
pub const WRITE_SCOPES: [&str; 1] = ["write:pullrequest:bitbucket"];

#[derive(Deserialize)]
struct User {
    username: Option<String>,
    nickname: Option<String>,
    display_name: Option<String>,
    uuid: String,
}

/// Check an email and API token with `GET /user`, whose answer also lists
/// the token's scopes (`x-oauth-scopes`), so a missing one is named at once.
pub async fn check_account(
    http: &Http,
    api: &str,
    email: &str,
    token: &Token,
) -> AppResult<AccountCheck> {
    let response = http
        .get(
            PROVIDER,
            &format!("{api}/user"),
            Auth::Basic { user: email, token },
            &[],
        )
        .await?;
    interpret_user(&response)
}

fn interpret_user(response: &Response) -> AppResult<AccountCheck> {
    let scopes = response.header("x-oauth-scopes").map(split_list);
    let missing_reads = scopes
        .as_ref()
        .map(|s| missing(s, &READ_SCOPES))
        .unwrap_or_default();
    match response.status {
        200 => {}
        401 => {
            return Err(AppError::new(
                ErrorCode::PermissionDenied,
                "Bitbucket did not accept this email and API token. Use your Atlassian account's email, and check that the whole token was pasted: it is about 190 characters.",
            ))
        }
        403 if !missing_reads.is_empty() => return Err(cannot_read(&missing_reads)),
        403 => {
            return Err(AppError::new(
                ErrorCode::PermissionDenied,
                format!(
                    "Bitbucket refused to say who this token belongs to. The token needs the scope {}.",
                    READ_SCOPES[0]
                ),
            ))
        }
        _ => return Err(unexpected(PROVIDER, response)),
    }
    if !missing_reads.is_empty() {
        return Err(cannot_read(&missing_reads));
    }
    let user: User = response.json(PROVIDER)?;
    let login = user
        .username
        .or(user.nickname)
        .unwrap_or_else(|| user.uuid.clone());
    Ok(AccountCheck {
        login,
        user_id: user.uuid,
        display_name: user.display_name.filter(|n| !n.trim().is_empty()),
        token_kind: ForgeTokenKind::ApiToken,
        expires_at: None,
        missing: scopes
            .as_ref()
            .map(|s| missing(s, &WRITE_SCOPES))
            .unwrap_or_default(),
        scopes,
    })
}

fn cannot_read(missing: &[String]) -> AppError {
    AppError::new(
        ErrorCode::PermissionDenied,
        format!(
            "This token cannot read pull requests. Create a token that also has {}.",
            missing.join(", ")
        ),
    )
}

/// The scopes of `required` the token lacks. A write scope also counts as its
/// read scope.
fn missing(scopes: &[String], required: &[&str]) -> Vec<String> {
    required
        .iter()
        .filter(|r| {
            let write = r.replacen("read:", "write:", 1);
            !scopes.iter().any(|s| s == *r || *s == write)
        })
        .map(|r| r.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_write_scope_counts_as_its_read_scope() {
        let scopes = split_list("write:pullrequest:bitbucket, read:repository:bitbucket");
        assert_eq!(missing(&scopes, &READ_SCOPES), vec!["read:user:bitbucket"]);
        assert!(missing(&scopes, &WRITE_SCOPES).is_empty());
    }
}
