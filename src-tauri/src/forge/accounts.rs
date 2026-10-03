//! Settings → Accounts (SPEC.md, Accounts): one GitHub and one Bitbucket
//! Cloud account, each checked with one request when it is added or its token
//! replaced. The token goes to the Keychain only after the check passes, and
//! nothing about it but what the check found is stored.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::http::Http;
use super::keychain::{Keychain, Token};
use super::{bitbucket, github, store};
use crate::db::Db;
use crate::models::{
    now_rfc3339, AppError, AppResult, ForgeAccount, ForgeAccountSlot, ForgeKind, ForgeTokenKind,
    SaveForgeAccountOutcome, SaveForgeAccountRequest,
};

const KINDS: [ForgeKind; 2] = [ForgeKind::Github, ForgeKind::BitbucketCloud];

/// What a provider said about a token: whose it is and what it cannot do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountCheck {
    pub login: String,
    /// The provider's stable ID for the user.
    pub user_id: String,
    pub display_name: Option<String>,
    pub token_kind: ForgeTokenKind,
    pub expires_at: Option<String>,
    pub scopes: Option<Vec<String>>,
    /// What the token lacks for writing; empty when it can write, or when
    /// the provider does not say (GitHub fine-grained tokens).
    pub missing: Vec<String>,
}

/// The API base of each provider. Always `production()` in the app; tests
/// point them at a local server.
#[derive(Debug, Clone)]
pub struct Endpoints {
    pub github: String,
    pub bitbucket: String,
}

impl Endpoints {
    pub fn production() -> Self {
        Endpoints {
            github: github::API.to_string(),
            bitbucket: bitbucket::API.to_string(),
        }
    }
}

pub struct AccountService {
    db: Db,
    // `Arc<dyn Keychain>`: the real Keychain in the app, memory in tests,
    // shared with the blocking threads that call it.
    keychain: Arc<dyn Keychain>,
    http: Http,
    endpoints: Endpoints,
    /// Tokens read once per run: the Keychain may ask the user to allow each read.
    tokens: Mutex<HashMap<ForgeKind, Token>>,
}

impl AccountService {
    pub fn new(db: Db, keychain: Arc<dyn Keychain>, http: Http, endpoints: Endpoints) -> Self {
        AccountService {
            db,
            keychain,
            http,
            endpoints,
            tokens: Mutex::new(HashMap::new()),
        }
    }

    /// The provider's token, for the adapters only.
    pub async fn token(&self, kind: ForgeKind) -> AppResult<Token> {
        if let Some(token) = self.tokens.lock().expect("tokens lock").get(&kind) {
            return Ok(token.clone());
        }
        let token = self.keychain(move |k| k.get(kind)).await?.ok_or_else(|| {
            AppError::new(
                crate::models::ErrorCode::PermissionDenied,
                format!(
                    "The {} account's token is no longer in the Keychain. Replace it in Settings → Accounts.",
                    kind.label()
                ),
            )
        })?;
        self.tokens
            .lock()
            .expect("tokens lock")
            .insert(kind, token.clone());
        Ok(token)
    }

    /// Run a Keychain call on a blocking thread: it may wait for the user to
    /// answer macOS's prompt.
    async fn keychain<T, F>(&self, f: F) -> AppResult<T>
    where
        T: Send + 'static,
        F: FnOnce(&dyn Keychain) -> AppResult<T> + Send + 'static,
    {
        let keychain = Arc::clone(&self.keychain);
        tokio::task::spawn_blocking(move || f(keychain.as_ref()))
            .await
            .map_err(|e| {
                AppError::io("The Keychain call did not finish.").with_details(e.to_string())
            })?
    }

