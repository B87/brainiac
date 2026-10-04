//! Bitbucket Cloud (bitbucket.org), REST API 2.0 (docs/architecture.md,
//! Pull requests — v0.3).

use serde::Deserialize;

use std::collections::HashMap;

use super::accounts::AccountCheck;
use super::adapter::{
    base_actions, edited, normalize_time, own_branch, read_error, sort_threads, summarize_checks,
    write_error, Client, ForgeAdapter, ListOutcome, ReviewToSend, SentPart, Session,
    BITBUCKET_WRITE,
};
use super::github::split_list;
use super::http::{unexpected, Auth, Http, Response};
use super::markdown;
use super::{ForgeRepository, PullRequestRef};
use crate::credentials::Token;
use crate::models::{
    AppError, AppResult, ChangedFile, ChangedFileStatus, Check, CheckState, Comment, DiffSide,
    ErrorCode, ForgeKind, ForgeTokenKind, ForgeUser, MergeMethod, MergeOptions, MergeRequest,
    Mergeability, PullRequest, PullRequestCounts, PullRequestState, ReviewState, ReviewVerdict,
    Reviewer, Thread, ThreadAnchor,
};

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
                ErrorCode::Unauthenticated,
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

// --- Reads --------------------------------------------------------------------

/// Bitbucket Cloud's reads over REST 2.0. Its list says nothing about a pull
/// request's checks, size, or unresolved threads, so those are read per pull
/// request when its version changed (`detail`), which the service caches.
pub struct Bitbucket {
    client: Client,
}

/// Closed pull requests older than this are not listed.
const CLOSED_DAYS: i64 = 30;
const MAX_PAGES: usize = 4;

const LIST_FIELDS: &str = "next,values.id,values.title,values.state,values.draft,values.author,values.source,values.destination,values.reviewers,values.participants,values.comment_count,values.created_on,values.updated_on,values.closed_on,values.links.html,values.summary.raw";

#[derive(Deserialize)]
struct Page<T> {
    next: Option<String>,
    values: Vec<T>,
}

#[derive(Deserialize)]
struct Pr {
    id: u64,
    title: String,
    state: String,
    #[serde(default)]
    draft: bool,
    author: Option<Account>,
    source: Endpoint,
    destination: Endpoint,
    #[serde(default)]
    reviewers: Vec<Account>,
    #[serde(default)]
    participants: Vec<Participant>,
    #[serde(default)]
    comment_count: u32,
    created_on: String,
    updated_on: String,
    closed_on: Option<String>,
    links: Links,
    summary: Option<Raw>,
    description: Option<String>,
}

#[derive(Deserialize)]
struct Account {
    uuid: Option<String>,
    nickname: Option<String>,
    display_name: Option<String>,
}

#[derive(Deserialize)]
struct Participant {
    user: Account,
    role: String,
    #[serde(default)]
    approved: bool,
    state: Option<String>,
    participated_on: Option<String>,
}

#[derive(Deserialize)]
struct Endpoint {
    branch: Option<Named>,
    commit: Option<Hash>,
    repository: Option<FullName>,
}

#[derive(Deserialize)]
struct Named {
    name: String,
}

#[derive(Deserialize)]
struct Hash {
    hash: String,
}

#[derive(Deserialize)]
struct FullName {
    full_name: String,
}

#[derive(Deserialize)]
struct Links {
    html: Option<Href>,
}

#[derive(Deserialize)]
struct Href {
    href: String,
}

#[derive(Deserialize)]
struct Raw {
    raw: String,
}

#[derive(Deserialize)]
struct Diffstat {
    status: String,
    #[serde(default)]
    lines_added: u32,
    #[serde(default)]
    lines_removed: u32,
    old: Option<Pathed>,
    new: Option<Pathed>,
}

#[derive(Deserialize)]
struct Pathed {
    path: String,
}

#[derive(Deserialize)]
struct Status {
    key: String,
    name: Option<String>,
    state: String,
    description: Option<String>,
    url: Option<String>,
}

/// A comment as Bitbucket lists it: a root one, or a reply through `parent`.
#[derive(Deserialize)]
struct CommentValue {
    id: u64,
    parent: Option<Id>,
    content: Option<Raw>,
    inline: Option<Inline>,
    resolution: Option<serde_json::Value>,
    #[serde(default)]
    pending: bool,
    #[serde(default)]
    deleted: bool,
    user: Option<Account>,
    created_on: String,
    updated_on: Option<String>,
    links: Option<Links>,
}

