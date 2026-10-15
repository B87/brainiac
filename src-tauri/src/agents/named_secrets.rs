//! Named secrets (docs/design/scheduled-runs.md, Credentials on the host): a
//! Sentry token, a Google service account key, anything a run's scripts need
//! that is not a model credential. Each is kept in Settings → Secrets like
//! an account's token (the row holds where it is read from, owner
//! `named:<id>`, never the value), and can be **shared** with a run host,
//! which keeps a sealed copy for runs it starts without this Mac.
//!
//! Sharing is a copy. Brainiac records which credential revision it sent, so
//! a changed source reads as out of date; the host's own list says whether
//! the copy is still there.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use rusqlite::{params, Connection, OptionalExtension, Row};

use super::controller::protocol::{valid_secret_name, MAX_EXTRA_VALUE, MIN_EXTRA_VALUE};
use super::runtime::RunRuntime;
use crate::credentials::{
    check_source, Binding, CredentialService, OwnerGate, Resolution, SecretBytes,
};
use crate::db::Db;
use crate::models::{
    now_rfc3339, AppError, AppResult, CredentialOwner, CredentialPending, CredentialState,
    ErrorCode, HostSecretOnly, HostSecretRow, HostSecretState, HostSecrets, NamedSecret,
    SaveNamedSecretRequest, SecretEntry, SecretSource,
};

/// The credential owner key of a named secret, which is also its Keychain account.
pub fn credential_owner(id: &str) -> String {
    format!("named:{id}")
}

const MAX_LABEL: usize = 80;
const MAX_NOTE: usize = 300;

pub struct NamedSecretService {
    db: Db,
    credentials: Arc<CredentialService>,
}

/// What Brainiac recorded for one secret on one host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Share {
    pub secret_id: String,
    pub shared_revision: i64,
    pub shared_at: String,
    pub failure: Option<String>,
}

/// Where a named secret stands on a host, from what Brainiac sent and, when
/// the host could be asked, what it lists.
pub fn row_state(
    secret: &NamedSecret,
    share: Option<&Share>,
    on_host: Option<&HashSet<String>>,
) -> HostSecretState {
    let listed = on_host.map(|names| names.contains(&secret.env_name));
    match share {
        // Brainiac did not send it, or no longer remembers (a restore), yet
        // the host holds one under that name: it cannot be vouched for.
        None if listed == Some(true) => HostSecretState::OutOfDate,
        None => HostSecretState::NotShared,
        Some(s) if s.failure.is_some() => HostSecretState::Failed,
        Some(s) if s.shared_revision != secret.credential.revision => HostSecretState::OutOfDate,
        Some(_) if listed == Some(false) => HostSecretState::MissingOnHost,
        Some(_) => HostSecretState::Shared,
    }
}

impl NamedSecretService {
    pub fn new(db: Db, credentials: Arc<CredentialService>) -> Self {
        NamedSecretService { db, credentials }
    }

    pub async fn list(&self) -> AppResult<Vec<NamedSecret>> {
        self.db.call(|conn| list(conn)).await
    }

    pub async fn get(&self, id: &str) -> AppResult<NamedSecret> {
        let id = id.to_string();
        self.db
            .call(move |conn| get(conn, &id))
            .await?
            .ok_or_else(|| AppError::not_found("There is no such secret."))
    }

    // -----------------------------------------------------------------------
    // Saving
    // -----------------------------------------------------------------------

