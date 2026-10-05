//! Settings → Secrets (SPEC.md, Secrets): every account, connection, and
//! agent profile with a secret, where it comes from, and its state. Listing reads no secret,
//! runs no program, and unlocks no store; the actions are routed to the
//! domain service that owns the account or connection.

use std::sync::Arc;

use crate::agents::AgentSettingsService;
use crate::credentials::CredentialService;
use crate::databases::ConnectionService;
use crate::forge::AccountService;
use crate::models::{AppResult, CredentialOwner, SecretsOverview};

pub struct SecretsService {
    credentials: Arc<CredentialService>,
    accounts: Arc<AccountService>,
    connections: Arc<ConnectionService>,
    agents: Arc<AgentSettingsService>,
}

impl SecretsService {
    pub fn new(
        credentials: Arc<CredentialService>,
        accounts: Arc<AccountService>,
        connections: Arc<ConnectionService>,
        agents: Arc<AgentSettingsService>,
    ) -> Self {
        SecretsService {
            credentials,
            accounts,
            connections,
            agents,
        }
    }

    pub async fn overview(&self) -> AppResult<SecretsOverview> {
        let mut entries = self.accounts.secret_entries().await?;
        entries.extend(self.connections.secret_entries().await?);
        entries.extend(self.agents.secret_entries().await?);
        Ok(SecretsOverview {
            store: self.credentials.store_name().to_string(),
            entries,
        })
    }

    /// **Allow This Source** for a restored source, at the revision shown.
    pub async fn approve(&self, owner: &CredentialOwner, revision: i64) -> AppResult<()> {
        match owner {
            CredentialOwner::ForgeAccount { provider } => {
                self.accounts.approve(*provider, revision).await
            }
            CredentialOwner::DbConnection { id } => {
                self.connections.approve(id, revision).await.map(|_| ())
            }
            CredentialOwner::AgentProfile { id } => {
                self.agents.approve(id, revision).await.map(|_| ())
            }
        }
    }

    /// **Refresh**: the next use reads the source, or asks, again.
    pub fn refresh(&self, owner: &CredentialOwner) {
        match owner {
            CredentialOwner::ForgeAccount { provider } => self.accounts.refresh(*provider),
            CredentialOwner::DbConnection { id } => self.connections.refresh(id),
            CredentialOwner::AgentProfile { id } => self.agents.refresh(id),
        }
    }

    /// **Retry** a cleanup or removal that did not finish.
    pub async fn retry(&self, owner: &CredentialOwner) -> AppResult<()> {
        match owner {
            CredentialOwner::ForgeAccount { provider } => {
                self.accounts.retry_cleanup(*provider).await
            }
            CredentialOwner::DbConnection { id } => self.connections.retry_cleanup(id).await,
            CredentialOwner::AgentProfile { id } => self.agents.retry_cleanup(id).await,
        }
    }
}
