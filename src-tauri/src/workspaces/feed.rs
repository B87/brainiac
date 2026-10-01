//! The workspace activity feed, as the service offers it (SPEC.md, Workspace
//! activity). `crate::activity::ActivityTracker` records and filters events;
//! this module supplies checkouts and settings, shows notifications, and
//! sends the morning digest.

use std::collections::HashSet;
use std::path::Path;

use super::RepositoryService;
use crate::db::{self, RepositoryRow, WatchConfig};
use crate::fetcher::last_fetch_at;
use crate::git::Checkout;
use crate::models::{
    now_rfc3339, ActivitySettings, AppError, AppResult, RepositoryFreshness, StatusSnapshot,
    TeamPulse, Workspace, WorkspaceActivity,
};

/// Local hour of the morning digest.
const DIGEST_HOUR: u32 = 9;

impl RepositoryService {
    /// Record activity for the refs of `row`'s Git directory after a status
    /// observation, and show any notifications. Returns the number of new
    /// events; a failure is logged and counts as none.
    pub(super) async fn track(&self, row: &RepositoryRow, status: &StatusSnapshot) -> usize {
        let Ok(git) = self.git() else {
            return 0;
        };
        match self
            .tracker
            .observe(git, &Checkout::from(row), status)
            .await
        {
            Ok(pass) => {
                for n in pass.notices {
                    self.notify(n.title, n.body);
                }
                pass.new_events
            }
            Err(e) => {
                tracing::warn!(repository = %row.display_path, error = %e, "activity tracking failed");
                0
            }
        }
    }

    /// The Activity tab: feed and freshness of each member. The team pulse
    /// is separate (`team_pulse`), so the feed never waits for it.
    pub async fn workspace_activity(&self, workspace_id: &str) -> AppResult<WorkspaceActivity> {
        let scope = self.workspace_scope(workspace_id).await?;
        let feed = self.tracker.feed(&scope.config, &scope.checkouts).await?;
        let (workspace, rows) = (scope.workspace, scope.rows);
        // One line per Git directory: linked worktrees share their fetches.
        let mut seen = HashSet::new();
        let mut freshness: Vec<RepositoryFreshness> = rows
            .iter()
            .filter(|r| seen.insert(r.common_git_dir.clone()))
            .map(|r| RepositoryFreshness {
                repository_id: r.id.clone(),
                name: Checkout::from(r).name,
                last_fetch_at: last_fetch_at(
                    r.last_fetch_at.as_deref(),
                    Path::new(&r.git_dir),
                    Path::new(&r.common_git_dir),
                ),
                fetch_error: r.last_fetch_error.clone(),
            })
            .collect();
        // `None` (never fetched) sorts first, then oldest.
        freshness.sort_by(|a, b| a.last_fetch_at.cmp(&b.last_fetch_at));
        Ok(WorkspaceActivity {
            workspace_id: workspace.id,
            settings: workspace.activity,
            items: feed.items,
            unseen: feed.unseen,
            freshness,
        })
    }

    /// Commits, merges, releases, and the most active people on a
    /// workspace's watched refs over the last seven days.
    pub async fn team_pulse(&self, workspace_id: &str) -> AppResult<TeamPulse> {
        let scope = self.workspace_scope(workspace_id).await?;
        Ok(self
            .tracker
            .pulse(self.git()?, &scope.config.settings, &scope.checkouts)
            .await)
    }

    /// Mark the listed events, or every unread event the workspace shows, as seen.
    pub async fn mark_activity_seen(
        &self,
        workspace_id: &str,
        event_ids: Option<&[String]>,
    ) -> AppResult<()> {
        let scope = self.workspace_scope(workspace_id).await?;
        self.tracker
            .mark_seen(&scope.config, &scope.checkouts, event_ids)
            .await?;
        self.bump_version();
        Ok(())
    }

