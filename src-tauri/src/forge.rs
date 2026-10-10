//! Pull requests (SPEC.md, section 10): GitHub and Bitbucket Cloud.
//!
//! Which hosted repository a local repository's `origin` names is the core's
//! (`hosting`); here, how a pull request is referred to
//! (`github.com/acme/api#42`). Accounts are checked with one request and keep
//! their token where its source says (SPEC.md, Secrets). The pull request
//! service, the adapters, and the cache follow (docs/architecture.md, Pull
//! requests — v0.3).

mod accounts;
pub mod adapter;
pub mod bitbucket;
pub mod budget;
mod cache;
mod drafts;
pub mod github;
pub mod http;
pub mod markdown;
pub mod patch;
mod service;
mod store;

use std::fmt;
use std::str::FromStr;

use crate::hosting::ForgeRepository;
use crate::models::{AppError, AppResult};

pub use crate::models::ForgeKind;
pub use accounts::{AccountService, Endpoints};
pub use service::{
    PullRequestFacts, PullRequestService, DETAIL_MAX_AGE_SECONDS, LIST_MAX_AGE_SECONDS,
};

// A type's methods can be split across modules of one crate: those that say
// where a forge is live in `hosting`, and these, about accounts, stay here.
impl ForgeKind {
    /// The account name of its Keychain item (service `brainiac`).
    pub fn keychain_account(self) -> &'static str {
        match self {
            ForgeKind::Github => "github",
            ForgeKind::BitbucketCloud => "bitbucket",
        }
    }

    /// The provider's name in messages.
    pub fn label(self) -> &'static str {
        match self {
            ForgeKind::Github => "GitHub",
            ForgeKind::BitbucketCloud => "Bitbucket",
        }
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
