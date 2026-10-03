//! What every provider adapter offers (docs/architecture.md, Pull requests —
//! v0.3): the same reads over the neutral model, so the service and the UI
//! never see a provider's own shapes.

use std::sync::Arc;

use super::budget::Budget;
use super::http::{Auth, Http, Response};
use super::keychain::Token;
use super::{ForgeRepository, PullRequestRef};
use crate::models::{
    ActionAvailability, AppError, AppResult, AvailableActions, ChangedFile, Check, CheckState,
    ChecksSummary, ForgeAccount, ForgeKind, PullRequest, PullRequestState, ReviewState, Reviewer,
};

/// Who is asking, and with what.
pub struct Session {
    pub token: Token,
    /// Bitbucket: the account's email, for Basic authentication.
    pub email: Option<String>,
    pub account: ForgeAccount,
    /// The provider's ID of the account's user, as pull requests name people.
    pub user_id: String,
}

impl Session {
    pub fn auth(&self) -> Auth<'_> {
        match self.account.kind {
            ForgeKind::Github => Auth::Bearer(&self.token),
            ForgeKind::BitbucketCloud => Auth::Basic {
                user: self.email.as_deref().unwrap_or_default(),
                token: &self.token,
            },
        }
    }
}

/// A repository's list as read: its pull requests, or nothing when the
/// provider said nothing changed since the ETag given.
pub struct ListOutcome {
    pub pull_requests: Vec<PullRequest>,
    pub etag: Option<String>,
    pub not_modified: bool,
}

/// One provider's reads. `async fn` in a trait: each implementation returns
/// a future the caller awaits, as any other `async fn`. The lint below warns
/// that such a trait cannot promise its futures are `Send`; both adapters'
/// are, and only this crate implements the trait.
#[allow(async_fn_in_trait)]
pub trait ForgeAdapter {
    /// A repository's open pull requests, or its merged and closed ones of
    /// the last 30 days.
    async fn list(
        &self,
        session: &Session,
        repository: &ForgeRepository,
        closed: bool,
        etag: Option<&str>,
    ) -> AppResult<ListOutcome>;
    async fn get(&self, session: &Session, pr: &PullRequestRef) -> AppResult<PullRequest>;
    async fn files(&self, session: &Session, pr: &PullRequestRef) -> AppResult<Vec<ChangedFile>>;
    async fn checks(
        &self,
        session: &Session,
        pr: &PullRequestRef,
        head_sha: &str,
    ) -> AppResult<Vec<Check>>;
}

/// The HTTP client of one provider, with its request budget: every request
/// is checked against the budget first and counted after.
#[derive(Clone)]
pub struct Client {
    pub kind: ForgeKind,
    pub api: String,
    http: Http,
    budget: Arc<Budget>,
}

impl Client {
    pub fn new(kind: ForgeKind, api: String, http: Http, budget: Arc<Budget>) -> Self {
        Client {
            kind,
            api,
            http,
            budget,
        }
    }