    /// Add a secret, or change one's label, note, name, or source. A paste
    /// goes to the Keychain after a marker in the row says a write is under
    /// way, so a save cut off between the two leaves a secret that is blocked,
    /// not one that looks usable with an unknown value.
    pub async fn save(&self, request: SaveNamedSecretRequest) -> AppResult<NamedSecret> {
        let label = request.label.trim().to_string();
        if label.is_empty() || label.chars().count() > MAX_LABEL {
            return Err(AppError::validation(
                "Give the secret a name of up to 80 characters.",
            ));
        }
        let note = request.note.trim().to_string();
        if note.chars().count() > MAX_NOTE {
            return Err(AppError::validation("The note is up to 300 characters."));
        }
        let env_name = request.env_name.trim().to_string();
        if !valid_secret_name(&env_name) {
            return Err(AppError::validation(
                "The variable name is capitals, digits and underscores, starts with a capital, and is not one the agent or the container uses (PATH, NODE_*, GIT_*, *_PROXY and the like).",
            ));
        }
        let source = credential_source(&request.source)?;
        let pasted = match (&source, request.secret.as_deref()) {
            (SecretSource::Store, Some(text)) => Some(check_value(text)?),
            _ => None,
        };

        let id = match &request.id {
            Some(id) => id.clone(),
            None => {
                if source == SecretSource::Store && pasted.is_none() {
                    return Err(AppError::validation("Paste the value."));
                }
                let (id, now) = (uuid::Uuid::new_v4().simple().to_string(), now_rfc3339());
                let (row_id, name, text, label) =
                    (id.clone(), env_name.clone(), note.clone(), label.clone());
                self.db
                    .call(move |conn| {
                        conn.execute(
                            "INSERT INTO named_secrets (id, label, env_name, note, created_at, updated_at)
                             VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
                            params![row_id, label, name, text, now],
                        )
                        .map_err(|e| unique(e, "Another secret already has that variable name."))?;
                        Ok(())
                    })
                    .await?;
                id
            }
        };
        let owner = credential_owner(&id);
        let gate = self.credentials.gate(&owner).await;
        let stored = self.get(&id).await?;
        let adding = request.id.is_none();
        if !adding && stored.version != request.expected_version {
            return Err(conflict());
        }
        if !adding && stored.env_name != env_name && self.has_shares(&id).await? {
            return Err(AppError::validation(
                "This secret is on a host under its current variable name. Remove it from the host before renaming it.",
            ));
        }
        let old_source = stored.credential_source.clone();
        let old_pending = stored.credential.pending;
        if source == SecretSource::Store
            && pasted.is_none()
            && (old_source != SecretSource::Store || old_pending == Some(CredentialPending::Save))
        {
            return Err(AppError::validation("Paste the value."));
        }
        let changed = pasted.is_some() || source != old_source || adding;
        if !changed {
            // Only the label, the note, or the name: the credential is untouched.
            let (id2, now, version) = (id.clone(), now_rfc3339(), stored.version);
            self.db
                .call(move |conn| {
                    let n = conn
                        .execute(
                            "UPDATE named_secrets SET label = ?2, env_name = ?3, note = ?4,
                               version = version + 1, updated_at = ?5
                             WHERE id = ?1 AND version = ?6",
                            params![id2, label, env_name, note, now, version],
                        )
                        .map_err(|e| unique(e, "Another secret already has that variable name."))?;
                    if n == 0 {
                        return Err(conflict());
                    }
                    Ok(())
                })
                .await?;
            return self.get(&id).await;
        }

        let revision = (stored.credential.revision + 1).max(self.credentials.next_revision(&owner));
        let uncertain = source != SecretSource::Store
            && (old_source == SecretSource::Store || old_pending == Some(CredentialPending::Save));
        let pending_cleanup = source != SecretSource::Store
            && (uncertain || old_pending == Some(CredentialPending::Cleanup));
        let cleanup = uncertain && !stored.credential.needs_approval;

        if let Some(value) = &pasted {
            let (expected, id2) = (stored.version, id.clone());
            self.db
                .call(move |conn| mark_pending(conn, &id2, CredentialPending::Save, expected))
                .await?;
            self.credentials.begin(&gate);
            self.credentials
                .write_item(&gate, SecretBytes::from_text(value))
                .await
                .map_err(partial_save)?;
        }
        let committed = {
            let (source, now, id2) = (source.clone(), now_rfc3339(), id.clone());
            let pending = pending_cleanup.then_some(CredentialPending::Cleanup);
            let (label, env_name, note) = (label.clone(), env_name.clone(), note.clone());
            self.db
                .call(move |conn| {
                    conn.execute(
                        "UPDATE named_secrets SET label = ?2, env_name = ?3, note = ?4,
                           secret_source = ?5, credential_revision = ?6, source_approved = 1,
                           credential_pending = ?7, credential_saved_at = ?8,
                           version = version + 1, updated_at = ?8
                         WHERE id = ?1",
                        params![
                            id2,
                            label,
                            env_name,
                            note,
                            serde_json::to_string(&source)
                                .map_err(|e| AppError::db(e.to_string()))?,
                            revision,
                            pending.as_ref().map(word),
                            now,
                        ],
                    )
                    .map_err(|e| unique(e, "Another secret already has that variable name."))?;
                    Ok(())
                })
                .await
        };
        match committed {
            Err(e) if pasted.is_some() => return Err(partial_save(e)),
            other => other?,
        }
        self.credentials.commit(&gate, revision);
        if let Some(value) = &pasted {
            self.credentials
                .prime(&gate, revision, SecretBytes::from_text(value), None);
        }
        if cleanup {
            if let Err(e) = self.cleanup(&gate, &id).await {
                tracing::warn!(error = %e, "the old Keychain item of a named secret was not deleted");
            }
        }
        drop(gate);
        tracing::info!("named secret saved");
        self.get(&id).await
    }