    /// One entry per provider, with its account or whether the Keychain
    /// already holds a token for one.
    pub async fn list(&self) -> AppResult<Vec<ForgeAccountSlot>> {
        let accounts = self.db.call(|conn| store::list(conn)).await?;
        let mut slots = Vec::new();
        for kind in KINDS {
            let account = accounts.iter().find(|a| a.kind == kind).cloned();
            let keychain_token = match account {
                Some(_) => false,
                None => self
                    .keychain(move |k| k.contains(kind))
                    .await
                    .unwrap_or_else(|e| {
                        tracing::warn!(error = %e, "cannot look for a token in the Keychain");
                        false
                    }),
            };
            slots.push(ForgeAccountSlot {
                kind,
                account,
                keychain_token,
            });
        }
        Ok(slots)
    }

    /// The provider's account, if one was added.
    pub async fn account(&self, kind: ForgeKind) -> AppResult<Option<ForgeAccount>> {
        self.db.call(move |conn| store::get(conn, kind)).await
    }

    /// Add an account or replace its token: check the token with one request,
    /// then keep the token in the Keychain and what the check found here. A
    /// token that cannot write is saved only when the request says read-only.
    pub async fn save(
        &self,
        request: SaveForgeAccountRequest,
    ) -> AppResult<SaveForgeAccountOutcome> {
        let kind = request.kind;
        let stored = self.account(kind).await?;
        let email = match kind {
            ForgeKind::Github => None,
            ForgeKind::BitbucketCloud => {
                let email = request
                    .email
                    .as_deref()
                    .map(str::trim)
                    .filter(|e| !e.is_empty())
                    .map(str::to_string)
                    .or_else(|| stored.as_ref().and_then(|a| a.email.clone()))
                    .ok_or_else(|| {
                        AppError::validation("Enter the email of your Atlassian account.")
                    })?;
                if !email.contains('@') || email.chars().any(char::is_whitespace) {
                    return Err(AppError::validation(format!(
                        "{email} is not an email address."
                    )));
                }
                Some(email)
            }
        };
        // `Option<Token>`: `Some` when pasted, `None` when it is already in the Keychain.
        let pasted = request.token.as_deref().map(Token::new).transpose()?;
        let token = match &pasted {
            Some(token) => token.clone(),
            None => self.keychain(move |k| k.get(kind)).await?.ok_or_else(|| {
                AppError::not_found(format!(
                    "The Keychain has no {} token to use. Paste one.",
                    kind.label()
                ))
            })?,
        };

        let check = match kind {
            ForgeKind::Github => {
                github::check_account(&self.http, &self.endpoints.github, &token).await?
            }
            ForgeKind::BitbucketCloud => {
                let email = email.as_deref().unwrap_or_default();
                bitbucket::check_account(&self.http, &self.endpoints.bitbucket, email, &token)
                    .await?
            }
        };
        if !check.missing.is_empty() && !request.read_only {
            return Ok(SaveForgeAccountOutcome::ReadOnly {
                login: check.login,
                missing: check.missing,
            });
        }

        if let Some(token) = pasted {
            self.keychain(move |k| k.set(kind, &token)).await?;
        }
        self.tokens
            .lock()
            .expect("tokens lock")
            .insert(kind, token.clone());
        let read_only = !check.missing.is_empty();
        let now = now_rfc3339();
        let account = self
            .db
            .call(move |conn| {
                store::save(conn, kind, &check, email.as_deref(), read_only, &now)?;
                store::get(conn, kind)
            })
            .await?
            .ok_or_else(|| AppError::db("The account was not saved."))?;
        tracing::info!(provider = kind.label(), login = %account.login, read_only, "account saved");
        Ok(SaveForgeAccountOutcome::Saved { account })
    }

    /// Remove the provider's account and delete its token from the Keychain.
    pub async fn remove(&self, kind: ForgeKind) -> AppResult<Vec<ForgeAccountSlot>> {
        self.keychain(move |k| k.delete(kind)).await?;
        self.tokens.lock().expect("tokens lock").remove(&kind);
        self.db.call(move |conn| store::delete(conn, kind)).await?;
        tracing::info!(provider = kind.label(), "account removed");
        self.list().await
    }
}
