//! Each account's request budget (docs/architecture.md, Sync, cache, and
//! request budget). GitHub says in every answer how much is left and when it
//! resets; Bitbucket says nothing, so its requests of the last hour are
//! counted here and assumed to count against its 1,000. A refusal or an
//! unreachable provider blocks requests for a while instead of retrying blindly.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use chrono::{DateTime, Duration, Utc};

use super::http::Response;
use crate::models::{AppError, AppResult, ErrorCode, ForgeKind, RequestBudget};

/// Bitbucket Cloud's documented allowance for repository data, per user per hour.
const BITBUCKET_LIMIT: u32 = 1000;
/// GitHub's for a personal access token, until its headers say otherwise.
const GITHUB_LIMIT: u32 = 5000;
/// How long an unreachable provider is left alone.
const UNREACHABLE_PAUSE: Duration = Duration::minutes(1);
/// How long a provider that refused for too many requests is left alone when
/// it does not say.
const REFUSED_PAUSE: Duration = Duration::minutes(5);
/// Keep this much of the allowance for what the user does by hand.
const RESERVE: u32 = 50;

#[derive(Debug)]
struct Window {
    limit: u32,
    /// GitHub: what its last answer said.
    used: u32,
    resets_at: Option<DateTime<Utc>>,
    /// Bitbucket: when each request of the last hour was sent.
    sent: VecDeque<DateTime<Utc>>,
    retry_at: Option<DateTime<Utc>>,
}

impl Window {
    fn new(kind: ForgeKind) -> Self {
        Window {
            limit: match kind {
                ForgeKind::Github => GITHUB_LIMIT,
                ForgeKind::BitbucketCloud => BITBUCKET_LIMIT,
            },
            used: 0,
            resets_at: None,
            sent: VecDeque::new(),
            retry_at: None,
        }
    }

    fn used_at(&mut self, kind: ForgeKind, now: DateTime<Utc>) -> u32 {
        match kind {
            ForgeKind::Github => {
                if self.resets_at.is_some_and(|t| t <= now) {
                    self.used = 0;
                    self.resets_at = None;
                }
                self.used
            }
            ForgeKind::BitbucketCloud => {
                let hour_ago = now - Duration::hours(1);
                while self.sent.front().is_some_and(|t| *t < hour_ago) {
                    self.sent.pop_front();
                }
                self.sent.len() as u32
            }
        }
    }
}

#[derive(Debug, Default)]
pub struct Budget {
    // `Mutex`: adapters observe answers from several tasks at once.
    windows: Mutex<HashMap<ForgeKind, Window>>,
}

impl Budget {
    /// Refuse to send when the provider asked for a pause or the hour's
    /// allowance is down to the reserve.
    pub fn check(&self, kind: ForgeKind) -> AppResult<()> {
        let now = Utc::now();
        let mut windows = self.windows.lock().expect("budget lock");
        let w = windows.entry(kind).or_insert_with(|| Window::new(kind));
        if let Some(at) = w.retry_at {
            if at > now {
                return Err(paused(kind, at, "is not answering right now"));
            }
            w.retry_at = None;
        }
        let used = w.used_at(kind, now);
        if used + RESERVE >= w.limit {
            let at = match kind {
                ForgeKind::Github => w.resets_at.unwrap_or(now + Duration::minutes(5)),
                ForgeKind::BitbucketCloud => w
                    .sent
                    .front()
                    .map(|t| *t + Duration::hours(1))
                    .unwrap_or(now + Duration::minutes(5)),
            };
            return Err(paused(kind, at, "has no requests left for this hour"));
        }
        Ok(())
    }