    /// Delete a secret. One a host still holds is removed from it first: the
    /// copy is the thing that would be left behind.
    pub async fn delete(&self, id: &str, expected_version: i64) -> AppResult<()> {
        let gate = self.credentials.gate(&credential_owner(id)).await;
        let stored = self.get(id).await?;
        if stored.version != expected_version {
            return Err(conflict());
        }
        if self.has_shares(id).await? {
            return Err(AppError::validation(
                "This secret is on a run host. Remove it from the host first, so no copy is left behind.",
            ));
        }
        let uncertain =
            stored.credential_source == SecretSource::Store || stored.credential.pending.is_some();
        if uncertain && !stored.credential.needs_approval {
            let (id2, expected) = (id.to_string(), stored.version);
            self.db
                .call(move |conn| mark_pending(conn, &id2, CredentialPending::Removal, expected))
                .await?;
            self.cleanup_and_remove(&gate, id).await?;
        } else {
            self.remove_row(id).await?;
        }
        self.credentials.forget(&gate);
        Ok(())
    }

    async fn remove_row(&self, id: &str) -> AppResult<()> {
        let id = id.to_string();
        self.db
            .call(move |conn| {
                conn.execute("DELETE FROM named_secrets WHERE id = ?1", params![id])?;
                Ok(())
            })
            .await
    }

    async fn cleanup_and_remove(&self, gate: &OwnerGate, id: &str) -> AppResult<()> {
        self.credentials.delete_item(gate).await?;
        self.remove_row(id).await
    }

    /// Delete the old Keychain item after a move to another source.
    async fn cleanup(&self, gate: &OwnerGate, id: &str) -> AppResult<()> {
        self.credentials.delete_item(gate).await?;
        let id = id.to_string();
        self.db
            .call(move |conn| clear_pending(conn, &id, CredentialPending::Cleanup))
            .await
    }

    /// Settings → Secrets, **Retry**: finish a cleanup or removal.
    pub async fn retry_cleanup(&self, id: &str) -> AppResult<()> {
        let secret = self.get(id).await?;
        let gate = self.credentials.gate(&credential_owner(id)).await;
        match secret.credential.pending {
            Some(CredentialPending::Cleanup) if secret.credential_source == SecretSource::Store => {
                let id = id.to_string();
                self.db
                    .call(move |conn| clear_pending(conn, &id, CredentialPending::Cleanup))
                    .await
            }
            Some(CredentialPending::Cleanup) => self.cleanup(&gate, id).await,
            Some(CredentialPending::Removal) => self.cleanup_and_remove(&gate, id).await,
            Some(CredentialPending::Save) => Err(AppError::validation(
                "Paste the value again in Settings → Secrets.",
            )),
            None => Ok(()),
        }
    }

    /// **Allow This Source** for a restored secret, at the revision shown.
    pub async fn approve(&self, id: &str, revision: i64) -> AppResult<()> {
        self.get(id).await?;
        let gate = self.credentials.gate(&credential_owner(id)).await;
        let id2 = id.to_string();
        self.db
            .call(move |conn| {
                let changed = conn.execute(
                    "UPDATE named_secrets SET source_approved = 1, version = version + 1
                     WHERE id = ?1 AND credential_revision = ?2",
                    params![id2, revision],
                )?;
                if changed == 0 {
                    return Err(conflict());
                }
                Ok(())
            })
            .await?;
        self.credentials.commit(&gate, revision);
        Ok(())
    }

    /// **Refresh**: the next read of the source starts afresh.
    pub fn refresh(&self, id: &str) {
        self.credentials.invalidate(&credential_owner(id));
    }

    pub fn binding(secret: &NamedSecret) -> Binding {
        Binding {
            owner: credential_owner(&secret.id),
            source: secret.credential_source.clone(),
            revision: secret.credential.revision,
            approved: !secret.credential.needs_approval,
            pending: secret.credential.pending,
            expires_at: None,
        }
    }

