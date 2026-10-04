//! Settings → Accounts (SPEC.md, Accounts): one GitHub and one Bitbucket
//! Cloud account, each checked with one request when it is added or its
//! token replaced. The token comes from the account's source (SPEC.md,
//! Secrets): the Keychain, where it is stored only after the check passes,
//! an environment variable, or a command. Nothing about it but what the
//! check found is stored here.
//!
//! A token read afresh is checked again before it is used: it must still
//! belong to the saved account's user, so a command whose tool switched
//! accounts cannot switch Brainiac's silently.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::http::Http;
use super::{bitbucket, github, store};
use crate::credentials::{
    describe, Binding, CredentialService, Lease, LeaseHandle, OwnerGate, Probe, Resolution,
    SecretBytes, Token,
};
use crate::db::Db;
use crate::models::{
    now_rfc3339, AppError, AppResult, CredentialOwner, CredentialPending, CredentialTest,
    ErrorCode, ForgeAccount, ForgeAccountSlot, ForgeAccountTestResult, ForgeKind, ForgeTokenKind,
    SaveForgeAccountOutcome, SaveForgeAccountRequest, SecretEntry, SecretSource,
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

fn binding(account: &ForgeAccount) -> Binding {
    Binding {
        owner: account.kind.keychain_account().to_string(),
        source: account.token_source.clone(),
        revision: account.credential.revision,
        approved: !account.credential.needs_approval,
        pending: account.credential.pending,
        expires_at: account
            .expires_at
            .as_deref()
            .and_then(|e| chrono::DateTime::parse_from_rfc3339(e).ok())
            .map(|e| e.with_timezone(&chrono::Utc)),
    }
}

fn expiry(check: &AccountCheck) -> Option<chrono::DateTime<chrono::Utc>> {
    check
        .expires_at
        .as_deref()
        .and_then(|e| chrono::DateTime::parse_from_rfc3339(e).ok())
        .map(|e| e.with_timezone(&chrono::Utc))
}

/// Accounts take their token from the Keychain, a variable, or a command.
fn account_source(source: &SecretSource) -> AppResult<SecretSource> {
    match source {
        SecretSource::Ask | SecretSource::None => Err(AppError::validation(
            "An account's token comes from the Keychain, an environment variable, or a command.",
        )),
        other => crate::credentials::check_source(other),
    }
}

fn no_account(kind: ForgeKind) -> AppError {
    AppError::new(
        ErrorCode::PermissionDenied,
        format!(
            "No {} account. Add one in Settings → Accounts.",
            kind.label()
        ),
    )
}

/// A save cut off after its marker: the account stays blocked until saved again.
fn partial_save(e: AppError) -> AppError {
    AppError::new(
        e.code,
        format!(
            "{} The account cannot be used until its token is saved again.",
            e.message
        ),
    )
}

pub struct AccountService {
    db: Db,
    // `Arc` because accounts and connections share one credentials layer,
    // and adapters hold a handle to report a refused token.
    credentials: Arc<CredentialService>,
    http: Http,
    endpoints: Endpoints,
    /// The lease each provider's token was last confirmed for: the token
    /// belongs to the saved account's user.
    verified: Mutex<HashMap<ForgeKind, Lease>>,
    /// One identity check at a time, so callers that need a token at once
    /// share one request.
    checking: tokio::sync::Mutex<()>,
}

impl AccountService {
    pub fn new(
        db: Db,
        credentials: Arc<CredentialService>,
        http: Http,
        endpoints: Endpoints,
    ) -> Self {
        AccountService {
            db,
            credentials,
            http,
            endpoints,
            verified: Mutex::new(HashMap::new()),
            checking: tokio::sync::Mutex::new(()),
        }
    }

    fn is_verified(&self, kind: ForgeKind, lease: &Lease) -> bool {
        self.verified
            .lock()
            .expect("verified lock")
            .get(&kind)
            .is_some_and(|v| v.same_read(lease))
    }

    /// The provider's token, for the adapters only, with the lease it came
    /// from so a refusal forgets exactly it.
    pub async fn token(&self, kind: ForgeKind) -> AppResult<(Token, LeaseHandle)> {
        for _ in 0..2 {
            let account = self.account(kind).await?.ok_or_else(|| no_account(kind))?;
            let binding = binding(&account);
            let source = describe(&account.token_source, &binding.owner);
            let resolved = self.credentials.resolve(&binding).await.map_err(|e| {
                if e.code == ErrorCode::NotFound && account.token_source == SecretSource::Store {
                    AppError::new(
                        ErrorCode::PermissionDenied,
                        format!(
                            "The {} account's token is no longer in the Keychain. Replace it in Settings → Accounts.",
                            kind.label()
                        ),
                    )
                } else {
                    e
                }
            })?;
            let Resolution::Secret(lease) = resolved else {
                return Err(AppError::validation("This account has no token source."));
            };
            let handle = LeaseHandle::new(Arc::clone(&self.credentials), lease);
            let token = match Token::from_bytes(handle.lease().bytes(), &source) {
                Ok(token) => token,
                Err(e) => {
                    handle.reject();
                    return Err(e);
                }
            };
            if self.is_verified(kind, handle.lease()) {
                return Ok((token, handle));
            }
            let _checking = self.checking.lock().await;
            if self.is_verified(kind, handle.lease()) {
                return Ok((token, handle));
            }
            if !handle.is_current() {
                continue;
            }
            let check = match self.check(kind, account.email.as_deref(), &token).await {
                Ok(check) => check,
                Err(e) => {
                    if e.code == ErrorCode::Unauthenticated {
                        handle.reject();
                    }
                    return Err(e);
                }
            };
            if check.user_id != account.user_id {
                return Err(AppError::new(
                    ErrorCode::PermissionDenied,
                    format!(
                        "The token from {source} belongs to {}, not to the saved {} account {}. Make it give {}'s token, or replace the account in Settings → Accounts.",
                        check.login,
                        kind.label(),
                        account.login,
                        account.login
                    ),
                ));
            }
            let (revision, now) = (handle.lease().revision(), now_rfc3339());
            let gate = self.credentials.gate(&binding.owner).await;
            let refreshed = self
                .db
                .call(move |conn| store::refresh_check(conn, kind, &check, revision, &now))
                .await;
            drop(gate);
            if let Err(e) = refreshed {
                tracing::warn!(error = %e, "could not record what the token check found");
            }
            if handle.is_current() {
                self.verified
                    .lock()
                    .expect("verified lock")
                    .insert(kind, handle.lease().clone());
            }
            return Ok((token, handle));
        }
        Err(AppError::new(
            ErrorCode::Conflict,
            "The account's token changed while it was being read. Try again.",
        ))
    }

    async fn check(
        &self,
        kind: ForgeKind,
        email: Option<&str>,
        token: &Token,
    ) -> AppResult<AccountCheck> {
        match kind {
            ForgeKind::Github => {
                github::check_account(&self.http, &self.endpoints.github, token).await
            }
            ForgeKind::BitbucketCloud => {
                bitbucket::check_account(
                    &self.http,
                    &self.endpoints.bitbucket,
                    email.unwrap_or_default(),
                    token,
                )
                .await
            }
        }
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
                    .credentials
                    .item_exists(kind.keychain_account())
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

    /// The email a Bitbucket request uses: typed, else the saved one.
    fn email(
        kind: ForgeKind,
        request: &SaveForgeAccountRequest,
        stored: Option<&ForgeAccount>,
    ) -> AppResult<Option<String>> {
        match kind {
            ForgeKind::Github => Ok(None),
            ForgeKind::BitbucketCloud => {
                let email = request
                    .email
                    .as_deref()
                    .map(str::trim)
                    .filter(|e| !e.is_empty())
                    .map(str::to_string)
                    .or_else(|| stored.and_then(|a| a.email.clone()))
                    .ok_or_else(|| {
                        AppError::validation("Enter the email of your Atlassian account.")
                    })?;
                if !email.contains('@') || email.chars().any(char::is_whitespace) {
                    return Err(AppError::validation(format!(
                        "{email} is not an email address."
                    )));
                }
                Ok(Some(email))
            }
        }
    }

    /// The token a form names: pasted, the Keychain item, or read afresh
    /// from the form's source. Nothing is cached.
    async fn draft_token(
        &self,
        kind: ForgeKind,
        source: &SecretSource,
        pasted: Option<&Token>,
        stored: Option<&ForgeAccount>,
    ) -> AppResult<Token> {
        if let Some(token) = pasted {
            return Ok(token.clone());
        }
        let owner = kind.keychain_account();
        if *source == SecretSource::Store
            && stored.is_some_and(|s| s.credential.pending == Some(CredentialPending::Save))
        {
            return Err(AppError::validation(
                "The last save of this token did not finish. Paste the token again.",
            ));
        }
        let read = self.credentials.probe(owner, source).await.map_err(|e| {
            if e.code == ErrorCode::NotFound && *source == SecretSource::Store {
                AppError::not_found(format!(
                    "The Keychain has no {} token to use. Paste one.",
                    kind.label()
                ))
            } else {
                e
            }
        })?;
        match read {
            Probe::Secret(bytes) => Token::from_bytes(&bytes, &describe(source, owner)),
            _ => Err(AppError::validation("Choose where the token comes from.")),
        }
    }

    /// Test on the account form: check the form's token with the provider
    /// without saving anything.
    pub async fn test(
        &self,
        request: SaveForgeAccountRequest,
    ) -> AppResult<ForgeAccountTestResult> {
        let kind = request.kind;
        let source = account_source(&request.source)?;
        let stored = self.account(kind).await?;
        let email = Self::email(kind, &request, stored.as_ref())?;
        let pasted = match source {
            SecretSource::Store => request.token.as_deref().map(Token::new).transpose()?,
            _ => None,
        };
        let saved = stored
            .as_ref()
            .filter(|s| pasted.is_none() && s.token_source == source);
        if saved.is_some_and(|s| s.credential.needs_approval) {
            return Err(AppError::new(
                ErrorCode::PermissionDenied,
                "This account's token source came from a restored backup. Allow it in Settings → Secrets before testing it.",
            ));
        }
        let result = async {
            let token = self
                .draft_token(kind, &source, pasted.as_ref(), stored.as_ref())
                .await?;
            self.check(kind, email.as_deref(), &token).await
        }
        .await;
        if let Some(s) = saved {
            self.credentials.record_test(
                kind.keychain_account(),
                s.credential.revision,
                CredentialTest {
                    at: now_rfc3339(),
                    ok: result.as_ref().is_ok_and(|c| c.user_id == s.user_id),
                    message: match &result {
                        Ok(c) if c.user_id != s.user_id => {
                            format!("The token belongs to {}, not to {}.", c.login, s.login)
                        }
                        Ok(c) if c.missing.is_empty() => format!("Belongs to {}.", c.login),
                        Ok(c) => format!(
                            "Belongs to {}; read only, missing {}.",
                            c.login,
                            c.missing.join(", ")
                        ),
                        Err(e) => e.message.clone(),
                    },
                },
            );
        }
        let check = result?;
        Ok(ForgeAccountTestResult {
            login: check.login,
            missing: check.missing,
        })
    }

    /// Add an account or replace its token: check the token with one
    /// request, then keep it where its source says and what the check found
    /// here (docs/architecture.md, Credentials: saving). A token that cannot
    /// write is saved only when the request says read-only.
    pub async fn save(
        &self,
        request: SaveForgeAccountRequest,
    ) -> AppResult<SaveForgeAccountOutcome> {
        let kind = request.kind;
        let source = account_source(&request.source)?;
        let gate = self.credentials.gate(kind.keychain_account()).await;
        let stored = self.account(kind).await?;
        if stored
            .as_ref()
            .is_some_and(|s| s.credential.pending == Some(CredentialPending::Removal))
        {
            return Err(AppError::validation(
                "This account is being removed. Retry the removal in Settings → Secrets first.",
            ));
        }
        let email = Self::email(kind, &request, stored.as_ref())?;
        // `Option<Token>`: `Some` when pasted, `None` when read from the source.
        let pasted = match source {
            SecretSource::Store => request.token.as_deref().map(Token::new).transpose()?,
            _ => None,
        };
        let token = self
            .draft_token(kind, &source, pasted.as_ref(), stored.as_ref())
            .await?;
        let check = self.check(kind, email.as_deref(), &token).await?;
        if !check.missing.is_empty() && !request.read_only {
            return Ok(SaveForgeAccountOutcome::ReadOnly {
                login: check.login,
                missing: check.missing,
            });
        }

        let read_only = !check.missing.is_empty();
        let old_source = stored.as_ref().map(|s| s.token_source.clone());
        let old_pending = stored.as_ref().and_then(|s| s.credential.pending);
        // A save always binds anew: the check may have found another user.
        let revision = stored.as_ref().map_or(0, |s| s.credential.revision) + 1;
        let cleanup = source != SecretSource::Store
            && (old_source == Some(SecretSource::Store)
                || matches!(
                    old_pending,
                    Some(CredentialPending::Save | CredentialPending::Cleanup)
                ));
        let expires_at = expiry(&check);

        if pasted.is_some() {
            // Marker first: a save cut off before the row is committed leaves
            // a blocked account, never one that looks usable.
            let (marker_check, marker_email, marker_source) =
                (check.clone(), email.clone(), source.clone());
            let now = now_rfc3339();
            let existing = stored.is_some();
            self.db
                .call(move |conn| {
                    if existing {
                        store::mark_pending(conn, kind, CredentialPending::Save)
                    } else {
                        store::save(
                            conn,
                            kind,
                            &store::Saved {
                                check: &marker_check,
                                email: marker_email.as_deref(),
                                read_only,
                                source: &marker_source,
                                revision: 0,
                                pending: Some(CredentialPending::Save),
                                now: &now,
                            },
                        )
                    }
                })
                .await?;
            self.credentials.begin(&gate);
            self.credentials
                .write_item(&gate, SecretBytes::from_text(token.expose()))
                .await
                .map_err(partial_save)?;
        }
        let (saved_source, now) = (source.clone(), now_rfc3339());
        let committed = self
            .db
            .call(move |conn| {
                store::save(
                    conn,
                    kind,
                    &store::Saved {
                        check: &check,
                        email: email.as_deref(),
                        read_only,
                        source: &saved_source,
                        revision,
                        pending: cleanup.then_some(CredentialPending::Cleanup),
                        now: &now,
                    },
                )?;
                store::get(conn, kind)
            })
            .await;
        let account = match committed {
            Ok(Some(account)) => account,
            Ok(None) => return Err(AppError::db("The account was not saved.")),
            Err(e) if pasted.is_some() => return Err(partial_save(e)),
            Err(e) => return Err(e),
        };
        self.credentials.commit(&gate, revision);
        if let Some(lease) = self.credentials.prime(
            &gate,
            revision,
            SecretBytes::from_text(token.expose()),
            expires_at,
        ) {
            self.verified
                .lock()
                .expect("verified lock")
                .insert(kind, lease);
        }
        let warning = if cleanup {
            self.cleanup(&gate, kind).await.err().map(|e| {
                format!(
                    "The account now reads its token from {}, but its old Keychain item could not be deleted: {} Retry in Settings → Secrets.",
                    describe(&source, kind.keychain_account()),
                    e.message
                )
            })
        } else {
            None
        };
        drop(gate);
        let account = self.account(kind).await?.unwrap_or(account);
        tracing::info!(provider = kind.label(), login = %account.login, read_only, "account saved");
        Ok(SaveForgeAccountOutcome::Saved {
            account: Box::new(account),
            warning,
        })
    }

    /// Delete the old Keychain item after a move to another source.
    async fn cleanup(&self, gate: &OwnerGate, kind: ForgeKind) -> AppResult<()> {
        self.credentials.delete_item(gate).await?;
        self.db
            .call(move |conn| store::clear_pending(conn, kind, CredentialPending::Cleanup))
            .await
    }

    /// The provider refused an action for a permission the token lacks: keep
    /// that with the account, so the action is off until the token is
    /// replaced (SPEC.md, Accounts: GitHub shows no fine-grained permissions).
    pub async fn mark_missing(&self, kind: ForgeKind, permission: &str) -> AppResult<()> {
        let permission = permission.to_string();
        tracing::info!(
            provider = kind.label(),
            permission,
            "the token lacks a permission"
        );
        self.db
            .call(move |conn| store::add_missing(conn, kind, &permission))
            .await
    }

    /// Remove the provider's account: marked as being removed, its Keychain
    /// item deleted (whatever the source is now), then the row. A failed
    /// deletion leaves the removal pending, for Settings → Secrets to retry.
    pub async fn remove(&self, kind: ForgeKind) -> AppResult<Vec<ForgeAccountSlot>> {
        let gate = self.credentials.gate(kind.keychain_account()).await;
        if self.account(kind).await?.is_some() {
            self.db
                .call(move |conn| store::mark_pending(conn, kind, CredentialPending::Removal))
                .await?;
        }
        self.credentials.begin(&gate);
        self.finish_removal(&gate, kind).await?;
        drop(gate);
        tracing::info!(provider = kind.label(), "account removed");
        self.list().await
    }

    async fn finish_removal(&self, gate: &OwnerGate, kind: ForgeKind) -> AppResult<()> {
        self.credentials.delete_item(gate).await.map_err(|e| {
            AppError::new(
                e.code,
                format!(
                    "The account is no longer used, but its Keychain item could not be deleted: {} Retry in Settings → Secrets.",
                    e.message
                ),
            )
        })?;
        self.db.call(move |conn| store::delete(conn, kind)).await?;
        self.credentials.forget(gate);
        self.verified.lock().expect("verified lock").remove(&kind);
        Ok(())
    }

    /// Settings → Secrets, **Retry**: finish a pending cleanup or removal.
    pub async fn retry_cleanup(&self, kind: ForgeKind) -> AppResult<()> {
        let gate = self.credentials.gate(kind.keychain_account()).await;
        let Some(account) = self.account(kind).await? else {
            return Ok(());
        };
        match account.credential.pending {
            Some(CredentialPending::Cleanup) => {
                if account.token_source == SecretSource::Store {
                    // Moving back to the Keychain cleared the marker in its own save.
                    self.db
                        .call(move |conn| {
                            store::clear_pending(conn, kind, CredentialPending::Cleanup)
                        })
                        .await
                } else {
                    self.cleanup(&gate, kind).await
                }
            }
            Some(CredentialPending::Removal) => self.finish_removal(&gate, kind).await,
            Some(CredentialPending::Save) => Err(AppError::validation(
                "Replace the token in Settings → Accounts: paste it again, or choose another source.",
            )),
            None => Ok(()),
        }
    }

    /// Settings → Secrets, **Allow This Source**: confirm a restored
    /// source, for the revision the user was shown.
    pub async fn approve(&self, kind: ForgeKind, revision: i64) -> AppResult<()> {
        let gate = self.credentials.gate(kind.keychain_account()).await;
        self.db
            .call(move |conn| store::approve(conn, kind, revision))
            .await?;
        self.credentials.commit(&gate, revision);
        Ok(())
    }

    /// Refresh credential: the next use reads the token again and checks it.
    pub fn refresh(&self, kind: ForgeKind) {
        self.credentials.invalidate(kind.keychain_account());
        self.verified.lock().expect("verified lock").remove(&kind);
    }

    /// Settings → Secrets: every account. Reads no token.
    pub async fn secret_entries(&self) -> AppResult<Vec<SecretEntry>> {
        let accounts = self.db.call(|conn| store::list(conn)).await?;
        Ok(accounts
            .into_iter()
            .map(|a| {
                let api = match a.kind {
                    ForgeKind::Github => &self.endpoints.github,
                    ForgeKind::BitbucketCloud => &self.endpoints.bitbucket,
                };
                let host = api
                    .split_once("://")
                    .map_or(api.as_str(), |(_, rest)| rest)
                    .split('/')
                    .next()
                    .unwrap_or_default()
                    .to_string();
                SecretEntry {
                    owner: CredentialOwner::ForgeAccount { provider: a.kind },
                    label: a.kind.label().to_string(),
                    destination: format!("{host} as {}", a.login),
                    input_required: false,
                    last_test: self
                        .credentials
                        .last_test(a.kind.keychain_account(), a.credential.revision),
                    source: a.token_source,
                    state: a.credential,
                }
            })
            .collect())
    }
}
