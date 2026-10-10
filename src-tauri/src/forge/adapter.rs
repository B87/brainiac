//! What every provider adapter offers (docs/architecture.md, Pull requests —
//! v0.3): the same reads over the neutral model, so the service and the UI
//! never see a provider's own shapes.

use std::sync::Arc;

use super::budget::Budget;
use super::http::{Auth, Http, Response};
use super::PullRequestRef;
use crate::credentials::{LeaseHandle, Token};
use crate::hosting::ForgeRepository;
use crate::models::{
    ActionAvailability, AppError, AppResult, AvailableActions, ChangedFile, Check, CheckState,
    ChecksSummary, ErrorCode, ForgeAccount, ForgeKind, MergeOptions, MergeRequest, PullRequest,
    PullRequestState, ReviewDraft, ReviewState, ReviewVerdict, Reviewer, Thread,
};

/// GitHub's fine-grained permissions, as its settings page names them; a
/// refusal names the one to add (SPEC.md, Accounts).
pub const GITHUB_PULL_REQUESTS_WRITE: &str = "Pull requests: Read and write";
pub const GITHUB_CONTENTS_WRITE: &str = "Contents: Read and write";
/// Bitbucket's write scope.
pub const BITBUCKET_WRITE: &str = "write:pullrequest:bitbucket";

/// Who is asking, and with what.
pub struct Session {
    pub token: Token,
    /// Bitbucket: the account's email, for Basic authentication.
    pub email: Option<String>,
    pub account: ForgeAccount,
    /// The provider's ID of the account's user, as pull requests name people.
    pub user_id: String,
    /// The lease the token came from: a provider's 401 forgets exactly it,
    /// so the next use reads the token again. `None` in the live tests.
    pub credential: Option<LeaseHandle>,
}

impl Session {
    /// Forget the token when the provider no longer accepts it.
    fn observe(&self, result: &AppResult<Response>) {
        if let (Ok(response), Some(credential)) = (result, &self.credential) {
            if response.status == 401 {
                credential.reject();
            }
        }
    }

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
    /// The threads and comments, oldest first (`sort_threads`).
    async fn conversation(&self, session: &Session, pr: &PullRequestRef) -> AppResult<Vec<Thread>>;
    /// The provider's unified diff of the whole pull request, for a head
    /// that is not on the Mac.
    async fn patch(&self, session: &Session, pr: &PullRequestRef) -> AppResult<String>;

    // --- Writes (SPEC.md, Reviewing), each on an explicit user action ---

    /// Comment on the whole pull request. Returns the provider's comment ID.
    async fn comment(
        &self,
        session: &Session,
        pr: &PullRequestRef,
        body: &str,
    ) -> AppResult<String>;
    /// Reply to a thread. Returns the provider's comment ID.
    async fn reply(
        &self,
        session: &Session,
        pr: &PullRequestRef,
        thread: &Thread,
        body: &str,
    ) -> AppResult<String>;
    /// Resolve or reopen a thread on a line.
    async fn resolve(
        &self,
        session: &Session,
        pr: &PullRequestRef,
        thread: &Thread,
        resolved: bool,
    ) -> AppResult<()>;
    /// The source branch's tip right now, in full, for the head check before
    /// a review or a merge.
    async fn current_head(&self, session: &Session, pr: &PullRequest) -> AppResult<String>;
    /// Send a review: GitHub takes it whole in one request; Bitbucket takes
    /// one request per comment, so `progress` is told each comment's ID as
    /// it is posted and a cut-off submission can resume with what is left.
    async fn submit_review(
        &self,
        session: &Session,
        pr: &PullRequest,
        review: &ReviewToSend<'_>,
        progress: &mut (dyn FnMut(SentPart) + Send),
    ) -> AppResult<()>;

    // --- Merging (SPEC.md, Merging) ---

    /// What the Merge confirmation offers: the methods the repository
    /// allows and its default for deleting the source branch.
    async fn merge_options(&self, session: &Session, pr: &PullRequest) -> AppResult<MergeOptions>;
    /// Merge, for the head the user looked at; the service compared the
    /// branch's tip first. Returns a warning when the merge went through but
    /// the branch could not be deleted afterwards.
    async fn merge(
        &self,
        session: &Session,
        pr: &PullRequest,
        request: &MergeRequest,
    ) -> AppResult<Option<String>>;
}