    /// Settings → Secrets: each named secret. Reads no secret.
    pub async fn secret_entries(&self) -> AppResult<Vec<SecretEntry>> {
        Ok(self
            .list()
            .await?
            .into_iter()
            .map(|s| SecretEntry {
                owner: CredentialOwner::NamedSecret { id: s.id.clone() },
                label: s.label.clone(),
                destination: format!("run hosts, as {}", s.env_name),
                input_required: false,
                last_test: self
                    .credentials
                    .last_test(&credential_owner(&s.id), s.credential.revision),
                source: s.credential_source.clone(),
                state: s.credential.clone(),
            })
            .collect())
    }

    // -----------------------------------------------------------------------
    // Hosts
    // -----------------------------------------------------------------------

    async fn has_shares(&self, id: &str) -> AppResult<bool> {
        let id = id.to_string();
        self.db
            .call(move |conn| {
                Ok(conn.query_row(
                    "SELECT EXISTS (SELECT 1 FROM host_secret_shares WHERE secret_id = ?1)",
                    params![id],
                    |r| r.get::<_, bool>(0),
                )?)
            })
            .await
    }

    /// **Share**: read the secret here, send it to the host, and record what
    /// was sent. The host's copy is sealed by the host and never read back.
    /// A failure is recorded against the share, so it is shown as failed and
    /// never as shared.
    pub async fn share(
        &self,
        id: &str,
        host_id: &str,
        runtime: &RunRuntime,
    ) -> AppResult<HostSecrets> {
        let secret = self.get(id).await?;
        if secret.credential_source == SecretSource::None {
            return Err(AppError::validation("This secret has no source yet."));
        }
        let lease = match self.credentials.resolve(&Self::binding(&secret)).await? {
            Resolution::Secret(lease) => lease,
            Resolution::NoCredential | Resolution::InputRequired => {
                return Err(AppError::new(
                    ErrorCode::Unauthenticated,
                    "This secret has no value to share.",
                ))
            }
        };
        let revision = lease.revision();
        let value = String::from_utf8(lease.bytes().expose().to_vec())
            .map_err(|_| AppError::validation("The secret is not text."))?;
        // A source may end its output with a newline; a value does not.
        let value = value.trim_end_matches(['\n', '\r']).to_string();
        check_value(&value)?;
        match runtime.put_secret(&secret.env_name, value).await {
            Ok(_) => {
                self.record_share(host_id, id, Some(revision), None).await?;
            }
            Err(e) => {
                self.record_share(host_id, id, None, Some(e.message.clone()))
                    .await?;
                return Err(e);
            }
        }
        self.host_secrets(host_id, Some(runtime)).await
    }

    /// **Remove from host**: the host forgets the secret, then so does the ledger.
    pub async fn unshare(
        &self,
        id: &str,
        host_id: &str,
        runtime: &RunRuntime,
    ) -> AppResult<HostSecrets> {
        let secret = self.get(id).await?;
        match runtime.delete_secret(&secret.env_name).await {
            Ok(_) => {
                let (host, id2) = (host_id.to_string(), id.to_string());
                self.db
                    .call(move |conn| {
                        conn.execute(
                            "DELETE FROM host_secret_shares WHERE host_id = ?1 AND secret_id = ?2",
                            params![host, id2],
                        )?;
                        Ok(())
                    })
                    .await?;
            }
            Err(e) => {
                self.record_share(host_id, id, None, Some(e.message.clone()))
                    .await?;
                return Err(e);
            }
        }
        self.host_secrets(host_id, Some(runtime)).await
    }

