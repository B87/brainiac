//! Relocating a registration whose folder moved, and Rescan's move
//! suggestions (SPEC.md, Relocating a repository).
//!
//! A child module of `workspaces`, like `membership`, so it can use the
//! service's private fields.
//!
//! Rules decided here:
//! - Identity is evidence, not a path: a working tree is the same repository
//!   when it contains a commit recorded for the registration (the last
//!   observed `HEAD` or a tip in its activity baseline).
//! - A relocation is planned first (Git work, no writes), then applied in one
//!   transaction that re-checks the plan, then the activity tracker and
//!   fetcher are told which Git directories moved or were dropped.
//! - Things registered inside a folder that is gone move along with it. That
//!   folder is the highest one whose name the old and new paths share
//!   (`code/web` → `src/web` means `code` became `src`); relocating to a
//!   second clone while the first still exists moves just the one registration.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::membership::{existing_folder, folder_name, resolve_optional, DiscoveryScope};
use super::RepositoryService;
use crate::db::{self, Location, RepositoryRow};
use crate::git::ResolvedRepository;
use crate::models::{
    AppError, AppResult, ChangeOrigin, DiscoveryMode, ErrorCode, MemberOrigin, MemberStatus,
    PreviewStatus, RelocateRepositoryRequest, RelocationConcern, RelocationOutcome, SuggestedMove,
    Workspace, WorkspacePreviewEntry, WorkspaceRescan,
};

/// Recorded tips used as identity evidence, besides the last `HEAD`.
const MAX_EVIDENCE: u32 = 64;
/// Rescan looks for at most this many missing members among at most this many
/// untracked repositories (docs/architecture.md, Workspaces and discovery).
const MAX_SUGGESTED_MEMBERS: usize = 16;
const MAX_SUGGESTION_TARGETS: usize = 64;

/// What a relocation produced. The command layer re-watches `moved`, which
/// this service cannot do without Tauri state.
#[derive(Debug)]
pub struct Relocation {
    pub outcome: RelocationOutcome,
    /// Every registration whose location changed (or was confirmed), as stored now.
    pub moved: Vec<RepositoryRow>,
}

/// How a working tree compares with what was recorded for a registration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum History {
    Same,
    Unrelated,
    /// Nothing was recorded to compare with, or Git cannot tell offline.
    Unverified,
}

/// One registration and where it goes.
struct Move {
    row: RepositoryRow,
    to: ResolvedRepository,
    history: History,
}

/// A folder that is gone and the folder it became; everything registered
/// inside the first moves to the same place inside the second.
struct Along {
    from: PathBuf,
    to: PathBuf,
}

impl Along {
    /// `path` re-rooted into the new folder, when it was inside the old one.
    fn map(&self, path: &Path) -> Option<PathBuf> {
        path.strip_prefix(&self.from).ok().map(|rel| {
            let moved = self.to.join(rel);
            std::fs::canonicalize(&moved).unwrap_or(moved)
        })
    }
}

/// What applying the moves changed, for the follow-up outside the transaction.
#[derive(Default)]
struct Applied {
    /// Registrations whose location changed (moves and their linked worktrees).
    touched: Vec<String>,
    /// Git directories whose activity follows them, `(from, to)`.
    moved_stores: Vec<(String, String)>,
    /// Git directories no registration uses any more.
    dropped_stores: Vec<String>,
}