#[derive(Deserialize)]
struct Id {
    id: u64,
}

/// Where an inline comment hangs: `to` is a line of the new file, `from`
/// of the old one; both for a line that is in both.
#[derive(Deserialize)]
struct Inline {
    path: String,
    from: Option<u32>,
    to: Option<u32>,
    #[serde(default)]
    outdated: bool,
    src_rev: Option<String>,
}

const COMMENT_FIELDS: &str = "next,values.id,values.parent.id,values.content.raw,values.inline,values.resolution.type,values.pending,values.deleted,values.user,values.created_on,values.updated_on,values.links.html.href";

fn comment_of(c: &CommentValue, me: &str) -> Comment {
    let author = c.user.as_ref().map(user).unwrap_or_else(|| ForgeUser {
        id: String::new(),
        login: "unknown".into(),
        display_name: None,
    });
    let body = c
        .content
        .as_ref()
        .map(|r| r.raw.clone())
        .unwrap_or_default();
    let created_at = normalize_time(&c.created_on);
    Comment {
        id: c.id.to_string(),
        mine: author.id == me,
        author,
        html: markdown::render(&body),
        body,
        review: None,
        updated_at: c
            .updated_on
            .as_deref()
            .map(normalize_time)
            .filter(|u| edited(&created_at, u)),
        created_at,
        web_url: c
            .links
            .as_ref()
            .and_then(|l| l.html.as_ref())
            .map(|h| h.href.clone()),
    }
}

/// Root comments as threads with their replies, however deep the replies
/// nest. Pending comments are drafts on Bitbucket's site and are left out.
fn threads_of(comments: &[CommentValue], me: &str) -> Vec<Thread> {
    let by_id: HashMap<u64, &CommentValue> = comments.iter().map(|c| (c.id, c)).collect();
    // A named function rather than a closure, so the borrow checker can see
    // the comment handed back lives as long as the slice it came from.
    fn root_of<'a>(by_id: &HashMap<u64, &'a CommentValue>, mut c: &'a CommentValue) -> u64 {
        let mut hops = 0;
        while let Some(parent) = c.parent.as_ref().and_then(|p| by_id.get(&p.id)) {
            c = parent;
            hops += 1;
            if hops > 100 {
                break;
            }
        }
        c.id
    }
    let mut threads: Vec<Thread> = Vec::new();
    let mut index: HashMap<u64, usize> = HashMap::new();
    for c in comments.iter().filter(|c| !c.deleted && !c.pending) {
        let root = root_of(&by_id, c);
        let at = match index.get(&root) {
            Some(&i) => i,
            None => {
                let Some(root_comment) = by_id.get(&root) else {
                    continue;
                };
                threads.push(Thread {
                    id: root.to_string(),
                    anchor: root_comment.inline.as_ref().map(|i| ThreadAnchor {
                        path: i.path.clone(),
                        side: if i.to.is_some() {
                            DiffSide::New
                        } else {
                            DiffSide::Old
                        },
                        line: i.to.or(i.from),
                        start_line: None,
                        commit: i.src_rev.clone(),
                    }),
                    resolved: root_comment.resolution.is_some(),
                    outdated: root_comment.inline.as_ref().is_some_and(|i| i.outdated),
                    comments: Vec::new(),
                });
                index.insert(root, threads.len() - 1);
                threads.len() - 1
            }
        };
        threads[at].comments.push(comment_of(c, me));
    }
    for t in &mut threads {
        t.comments.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    }
    sort_threads(&mut threads);
    threads
}

fn user(a: &Account) -> ForgeUser {
    let login = a
        .nickname
        .clone()
        .or_else(|| a.display_name.clone())
        .unwrap_or_default();
    ForgeUser {
        id: a.uuid.clone().unwrap_or_else(|| login.clone()),
        login,
        display_name: a.display_name.clone().filter(|n| !n.trim().is_empty()),
    }
}