    /// Record a Share: the revision sent, or the failure. A failure keeps the
    /// earlier revision and time, so a share that was fine stays known.
    async fn record_share(
        &self,
        host_id: &str,
        id: &str,
        revision: Option<i64>,
        failure: Option<String>,
    ) -> AppResult<()> {
        let (host, id, now) = (host_id.to_string(), id.to_string(), now_rfc3339());
        self.db
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO host_secret_shares (host_id, secret_id, shared_revision, shared_at, failure)
                     VALUES (?1, ?2, COALESCE(?3, 0), ?4, ?5)
                     ON CONFLICT (host_id, secret_id) DO UPDATE SET
                       shared_revision = COALESCE(?3, shared_revision),
                       shared_at = CASE WHEN ?3 IS NULL THEN shared_at ELSE ?4 END,
                       failure = ?5",
                    params![host, id, revision, now, failure],
                )?;
                Ok(())
            })
            .await
    }

    /// The host's page: each named secret's state on it, and what the host
    /// holds that no named secret names. With a runtime it asks the host; a
    /// host that cannot be asked gives a `problem` and Brainiac's own record.
    pub async fn host_secrets(
        &self,
        host_id: &str,
        runtime: Option<&RunRuntime>,
    ) -> AppResult<HostSecrets> {
        let secrets = self.list().await?;
        let host = host_id.to_string();
        let shares: HashMap<String, Share> = self
            .db
            .call(move |conn| shares_on(conn, &host))
            .await?
            .into_iter()
            .map(|s| (s.secret_id.clone(), s))
            .collect();
        let (listed, problem) = match runtime {
            Some(runtime) => match runtime.list_secrets().await {
                Ok(list) => (Some(list), None),
                Err(e) => (None, Some(e.message)),
            },
            None => (None, None),
        };
        let names: Option<HashSet<String>> = listed
            .as_ref()
            .map(|l| l.iter().map(|s| s.name.clone()).collect());
        let rows = secrets
            .iter()
            .map(|s| {
                let share = shares.get(&s.id);
                HostSecretRow {
                    secret_id: s.id.clone(),
                    label: s.label.clone(),
                    env_name: s.env_name.clone(),
                    state: row_state(s, share, names.as_ref()),
                    shared_at: share.map(|x| x.shared_at.clone()),
                    failure: share.and_then(|x| x.failure.clone()),
                }
            })
            .collect();
        let known: HashSet<&str> = secrets.iter().map(|s| s.env_name.as_str()).collect();
        let only_on_host = listed
            .unwrap_or_default()
            .into_iter()
            .filter(|s| !known.contains(s.name.as_str()))
            .map(|s| HostSecretOnly {
                name: s.name,
                set_at: s.set_at,
            })
            .collect();
        Ok(HostSecrets {
            host_id: host_id.to_string(),
            rows,
            only_on_host,
            problem,
        })
    }
}

fn credential_source(source: &SecretSource) -> AppResult<SecretSource> {
    match source {
        SecretSource::Ask | SecretSource::None => Err(AppError::validation(
            "The value comes from the Keychain, an environment variable, or a command.",
        )),
        other => check_source(other),
    }
}

/// A value a host can seal: 8 bytes to 16 KiB, no NUL. Spaces and line
/// breaks are allowed (a service account key is JSON).
fn check_value(text: &str) -> AppResult<String> {
    let text = text.trim_end_matches(['\n', '\r']);
    if !(MIN_EXTRA_VALUE..=MAX_EXTRA_VALUE).contains(&text.len()) || text.contains('\0') {
        return Err(AppError::validation(
            "A secret's value is from 8 characters to 16 KiB, without NUL characters.",
        ));
    }
    Ok(text.to_string())
}

fn conflict() -> AppError {
    AppError::new(
        ErrorCode::Conflict,
        "Settings → Secrets changed since it was shown. Look again and redo the change.",
    )
}

fn unique(e: rusqlite::Error, message: &str) -> AppError {
    match &e {
        rusqlite::Error::SqliteFailure(f, _)
            if f.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            AppError::validation(message)
        }
        _ => AppError::from(e),
    }
}

/// A save cut off after its marker: the secret stays blocked until saved again.
fn partial_save(e: AppError) -> AppError {
    AppError::new(
        e.code,
        format!(
            "{} The secret cannot be used until its value is saved again.",
            e.message
        ),
    )
}

fn word<T: serde::Serialize>(value: &T) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(s)) => s,
        _ => String::new(),
    }
}

const COLUMNS: &str = "id, label, env_name, note, secret_source, credential_revision,
    source_approved, credential_pending, version, created_at, updated_at";

fn from_row(r: &Row<'_>) -> rusqlite::Result<NamedSecret> {
    let source: String = r.get(4)?;
    let pending: Option<String> = r.get(7)?;
    let bad = |e: serde_json::Error| {
        rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(e))
    };
    Ok(NamedSecret {
        id: r.get(0)?,
        label: r.get(1)?,
        env_name: r.get(2)?,
        note: r.get(3)?,
        credential_source: serde_json::from_str(&source).map_err(bad)?,
        credential: CredentialState {
            needs_approval: !r.get::<_, bool>(6)?,
            pending: match pending {
                Some(p) => Some(serde_json::from_value(serde_json::Value::String(p)).map_err(bad)?),
                None => None,
            },
            revision: r.get(5)?,
        },
        version: r.get(8)?,
        created_at: r.get(9)?,
        updated_at: r.get(10)?,
    })
}

