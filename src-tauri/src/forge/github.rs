//! GitHub (github.com), REST and, for review threads, GraphQL
//! (docs/architecture.md, Pull requests — v0.3).

use chrono::{DateTime, NaiveDateTime, Utc};
use serde::Deserialize;

use super::accounts::AccountCheck;
use super::adapter::{
    base_actions, edited, normalize_time, own_branch, read_error, sort_threads, summarize_checks,
    write_error, Client, ForgeAdapter, ListOutcome, ReviewToSend, SentPart, Session,
    GITHUB_CONTENTS_WRITE, GITHUB_PULL_REQUESTS_WRITE,
};
use super::http::{unexpected, Auth, Http, Response};
use super::markdown;
use super::PullRequestRef;
use crate::credentials::Token;
use crate::hosting::ForgeRepository;
use crate::models::{
    ActionAvailability, AppError, AppResult, ChangedFile, ChangedFileStatus, Check, CheckState,
    Comment, DiffSide, ErrorCode, ForgeKind, ForgeTokenKind, ForgeUser, MergeMethod, MergeOptions,
    MergeRequest, Mergeability, PullRequest, PullRequestCounts, PullRequestState, ReviewState,
    ReviewVerdict, Reviewer, Thread, ThreadAnchor,
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
                ErrorCode::Unauthenticated,
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
            let forbidden = |e: &GraphqlError| e.kind.as_deref() == Some("FORBIDDEN");
            let code = if errors.iter().any(|e| not_found(e) && on_pull_request(e)) {
                ErrorCode::NotFound
            } else if errors.iter().any(|e| not_found(e) || forbidden(e)) {
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
  headRefName headRefOid headRepository { nameWithOwner } baseRefName baseRefOid
  mergeable additions deletions changedFiles
  commits { totalCount }
  comments { totalCount }
  reviewThreads(first: 100) { nodes { isResolved } }
  reviewRequests(first: 50) {
    nodes { requestedReviewer { __typename ... on User { login databaseId name } ... on Team { name slug } } }
  }
  latestReviews(first: 50) { nodes { state commit { oid } author { login ... on User { databaseId name } } } }
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
    #[serde(default)]
    base_ref_oid: String,
    mergeable: Option<String>,
    additions: u32,
    deletions: u32,
    changed_files: u32,
    commits: Count,
    comments: Count,
    review_threads: Nodes<ThreadCount>,
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
struct ThreadCount {
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
    commit: Option<Oid>,
}

#[derive(Deserialize)]
struct Oid {
    oid: String,
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
    // The commit the account's user last reviewed: their latest review's.
    let reviewed_sha = node
        .latest_reviews
        .nodes
        .iter()
        .flatten()
        .filter(|r| r.state != "PENDING")
        .find(|r| {
            r.author
                .as_ref()
                .is_some_and(|a| user(a).id == session.user_id)
        })
        .and_then(|r| r.commit.as_ref().map(|c| c.oid.clone()));
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
        description_html: markdown::render(node.body.as_deref().unwrap_or_default()),
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
        base_sha: node.base_ref_oid.clone(),
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
        reviewed_sha,
        commits_since_review: None,
    }
}

// --- The conversation ---------------------------------------------------------

/// Comments on the pull request, reviews, and review threads with their
/// comments. The first page carries the comments and reviews; later pages of
/// threads leave them out (`$top`).
const CONVERSATION_QUERY: &str = r#"
query($owner: String!, $name: String!, $number: Int!, $after: String, $top: Boolean!) {
  repository(owner: $owner, name: $name) {
    pullRequest(number: $number) {
      comments(first: 100) @include(if: $top) {
        nodes { databaseId body createdAt updatedAt url author { login ... on User { databaseId name } } }
      }
      reviews(first: 100) @include(if: $top) {
        nodes { databaseId state body submittedAt url commit { oid } author { login ... on User { databaseId name } } }
      }
      reviewThreads(first: 50, after: $after) {
        pageInfo { hasNextPage endCursor }
        nodes {
          id isResolved isOutdated path line startLine diffSide originalLine originalStartLine
          comments(first: 100) {
            nodes { databaseId body createdAt updatedAt url commit { oid } originalCommit { oid } author { login ... on User { databaseId name } } }
          }
        }
      }
    }
  }
}
"#;

#[derive(Deserialize)]
struct ConversationData {
    repository: Option<ConversationRepository>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConversationRepository {
    pull_request: Option<ConversationPr>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConversationPr {
    comments: Option<Nodes<CommentNode>>,
    reviews: Option<Nodes<ReviewNode>>,
    review_threads: ThreadConnection,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThreadConnection {
    page_info: PageInfo,
    nodes: Vec<Option<ThreadNode>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThreadNode {
    id: String,
    is_resolved: bool,
    is_outdated: bool,
    path: String,
    line: Option<u32>,
    start_line: Option<u32>,
    diff_side: Option<String>,
    original_line: Option<u32>,
    original_start_line: Option<u32>,
    comments: Nodes<CommentNode>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CommentNode {
    database_id: Option<u64>,
    body: String,
    created_at: String,
    updated_at: Option<String>,
    url: Option<String>,
    author: Option<Actor>,
    commit: Option<Oid>,
    original_commit: Option<Oid>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReviewNode {
    database_id: Option<u64>,
    state: String,
    body: String,
    submitted_at: Option<String>,
    url: Option<String>,
    author: Option<Actor>,
}

fn ghost() -> ForgeUser {
    ForgeUser {
        id: String::new(),
        login: "ghost".into(),
        display_name: None,
    }
}

fn comment_of(node: &CommentNode, me: &str) -> Comment {
    let author = node.author.as_ref().map(user).unwrap_or_else(ghost);
    Comment {
        id: node
            .database_id
            .map(|id| id.to_string())
            .unwrap_or_default(),
        mine: author.id == me,
        author,
        html: markdown::render(&node.body),
        body: node.body.clone(),
        review: None,
        created_at: normalize_time(&node.created_at),
        updated_at: node
            .updated_at
            .as_deref()
            .map(normalize_time)
            .filter(|u| edited(&node.created_at, u)),
        web_url: node.url.clone(),
    }
}

/// A review's summary as a thread of one comment, when it says something:
/// a verdict, or words.
fn review_thread(node: &ReviewNode, me: &str) -> Option<Thread> {
    let review = match node.state.as_str() {
        "APPROVED" => Some(ReviewState::Approved),
        "CHANGES_REQUESTED" => Some(ReviewState::ChangesRequested),
        "COMMENTED" | "DISMISSED" => None,
        // PENDING is an unsubmitted draft on GitHub's site.
        _ => return None,
    };
    if review.is_none() && node.body.trim().is_empty() {
        return None;
    }
    let author = node.author.as_ref().map(user).unwrap_or_else(ghost);
    let id = node
        .database_id
        .map(|id| id.to_string())
        .unwrap_or_default();
    Some(Thread {
        id: format!("review:{id}"),
        anchor: None,
        resolved: false,
        outdated: false,
        comments: vec![Comment {
            id,
            mine: author.id == me,
            author,
            html: markdown::render(&node.body),
            body: node.body.clone(),
            review,
            created_at: node
                .submitted_at
                .as_deref()
                .map(normalize_time)
                .unwrap_or_default(),
            updated_at: None,
            web_url: node.url.clone(),
        }],
    })
}

fn review_thread_of(node: &ThreadNode, me: &str) -> Thread {
    let comments: Vec<Comment> = node
        .comments
        .nodes
        .iter()
        .flatten()
        .map(|c| comment_of(c, me))
        .collect();
    let commit = node
        .comments
        .nodes
        .iter()
        .flatten()
        .next()
        .and_then(|c| c.commit.as_ref().or(c.original_commit.as_ref()))
        .map(|c| c.oid.clone());
    Thread {
        id: node.id.clone(),
        anchor: Some(ThreadAnchor {
            path: node.path.clone(),
            side: if node.diff_side.as_deref() == Some("LEFT") {
                DiffSide::Old
            } else {
                DiffSide::New
            },
            line: node.line.or(node.original_line),
            // A one-line thread names its line as its start too.
            start_line: node
                .start_line
                .or(node.original_start_line)
                .filter(|s| Some(*s) != node.line.or(node.original_line)),
            commit,
        }),
        resolved: node.is_resolved,
        outdated: node.is_outdated,
        comments,
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

// --- Writes (SPEC.md, Reviewing) ----------------------------------------------

/// What REST answers a created comment or review with.
#[derive(Deserialize)]
struct Created {
    id: u64,
}

const RESOLVE_MUTATION: &str = r#"
mutation($id: ID!) { resolveReviewThread(input: {threadId: $id}) { thread { isResolved } } }
"#;

/// The repository's merge settings (SPEC.md, Merging).
const MERGE_SETTINGS_QUERY: &str = r#"
query($owner: String!, $name: String!) {
  repository(owner: $owner, name: $name) {
    mergeCommitAllowed squashMergeAllowed rebaseMergeAllowed deleteBranchOnMerge
  }
}
"#;

#[derive(Deserialize)]
struct MergeSettingsData {
    repository: Option<MergeSettings>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MergeSettings {
    merge_commit_allowed: bool,
    squash_merge_allowed: bool,
    rebase_merge_allowed: bool,
    delete_branch_on_merge: bool,
}
const UNRESOLVE_MUTATION: &str = r#"
mutation($id: ID!) { unresolveReviewThread(input: {threadId: $id}) { thread { isResolved } } }
"#;

impl Github {
    fn rest(&self, pr: &PullRequestRef, tail: &str) -> String {
        format!(
            "{}/repos/{}/{}/{tail}",
            self.client.api, pr.repository.owner, pr.repository.name
        )
    }

    /// `POST` a JSON body and read the created object's ID.
    async fn create(
        &self,
        session: &Session,
        url: &str,
        body: &serde_json::Value,
        what: &str,
    ) -> AppResult<String> {
        let response = self
            .client
            .post_json(session, url, &[API_VERSION, ACCEPT], body)
            .await?;
        if !matches!(response.status, 200 | 201) {
            return Err(write_error(
                ForgeKind::Github,
                what,
                GITHUB_PULL_REQUESTS_WRITE,
                &response,
            ));
        }
        let created: Created = response.json(PROVIDER)?;
        Ok(created.id.to_string())
    }
}

/// A draft as GitHub's review API takes it: `line` and `side` on the head's
/// diff, with `start_line` for a range.
fn review_comment(draft: &crate::models::ReviewDraft) -> AppResult<serde_json::Value> {
    let a = &draft.anchor;
    let line = a
        .line
        .ok_or_else(|| AppError::validation(format!("The draft on {} names no line.", a.path)))?;
    let side = match a.side {
        DiffSide::Old => "LEFT",
        DiffSide::New => "RIGHT",
    };
    let mut comment = serde_json::json!({
        "path": a.path, "body": draft.body, "line": line, "side": side,
    });
    if let Some(start) = a.start_line.filter(|s| *s < line) {
        comment["start_line"] = serde_json::json!(start);
        comment["start_side"] = serde_json::json!(side);
    }
    Ok(comment)
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

    async fn conversation(&self, session: &Session, pr: &PullRequestRef) -> AppResult<Vec<Thread>> {
        let what = format!("the conversation of {pr}");
        let me = session.user_id.as_str();
        let mut threads = Vec::new();
        let mut after: Option<String> = None;
        for page in 0..MAX_PAGES {
            let variables = serde_json::json!({
                "owner": pr.repository.owner, "name": pr.repository.name, "number": pr.number,
                "after": after, "top": page == 0,
            });
            let data: ConversationData = self
                .query(session, CONVERSATION_QUERY, variables, &what)
                .await?;
            let node = data
                .repository
                .and_then(|r| r.pull_request)
                .ok_or_else(|| AppError::not_found(format!("GitHub has no pull request {pr}.")))?;
            for c in node.comments.iter().flat_map(|n| n.nodes.iter()).flatten() {
                let comment = comment_of(c, me);
                threads.push(Thread {
                    id: format!("comment:{}", comment.id),
                    anchor: None,
                    resolved: false,
                    outdated: false,
                    comments: vec![comment],
                });
            }
            threads.extend(
                node.reviews
                    .iter()
                    .flat_map(|n| n.nodes.iter())
                    .flatten()
                    .filter_map(|r| review_thread(r, me)),
            );
            threads.extend(
                node.review_threads
                    .nodes
                    .iter()
                    .flatten()
                    .map(|t| review_thread_of(t, me)),
            );
            if !node.review_threads.page_info.has_next_page {
                break;
            }
            after = node.review_threads.page_info.end_cursor;
        }
        sort_threads(&mut threads);
        Ok(threads)
    }

    async fn patch(&self, session: &Session, pr: &PullRequestRef) -> AppResult<String> {
        let url = format!(
            "{}/repos/{}/{}/pulls/{}",
            self.client.api, pr.repository.owner, pr.repository.name, pr.number
        );
        let response = self
            .client
            .get(
                session,
                &url,
                &[API_VERSION, ("Accept", "application/vnd.github.diff")],
            )
            .await?;
        match response.status {
            200 => Ok(String::from_utf8_lossy(&response.body).into_owned()),
            // GitHub serves no diff past 300 files or 20,000 lines.
            406 => Err(AppError::new(
                ErrorCode::Validation,
                format!("GitHub does not serve the diff of {pr}: it is too large. Fetch the repository to see it from local Git."),
            )),
            _ => Err(read_error(
                ForgeKind::Github,
                &format!("the diff of {pr}"),
                &response,
            )),
        }
    }

    async fn comment(
        &self,
        session: &Session,
        pr: &PullRequestRef,
        body: &str,
    ) -> AppResult<String> {
        // A pull request is an issue to the comments API.
        let url = self.rest(pr, &format!("issues/{}/comments", pr.number));
        self.create(
            session,
            &url,
            &serde_json::json!({ "body": body }),
            "post the comment",
        )
        .await
    }

    async fn reply(
        &self,
        session: &Session,
        pr: &PullRequestRef,
        thread: &Thread,
        body: &str,
    ) -> AppResult<String> {
        // Comments on the pull request and review summaries have no replies
        // on GitHub: a reply to one is another comment on the pull request.
        if thread.anchor.is_none() {
            return self.comment(session, pr, body).await;
        }
        let first = thread
            .comments
            .first()
            .map(|c| c.id.as_str())
            .filter(|id| !id.is_empty())
            .ok_or_else(|| AppError::validation("The thread has no comment to reply to."))?;
        let url = self.rest(pr, &format!("pulls/{}/comments/{first}/replies", pr.number));
        self.create(
            session,
            &url,
            &serde_json::json!({ "body": body }),
            "post the reply",
        )
        .await
    }

    async fn resolve(
        &self,
        session: &Session,
        pr: &PullRequestRef,
        thread: &Thread,
        resolved: bool,
    ) -> AppResult<()> {
        if thread.anchor.is_none() {
            return Err(AppError::validation(
                "Only a thread on a line can be resolved.",
            ));
        }
        let what = format!(
            "{} the thread on {pr}",
            if resolved { "resolve" } else { "reopen" }
        );
        let mutation = if resolved {
            RESOLVE_MUTATION
        } else {
            UNRESOLVE_MUTATION
        };
        let _: serde_json::Value = self
            .query(
                session,
                mutation,
                serde_json::json!({ "id": thread.id }),
                &what,
            )
            .await?;
        Ok(())
    }

    async fn current_head(&self, session: &Session, pr: &PullRequest) -> AppResult<String> {
        let reference: PullRequestRef = pr.reference.parse()?;
        Ok(self.get(session, &reference).await?.head_sha)
    }

    /// One request with the summary, the verdict, and every draft, on the
    /// head commit; GitHub accepts a review on an older commit, so the
    /// service compared the head first.
    async fn submit_review(
        &self,
        session: &Session,
        pr: &PullRequest,
        review: &ReviewToSend<'_>,
        _progress: &mut (dyn FnMut(SentPart) + Send),
    ) -> AppResult<()> {
        let reference: PullRequestRef = pr.reference.parse()?;
        let comments = review
            .drafts
            .iter()
            .map(review_comment)
            .collect::<AppResult<Vec<_>>>()?;
        let event = match review.verdict {
            ReviewVerdict::Comment => "COMMENT",
            ReviewVerdict::Approve => "APPROVE",
            ReviewVerdict::RequestChanges => "REQUEST_CHANGES",
        };
        let body = serde_json::json!({
            "commit_id": review.head_sha, "body": review.body, "event": event, "comments": comments,
        });
        let url = self.rest(&reference, &format!("pulls/{}/reviews", reference.number));
        self.create(session, &url, &body, "submit the review")
            .await?;
        Ok(())
    }

    async fn merge_options(&self, session: &Session, pr: &PullRequest) -> AppResult<MergeOptions> {
        let reference: PullRequestRef = pr.reference.parse()?;
        let data: MergeSettingsData = self
            .query(
                session,
                MERGE_SETTINGS_QUERY,
                serde_json::json!({ "owner": reference.repository.owner, "name": reference.repository.name }),
                "read the repository's merge settings",
            )
            .await?;
        let settings = data.repository.ok_or_else(|| {
            AppError::not_found(format!(
                "GitHub has no repository {} this account can see.",
                reference.repository
            ))
        })?;
        let methods: Vec<MergeMethod> = [
            (settings.merge_commit_allowed, MergeMethod::MergeCommit),
            (settings.squash_merge_allowed, MergeMethod::Squash),
            (settings.rebase_merge_allowed, MergeMethod::Rebase),
        ]
        .into_iter()
        .filter_map(|(allowed, method)| allowed.then_some(method))
        .collect();
        let default_method = *methods.first().ok_or_else(|| {
            AppError::validation("The repository allows no merge method; change that on GitHub.")
        })?;
        // A fork's branch belongs to someone else's repository.
        let can_delete_branch = own_branch(pr, &reference.repository);
        Ok(MergeOptions {
            reference: pr.reference.clone(),
            methods,
            default_method,
            can_delete_branch,
            delete_branch: can_delete_branch && settings.delete_branch_on_merge,
            deletes_branch_itself: can_delete_branch && settings.delete_branch_on_merge,
        })
    }

    /// `PUT /merge` with the full head commit, which GitHub enforces with
    /// `409` when the branch moved. The branch is deleted afterwards when
    /// asked, which GitHub may have done itself already.
    async fn merge(
        &self,
        session: &Session,
        pr: &PullRequest,
        request: &MergeRequest,
    ) -> AppResult<Option<String>> {
        let reference: PullRequestRef = pr.reference.parse()?;
        let method = match request.method {
            MergeMethod::MergeCommit => "merge",
            MergeMethod::Squash => "squash",
            MergeMethod::Rebase => "rebase",
            MergeMethod::FastForward => {
                return Err(AppError::validation(
                    "GitHub cannot fast-forward; choose another method.",
                ))
            }
        };
        let mut body =
            serde_json::json!({ "sha": request.expected_head_sha, "merge_method": method });
        // A rebase keeps the commits' own messages; a title or message
        // given for it is refused.
        if request.method != MergeMethod::Rebase {
            if !request.commit_title.trim().is_empty() {
                body["commit_title"] = serde_json::json!(request.commit_title.trim());
            }
            if !request.commit_message.trim().is_empty() {
                body["commit_message"] = serde_json::json!(request.commit_message.trim());
            }
        }
        let url = self.rest(&reference, &format!("pulls/{}/merge", reference.number));
        let response = self
            .client
            .put_json(session, &url, &[API_VERSION, ACCEPT], &body)
            .await?;
        if response.status != 200 {
            return Err(write_error(
                ForgeKind::Github,
                "merge the pull request",
                GITHUB_CONTENTS_WRITE,
                &response,
            ));
        }
        if !request.delete_branch || !own_branch(pr, &reference.repository) {
            return Ok(None);
        }
        let url = self.rest(&reference, &format!("git/refs/heads/{}", pr.source_branch));
        let deleted = self
            .client
            .delete(session, &url, &[API_VERSION, ACCEPT])
            .await;
        Ok(match deleted {
            // 404 and 422: the branch is gone already (GitHub deleted it).
            Ok(r) if matches!(r.status, 204 | 404 | 422) => None,
            Ok(r) => Some(format!(
                "Merged, but the branch {} was not deleted: {}",
                pr.source_branch,
                write_error(
                    ForgeKind::Github,
                    "delete the branch",
                    GITHUB_CONTENTS_WRITE,
                    &r
                )
                .message
            )),
            Err(e) => Some(format!(
                "Merged, but the branch {} was not deleted: {}",
                pr.source_branch, e.message
            )),
        })
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
