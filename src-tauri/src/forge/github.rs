//! GitHub (github.com), REST and, for review threads, GraphQL
//! (docs/architecture.md, Pull requests — v0.3).

use chrono::{DateTime, NaiveDateTime, Utc};
use serde::Deserialize;

use super::accounts::AccountCheck;
use super::adapter::{
    base_actions, normalize_time, read_error, summarize_checks, Client, ForgeAdapter, ListOutcome,
    Session,
};
use super::http::{unexpected, Auth, Http, Response};
use super::keychain::Token;
use super::{ForgeRepository, PullRequestRef};
use crate::models::{
    ActionAvailability, AppError, AppResult, ChangedFile, ChangedFileStatus, Check, CheckState,
    ErrorCode, ForgeKind, ForgeTokenKind, ForgeUser, Mergeability, PullRequest, PullRequestCounts,
    PullRequestState, ReviewState, Reviewer,
};

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

// --- Reads --------------------------------------------------------------------

/// GitHub's reads: GraphQL for pull requests, which answers in one request
/// what REST spreads over five (the pull request, its reviews, its threads,
/// its checks, its files count); REST for the files themselves.
pub struct Github {
    client: Client,
}

impl Github {
    pub fn new(client: Client) -> Self {
        Github { client }
    }

    fn graphql_url(&self) -> String {
        format!("{}/graphql", self.client.api)
    }

    /// Run a query; GraphQL puts errors in a `200` body, so both are checked.
    async fn query<T: serde::de::DeserializeOwned>(
        &self,
        session: &Session,
        query: &str,
        variables: serde_json::Value,
        what: &str,
    ) -> AppResult<T> {
        let body = serde_json::json!({ "query": query, "variables": variables });
        let response = self
            .client
            .post_json(session, &self.graphql_url(), &[API_VERSION], &body)
            .await?;
        if response.status != 200 {
            return Err(read_error(ForgeKind::Github, what, &response));
        }
        let envelope: GraphqlEnvelope<T> = response.json(PROVIDER)?;
        if let Some(errors) = envelope.errors.filter(|e| !e.is_empty()) {
            let messages: Vec<&str> = errors.iter().map(|e| e.message.as_str()).collect();
            let text = messages.join("; ");
            // NOT_FOUND on the pull request is a wrong number; on the
            // repository it also covers one the token cannot see.
            let not_found = |e: &GraphqlError| e.kind.as_deref() == Some("NOT_FOUND");
            let on_pull_request = |e: &GraphqlError| {
                e.path
                    .as_ref()
                    .is_some_and(|p| p.last().and_then(|v| v.as_str()) == Some("pullRequest"))
            };
            let code = if errors.iter().any(|e| not_found(e) && on_pull_request(e)) {
                ErrorCode::NotFound
            } else if errors.iter().any(not_found) {
                ErrorCode::PermissionDenied
            } else {
                ErrorCode::DependencyUnavailable
            };
            return Err(AppError::new(
                code,
                format!("GitHub could not read {what}: {text}"),
            ));
        }
        envelope
            .data
            .ok_or_else(|| AppError::dependency(format!("GitHub sent no data for {what}.")))
    }
}

#[derive(Deserialize)]
struct GraphqlEnvelope<T> {
    data: Option<T>,
    errors: Option<Vec<GraphqlError>>,
}

#[derive(Deserialize)]
struct GraphqlError {
    message: String,
    #[serde(rename = "type")]
    kind: Option<String>,
    path: Option<Vec<serde_json::Value>>,
}

/// The fields of a pull request the neutral model needs.
const PR_FRAGMENT: &str = r#"
fragment pr on PullRequest {
  number title body isDraft state mergedAt closedAt createdAt updatedAt url
  author { login ... on User { databaseId name } }
  headRefName headRefOid headRepository { nameWithOwner } baseRefName
  mergeable additions deletions changedFiles
  commits { totalCount }
  comments { totalCount }
  reviewThreads(first: 100) { nodes { isResolved } }
  reviewRequests(first: 50) {
    nodes { requestedReviewer { __typename ... on User { login databaseId name } ... on Team { name slug } } }
  }
  latestReviews(first: 50) { nodes { state author { login ... on User { databaseId name } } } }
  lastCommit: commits(last: 1) {
    nodes { commit { statusCheckRollup { contexts(first: 100) { nodes {
      __typename
      ... on CheckRun { name status conclusion detailsUrl title }
      ... on StatusContext { context state targetUrl description }
    } } } } }
  }
}
"#;