impl RepositoryService {
    /// Point a registration at the folder its repository moved to. Returns
    /// `NeedsConfirmation` without writing when the folder is not clearly the
    /// same repository, unless `request.confirmed_root` is the root it names.
    pub async fn relocate_repository(
        self: &Arc<Self>,
        request: RelocateRepositoryRequest,
    ) -> AppResult<Relocation> {
        let row = self.row(&request.repository_id).await?;
        let git = self.git()?;
        let picked = existing_folder(&request.path).await?;
        let target = git.resolve(&picked).await?;
        let old_root = PathBuf::from(&row.canonical_root);

        if target.root == old_root
            && target.git_dir == Path::new(&row.git_dir)
            && target.common_git_dir == Path::new(&row.common_git_dir)
        {
            // Already there (moved back, say): refresh and watch it again.
            let repository = self.refresh(&row.id, ChangeOrigin::Refresh).await?;
            return Ok(Relocation {
                outcome: RelocationOutcome::Relocated {
                    repository: Box::new(repository),
                    carried: Vec::new(),
                },
                moved: vec![self.row(&row.id).await?],
            });
        }
        let new_root = target.root.display().to_string();
        self.ensure_unclaimed(&new_root, &row.id).await?;

        let history = self.history(&row, &target.root).await?;
        let mut concerns = Vec::new();
        if target.root != picked {
            concerns.push(RelocationConcern::InsideRepository);
        }
        match history {
            History::Same => {}
            History::Unrelated => concerns.push(RelocationConcern::UnrelatedHistory),
            History::Unverified => concerns.push(RelocationConcern::UnverifiedHistory),
        }
        if !concerns.is_empty() && request.confirmed_root.as_deref() != Some(new_root.as_str()) {
            return Ok(Relocation {
                outcome: RelocationOutcome::NeedsConfirmation {
                    root: new_root,
                    concerns,
                },
                moved: Vec::new(),
            });
        }

        let along = moved_folder(&old_root, &target.root);
        let mut moves = vec![Move {
            row: row.clone(),
            to: target,
            history,
        }];
        if let Some(along) = &along {
            moves.extend(self.carried_moves(&row, along).await?);
        }
        let carried: Vec<String> = moves[1..].iter().map(|m| folder_name(&m.to.root)).collect();

        let applied = self
            .db
            .call(move |conn| {
                // Dropping a transaction without `commit` rolls it back.
                let tx = conn.transaction()?;
                let applied = apply(&tx, &moves, along.as_ref())?;
                tx.commit()?;
                Ok(applied)
            })
            .await?;

        for (from, to) in &applied.moved_stores {
            self.tracker.relocate(from, to).await?;
            self.fetcher.forget(from);
        }
        for store in &applied.dropped_stores {
            self.tracker.forget(store, true).await?;
            self.fetcher.forget(store);
        }
        self.bump_version();
        for id in applied.touched.iter().filter(|id| **id != row.id) {
            self.request_refresh(id, ChangeOrigin::Refresh);
        }
        let repository = self.refresh(&row.id, ChangeOrigin::Refresh).await?;
        let mut moved = Vec::new();
        for id in &applied.touched {
            moved.push(self.row(id).await?);
        }
        Ok(Relocation {
            outcome: RelocationOutcome::Relocated {
                repository: Box::new(repository),
                carried,
            },
            moved,
        })
    }