    /// Learn from an answer, or from the lack of one.
    pub fn observe(&self, kind: ForgeKind, result: &AppResult<Response>) {
        let now = Utc::now();
        let mut windows = self.windows.lock().expect("budget lock");
        let w = windows.entry(kind).or_insert_with(|| Window::new(kind));
        if kind == ForgeKind::BitbucketCloud {
            w.sent.push_back(now);
        }
        match result {
            Ok(response) => {
                if let (Some(limit), Some(remaining)) = (
                    header_u32(response, "x-ratelimit-limit"),
                    header_u32(response, "x-ratelimit-remaining"),
                ) {
                    w.limit = limit.max(1);
                    w.used = limit.saturating_sub(remaining);
                    w.resets_at = header_u32(response, "x-ratelimit-reset")
                        .and_then(|s| DateTime::from_timestamp(s as i64, 0));
                }
                let refused = response.status == 429
                    || (response.status == 403
                        && response.header("x-ratelimit-remaining") == Some("0"));
                if refused {
                    let wait = response
                        .header("retry-after")
                        .and_then(|s| s.trim().parse::<i64>().ok())
                        .map(Duration::seconds)
                        .or_else(|| w.resets_at.map(|t| t - now))
                        .filter(|d| *d > Duration::zero())
                        .unwrap_or(REFUSED_PAUSE);
                    w.retry_at = Some(now + wait);
                }
            }
            // No answer at all: leave the provider alone for a while. One
            // request that got no answer in time is not that: a write that
            // timed out is looked for right away (SPEC.md, Reviewing).
            Err(e) if e.code == ErrorCode::DependencyUnavailable => {
                w.retry_at = Some(now + UNREACHABLE_PAUSE);
            }
            Err(_) => {}
        }
    }

    pub fn snapshot(&self) -> Vec<RequestBudget> {
        let now = Utc::now();
        let mut windows = self.windows.lock().expect("budget lock");
        let mut out: Vec<RequestBudget> = windows
            .iter_mut()
            .map(|(kind, w)| RequestBudget {
                kind: *kind,
                used: w.used_at(*kind, now),
                limit: w.limit,
                resets_at: w.resets_at.map(rfc3339),
                retry_at: w.retry_at.filter(|t| *t > now).map(rfc3339),
            })
            .collect();
        out.sort_by_key(|b| b.kind as u8);
        out
    }
}

fn header_u32(response: &Response, name: &str) -> Option<u32> {
    response.header(name)?.trim().parse().ok()
}

fn rfc3339(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn paused(kind: ForgeKind, until: DateTime<Utc>, why: &str) -> AppError {
    AppError::dependency(format!(
        "{} {why}. Brainiac tries again at {}.",
        kind.label(),
        until.with_timezone(&chrono::Local).format("%H:%M")
    ))
    .with_details(format!("retry_at={}", rfc3339(until)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::HeaderMap;

    fn answer(status: u16, headers: &[(&'static str, &str)]) -> AppResult<Response> {
        let mut map = HeaderMap::new();
        for (name, value) in headers {
            map.insert(*name, value.parse().unwrap());
        }
        Ok(Response {
            status,
            headers: map,
            body: Vec::new(),
        })
    }

    #[test]
    fn github_follows_its_headers_and_pauses_when_refused() {
        let budget = Budget::default();
        budget.check(ForgeKind::Github).unwrap();
        budget.observe(
            ForgeKind::Github,
            &answer(
                200,
                &[
                    ("x-ratelimit-limit", "5000"),
                    ("x-ratelimit-remaining", "4990"),
                    ("x-ratelimit-reset", "4102444800"),
                ],
            ),
        );
        let b = &budget.snapshot()[0];
        assert_eq!((b.used, b.limit), (10, 5000));
        assert_eq!(b.resets_at.as_deref(), Some("2100-01-01T00:00:00.000Z"));
        budget.observe(ForgeKind::Github, &answer(429, &[("retry-after", "120")]));
        let err = budget.check(ForgeKind::Github).unwrap_err();
        assert_eq!(err.code, ErrorCode::DependencyUnavailable);
        assert!(budget.snapshot()[0].retry_at.is_some());
    }

    #[test]
    fn bitbucket_is_counted_here_and_keeps_a_reserve() {
        let budget = Budget::default();
        for _ in 0..(BITBUCKET_LIMIT - RESERVE - 1) {
            budget.observe(ForgeKind::BitbucketCloud, &answer(200, &[]));
        }
        budget.check(ForgeKind::BitbucketCloud).unwrap();
        budget.observe(ForgeKind::BitbucketCloud, &answer(200, &[]));
        let err = budget.check(ForgeKind::BitbucketCloud).unwrap_err();
        assert!(err.message.contains("no requests left"), "{}", err.message);
        assert_eq!(budget.snapshot()[0].used, BITBUCKET_LIMIT - RESERVE);
    }

    #[test]
    fn an_unreachable_provider_is_left_alone_for_a_minute() {
        let budget = Budget::default();
        budget.observe(
            ForgeKind::Github,
            &Err(AppError::dependency("Cannot reach GitHub.")),
        );
        assert!(budget.check(ForgeKind::Github).is_err());
        assert!(budget.snapshot()[0].retry_at.is_some());
    }
}