fn list(conn: &Connection) -> AppResult<Vec<NamedSecret>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM named_secrets ORDER BY label COLLATE NOCASE, id"
    ))?;
    let rows = stmt.query_map([], from_row)?.collect::<Result<_, _>>()?;
    Ok(rows)
}

fn get(conn: &Connection, id: &str) -> AppResult<Option<NamedSecret>> {
    Ok(conn
        .query_row(
            &format!("SELECT {COLUMNS} FROM named_secrets WHERE id = ?1"),
            params![id],
            from_row,
        )
        .optional()?)
}

fn shares_on(conn: &Connection, host_id: &str) -> AppResult<Vec<Share>> {
    let mut stmt = conn.prepare(
        "SELECT secret_id, shared_revision, shared_at, failure FROM host_secret_shares
         WHERE host_id = ?1",
    )?;
    let rows = stmt
        .query_map(params![host_id], |r| {
            Ok(Share {
                secret_id: r.get(0)?,
                shared_revision: r.get(1)?,
                shared_at: r.get(2)?,
                failure: r.get(3)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    Ok(rows)
}

fn mark_pending(
    conn: &Connection,
    id: &str,
    pending: CredentialPending,
    expected: i64,
) -> AppResult<()> {
    let changed = conn.execute(
        "UPDATE named_secrets SET credential_pending = ?2, version = version + 1
         WHERE id = ?1 AND version = ?3",
        params![id, word(&pending), expected],
    )?;
    if changed == 0 {
        return Err(conflict());
    }
    Ok(())
}

fn clear_pending(conn: &Connection, id: &str, which: CredentialPending) -> AppResult<()> {
    conn.execute(
        "UPDATE named_secrets SET credential_pending = NULL
         WHERE id = ?1 AND credential_pending = ?2",
        params![id, word(&which)],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secret(revision: i64) -> NamedSecret {
        NamedSecret {
            id: "s1".into(),
            label: "Sentry".into(),
            env_name: "SENTRY_TOKEN".into(),
            note: String::new(),
            credential_source: SecretSource::Store,
            credential: CredentialState {
                needs_approval: false,
                pending: None,
                revision,
            },
            version: 1,
            created_at: "t".into(),
            updated_at: "t".into(),
        }
    }

    fn share(revision: i64, failure: Option<&str>) -> Share {
        Share {
            secret_id: "s1".into(),
            shared_revision: revision,
            shared_at: "t".into(),
            failure: failure.map(str::to_string),
        }
    }

    fn host(names: &[&str]) -> HashSet<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    #[test]
    fn a_secret_is_shared_only_while_it_matches_what_was_sent_and_the_host_lists_it() {
        let s = secret(3);
        let listed = host(&["SENTRY_TOKEN"]);
        assert_eq!(
            row_state(&s, None, Some(&host(&[]))),
            HostSecretState::NotShared
        );
        assert_eq!(row_state(&s, None, None), HostSecretState::NotShared);
        assert_eq!(
            row_state(&s, Some(&share(3, None)), Some(&listed)),
            HostSecretState::Shared
        );
        // The host could not be asked: Brainiac's record stands.
        assert_eq!(
            row_state(&s, Some(&share(3, None)), None),
            HostSecretState::Shared
        );
        // The source changed since.
        assert_eq!(
            row_state(&s, Some(&share(2, None)), Some(&listed)),
            HostSecretState::OutOfDate
        );
        // The host lost it.
        assert_eq!(
            row_state(&s, Some(&share(3, None)), Some(&host(&[]))),
            HostSecretState::MissingOnHost
        );
        // A failed Share or Remove is never shown as shared.
        assert_eq!(
            row_state(
                &s,
                Some(&share(3, Some("The host did not answer."))),
                Some(&listed)
            ),
            HostSecretState::Failed
        );
        // The host holds one Brainiac has no record of (a restore): not vouched for.
        assert_eq!(
            row_state(&s, None, Some(&listed)),
            HostSecretState::OutOfDate
        );
    }

    #[test]
    fn a_value_is_checked_for_a_host_and_loses_a_final_newline() {
        assert_eq!(check_value("abc12345\n").unwrap(), "abc12345");
        assert!(check_value("short").is_err());
        assert!(check_value("has\0nul-in-it").is_err());
        assert!(check_value(&"x".repeat(MAX_EXTRA_VALUE + 1)).is_err());
        // A key file's inner line breaks stay.
        assert_eq!(check_value("{\n \"a\": 1\n}\n").unwrap(), "{\n \"a\": 1\n}");
    }
}