/// A review as the adapters send it: the drafts still to post, the summary
/// unless it was posted already, and the verdict.
pub struct ReviewToSend<'a> {
    pub drafts: &'a [ReviewDraft],
    pub body: &'a str,
    pub summary_sent: bool,
    pub verdict: ReviewVerdict,
    pub head_sha: &'a str,
}

/// One part of a review posted (Bitbucket), with the provider's comment ID.
pub enum SentPart {
    Draft { id: String, remote_id: String },
    Summary { remote_id: String },
}

/// A comment of the account's user in the conversation that matches what
/// was being posted: the body, and the file for a line comment. Finds a
/// write whose answer never came (`TIMEOUT`), so it is not posted twice.
pub fn find_posted(
    threads: &[Thread],
    path: Option<&str>,
    thread_id: Option<&str>,
    body: &str,
) -> Option<String> {
    let wanted = body.trim();
    threads
        .iter()
        .filter(|t| thread_id.is_none_or(|id| t.id == id))
        .filter(|t| path.is_none_or(|p| t.anchor.as_ref().is_some_and(|a| a.path == p)))
        .flat_map(|t| t.comments.iter())
        .filter(|c| c.mine && c.body.trim() == wanted)
        .map(|c| c.id.clone())
        .next_back()
}

/// Whether `updated` is an edit: later than `created` by more than the
/// moment a provider takes to store a comment (Bitbucket stamps both a few
/// milliseconds apart).
pub fn edited(created: &str, updated: &str) -> bool {
    let parse = |t: &str| chrono::DateTime::parse_from_rfc3339(t).ok();
    match (parse(created), parse(updated)) {
        (Some(c), Some(u)) => (u - c).num_seconds() >= 2,
        _ => created != updated,
    }
}