#[derive(Deserialize)]
struct ListData {
    repository: Option<ListRepository>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListRepository {
    viewer_permission: Option<String>,
    pull_requests: Connection,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Connection {
    page_info: PageInfo,
    nodes: Vec<PrNode>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageInfo {
    has_next_page: bool,
    end_cursor: Option<String>,
}

#[derive(Deserialize)]
struct GetData {
    repository: Option<GetRepository>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetRepository {
    viewer_permission: Option<String>,
    pull_request: Option<PrNode>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrNode {
    number: u64,
    title: String,
    body: Option<String>,
    is_draft: bool,
    state: String,
    merged_at: Option<String>,
    closed_at: Option<String>,
    created_at: String,
    updated_at: String,
    url: String,
    author: Option<Actor>,
    head_ref_name: String,
    head_ref_oid: String,
    head_repository: Option<NameWithOwner>,
    base_ref_name: String,
    mergeable: Option<String>,
    additions: u32,
    deletions: u32,
    changed_files: u32,
    commits: Count,
    comments: Count,
    review_threads: Nodes<Thread>,
    review_requests: Nodes<ReviewRequest>,
    latest_reviews: Nodes<Review>,
    last_commit: Nodes<CommitNode>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Actor {
    login: String,
    database_id: Option<u64>,
    name: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NameWithOwner {
    name_with_owner: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Count {
    total_count: u32,
}

#[derive(Deserialize)]
struct Nodes<T> {
    nodes: Vec<Option<T>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Thread {
    is_resolved: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReviewRequest {
    requested_reviewer: Option<RequestedReviewer>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RequestedReviewer {
    #[serde(rename = "__typename")]
    typename: String,
    login: Option<String>,
    database_id: Option<u64>,
    name: Option<String>,
    slug: Option<String>,
}

#[derive(Deserialize)]
struct Review {
    state: String,
    author: Option<Actor>,
}

#[derive(Deserialize)]
struct CommitNode {
    commit: CommitChecks,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CommitChecks {
    status_check_rollup: Option<Rollup>,
}

#[derive(Deserialize)]
struct Rollup {
    contexts: Nodes<Context>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Context {
    #[serde(rename = "__typename")]
    typename: String,
    // CheckRun
    name: Option<String>,
    status: Option<String>,
    conclusion: Option<String>,
    details_url: Option<String>,
    title: Option<String>,
    // StatusContext
    context: Option<String>,
    state: Option<String>,
    target_url: Option<String>,
    description: Option<String>,
}

fn user(actor: &Actor) -> ForgeUser {
    ForgeUser {
        id: actor
            .database_id
            .map(|id| id.to_string())
            .unwrap_or_else(|| actor.login.clone()),
        login: actor.login.clone(),
        display_name: actor.name.clone().filter(|n| !n.trim().is_empty()),
    }
}

/// A check run's or a status's state in the neutral model.
fn check_state(status: Option<&str>, conclusion: Option<&str>, state: Option<&str>) -> CheckState {
    if let Some(state) = state {
        return match state {
            "SUCCESS" => CheckState::Success,
            "FAILURE" | "ERROR" => CheckState::Failure,
            _ => CheckState::Pending,
        };
    }
    if status != Some("COMPLETED") {
        return CheckState::Pending;
    }
    match conclusion {
        Some("SUCCESS") => CheckState::Success,
        Some("FAILURE") | Some("TIMED_OUT") | Some("ACTION_REQUIRED") | Some("STARTUP_FAILURE") => {
            CheckState::Failure
        }
        _ => CheckState::Neutral,
    }
}

fn checks_of(node: &PrNode) -> Vec<Check> {
    node.last_commit
        .nodes
        .iter()
        .flatten()
        .filter_map(|c| c.commit.status_check_rollup.as_ref())
        .flat_map(|r| r.contexts.nodes.iter().flatten())
        .map(|c| {
            if c.typename == "CheckRun" {
                Check {
                    name: c.name.clone().unwrap_or_default(),
                    state: check_state(c.status.as_deref(), c.conclusion.as_deref(), None),
                    description: c.title.clone(),
                    url: c.details_url.clone(),
                }
            } else {
                Check {
                    name: c.context.clone().unwrap_or_default(),
                    state: check_state(None, None, c.state.as_deref()),
                    description: c.description.clone(),
                    url: c.target_url.clone(),
                }
            }
        })
        .collect()
}

/// Reviewers: those asked and not yet answered, then the latest review of
/// everyone who gave one.
fn reviewers_of(node: &PrNode, me: &str) -> Vec<Reviewer> {
    let mut out: Vec<Reviewer> = node
        .review_requests
        .nodes
        .iter()
        .flatten()
        .filter_map(|r| r.requested_reviewer.as_ref())
        .map(|r| {
            let user = if r.typename == "Team" {
                ForgeUser {
                    id: format!("team:{}", r.slug.clone().unwrap_or_default()),
                    login: r.slug.clone().unwrap_or_default(),
                    display_name: r.name.clone(),
                }
            } else {
                ForgeUser {
                    id: r
                        .database_id
                        .map(|id| id.to_string())
                        .unwrap_or_else(|| r.login.clone().unwrap_or_default()),
                    login: r.login.clone().unwrap_or_default(),
                    display_name: r.name.clone(),
                }
            };
            Reviewer {
                is_me: user.id == me,
                user,
                state: ReviewState::Requested,
            }
        })
        .collect();
    for review in node.latest_reviews.nodes.iter().flatten() {
        let state = match review.state.as_str() {
            "APPROVED" => ReviewState::Approved,
            "CHANGES_REQUESTED" => ReviewState::ChangesRequested,
            "COMMENTED" => ReviewState::Commented,
            // PENDING is an unsubmitted draft; DISMISSED no longer counts.
            _ => continue,
        };
        let Some(author) = &review.author else {
            continue;
        };
        let user = user(author);
        // A re-request after a review puts the reviewer back to "requested".
        if out.iter().any(|r| r.user.id == user.id) {
            continue;
        }
        out.push(Reviewer {
            is_me: user.id == me,
            user,
            state,
        });
    }
    out
}

fn pull_request(
    session: &Session,
    repository: &ForgeRepository,
    permission: Option<&str>,
    node: &PrNode,
) -> PullRequest {
    let author = node.author.as_ref().map(user).unwrap_or_else(|| ForgeUser {
        id: String::new(),
        login: "ghost".into(),
        display_name: None,
    });
    let state = match (node.state.as_str(), node.is_draft) {
        ("MERGED", _) => PullRequestState::Merged,
        ("CLOSED", _) => PullRequestState::Closed,
        (_, true) => PullRequestState::Draft,
        _ => PullRequestState::Open,
    };
    let mine = author.id == session.user_id;
    let reviewers = reviewers_of(node, &session.user_id);
    let checks = checks_of(node);
    let mut actions = base_actions(session, state, mine, &reviewers);
    if actions.merge.allowed
        && !matches!(permission, Some("WRITE") | Some("MAINTAIN") | Some("ADMIN"))
    {
        actions.merge = ActionAvailability::not(
            "The account cannot push to this repository, so it cannot merge.",
        );
    }
    if actions.merge.allowed && node.mergeable.as_deref() == Some("CONFLICTING") {
        actions.merge = ActionAvailability::not("The pull request has conflicts.");
    }
    let unresolved = node
        .review_threads
        .nodes
        .iter()
        .flatten()
        .filter(|t| !t.is_resolved)
        .count() as u32;
    let updated_at = normalize_time(&node.updated_at);
    PullRequest {
        reference: PullRequestRef {
            repository: repository.clone(),
            number: node.number,
        }
        .to_string(),
        number: node.number,
        kind: ForgeKind::Github,
        title: node.title.clone(),
        description: node.body.clone().unwrap_or_default(),
        awaiting_my_review: reviewers
            .iter()
            .any(|r| r.is_me && r.state == ReviewState::Requested),
        mine,
        author,
        state,
        source_repository: node
            .head_repository
            .as_ref()
            .map(|r| r.name_with_owner.clone())
            .unwrap_or_else(|| format!("{}/{}", repository.owner, repository.name)),
        source_branch: node.head_ref_name.clone(),
        head_sha: node.head_ref_oid.clone(),
        target_branch: node.base_ref_name.clone(),
        reviewers,
        checks: summarize_checks(&checks),
        mergeability: match node.mergeable.as_deref() {
            Some("MERGEABLE") => Mergeability::Mergeable,
            Some("CONFLICTING") => Mergeability::Conflicting,
            _ => Mergeability::Computing,
        },
        counts: PullRequestCounts {
            comments: node.comments.total_count,
            unresolved_threads: Some(unresolved),
            additions: Some(node.additions),
            deletions: Some(node.deletions),
            changed_files: Some(node.changed_files),
            commits: Some(node.commits.total_count),
        },
        web_url: node.url.clone(),
        created_at: normalize_time(&node.created_at),
        closed_at: node
            .merged_at
            .as_deref()
            .or(node.closed_at.as_deref())
            .map(normalize_time),
        version: format!("{updated_at}:{}", node.head_ref_oid),
        updated_at,
        actions,
    }
}

const LIST_QUERY: &str = r#"
query($owner: String!, $name: String!, $states: [PullRequestState!]!, $after: String) {
  repository(owner: $owner, name: $name) {
    viewerPermission
    pullRequests(states: $states, first: 50, after: $after, orderBy: {field: UPDATED_AT, direction: DESC}) {
      pageInfo { hasNextPage endCursor }
      nodes { ...pr }
    }
  }
}
"#;

const GET_QUERY: &str = r#"
query($owner: String!, $name: String!, $number: Int!) {
  repository(owner: $owner, name: $name) {
    viewerPermission
    pullRequest(number: $number) { ...pr }
  }
}
"#;

/// Closed pull requests older than this are not listed.
const CLOSED_DAYS: i64 = 30;
/// At most this many pages of 50 per list.
const MAX_PAGES: usize = 4;

#[derive(Deserialize)]
struct RestFile {
    filename: String,
    previous_filename: Option<String>,
    status: String,
    additions: u32,
    deletions: u32,
    patch: Option<String>,
}

impl ForgeAdapter for Github {
    async fn list(
        &self,
        session: &Session,
        repository: &ForgeRepository,
        closed: bool,
        _etag: Option<&str>,
    ) -> AppResult<ListOutcome> {
        let what = format!("the pull requests of {repository}");
        let states = if closed {
            serde_json::json!(["MERGED", "CLOSED"])
        } else {
            serde_json::json!(["OPEN"])
        };
        let since = (Utc::now() - chrono::Duration::days(CLOSED_DAYS))
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let mut after: Option<String> = None;
        let mut out = Vec::new();
        for _ in 0..MAX_PAGES {
            let variables = serde_json::json!({
                "owner": repository.owner, "name": repository.name, "states": states, "after": after,
            });
            let data: ListData = self
                .query(
                    session,
                    &format!("{LIST_QUERY}{PR_FRAGMENT}"),
                    variables,
                    &what,
                )
                .await?;
            let repo = data.repository.ok_or_else(|| {
                AppError::new(
                    ErrorCode::PermissionDenied,
                    format!("GitHub has no repository {repository} this account can see."),
                )
            })?;
            let permission = repo.viewer_permission.as_deref();
            let mut done = !repo.pull_requests.page_info.has_next_page;
            for node in &repo.pull_requests.nodes {
                let pr = pull_request(session, repository, permission, node);
                if closed && pr.updated_at < since {
                    done = true;
                    break;
                }
                out.push(pr);
            }
            if done {
                break;
            }
            after = repo.pull_requests.page_info.end_cursor;
        }
        Ok(ListOutcome {
            pull_requests: out,
            etag: None,
            not_modified: false,
        })
    }

    async fn get(&self, session: &Session, pr: &PullRequestRef) -> AppResult<PullRequest> {
        let what = format!("pull request {pr}");
        let variables = serde_json::json!({
            "owner": pr.repository.owner, "name": pr.repository.name, "number": pr.number,
        });
        let data: GetData = self
            .query(
                session,
                &format!("{GET_QUERY}{PR_FRAGMENT}"),
                variables,
                &what,
            )
            .await?;
        let repo = data.repository.ok_or_else(|| {
            AppError::new(
                ErrorCode::PermissionDenied,
                format!(
                    "GitHub has no repository {} this account can see.",
                    pr.repository
                ),
            )
        })?;
        let node = repo
            .pull_request
            .ok_or_else(|| AppError::not_found(format!("GitHub has no pull request {pr}.")))?;
        Ok(pull_request(
            session,
            &pr.repository,
            repo.viewer_permission.as_deref(),
            &node,
        ))
    }

    async fn files(&self, session: &Session, pr: &PullRequestRef) -> AppResult<Vec<ChangedFile>> {
        let mut out = Vec::new();
        for page in 1..=30u32 {
            let url = format!(
                "{}/repos/{}/{}/pulls/{}/files?per_page=100&page={page}",
                self.client.api, pr.repository.owner, pr.repository.name, pr.number
            );
            let response = self
                .client
                .get(session, &url, &[API_VERSION, ACCEPT])
                .await?;
            if response.status != 200 {
                return Err(read_error(
                    ForgeKind::Github,
                    &format!("the files of {pr}"),
                    &response,
                ));
            }
            let files: Vec<RestFile> = response.json(PROVIDER)?;
            let n = files.len();
            out.extend(files.into_iter().map(|f| ChangedFile {
                status: match f.status.as_str() {
                    "added" => ChangedFileStatus::Added,
                    "removed" => ChangedFileStatus::Removed,
                    "renamed" => ChangedFileStatus::Renamed,
                    "modified" | "changed" => ChangedFileStatus::Modified,
                    _ => ChangedFileStatus::Other,
                },
                binary: f.patch.is_none() && f.additions == 0 && f.deletions == 0,
                path: f.filename,
                old_path: f.previous_filename,
                additions: f.additions,
                deletions: f.deletions,
            }));
            if n < 100 {
                break;
            }
        }
        Ok(out)
    }

    async fn checks(
        &self,
        session: &Session,
        pr: &PullRequestRef,
        _head_sha: &str,
    ) -> AppResult<Vec<Check>> {
        // The same query as `get`, which already carries the head's checks.
        let what = format!("the checks of {pr}");
        let variables = serde_json::json!({
            "owner": pr.repository.owner, "name": pr.repository.name, "number": pr.number,
        });
        let data: GetData = self
            .query(
                session,
                &format!("{GET_QUERY}{PR_FRAGMENT}"),
                variables,
                &what,
            )
            .await?;
        let node = data
            .repository
            .and_then(|r| r.pull_request)
            .ok_or_else(|| AppError::not_found(format!("GitHub has no pull request {pr}.")))?;
        Ok(checks_of(&node))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_states_follow_status_then_conclusion() {
        assert_eq!(
            check_state(Some("IN_PROGRESS"), None, None),
            CheckState::Pending
        );
        assert_eq!(
            check_state(Some("COMPLETED"), Some("SUCCESS"), None),
            CheckState::Success
        );
        assert_eq!(
            check_state(Some("COMPLETED"), Some("TIMED_OUT"), None),
            CheckState::Failure
        );
        assert_eq!(
            check_state(Some("COMPLETED"), Some("SKIPPED"), None),
            CheckState::Neutral
        );
        assert_eq!(check_state(None, None, Some("ERROR")), CheckState::Failure);
        assert_eq!(
            check_state(None, None, Some("EXPECTED")),
            CheckState::Pending
        );
    }

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
