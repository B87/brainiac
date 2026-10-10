//! Where pull requests come from (SPEC.md, Which pull requests a repository
//! has): each repository's forge repository, derived from `origin` unless
//! chosen with **Change…**, and each workspace's pull request switch.

use crate::db;
use crate::forge::ForgeRepository;
use crate::models::{
    now_rfc3339, AppResult, ChangeOrigin, RepositoryChangedEvent, RepositorySummary,
    SetRepositoryForgeRequest, Workspace,
};
use crate::workspaces::RepositoryService;

impl RepositoryService {
    /// Point a repository's pull requests at another hosted repository, or
    /// back at its `origin`. The choice outlives changes to `origin`.
    pub async fn set_forge(
        &self,
        request: SetRepositoryForgeRequest,
    ) -> AppResult<RepositorySummary> {
        let forge = request
            .forge
            .as_ref()
            .map(ForgeRepository::from_target)
            .transpose()?;
        let id = request.repository_id.clone();
        self.row(&id).await?;
        let now = now_rfc3339();
        let id2 = id.clone();
        self.db
            .call(move |conn| db::set_repository_forge(conn, &id2, forge.as_ref(), &now))
            .await?;
        let version = self.bump_version();
        (self.emitter)(&RepositoryChangedEvent {
            repository_id: id.clone(),
            snapshot_version: version,
            origin: ChangeOrigin::Refresh,
            changed: true,
        });
        let row = self.row(&id).await?;
        Ok(self.summarize(&row, &self.settings(), false))
    }

    /// Turn a workspace's pull request tracking on or off.
    pub async fn set_pull_requests(
        &self,
        workspace_id: &str,
        enabled: bool,
    ) -> AppResult<Workspace> {
        let id = workspace_id.to_string();
        let found = self
            .db
            .call(move |conn| db::set_workspace_pull_requests(conn, &id, enabled))
            .await?;
        if !found {
            return Err(crate::models::AppError::not_found(
                "That workspace no longer exists.",
            ));
        }
        self.bump_version();
        self.workspace(workspace_id).await
    }

    /// The repositories whose pull requests are tracked, with their forge
    /// repository; members of workspaces with pull requests on only.
    pub async fn tracked_forges(&self) -> AppResult<Vec<(db::RepositoryRow, ForgeRepository)>> {
        let rows = self.db.call(|conn| db::tracked_repositories(conn)).await?;
        Ok(rows
            .into_iter()
            .filter_map(|row| {
                let forge = match &row.forge_override {
                    Some(f) => f.clone(),
                    None => ForgeRepository::from_remote_url(row.remote_url.as_deref()?)?,
                };
                Some((row, forge))
            })
            .collect())
    }
}
