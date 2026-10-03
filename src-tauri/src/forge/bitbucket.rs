//! Bitbucket Cloud (bitbucket.org), REST API 2.0 (docs/architecture.md,
//! Pull requests — v0.3).

use serde::Deserialize;

use super::accounts::AccountCheck;
use super::adapter::{
    base_actions, normalize_time, read_error, summarize_checks, Client, ForgeAdapter, ListOutcome,
    Session,
};
use super::github::split_list;
use super::http::{unexpected, Auth, Http, Response};
use super::keychain::Token;
use super::{ForgeRepository, PullRequestRef};
use crate::models::{
    AppError, AppResult, ChangedFile, ChangedFileStatus, Check, CheckState, ErrorCode, ForgeKind,
    ForgeTokenKind, ForgeUser, Mergeability, PullRequest, PullRequestCounts, PullRequestState,
    ReviewState, Reviewer,
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

#[derive(Deserialize)]
struct Comment {
    parent: Option<Id>,
    inline: Option<serde_json::Value>,
    resolution: Option<serde_json::Value>,
    #[serde(default)]
    pending: bool,
    #[serde(default)]
    deleted: bool,
}

#[derive(Deserialize)]
struct Id {
    #[allow(dead_code)]
    id: u64,
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
    PullRequest {
        reference: PullRequestRef {
            repository: repository.clone(),
            number: pr.id,
        }
        .to_string(),
        number: pr.id,
        kind: ForgeKind::BitbucketCloud,
        title: pr.title.clone(),
        description: pr
            .description
            .clone()
            .or_else(|| pr.summary.as_ref().map(|s| s.raw.clone()))
            .unwrap_or_default(),
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

    /// What the list leaves out: the head's statuses, the diffstat, and how
    /// many inline threads are unresolved. Three requests.
    pub async fn detail(
        &self,
        session: &Session,
        pr: &mut PullRequest,
        reference: &PullRequestRef,
    ) -> AppResult<Vec<ChangedFile>> {
        let checks = self.checks(session, reference, &pr.head_sha).await?;
        pr.checks = summarize_checks(&checks);
        let files = self.files(session, reference).await?;
        pr.counts.additions = Some(files.iter().map(|f| f.additions).sum());
        pr.counts.deletions = Some(files.iter().map(|f| f.deletions).sum());
        pr.counts.changed_files = Some(files.len() as u32);
        let url = format!(
            "{}/pullrequests/{}/comments?pagelen=100&q=deleted=false&fields=next,values.parent.id,values.inline.path,values.resolution.type,values.pending,values.deleted",
            self.repo_url(&reference.repository),
            reference.number
        );
        let comments: Vec<Comment> = self
            .pages(
                session,
                url,
                &format!("the comments of {reference}"),
                |_| true,
            )
            .await?;
        pr.counts.unresolved_threads = Some(
            comments
                .iter()
                .filter(|c| {
                    c.parent.is_none()
                        && c.inline.is_some()
                        && c.resolution.is_none()
                        && !c.pending
                        && !c.deleted
                })
                .count() as u32,
        );
        Ok(files)
    }
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