/// Reviewers asked, with what they did; anyone else who approved or asked
/// for changes counts too, since Bitbucket lets any participant.
fn reviewers_of(pr: &Pr, me: &str) -> Vec<Reviewer> {
    let mut out: Vec<Reviewer> = Vec::new();
    for p in &pr.participants {
        let state = match p.state.as_deref() {
            Some("approved") => ReviewState::Approved,
            Some("changes_requested") => ReviewState::ChangesRequested,
            _ if p.approved => ReviewState::Approved,
            _ if p.role == "REVIEWER" && p.participated_on.is_some() => ReviewState::Commented,
            _ if p.role == "REVIEWER" => ReviewState::Requested,
            _ => continue,
        };
        let user = user(&p.user);
        out.push(Reviewer {
            is_me: user.id == me,
            user,
            state,
        });
    }
    // Reviewers absent from the participants (none seen in the spike, but allowed).
    for r in &pr.reviewers {
        let user = user(r);
        if !out.iter().any(|o| o.user.id == user.id) {
            out.push(Reviewer {
                is_me: user.id == me,
                user,
                state: ReviewState::Requested,
            });
        }
    }
    out
}

fn pull_request(session: &Session, repository: &ForgeRepository, pr: &Pr) -> PullRequest {
    let author = pr.author.as_ref().map(user).unwrap_or_else(|| ForgeUser {
        id: String::new(),
        login: "unknown".into(),
        display_name: None,
    });
    let state = match (pr.state.as_str(), pr.draft) {
        ("MERGED", _) => PullRequestState::Merged,
        ("DECLINED", _) | ("SUPERSEDED", _) => PullRequestState::Closed,
        (_, true) => PullRequestState::Draft,
        _ => PullRequestState::Open,
    };
    let mine = author.id == session.user_id;
    let reviewers = reviewers_of(pr, &session.user_id);
    let head_sha = pr
        .source
        .commit
        .as_ref()
        .map(|c| c.hash.clone())
        .unwrap_or_default();
    let updated_at = normalize_time(&pr.updated_on);
    let description = pr
        .description
        .clone()
        .or_else(|| pr.summary.as_ref().map(|s| s.raw.clone()))
        .unwrap_or_default();
    PullRequest {
        reference: PullRequestRef {
            repository: repository.clone(),
            number: pr.id,
        }
        .to_string(),
        number: pr.id,
        kind: ForgeKind::BitbucketCloud,
        title: pr.title.clone(),
        description_html: markdown::render(&description),
        description,
        awaiting_my_review: reviewers
            .iter()
            .any(|r| r.is_me && r.state == ReviewState::Requested),
        actions: base_actions(session, state, mine, &reviewers),
        mine,
        author,
        state,
        source_repository: pr
            .source
            .repository
            .as_ref()
            .map(|r| r.full_name.clone())
            .unwrap_or_else(|| format!("{}/{}", repository.owner, repository.name)),
        source_branch: pr
            .source
            .branch
            .as_ref()
            .map(|b| b.name.clone())
            .unwrap_or_default(),
        version: format!("{updated_at}:{head_sha}"),
        head_sha,
        base_sha: pr
            .destination
            .commit
            .as_ref()
            .map(|c| c.hash.clone())
            .unwrap_or_default(),
        target_branch: pr
            .destination
            .branch
            .as_ref()
            .map(|b| b.name.clone())
            .unwrap_or_default(),
        reviewers,
        // Filled in by `detail` when the version changed.
        checks: summarize_checks(&[]),
        mergeability: Mergeability::Unknown,
        counts: PullRequestCounts {
            comments: pr.comment_count,
            ..PullRequestCounts::default()
        },
        web_url: pr
            .links
            .html
            .as_ref()
            .map(|h| h.href.clone())
            .unwrap_or_default(),
        created_at: normalize_time(&pr.created_on),
        closed_at: match state {
            PullRequestState::Merged | PullRequestState::Closed => Some(
                pr.closed_on
                    .as_deref()
                    .map(normalize_time)
                    .unwrap_or_else(|| updated_at.clone()),
            ),
            _ => None,
        },
        updated_at,
        // Bitbucket does not record which commit a reviewer looked at.
        reviewed_sha: None,
        commits_since_review: None,
    }
}

