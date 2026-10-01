//! Fetch now and auto-fetch, as the service offers them (SPEC.md, Fetching).
//! `crate::fetcher::Fetcher` decides how a fetch runs; this module picks what
//! to fetch, schedules auto-fetches, and refreshes the checkouts afterwards.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use super::RepositoryService;
use crate::db::{self, RepositoryRow};
use crate::fetcher::{last_fetch_at, Scope};
use crate::git::Checkout;
use crate::models::{now_rfc3339, AppError, AppResult, ChangeOrigin, FetchResult};

/// Shortest auto-fetch interval a setting can ask for.
const MIN_INTERVAL: Duration = Duration::from_secs(5 * 60);

impl RepositoryService {
    /// Fetch the repository's remote. `auto` fetches only the branches the
    /// workspaces with auto-fetch watch. Every checkout sharing the Git
    /// directory is refreshed afterwards, which updates ahead/behind counts
    /// and the activity feed (and shows a failure in the next snapshot).
    pub async fn fetch(self: &Arc<Self>, id: &str, auto: bool) -> AppResult<FetchResult> {
        let row = self.row(id).await?;
        let checkout = Checkout::from(&row);
        if !checkout.root.is_dir() {
            return Err(AppError::not_found(format!(
                "{} no longer exists.",
                row.display_path
            )));
        }
        let git = self.git()?.clone();
        let store = checkout.store();
        let scope = if auto {
            Scope::Watched(self.auto_fetch_patterns(&store).await?)
        } else {
            Scope::Remote
        };
        let timeout = Duration::from_secs(self.settings().fetch_timeout_seconds.max(10));
        let result = self.fetcher.fetch(&git, &checkout, scope, timeout).await;
        // A busy repository changed nothing and recorded nothing.
        if !matches!(&result, Err(e) if e.code == crate::models::ErrorCode::Conflict) {
            self.refresh_store(&store).await;
        }
        let fetched = result?;
        Ok(FetchResult {
            repository_id: id.to_string(),
            remote: fetched.remote,
            fetched_at: fetched.fetched_at.unwrap_or_else(now_rfc3339),
            moved: fetched.moved,
        })
    }

    async fn refresh_store(self: &Arc<Self>, store: &str) {
        let ids: Vec<String> = match self.rows().await {
            Ok(rows) => rows
                .into_iter()
                .filter(|r| r.common_git_dir == store)
                .map(|r| r.id)
                .collect(),
            Err(_) => return,
        };
        for id in ids {
            if let Err(e) = self.refresh(&id, ChangeOrigin::Fetch).await {
                tracing::warn!(repository = %id, error = %e, "refresh after fetch failed");
            }
        }
    }

    /// Start the auto-fetches that are due, one per Git directory. Called
    /// about once a minute while the app runs.
    pub async fn auto_fetch_tick(self: &Arc<Self>) {
        let interval =
            Duration::from_secs(self.settings().auto_fetch_interval_minutes * 60).max(MIN_INTERVAL);
        let candidates = match self.auto_fetch_candidates().await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, "could not list auto-fetch repositories");
                return;
            }
        };
        for row in candidates {
            let store = row.common_git_dir.clone();
            let last = last_fetch_at(
                row.last_fetch_at.as_deref(),
                Path::new(&row.git_dir),
                Path::new(&row.common_git_dir),
            );
            if !self.fetcher.is_due(&store, last.as_deref(), interval) {
                continue;
            }
            let service = Arc::clone(self);
            tokio::spawn(async move {
                let result = service.fetch(&row.id, true).await;
                service
                    .fetcher
                    .record_auto(&store, result.as_ref().err(), interval);
            });
        }
    }

    /// One registered checkout per Git directory that an auto-fetching
    /// workspace contains, preferring main checkouts over linked worktrees.
    async fn auto_fetch_candidates(&self) -> AppResult<Vec<RepositoryRow>> {
        let (settings, members) = self
            .db
            .call(|conn| Ok((db::activity_settings(conn)?, db::list_members(conn, None)?)))
            .await?;
        let wanted: HashSet<String> = members
            .into_iter()
            .filter(|m| settings.get(&m.workspace_id).is_some_and(|s| s.auto_fetch))
            .filter_map(|m| m.repository_id)
            .collect();
        let mut by_store: HashMap<String, RepositoryRow> = HashMap::new();
        for row in self.rows().await? {
            if !wanted.contains(&row.id) || !Path::new(&row.canonical_root).is_dir() {
                continue;
            }
            let is_main = row.git_dir == row.common_git_dir;
            let replace = by_store
                .get(&row.common_git_dir)
                .is_none_or(|current| current.git_dir != current.common_git_dir && is_main);
            if replace {
                by_store.insert(row.common_git_dir.clone(), row);
            }
        }
        Ok(by_store.into_values().collect())
    }

    /// Watched branch patterns of the auto-fetching workspaces that contain a
    /// checkout of `store`.
    async fn auto_fetch_patterns(&self, store: &str) -> AppResult<Vec<String>> {
        let store = store.to_string();
        let watchers = self
            .db
            .call(move |conn| db::watchers_of_store(conn, &store))
            .await?;
        let mut patterns: Vec<String> = Vec::new();
        for w in watchers.iter().filter(|w| w.settings.auto_fetch) {
            for p in &w.settings.watched_branches {
                if !patterns.contains(p) {
                    patterns.push(p.clone());
                }
            }
        }
        Ok(patterns)
    }
}