/// Threads by the time of their first comment.
pub fn sort_threads(threads: &mut [Thread]) {
    threads.sort_by(|a, b| {
        let first = |t: &Thread| t.comments.first().map(|c| c.created_at.clone());
        first(a).cmp(&first(b))
    });
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
        session.observe(&result);
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
        session.observe(&result);
        result
    }

    pub async fn post_form(
        &self,
        session: &Session,
        url: &str,
        fields: &[(&str, &str)],
    ) -> AppResult<Response> {
        self.budget.check(self.kind)?;
        let result = self
            .http
            .post_form(self.kind.label(), url, session.auth(), &[], fields)
            .await;
        self.budget.observe(self.kind, &result);
        session.observe(&result);
        result
    }

    pub async fn put_json(
        &self,
        session: &Session,
        url: &str,
        headers: &[(&'static str, &str)],
        body: &serde_json::Value,
    ) -> AppResult<Response> {
        self.budget.check(self.kind)?;
        let result = self
            .http
            .put_json(self.kind.label(), url, session.auth(), headers, body)
            .await;
        self.budget.observe(self.kind, &result);
        session.observe(&result);
        result
    }

    pub async fn delete(
        &self,
        session: &Session,
        url: &str,
        headers: &[(&'static str, &str)],
    ) -> AppResult<Response> {
        self.budget.check(self.kind)?;
        let result = self
            .http
            .delete(self.kind.label(), url, session.auth(), headers)
            .await;
        self.budget.observe(self.kind, &result);
        session.observe(&result);
        result
    }
}

/// An answer to a write that is not what was asked for. A refusal names the
/// permission the token needs; the service then turns that action off.
pub fn write_error(kind: ForgeKind, what: &str, permission: &str, response: &Response) -> AppError {
    let provider = kind.label();
    let excerpt =
        String::from_utf8_lossy(&response.body[..response.body.len().min(500)]).into_owned();
    match response.status {
        401 => AppError::new(
            ErrorCode::Unauthenticated,
            format!("{provider} no longer accepts the account's token. Replace it in Settings → Accounts."),
        ),
        403 => AppError::new(
            ErrorCode::PermissionDenied,
            format!("{provider} did not let this account {what}. The token needs {permission}; add it and replace the token in Settings → Accounts."),
        )
        .with_details(excerpt),
        404 => AppError::new(
            ErrorCode::NotFound,
            format!("{provider} could not {what}: the pull request, or what it was on, is gone."),
        )
        .with_details(excerpt),
        409 => AppError::new(
            ErrorCode::Conflict,
            format!("{provider} did not {what}: the pull request changed meanwhile."),
        )
        .with_details(excerpt),
        // 405: GitHub's "not mergeable" (conflicts, branch protection).
        422 | 400 | 405 => AppError::new(
            ErrorCode::Validation,
            format!("{provider} did not {what}: {}", provider_message(&response.body)),
        )
        .with_details(excerpt),
        _ => super::http::unexpected(provider, response),
    }
}

/// What the provider said about a refused write, from either provider's
/// error JSON, or the start of the body.
fn provider_message(body: &[u8]) -> String {
    let json: Option<serde_json::Value> = serde_json::from_slice(body).ok();
    let text = json.as_ref().and_then(|v| {
        // GitHub: {"message": "...", "errors": [{"message": "..."}]};
        // Bitbucket: {"error": {"message": "..."}}.
        let top = v.get("message").and_then(|m| m.as_str());
        let nested = v
            .get("errors")
            .and_then(|e| e.as_array())
            .and_then(|e| e.first())
            .and_then(|e| e.get("message").and_then(|m| m.as_str()));
        let bitbucket = v
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str());
        nested.or(top).or(bitbucket).map(str::to_string)
    });
    text.unwrap_or_else(|| {
        String::from_utf8_lossy(&body[..body.len().min(200)])
            .trim()
            .to_string()
    })
}

/// Whether a 403 is GitHub refusing a fine-grained token a permission (as
/// opposed to a repository the account cannot write to).
pub fn lacks_permission(response: &Response) -> bool {
    response.status == 403 && String::from_utf8_lossy(&response.body).contains("not accessible by")
}

/// An answer to a read that is not what was asked for.
pub fn read_error(kind: ForgeKind, what: &str, response: &Response) -> AppError {
    let provider = kind.label();
    match response.status {
        401 => AppError::new(
            crate::models::ErrorCode::Unauthenticated,
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

/// Whether the source branch is in the pull request's own repository, so it
/// can be deleted from here; a fork's branch is someone else's.
pub fn own_branch(pr: &PullRequest, repository: &ForgeRepository) -> bool {
    pr.source_repository == format!("{}/{}", repository.owner, repository.name)
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
    let missing = &session.account.missing;
    if session.account.read_only {
        let add = if missing.is_empty() {
            String::new()
        } else {
            format!(" Add {} to the token.", missing.join(", "))
        };
        let not = ActionAvailability::not(format!("The {provider} account is read-only.{add}"));
        return AvailableActions {
            comment: not.clone(),
            review: not.clone(),
            approve: not.clone(),
            merge: not.clone(),
            resolve: not,
        };
    }
    // A permission GitHub refused once turns its actions off (SPEC.md, Accounts).
    let lacks = |permission: &str| {
        missing.iter().any(|m| m == permission).then(|| {
            ActionAvailability::not(format!(
                "The token lacks {permission}. Add it and replace the token in Settings → Accounts."
            ))
        })
    };
    let no_pull_requests_write =
        lacks(GITHUB_PULL_REQUESTS_WRITE).or_else(|| lacks(BITBUCKET_WRITE));
    let no_contents_write = lacks(GITHUB_CONTENTS_WRITE).or_else(|| lacks(BITBUCKET_WRITE));
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
    let review = if open {
        ActionAvailability::allowed()
    } else {
        closed()
    };
    AvailableActions {
        comment: no_pull_requests_write
            .clone()
            .unwrap_or_else(ActionAvailability::allowed),
        review: no_pull_requests_write.clone().unwrap_or(review),
        approve: no_pull_requests_write.clone().unwrap_or(approve),
        merge: no_contents_write.clone().unwrap_or(merge),
        resolve: no_pull_requests_write
            .or(no_contents_write)
            .unwrap_or_else(ActionAvailability::allowed),
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