fn file(d: Diffstat) -> ChangedFile {
    let status = match d.status.as_str() {
        "added" => ChangedFileStatus::Added,
        "removed" => ChangedFileStatus::Removed,
        "renamed" => ChangedFileStatus::Renamed,
        "modified" => ChangedFileStatus::Modified,
        _ => ChangedFileStatus::Other,
    };
    let new = d.new.map(|p| p.path);
    let old = d.old.map(|p| p.path);
    ChangedFile {
        path: new.clone().or_else(|| old.clone()).unwrap_or_default(),
        old_path: if status == ChangedFileStatus::Renamed {
            old
        } else {
            None
        },
        status,
        additions: d.lines_added,
        deletions: d.lines_removed,
        binary: false,
    }
}

impl Bitbucket {
    pub fn new(client: Client) -> Self {
        Bitbucket { client }
    }

    fn repo_url(&self, repository: &ForgeRepository) -> String {
        format!(
            "{}/repositories/{}/{}",
            self.client.api, repository.owner, repository.name
        )
    }

    /// Follow `next` links, up to `MAX_PAGES` pages.
    async fn pages<T: serde::de::DeserializeOwned>(
        &self,
        session: &Session,
        first: String,
        what: &str,
        mut keep_going: impl FnMut(&T) -> bool,
    ) -> AppResult<Vec<T>> {
        let mut url = Some(first);
        let mut out = Vec::new();
        for _ in 0..MAX_PAGES {
            let Some(u) = url.take() else { break };
            let response = self.client.get(session, &u, &[]).await?;
            if response.status != 200 {
                return Err(read_error(ForgeKind::BitbucketCloud, what, &response));
            }
            let page: Page<T> = response.json(PROVIDER)?;
            let mut stop = page.next.is_none();
            for value in page.values {
                if !keep_going(&value) {
                    stop = true;
                    break;
                }
                out.push(value);
            }
            if stop {
                break;
            }
            url = page.next;
        }
        Ok(out)
    }

    /// What the list leaves out: the head's statuses, the diffstat, and the
    /// conversation, whose unresolved inline threads are counted. Three
    /// requests; the files and threads are returned for the cache.
    pub async fn detail(
        &self,
        session: &Session,
        pr: &mut PullRequest,
        reference: &PullRequestRef,
    ) -> AppResult<(Vec<ChangedFile>, Vec<Thread>)> {
        let checks = self.checks(session, reference, &pr.head_sha).await?;
        pr.checks = summarize_checks(&checks);
        let files = self.files(session, reference).await?;
        pr.counts.additions = Some(files.iter().map(|f| f.additions).sum());
        pr.counts.deletions = Some(files.iter().map(|f| f.deletions).sum());
        pr.counts.changed_files = Some(files.len() as u32);
        let threads = self.conversation(session, reference).await?;
        pr.counts.unresolved_threads = Some(unresolved(&threads));
        Ok((files, threads))
    }
}

// --- Writes (SPEC.md, Reviewing) ----------------------------------------------

/// Where a new inline comment goes: `to` for a line of the new file, `from`
/// for one of the old. Bitbucket has no ranges; a range's last line is used.
fn inline_of(anchor: &ThreadAnchor) -> serde_json::Value {
    let mut inline = serde_json::json!({ "path": anchor.path });
    if let Some(line) = anchor.line {
        let key = match anchor.side {
            DiffSide::Old => "from",
            DiffSide::New => "to",
        };
        inline[key] = serde_json::json!(line);
    }
    inline
}

impl Bitbucket {
    fn pr_url(&self, pr: &PullRequestRef, tail: &str) -> String {
        format!(
            "{}/pullrequests/{}{tail}",
            self.repo_url(&pr.repository),
            pr.number
        )
    }

    /// `POST` a comment and read its ID.
    async fn post_comment(
        &self,
        session: &Session,
        pr: &PullRequestRef,
        body: serde_json::Value,
        what: &str,
    ) -> AppResult<String> {
        let url = self.pr_url(pr, "/comments");
        let response = self.client.post_json(session, &url, &[], &body).await?;
        if !matches!(response.status, 200 | 201) {
            return Err(write_error(
                ForgeKind::BitbucketCloud,
                what,
                BITBUCKET_WRITE,
                &response,
            ));
        }
        let created: Id = response.json(PROVIDER)?;
        Ok(created.id.to_string())
    }

    /// `POST` with an empty body: approve, request changes.
    async fn post_empty(&self, session: &Session, url: &str, what: &str) -> AppResult<Response> {
        let response = self
            .client
            .post_json(session, url, &[], &serde_json::json!({}))
            .await?;
        // 409: already approved, or changes already requested.
        if !matches!(response.status, 200 | 201 | 204 | 409) {
            return Err(write_error(
                ForgeKind::BitbucketCloud,
                what,
                BITBUCKET_WRITE,
                &response,
            ));
        }
        Ok(response)
    }
}