    /// Save a workspace's activity settings. When the watched patterns
    /// changed, take a tracking pass of each member Git directory right away
    /// (from its last status, without running Git status again), so refs that
    /// start matching join the baseline silently and branches created later
    /// still arrive as news.
    pub async fn update_activity_settings(
        &self,
        workspace_id: &str,
        settings: ActivitySettings,
    ) -> AppResult<Workspace> {
        let git = self.git()?;
        let settings = ActivitySettings {
            watched_branches: clean_patterns(git, &settings.watched_branches, true).await?,
            watched_tags: clean_patterns(git, &settings.watched_tags, false).await?,
            ..settings
        };
        let id = workspace_id.to_string();
        let now = now_rfc3339();
        let new = settings.clone();
        let previous = self
            .db
            .call(move |conn| db::set_activity_settings(conn, &id, &new, &now))
            .await?
            .ok_or_else(|| AppError::not_found("That workspace no longer exists."))?;
        let patterns_changed = previous.watched_branches != settings.watched_branches
            || previous.watched_tags != settings.watched_tags;
        if patterns_changed {
            let scope = self.workspace_scope(workspace_id).await?;
            let mut stores = HashSet::new();
            for row in &scope.rows {
                if let Some(status) = &row.status {
                    if stores.insert(row.common_git_dir.clone()) {
                        self.track(row, status).await;
                    }
                }
            }
        }
        self.bump_version();
        self.workspace(workspace_id).await
    }

    /// Show the morning digest for workspaces that asked for it: once per
    /// local day, from 09:00, and only when something is unread.
    pub async fn digest_tick(&self) {
        let now = chrono::Local::now();
        if chrono::Timelike::hour(&now) < DIGEST_HOUR {
            return;
        }
        let today = now.format("%Y-%m-%d").to_string();
        let (workspaces, dates) = match (
            self.workspaces().await,
            self.db.call(|conn| db::last_digest_dates(conn)).await,
        ) {
            (Ok(w), Ok(d)) => (w, d),
            _ => return,
        };
        for w in workspaces.iter().filter(|w| w.activity.morning_digest) {
            if dates.get(&w.id).and_then(Option::as_deref) == Some(today.as_str()) {
                continue;
            }
            let id = w.id.clone();
            let day = today.clone();
            if self
                .db
                .call(move |conn| db::set_last_digest_on(conn, &id, &day))
                .await
                .is_err()
            {
                continue;
            }
            if w.unseen_activity > 0 {
                let n = w.unseen_activity;
                self.notify(
                    format!("Good morning · {}", w.name),
                    format!(
                        "{n} {} on the watched branches since you last looked.",
                        if n == 1 { "update" } else { "updates" }
                    ),
                );
            }
        }
    }

    /// A workspace with its activity configuration and member checkouts.
    async fn workspace_scope(&self, workspace_id: &str) -> AppResult<ActivityScope> {
        let workspace = self.workspace(workspace_id).await?;
        let id = workspace_id.to_string();
        let config = self
            .db
            .call(move |conn| Ok(db::activity_settings(conn)?.remove(&id)))
            .await?
            .ok_or_else(|| AppError::not_found("That workspace no longer exists."))?;
        let ids: HashSet<&String> = workspace
            .members
            .iter()
            .filter_map(|m| m.repository_id.as_ref())
            .collect();
        let rows: Vec<RepositoryRow> = self
            .rows()
            .await?
            .into_iter()
            .filter(|r| ids.contains(&r.id))
            .collect();
        let checkouts = rows.iter().map(Checkout::from).collect();
        Ok(ActivityScope {
            workspace,
            config,
            rows,
            checkouts,
        })
    }
}

/// What the Activity features need about one workspace.
struct ActivityScope {
    workspace: Workspace,
    config: WatchConfig,
    /// Registered member repositories, each once.
    rows: Vec<RepositoryRow>,
    checkouts: Vec<Checkout>,
}

/// Trim, drop empties and duplicates, and reject patterns Git could not use
/// (`git check-ref-format`), plus `?`, `[` and `]`, which the feed's matching
/// does not support. Branch patterns become fetch refspecs and allow one `*`.
async fn clean_patterns(
    git: &crate::git::GitService,
    patterns: &[String],
    branches: bool,
) -> AppResult<Vec<String>> {
    let cleaned = crate::activity::clean_patterns(patterns, branches)?;
    for p in &cleaned {
        if !git.ref_pattern_ok(p, branches).await {
            return Err(AppError::validation(format!(
                "\"{p}\" is not a valid {} name or pattern.",
                if branches { "branch" } else { "tag" }
            )));
        }
    }
    Ok(cleaned)
}