    /// What a discovered workspace's folder holds that the workspace does not
    /// track, and which missing members appear to have moved there.
    pub async fn rescan_workspace(&self, workspace_id: &str) -> AppResult<WorkspaceRescan> {
        let ws = self.workspace(workspace_id).await?;
        let (DiscoveryMode::Discovered, Some(root)) =
            (ws.discovery_mode, ws.discovery_root.as_deref())
        else {
            return Err(AppError::validation(
                "Only a workspace created from a folder can be rescanned.",
            ));
        };
        let preview = self
            .discover_repositories(root, ws.discovery_path.as_deref())
            .await?;
        let members: HashSet<&str> = ws
            .members
            .iter()
            .map(|m| m.canonical_path.as_str())
            .collect();
        let untracked: Vec<WorkspacePreviewEntry> = preview
            .entries
            .into_iter()
            .filter(|e| !members.contains(e.resolved_path.as_deref().unwrap_or(&e.configured_path)))
            .collect();
        let skipped = untracked
            .iter()
            .filter(|e| {
                matches!(
                    e.status,
                    PreviewStatus::NotGit | PreviewStatus::Unsupported | PreviewStatus::Nested
                )
            })
            .count() as u32;
        let mut repositories: Vec<WorkspacePreviewEntry> = untracked
            .into_iter()
            .filter(|e| e.status == PreviewStatus::Ok)
            .collect();
        let moves = self.suggest_moves(&ws, &repositories).await;
        repositories.retain(|e| {
            !moves
                .iter()
                .any(|m| e.repository_root.as_ref() == Some(&m.to))
        });
        Ok(WorkspaceRescan {
            workspace_id: ws.id,
            repositories,
            skipped,
            moves,
        })
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    /// `CONFLICT` when another registration already uses `root`.
    async fn ensure_unclaimed(&self, root: &str, id: &str) -> AppResult<()> {
        let root2 = root.to_string();
        let other = self
            .db
            .call(move |conn| db::find_repository_by_display_path(conn, &root2))
            .await?;
        match other {
            Some(other) if other.id != id => Err(already_tracked(root)),
            _ => Ok(()),
        }
    }

    /// Commits recorded for a registration: its last `HEAD` and its baseline's tips.
    async fn evidence(&self, row: &RepositoryRow) -> AppResult<Vec<String>> {
        let mut ids: Vec<String> = row
            .status
            .as_ref()
            .and_then(|s| s.head.commit_id.clone())
            .into_iter()
            .collect();
        ids.extend(
            self.tracker
                .recorded_commits(&row.common_git_dir, MAX_EVIDENCE)
                .await?,
        );
        ids.sort();
        ids.dedup();
        Ok(ids)
    }

    async fn compare(&self, evidence: &[String], root: &Path) -> AppResult<History> {
        if evidence.is_empty() {
            return Ok(History::Unverified);
        }
        Ok(match self.git()?.contains_any(root, evidence).await? {
            Some(true) => History::Same,
            Some(false) => History::Unrelated,
            None => History::Unverified,
        })
    }

    async fn history(&self, row: &RepositoryRow, root: &Path) -> AppResult<History> {
        let evidence = self.evidence(row).await?;
        self.compare(&evidence, root).await
    }

    /// Missing registrations inside the folder that moved (other than
    /// `primary`) whose folder exists at the same place inside the new one and
    /// holds the same repository. The others, including any Git could not
    /// check, stay where they are, missing.
    async fn carried_moves(&self, primary: &RepositoryRow, along: &Along) -> AppResult<Vec<Move>> {
        let git = self.git()?;
        let mut moves = Vec::new();
        for row in self.rows().await? {
            let path = PathBuf::from(&row.canonical_root);
            if row.id == primary.id || path.is_dir() {
                continue;
            }
            let Some(candidate) = along.map(&path).filter(|c| c.is_dir()) else {
                continue;
            };
            let Ok(Some(resolved)) = resolve_optional(git, &candidate).await else {
                continue;
            };
            if resolved.root != candidate
                || self
                    .ensure_unclaimed(&candidate.display().to_string(), &row.id)
                    .await
                    .is_err()
                || !matches!(self.history(&row, &resolved.root).await, Ok(History::Same))
            {
                continue;
            }
            moves.push(Move {
                row,
                to: resolved,
                history: History::Same,
            });
        }
        Ok(moves)
    }

    /// One-to-one pairs of a missing member and an untracked, unregistered
    /// repository that is the same repository. One Git run per untracked
    /// repository checks the evidence of all missing members at once; only a
    /// hit is checked member by member. Git failures count as no match: a
    /// suggestion is a convenience and must not fail the rescan.
    async fn suggest_moves(
        &self,
        ws: &Workspace,
        untracked: &[WorkspacePreviewEntry],
    ) -> Vec<SuggestedMove> {
        let targets: Vec<&str> = untracked
            .iter()
            .filter(|e| e.existing_repository_id.is_none())
            .filter_map(|e| e.repository_root.as_deref())
            .take(MAX_SUGGESTION_TARGETS)
            .collect();
        let mut missing = Vec::new();
        for m in &ws.members {
            if missing.len() == MAX_SUGGESTED_MEMBERS {
                break;
            }
            let Some(id) = m.repository_id.as_deref() else {
                continue;
            };
            if m.status != MemberStatus::Missing {
                continue;
            }
            let Ok(row) = self.row(id).await else {
                continue;
            };
            match self.evidence(&row).await {
                Ok(evidence) if !evidence.is_empty() => missing.push((m, evidence)),
                _ => {}
            }
        }
        if targets.is_empty() || missing.is_empty() {
            return Vec::new();
        }
        let mut all: Vec<String> = missing.iter().flat_map(|(_, e)| e.clone()).collect();
        all.sort();
        all.dedup();

        let mut matches: Vec<(usize, usize)> = Vec::new();
        for (ti, target) in targets.iter().enumerate() {
            let target = Path::new(target);
            if !matches!(self.compare(&all, target).await, Ok(History::Same)) {
                continue;
            }
            for (mi, (_, evidence)) in missing.iter().enumerate() {
                if let Ok(History::Same) = self.compare(evidence, target).await {
                    matches.push((mi, ti));
                }
            }
        }
        // `HashMap::entry(..).or_default()` counts occurrences in one pass.
        let mut per_member: HashMap<usize, usize> = HashMap::new();
        let mut per_target: HashMap<usize, usize> = HashMap::new();
        for (mi, ti) in &matches {
            *per_member.entry(*mi).or_default() += 1;
            *per_target.entry(*ti).or_default() += 1;
        }
        matches
            .into_iter()
            .filter(|(mi, ti)| per_member[mi] == 1 && per_target[ti] == 1)
            .map(|(mi, ti)| {
                let member = missing[mi].0;
                SuggestedMove {
                    repository_id: member.repository_id.clone().unwrap_or_default(),
                    name: member.display_name.clone(),
                    from: member.canonical_path.clone(),
                    to: targets[ti].to_string(),
                }
            })
            .collect()
    }
}

/// The highest folder that is gone among `old_root` and its ancestors whose
/// names `new_root` and its ancestors share, paired with what it became.
/// `None` when `old_root` itself still exists.
fn moved_folder(old_root: &Path, new_root: &Path) -> Option<Along> {
    let (mut from, mut to) = (old_root, new_root);
    let mut found = None;
    loop {
        if from.exists() {
            break;
        }
        found = Some(Along {
            from: from.to_path_buf(),
            to: to.to_path_buf(),
        });
        match (from.parent(), to.parent()) {
            (Some(fp), Some(tp)) if from.file_name() == to.file_name() && fp != tp => {
                from = fp;
                to = tp;
            }
            _ => break,
        }
    }
    found
}

/// Write the moves: registrations, the linked worktrees of Git directories
/// that moved, discovery folders and plain members inside the folder that
/// moved, and memberships.
fn apply(conn: &rusqlite::Connection, moves: &[Move], along: Option<&Along>) -> AppResult<Applied> {
    let rows = db::list_repositories(conn)?;
    let moving: HashSet<&str> = moves.iter().map(|m| m.row.id.as_str()).collect();
    // Checked again inside the transaction: another relocation or
    // registration may have changed things since the plan was made.
    for m in moves {
        let current = rows
            .iter()
            .find(|r| r.id == m.row.id)
            .ok_or_else(|| AppError::not_found("That repository is no longer registered."))?;
        if current.canonical_root != m.row.canonical_root
            || current.common_git_dir != m.row.common_git_dir
        {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "This repository was relocated in the meantime. Try again.",
            ));
        }
        let root = m.to.root.display().to_string();
        if rows
            .iter()
            .any(|r| r.display_path == root && !moving.contains(r.id.as_str()))
        {
            return Err(already_tracked(&root));
        }
    }

    let mut applied = Applied::default();
    let mut new_paths: HashMap<&str, &Path> = HashMap::new();
    for m in moves {
        let location = Location {
            root: m.to.root.display().to_string(),
            git_dir: m.to.git_dir.display().to_string(),
            common_git_dir: m.to.common_git_dir.display().to_string(),
        };
        let same = m.history == History::Same;
        db::set_repository_location(conn, &m.row.id, &location, !same)?;
        applied.touched.push(m.row.id.clone());
        new_paths.insert(&m.row.id, &m.to.root);

        let (from, to) = (&m.row.common_git_dir, &location.common_git_dir);
        if from == to
            || applied.moved_stores.iter().any(|(f, _)| f == from)
            || applied.dropped_stores.contains(from)
        {
            continue;
        }
        let siblings: Vec<&RepositoryRow> = rows
            .iter()
            .filter(|r| &r.common_git_dir == from && !moving.contains(r.id.as_str()))
            .collect();
        let target_in_use = rows
            .iter()
            .any(|r| &r.common_git_dir == to && !moving.contains(r.id.as_str()));
        // The Git directory itself moved: a main checkout's, which is gone
        // from its old place. Its linked worktrees point into it.
        let directory_moved = same
            && !target_in_use
            && m.row.git_dir == m.row.common_git_dir
            && !Path::new(from).exists();
        if directory_moved {
            for wt in &siblings {
                let git_dir = match Path::new(&wt.git_dir).strip_prefix(from) {
                    Ok(rel) if !rel.as_os_str().is_empty() => {
                        Path::new(to).join(rel).display().to_string()
                    }
                    _ => to.clone(),
                };
                let location = Location {
                    root: wt.canonical_root.clone(),
                    git_dir,
                    common_git_dir: to.clone(),
                };
                db::set_repository_location(conn, &wt.id, &location, false)?;
                applied.touched.push(wt.id.clone());
            }
        }
        // Activity follows the registration when nothing else uses the old
        // Git directory; otherwise it stays with the registrations still there.
        if directory_moved || siblings.is_empty() {
            if same && !target_in_use {
                applied.moved_stores.push((from.clone(), to.clone()));
            } else {
                applied.dropped_stores.push(from.clone());
            }
        }
    }

    let workspaces = db::list_workspaces(conn)?;
    let mut scopes: HashMap<String, Option<DiscoveryScope>> = HashMap::new();
    for mut ws in workspaces {
        let moved_root = ws
            .discovery_root
            .as_deref()
            .filter(|root| !Path::new(root).exists())
            .and_then(|root| along.and_then(|a| a.map(Path::new(root))))
            .filter(|p| p.is_dir());
        if let Some(root) = moved_root {
            ws.discovery_root = Some(root.display().to_string());
            db::set_discovery_root(conn, &ws.id, &root.display().to_string())?;
        }
        // A root repository that now lives elsewhere than the discovery
        // folder is an ordinary member.
        if let Some(root_id) = ws.root_repository_id.as_deref() {
            if let Some(path) = new_paths.get(root_id) {
                if ws.discovery_root.as_deref().map(Path::new) != Some(*path) {
                    db::set_workspace_root(conn, &ws.id, None)?;
                }
            }
        }
        scopes.insert(ws.id.clone(), DiscoveryScope::of(&ws));
    }

    for member in db::list_members(conn, None)? {
        let path = match &member.repository_id {
            Some(id) => match new_paths.get(id.as_str()) {
                Some(p) => p.to_path_buf(),
                None => continue,
            },
            None => {
                let old = Path::new(&member.canonical_path);
                match along
                    .filter(|_| !old.exists())
                    .and_then(|a| a.map(old))
                    .filter(|p| p.is_dir())
                {
                    Some(p) => p,
                    None => continue,
                }
            }
        };
        let origin = scopes
            .get(&member.workspace_id)
            .and_then(Option::as_ref)
            .map_or(MemberOrigin::Manual, |s| s.origin_of(&path));
        db::move_member(
            conn,
            &member.id,
            &path.display().to_string(),
            &folder_name(&path),
            origin,
        )?;
    }
    Ok(applied)
}

fn already_tracked(root: &str) -> AppError {
    AppError::new(
        ErrorCode::Conflict,
        format!(
            "{root} is already tracked as another repository. Remove one of the two registrations first."
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_moved_folder_is_the_highest_shared_name_that_is_gone() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();
        std::fs::create_dir_all(base.join("src/web")).unwrap();
        std::fs::create_dir_all(base.join("code/api-v2")).unwrap();

        // `code/web` → `src/web`, with `code` gone: `code` became `src`.
        let gone = base.join("gone");
        let along = moved_folder(&gone.join("web"), &base.join("src/web")).unwrap();
        assert_eq!((along.from, along.to), (gone, base.join("src")));

        // A rename inside a folder that still exists moves only the folder.
        let along = moved_folder(&base.join("code/api"), &base.join("code/api-v2")).unwrap();
        assert_eq!(along.from, base.join("code/api"));

        // Nothing moves along when the old folder still exists.
        assert!(moved_folder(&base.join("src/web"), &base.join("code/api-v2")).is_none());
    }
}