/// Bitbucket's name for a merge strategy, and back. Its `rebase_merge` and
/// `squash_fast_forward` have no counterpart on GitHub and are not offered.
fn strategy_of(name: &str) -> Option<MergeMethod> {
    match name {
        "merge_commit" => Some(MergeMethod::MergeCommit),
        "squash" => Some(MergeMethod::Squash),
        "rebase_fast_forward" => Some(MergeMethod::Rebase),
        "fast_forward" => Some(MergeMethod::FastForward),
        _ => None,
    }
}

fn strategy_name(method: MergeMethod) -> &'static str {
    match method {
        MergeMethod::MergeCommit => "merge_commit",
        MergeMethod::Squash => "squash",
        MergeMethod::Rebase => "rebase_fast_forward",
        MergeMethod::FastForward => "fast_forward",
    }
}

/// How long a merge task is followed before giving up on its answer.
const MERGE_POLLS: usize = 60;
const MERGE_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

/// Inline threads still open.
pub fn unresolved(threads: &[Thread]) -> u32 {
    threads
        .iter()
        .filter(|t| t.anchor.is_some() && !t.resolved)
        .count() as u32
}

impl ForgeAdapter for Bitbucket {
    async fn list(
        &self,
        session: &Session,
        repository: &ForgeRepository,
        closed: bool,
        _etag: Option<&str>,
    ) -> AppResult<ListOutcome> {
        let since = (chrono::Utc::now() - chrono::Duration::days(CLOSED_DAYS))
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        // A `q` replaces the `state` parameters, so the states go in it too.
        let states = if closed {
            format!(
                "q=(state=\"MERGED\" OR state=\"DECLINED\") AND updated_on>={since}&sort=-updated_on"
            )
        } else {
            "state=OPEN&sort=-updated_on".to_string()
        };
        let url = format!(
            "{}/pullrequests?{states}&pagelen=50&fields={LIST_FIELDS}",
            self.repo_url(repository)
        );
        let prs: Vec<Pr> = self
            .pages(
                session,
                url,
                &format!("the pull requests of {repository}"),
                |_| true,
            )
            .await?;
        let wanted = |state: PullRequestState| {
            closed == matches!(state, PullRequestState::Merged | PullRequestState::Closed)
        };
        Ok(ListOutcome {
            pull_requests: prs
                .iter()
                .map(|p| pull_request(session, repository, p))
                .filter(|p| wanted(p.state))
                .collect(),
            etag: None,
            not_modified: false,
        })
    }

    async fn get(&self, session: &Session, pr: &PullRequestRef) -> AppResult<PullRequest> {
        let url = format!(
            "{}/pullrequests/{}",
            self.repo_url(&pr.repository),
            pr.number
        );
        let response = self.client.get(session, &url, &[]).await?;
        match response.status {
            200 => {}
            404 => {
                return Err(AppError::not_found(format!(
                    "Bitbucket has no pull request {pr}."
                )))
            }
            _ => {
                return Err(read_error(
                    ForgeKind::BitbucketCloud,
                    &format!("pull request {pr}"),
                    &response,
                ))
            }
        }
        let p: Pr = response.json(PROVIDER)?;
        Ok(pull_request(session, &pr.repository, &p))
    }

    async fn files(&self, session: &Session, pr: &PullRequestRef) -> AppResult<Vec<ChangedFile>> {
        let url = format!(
            "{}/pullrequests/{}/diffstat?pagelen=500&fields=next,values.status,values.lines_added,values.lines_removed,values.old.path,values.new.path",
            self.repo_url(&pr.repository),
            pr.number
        );
        let stats: Vec<Diffstat> = self
            .pages(session, url, &format!("the files of {pr}"), |_| true)
            .await?;
        Ok(stats.into_iter().map(file).collect())
    }

