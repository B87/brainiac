//! Pull requests (SPEC.md, section 10): GitHub and Bitbucket Cloud.
//!
//! This first part is identity, which needs no network: which hosted
//! repository a local repository's `origin` names (a forge repository), and
//! how a pull request is referred to (`github.com/acme/api#42`). The service,
//! the adapters, and the cache follow (docs/architecture.md, Pull requests —
//! v0.3).

use std::fmt;
use std::str::FromStr;

use crate::models::{AppError, AppResult};

/// A hosting service Brainiac reads pull requests from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ForgeKind {
    Github,
    BitbucketCloud,
}

impl ForgeKind {
    /// The host its repositories live on, as written in a forge repository.
    pub fn host(self) -> &'static str {
        match self {
            ForgeKind::Github => "github.com",
            ForgeKind::BitbucketCloud => "bitbucket.org",
        }
    }

    /// The forge a remote's host belongs to. Each provider also serves SSH on
    /// port 443 under another name, for networks that block port 22.
    fn from_host(host: &str) -> Option<Self> {
        match host.to_ascii_lowercase().as_str() {
            "github.com" | "www.github.com" | "ssh.github.com" => Some(ForgeKind::Github),
            "bitbucket.org" | "www.bitbucket.org" | "altssh.bitbucket.org" => {
                Some(ForgeKind::BitbucketCloud)
            }
            _ => None,
        }
    }
}

/// A repository on a forge: `github.com/acme/api`, `bitbucket.org/acme-team/api`.
/// The owner is a GitHub user or organization, or a Bitbucket workspace.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ForgeRepository {
    pub kind: ForgeKind,
    pub owner: String,
    pub name: String,
}

impl ForgeRepository {
    /// The forge repository a Git remote URL points to, from its HTTPS,
    /// `ssh://`, `git://`, and SCP-style (`git@github.com:acme/api.git`)
    /// forms. `None` for any other host (including an SSH alias from
    /// `~/.ssh/config`), a local path, or a path that is not `owner/name`;
    /// the user can then choose the repository by hand.
    pub fn from_remote_url(url: &str) -> Option<Self> {
        let url = url.trim();
        let (authority, path) = match url.split_once("://") {
            Some((scheme, rest)) => {
                if !matches!(
                    scheme.to_ascii_lowercase().as_str(),
                    "https" | "http" | "ssh" | "git" | "git+ssh" | "ssh+git"
                ) {
                    return None;
                }
                rest.split_once('/')?
            }
            // SCP-style: `[user@]host:path`, where the colon comes before any
            // slash (otherwise it is a local path such as `./a:b`).
            None => {
                let colon = url.find(':')?;
                if url[..colon].contains('/') {
                    return None;
                }
                (&url[..colon], &url[colon + 1..])
            }
        };
        // Drop `user[:password]@` and `:port` around the host.
        let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
        let host = host.split_once(':').map_or(host, |(h, _)| h);
        let kind = ForgeKind::from_host(host)?;
        let path = path.trim_matches('/');
        let path = path.strip_suffix(".git").unwrap_or(path);
        let (owner, name) = path.split_once('/')?;
        Self::new(kind, owner, name)
    }

    /// A forge repository from its parts, if both are plausible names. They
    /// end up in API paths, so anything outside the characters both providers
    /// allow (letters, digits, `.`, `_`, `-`) is refused rather than escaped:
    /// `..` or a `/` must never reach a request URL.
    pub fn new(kind: ForgeKind, owner: &str, name: &str) -> Option<Self> {
        let plausible = |s: &str| {
            !s.is_empty()
                && s != "."
                && s != ".."
                && s.len() <= 100
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        };
        (plausible(owner) && plausible(name)).then(|| ForgeRepository {
            kind,
            owner: owner.to_string(),
            name: name.to_string(),
        })
    }
}

// `Display` is the trait behind `format!("{}")` and `.to_string()`; this is
// the one place that decides how a forge repository is written.
impl fmt::Display for ForgeRepository {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}/{}", self.kind.host(), self.owner, self.name)
    }
}

/// A pull request: its forge repository and number, written
/// `github.com/acme/api#42`. Both providers number pull requests per
/// repository, so this identifies one everywhere (cache keys, drafts, events).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PullRequestRef {
    pub repository: ForgeRepository,
    pub number: u64,
}