    pub async fn get(
        &self,
        session: &Session,
        url: &str,
        headers: &[(&'static str, &str)],
    ) -> AppResult<Response> {
        self.budget.check(self.kind)?;
        let result = self
            .http
            .get(self.kind.label(), url, session.auth(), headers)
            .await;
        self.budget.observe(self.kind, &result);
        result
    }

    pub async fn post_json(
        &self,
        session: &Session,
        url: &str,
        headers: &[(&'static str, &str)],
        body: &serde_json::Value,
    ) -> AppResult<Response> {
        self.budget.check(self.kind)?;
        let result = self
            .http
            .post_json(self.kind.label(), url, session.auth(), headers, body)
            .await;
        self.budget.observe(self.kind, &result);
        result
    }
}

/// An answer to a read that is not what was asked for.
pub fn read_error(kind: ForgeKind, what: &str, response: &Response) -> AppError {
    let provider = kind.label();
    match response.status {
        401 => AppError::new(
            crate::models::ErrorCode::PermissionDenied,
            format!("{provider} no longer accepts the account's token. Replace it in Settings → Accounts."),
        ),
        403 | 404 => AppError::new(
            crate::models::ErrorCode::PermissionDenied,
            format!("{provider} does not let this account read {what}: the repository may be gone, or the token may lack access to it."),
        )
        .with_details(String::from_utf8_lossy(&response.body[..response.body.len().min(500)]).into_owned()),
        _ => super::http::unexpected(provider, response),
    }
}

/// Bitbucket's `2026-10-03T12:00:00.123456+00:00` and GitHub's
/// `2026-10-03T12:00:00Z` as the one form the rest of Brainiac uses.
pub fn normalize_time(text: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(text)
        .map(|t| {
            t.with_timezone(&chrono::Utc)
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
        })
        .unwrap_or_else(|_| text.to_string())
}

/// The checks of a head commit in one line (`ChecksSummary`).
pub fn summarize_checks(checks: &[Check]) -> ChecksSummary {
    let count = |s: CheckState| checks.iter().filter(|c| c.state == s).count() as u32;
    let failed = count(CheckState::Failure);
    let pending = count(CheckState::Pending);
    let passed = count(CheckState::Success);
    let state = if checks.is_empty() {
        None
    } else if failed > 0 {
        Some(CheckState::Failure)
    } else if pending > 0 {
        Some(CheckState::Pending)
    } else if passed > 0 {
        Some(CheckState::Success)
    } else {
        Some(CheckState::Neutral)
    };
    ChecksSummary {
        state,
        total: checks.len() as u32,
        passed,
        failed,
        pending,
    }
}

/// What the account may do with a pull request, from what both providers
/// have in common; an adapter refines it with what it knows.
pub fn base_actions(
    session: &Session,
    state: PullRequestState,
    mine: bool,
    reviewers: &[Reviewer],
) -> AvailableActions {
    let provider = session.account.kind.label();
    if session.account.read_only {
        let missing = if session.account.missing.is_empty() {
            String::new()
        } else {
            format!(" Add {} to the token.", session.account.missing.join(", "))
        };
        let not = ActionAvailability::not(format!("The {provider} account is read-only.{missing}"));
        return AvailableActions {
            comment: not.clone(),
            review: not.clone(),
            approve: not.clone(),
            merge: not,
        };
    }
    let open = matches!(state, PullRequestState::Open | PullRequestState::Draft);
    let closed = || {
        ActionAvailability::not(match state {
            PullRequestState::Merged => "The pull request is merged.",
            _ => "The pull request is closed.",
        })
    };
    let approve = if !open {
        closed()
    } else if mine && session.account.kind == ForgeKind::Github {
        ActionAvailability::not("GitHub does not let you approve your own pull request.")
    } else if reviewers
        .iter()
        .any(|r| r.is_me && r.state == ReviewState::Approved)
    {
        ActionAvailability::not("You already approved it.")
    } else {
        ActionAvailability::allowed()
    };
    let merge = if !open {
        closed()
    } else if state == PullRequestState::Draft {
        ActionAvailability::not("A draft cannot be merged.")
    } else {
        ActionAvailability::allowed()
    };
    AvailableActions {
        comment: ActionAvailability::allowed(),
        review: if open {
            ActionAvailability::allowed()
        } else {
            closed()
        },
        approve,
        merge,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_summarize_to_the_worst_state_and_none_without_checks() {
        let check = |state| Check {
            name: "ci".into(),
            state,
            description: None,
            url: None,
        };
        assert_eq!(summarize_checks(&[]).state, None);
        assert_eq!(
            summarize_checks(&[check(CheckState::Success), check(CheckState::Pending)]).state,
            Some(CheckState::Pending)
        );
        assert_eq!(
            summarize_checks(&[check(CheckState::Pending), check(CheckState::Failure)]).state,
            Some(CheckState::Failure)
        );
        assert_eq!(
            summarize_checks(&[check(CheckState::Neutral)]).state,
            Some(CheckState::Neutral)
        );
    }

    #[test]
    fn times_are_normalized_to_utc_milliseconds() {
        assert_eq!(
            normalize_time("2026-10-03T12:00:00.123456+02:00"),
            "2026-10-03T10:00:00.123Z"
        );
        assert_eq!(
            normalize_time("2026-10-03T12:00:00Z"),
            "2026-10-03T12:00:00.000Z"
        );
        assert_eq!(normalize_time("soon"), "soon");
    }
}