    async fn checks(
        &self,
        session: &Session,
        pr: &PullRequestRef,
        head_sha: &str,
    ) -> AppResult<Vec<Check>> {
        if head_sha.is_empty() {
            return Ok(Vec::new());
        }
        let url = format!(
            "{}/commit/{head_sha}/statuses?pagelen=100&fields=next,values.key,values.name,values.state,values.description,values.url",
            self.repo_url(&pr.repository)
        );
        let statuses: Vec<Status> = self
            .pages(session, url, &format!("the checks of {pr}"), |_| true)
            .await?;
        Ok(statuses
            .into_iter()
            .map(|s| Check {
                name: s.name.unwrap_or(s.key),
                state: match s.state.as_str() {
                    "SUCCESSFUL" => CheckState::Success,
                    "FAILED" => CheckState::Failure,
                    "STOPPED" => CheckState::Neutral,
                    _ => CheckState::Pending,
                },
                description: s.description,
                url: s.url,
            })
            .collect())
    }

    async fn conversation(&self, session: &Session, pr: &PullRequestRef) -> AppResult<Vec<Thread>> {
        let url = format!(
            "{}/pullrequests/{}/comments?pagelen=100&q=deleted=false&fields={COMMENT_FIELDS}",
            self.repo_url(&pr.repository),
            pr.number
        );
        let comments: Vec<CommentValue> = self
            .pages(session, url, &format!("the comments of {pr}"), |_| true)
            .await?;
        Ok(threads_of(&comments, &session.user_id))
    }

    async fn patch(&self, session: &Session, pr: &PullRequestRef) -> AppResult<String> {
        // Answers with a redirect to the diff itself, on the same host.
        let url = format!(
            "{}/pullrequests/{}/diff",
            self.repo_url(&pr.repository),
            pr.number
        );
        let response = self
            .client
            .get(session, &url, &[("Accept", "text/plain")])
            .await?;
        if response.status != 200 {
            return Err(read_error(
                ForgeKind::BitbucketCloud,
                &format!("the diff of {pr}"),
                &response,
            ));
        }
        Ok(String::from_utf8_lossy(&response.body).into_owned())
    }