impl fmt::Display for PullRequestRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}#{}", self.repository, self.number)
    }
}

// `FromStr` is the trait behind `"…".parse::<PullRequestRef>()`, the reverse
// of `Display` above; references come back this way from the frontend.
impl FromStr for PullRequestRef {
    type Err = AppError;

    fn from_str(s: &str) -> AppResult<Self> {
        let invalid = || AppError::validation(format!("Not a pull request reference: {s}"));
        let (repository, number) = s.rsplit_once('#').ok_or_else(invalid)?;
        let mut parts = repository.split('/');
        let (Some(host), Some(owner), Some(name), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(invalid());
        };
        let kind = ForgeKind::from_host(host)
            .filter(|k| k.host() == host)
            .ok_or_else(invalid)?;
        let repository = ForgeRepository::new(kind, owner, name).ok_or_else(invalid)?;
        let number = number
            .parse::<u64>()
            .ok()
            .filter(|n| *n > 0)
            .ok_or_else(invalid)?;
        Ok(PullRequestRef { repository, number })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo(url: &str) -> Option<String> {
        ForgeRepository::from_remote_url(url).map(|r| r.to_string())
    }

    #[test]
    fn remote_urls_name_their_forge_repository() {
        let cases = [
            ("https://github.com/acme/api.git", "github.com/acme/api"),
            ("https://github.com/acme/api", "github.com/acme/api"),
            ("https://github.com/acme/api/", "github.com/acme/api"),
            ("http://github.com/acme/api.git", "github.com/acme/api"),
            ("https://GitHub.com/Acme/API.git", "github.com/Acme/API"),
            ("git@github.com:acme/api.git", "github.com/acme/api"),
            ("github.com:acme/api", "github.com/acme/api"),
            ("ssh://git@github.com/acme/api.git", "github.com/acme/api"),
            (
                "ssh://git@ssh.github.com:443/acme/api.git",
                "github.com/acme/api",
            ),
            ("git://github.com/acme/api.git", "github.com/acme/api"),
            (
                "https://bitbucket.org/acme-team/api.git",
                "bitbucket.org/acme-team/api",
            ),
            (
                "https://jo@bitbucket.org/acme-team/api.git",
                "bitbucket.org/acme-team/api",
            ),
            (
                "git@bitbucket.org:acme-team/api.git",
                "bitbucket.org/acme-team/api",
            ),
            (
                "ssh://git@altssh.bitbucket.org:443/acme-team/web.app.git",
                "bitbucket.org/acme-team/web.app",
            ),
        ];
        for (url, expected) in cases {
            assert_eq!(repo(url).as_deref(), Some(expected), "{url}");
        }
    }

    #[test]
    fn other_remotes_name_no_forge_repository() {
        for url in [
            "",
            "/Users/me/src/api",
            "./a:b",
            "file:///Users/me/src/api.git",
            "https://gitlab.com/acme/api.git",
            "git@gh-work:acme/api.git",
            "https://github.com/acme",
            "https://github.com/acme/api/tree/main",
            "https://github.com/acme/.git",
            "https://github.com/../api",
            "https://github.com/acme/a%2Fb",
            "ftp://github.com/acme/api",
        ] {
            assert_eq!(repo(url), None, "{url}");
        }
    }

    #[test]
    fn pull_request_references_round_trip() {
        let pr: PullRequestRef = "bitbucket.org/acme-team/api#377".parse().unwrap();
        assert_eq!(pr.repository.kind, ForgeKind::BitbucketCloud);
        assert_eq!(pr.repository.owner, "acme-team");
        assert_eq!(pr.number, 377);
        assert_eq!(pr.to_string(), "bitbucket.org/acme-team/api#377");

        for bad in [
            "github.com/acme/api",
            "github.com/acme/api#0",
            "github.com/acme/api#-1",
            "github.com/acme/api#x",
            "github.com/acme#4",
            "github.com/acme/api/x#4",
            "ssh.github.com/acme/api#4",
            "gitlab.com/acme/api#4",
        ] {
            assert!(bad.parse::<PullRequestRef>().is_err(), "{bad}");
        }
    }
}