    async fn comment(
        &self,
        session: &Session,
        pr: &PullRequestRef,
        body: &str,
    ) -> AppResult<String> {
        self.post_comment(
            session,
            pr,
            serde_json::json!({ "content": { "raw": body } }),
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
        let root: u64 = thread
            .id
            .parse()
            .map_err(|_| AppError::validation("The thread cannot be replied to."))?;
        let mut value = serde_json::json!({
            "content": { "raw": body }, "parent": { "id": root },
        });
        // A reply to a line comment stays on that line.
        if let Some(anchor) = &thread.anchor {
            value["inline"] = inline_of(anchor);
        }
        self.post_comment(session, pr, value, "post the reply")
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
        let url = self.pr_url(pr, &format!("/comments/{}/resolve", thread.id));
        if resolved {
            self.post_empty(session, &url, "resolve the thread").await?;
        } else {
            let response = self.client.delete(session, &url, &[]).await?;
            if !matches!(response.status, 200 | 204 | 404) {
                return Err(write_error(
                    ForgeKind::BitbucketCloud,
                    "reopen the thread",
                    BITBUCKET_WRITE,
                    &response,
                ));
            }
        }
        Ok(())
    }

    /// The source branch's tip, which Bitbucket has at once while the pull
    /// request's own commit lags a push by a second or two (spike S6).
    async fn current_head(&self, session: &Session, pr: &PullRequest) -> AppResult<String> {
        let url = format!(
            "{}/repositories/{}/refs/branches/{}?fields=target.hash",
            self.client.api, pr.source_repository, pr.source_branch
        );
        let response = self.client.get(session, &url, &[]).await?;
        if response.status != 200 {
            return Err(read_error(
                ForgeKind::BitbucketCloud,
                &format!("the branch {}", pr.source_branch),
                &response,
            ));
        }
        #[derive(Deserialize)]
        struct Branch {
            target: Hash,
        }
        let branch: Branch = response.json(PROVIDER)?;
        Ok(branch.target.hash)
    }

    /// One request per draft, then the summary, then the verdict; each part
    /// is reported as it lands, so a submission cut off midway resumes with
    /// what is left and posts nothing twice.
    async fn submit_review(
        &self,
        session: &Session,
        pr: &PullRequest,
        review: &ReviewToSend<'_>,
        progress: &mut (dyn FnMut(SentPart) + Send),
    ) -> AppResult<()> {
        let reference: PullRequestRef = pr.reference.parse()?;
        for draft in review.drafts.iter().filter(|d| d.remote_id.is_none()) {
            let value = serde_json::json!({
                "content": { "raw": draft.body }, "inline": inline_of(&draft.anchor),
            });
            let remote_id = self
                .post_comment(
                    session,
                    &reference,
                    value,
                    &format!("post the comment on {}", draft.anchor.path),
                )
                .await?;
            progress(SentPart::Draft {
                id: draft.id.clone(),
                remote_id,
            });
        }
        if !review.summary_sent && !review.body.trim().is_empty() {
            let remote_id = self.comment(session, &reference, review.body).await?;
            progress(SentPart::Summary { remote_id });
        }
        match review.verdict {
            ReviewVerdict::Comment => {}
            ReviewVerdict::Approve => {
                let url = self.pr_url(&reference, "/approve");
                self.post_empty(session, &url, "approve the pull request")
                    .await?;
            }
            ReviewVerdict::RequestChanges => {
                let url = self.pr_url(&reference, "/request-changes");
                self.post_empty(session, &url, "request changes").await?;
            }
        }
        Ok(())
    }

    /// The target branch lists the strategies the repository allows and its
    /// default; the pull request carries its close-source-branch choice.
    async fn merge_options(&self, session: &Session, pr: &PullRequest) -> AppResult<MergeOptions> {
        let reference: PullRequestRef = pr.reference.parse()?;
        let url = format!(
            "{}/refs/branches/{}?fields=merge_strategies,default_merge_strategy",
            self.repo_url(&reference.repository),
            pr.target_branch
        );
        let response = self.client.get(session, &url, &[]).await?;
        if response.status != 200 {
            return Err(read_error(
                ForgeKind::BitbucketCloud,
                &format!("the branch {}", pr.target_branch),
                &response,
            ));
        }
        #[derive(Deserialize)]
        struct Branch {
            #[serde(default)]
            merge_strategies: Vec<String>,
            default_merge_strategy: Option<String>,
        }
        let branch: Branch = response.json(PROVIDER)?;
        let methods: Vec<MergeMethod> = branch
            .merge_strategies
            .iter()
            .filter_map(|s| strategy_of(s))
            .collect();
        let default_method = branch
            .default_merge_strategy
            .as_deref()
            .and_then(strategy_of)
            .filter(|m| methods.contains(m))
            .or_else(|| methods.first().copied())
            .ok_or_else(|| {
                AppError::validation(
                    "The repository allows no merge method Brainiac offers; merge it on Bitbucket.",
                )
            })?;

        let url = self.pr_url(&reference, "?fields=close_source_branch");
        let response = self.client.get(session, &url, &[]).await?;
        if response.status != 200 {
            return Err(read_error(
                ForgeKind::BitbucketCloud,
                &format!("pull request {reference}"),
                &response,
            ));
        }
        #[derive(Deserialize)]
        struct Choice {
            #[serde(default)]
            close_source_branch: bool,
        }
        let choice: Choice = response.json(PROVIDER)?;
        let can_delete_branch = own_branch(pr, &reference.repository);
        Ok(MergeOptions {
            reference: pr.reference.clone(),
            methods,
            default_method,
            can_delete_branch,
            delete_branch: can_delete_branch && choice.close_source_branch,
            deletes_branch_itself: false,
        })
    }

    /// `POST /merge?async=true`, then the task it names in `Location` is
    /// followed until it succeeds. Bitbucket takes no commit and merges the
    /// branch's tip of that moment; the service compared it just before.
    async fn merge(
        &self,
        session: &Session,
        pr: &PullRequest,
        request: &MergeRequest,
    ) -> AppResult<Option<String>> {
        let reference: PullRequestRef = pr.reference.parse()?;
        let message = match request.method {
            MergeMethod::Rebase | MergeMethod::FastForward => String::new(),
            _ => {
                let title = request.commit_title.trim();
                let text = request.commit_message.trim();
                match (title.is_empty(), text.is_empty()) {
                    (true, _) => text.to_string(),
                    (false, true) => title.to_string(),
                    (false, false) => format!("{title}\n\n{text}"),
                }
            }
        };
        let mut body = serde_json::json!({
            "type": "pullrequest",
            "close_source_branch": request.delete_branch,
            "merge_strategy": strategy_name(request.method),
        });
        if !message.is_empty() {
            body["message"] = serde_json::json!(message);
        }
        let what = "merge the pull request";
        let url = self.pr_url(&reference, "/merge?async=true");
        let response = self.client.post_json(session, &url, &[], &body).await?;
        let location = match response.status {
            200 => return Ok(None),
            202 => response.header("location").map(str::to_string),
            _ => {
                return Err(write_error(
                    ForgeKind::BitbucketCloud,
                    what,
                    BITBUCKET_WRITE,
                    &response,
                ))
            }
        };
        let Some(task) = location else {
            // Accepted without a task to follow: the service reads the pull
            // request again and reports what it finds.
            return Err(AppError::new(
                ErrorCode::Timeout,
                "Bitbucket accepted the merge but named no task to follow.",
            ));
        };
        #[derive(Deserialize)]
        struct Task {
            task_status: String,
        }
        for _ in 0..MERGE_POLLS {
            let response = self.client.get(session, &task, &[]).await?;
            if response.status != 200 {
                return Err(write_error(
                    ForgeKind::BitbucketCloud,
                    what,
                    BITBUCKET_WRITE,
                    &response,
                ));
            }
            let task: Task = response.json(PROVIDER)?;
            match task.task_status.as_str() {
                "SUCCESS" => return Ok(None),
                "PENDING" => tokio::time::sleep(MERGE_POLL_INTERVAL).await,
                other => {
                    return Err(AppError::new(
                        ErrorCode::Validation,
                        format!("Bitbucket did not {what}: the merge ended as {other}."),
                    )
                    .with_details(String::from_utf8_lossy(&response.body).into_owned()))
                }
            }
        }
        Err(AppError::new(
            ErrorCode::Timeout,
            "Bitbucket is still merging; Brainiac stopped waiting.",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replies_join_their_root_and_pending_comments_stay_out() {
        let json = r#"[
            {"id": 1, "parent": null, "content": {"raw": "root"}, "inline": {"path": "a.rs", "from": null, "to": 3}, "resolution": null, "user": {"uuid": "{u}", "nickname": "u"}, "created_on": "2026-10-01T10:00:00+00:00"},
            {"id": 3, "parent": {"id": 2}, "content": {"raw": "reply to reply"}, "inline": {"path": "a.rs", "to": 3}, "resolution": null, "user": {"uuid": "{v}", "nickname": "v"}, "created_on": "2026-10-01T12:00:00+00:00"},
            {"id": 2, "parent": {"id": 1}, "content": {"raw": "reply"}, "inline": {"path": "a.rs", "to": 3}, "resolution": null, "user": {"uuid": "{v}", "nickname": "v"}, "created_on": "2026-10-01T11:00:00+00:00"},
            {"id": 4, "parent": null, "content": {"raw": "draft"}, "inline": null, "resolution": null, "pending": true, "user": {"uuid": "{u}"}, "created_on": "2026-10-01T09:00:00+00:00"},
            {"id": 5, "parent": null, "content": {"raw": "general"}, "inline": null, "resolution": {"type": "resolution"}, "user": {"uuid": "{u}"}, "created_on": "2026-09-30T09:00:00+00:00"}
        ]"#;
        let comments: Vec<CommentValue> = serde_json::from_str(json).unwrap();
        let threads = threads_of(&comments, "{u}");
        assert_eq!(threads.len(), 2);
        assert_eq!(threads[0].id, "5");
        assert!(threads[0].resolved && threads[0].anchor.is_none());
        let inline = &threads[1];
        assert_eq!(
            inline
                .comments
                .iter()
                .map(|c| c.id.as_str())
                .collect::<Vec<_>>(),
            ["1", "2", "3"]
        );
        assert_eq!(inline.anchor.as_ref().unwrap().line, Some(3));
        assert_eq!(inline.anchor.as_ref().unwrap().side, DiffSide::New);
        assert!(inline.comments[0].mine && !inline.comments[1].mine);
        assert_eq!(unresolved(&threads), 1);
    }

    #[test]
    fn a_write_scope_counts_as_its_read_scope() {
        let scopes = split_list("write:pullrequest:bitbucket, read:repository:bitbucket");
        assert_eq!(missing(&scopes, &READ_SCOPES), vec!["read:user:bitbucket"]);
        assert!(missing(&scopes, &WRITE_SCOPES).is_empty());
    }
}
