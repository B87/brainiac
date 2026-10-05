//! Settings → Agents (SPEC.md, section 13): where runs execute, how they are
//! paid for, the agreement to send code to the provider, new runs'
//! defaults, and the image.
//!
//! The token or key is never stored here. Like an account's token, it is
//! read from its source through the credentials layer (owner
//! `agent:<profile id>`); the Keychain item is written only on a save, after
//! a non-secret marker in the row says a write is under way, so a save cut
//! off between the two leaves a profile that is blocked rather than one that
//! looks usable with an unknown token.

use std::os::unix::fs::FileTypeExt;
use std::path::Path;
use std::sync::Arc;

use rusqlite::{params, Connection, OptionalExtension, Row};

use super::image;
use crate::credentials::{check_source, Binding, CredentialService, OwnerGate, SecretBytes};
use crate::db::Db;
use crate::models::{
    now_rfc3339, AgentImage, AgentPayment, AgentProfile, AgentSettings, AppError, AppResult,
    CredentialOwner, CredentialPending, CredentialState, ErrorCode, SaveAgentCredentialRequest,
    SaveAgentSettingsRequest, SecretEntry, SecretSource,
};

/// The one profile of phase 1: Claude Code (created by migration 0008).
pub const PROFILE_ID: &str = "claude-code";

/// Whether this build offers paying with a Claude plan. Off in release
/// builds until Anthropic's answer on its terms is recorded (SPEC.md,
/// Settings → Agents; docs/design/agent-runs.md, Subscription token).
pub const PLAN_OFFERED: bool = cfg!(debug_assertions);

/// A plan token lasts a year; Settings warns from eleven months on.
const AGEING_DAYS: i64 = 335;

/// New run's time limit, in minutes: 30 minutes to 8 hours (SPEC.md, New run).
pub const TIME_LIMIT_MINUTES: (u32, u32) = (30, 480);
pub const CPUS: (u32, u32) = (1, 64);
/// Claude Code needs a few GiB to run at all.
pub const MEMORY_MIB: (u32, u32) = (2048, 256 * 1024);
pub const WORKSPACE_GIB: (u32, u32) = (1, 500);

/// The credential owner key of a profile, which is also its Keychain account.
pub fn credential_owner(id: &str) -> String {
    format!("agent:{id}")
}

/// A token or key as pasted or read from its source. Terminal wraps the
/// token `claude setup-token` prints, and copying the wrap brings line
/// breaks and the spaces that pad them, so every whitespace and invisible
/// character is removed and the pieces joined; two tokens, or anything that
/// still does not look like one, are refused. The error never repeats the
/// text.
pub fn normalize_credential(payment: AgentPayment, text: &str) -> AppResult<String> {
    let invisible = |c: char| {
        c.is_whitespace() || c.is_control() || matches!(c, '\u{200b}'..='\u{200d}' | '\u{feff}')
    };
    let joined: String = text.chars().filter(|c| !invisible(*c)).collect();
    let what = match payment {
        AgentPayment::ClaudePlan => "token",
        AgentPayment::ApiKey => "API key",
    };
    if joined.is_empty() {
        return Err(AppError::validation(format!("Paste the {what}.")));
    }
    if joined.matches("sk-ant-").count() > 1 {
        return Err(AppError::validation(format!(
            "That is more than one {what}. Paste only the {what} itself."
        )));
    }
    if !joined
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        || !(32..=512).contains(&joined.len())
    {
        return Err(AppError::validation(format!(
            "That does not look like an Anthropic {what}."
        )));
    }
    let plan = joined.starts_with("sk-ant-oat");
    match payment {
        AgentPayment::ClaudePlan if !plan => {
            Err(AppError::validation(if joined.starts_with("sk-ant-") {
                "That is an API key, not a Claude plan token. Choose API key, or paste what claude setup-token printed."
            } else {
                "That is not a Claude plan token: run claude setup-token in Terminal and paste what it prints."
            }))
        }
        AgentPayment::ApiKey if plan => Err(AppError::validation(
            "That is a Claude plan token, not an API key. Choose Claude plan to use it.",
        )),
        AgentPayment::ApiKey if !joined.starts_with("sk-ant-") => Err(AppError::validation(
            "That does not look like an Anthropic API key, which starts with sk-ant-.",
        )),
        _ => Ok(joined),
    }
}

fn conflict() -> AppError {
    AppError::new(
        ErrorCode::Conflict,
        "Settings → Agents changed since it was shown. Look again and redo the change.",
    )
}

/// A save cut off after its marker: the profile stays blocked until saved again.
fn partial_save(e: AppError) -> AppError {
    AppError::new(
        e.code,
        format!(
            "{} Runs cannot use it until the token or key is saved again.",
            e.message
        ),
    )
}

/// A model as typed: an alias such as `sonnet` or `opus[1m]`, or a full
/// name; empty means Claude Code's default. Only the characters model
/// names use, so the value is safe as an environment variable.
pub fn check_model(text: &str) -> AppResult<String> {
    let model = text.trim();
    let plain =
        |c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':' | '[' | ']');
    if model.len() > 64 || !model.chars().all(plain) {
        return Err(AppError::validation(
            "The model is an alias such as sonnet, or a full model name, with no spaces.",
        ));
    }
    Ok(model.to_string())
}

pub fn in_range(name: &str, value: u32, (min, max): (u32, u32)) -> AppResult<u32> {
    if (min..=max).contains(&value) {
        Ok(value)
    } else {
        Err(AppError::validation(format!(
            "{name} is from {min} to {max}."
        )))
    }
}

/// A profile's token or key comes from the Keychain, a variable, or a command.
fn credential_source(source: &SecretSource) -> AppResult<SecretSource> {
    match source {
        SecretSource::Ask | SecretSource::None => Err(AppError::validation(
            "The token or key comes from the Keychain, an environment variable, or a command.",
        )),
        other => check_source(other),
    }
}

/// The provider and plan, as Settings and Settings → Secrets name it.
fn destination(payment: AgentPayment) -> &'static str {
    match payment {
        AgentPayment::ClaudePlan => "Anthropic, under your Claude plan",
        AgentPayment::ApiKey => "Anthropic, with an API key",
    }
}

pub struct AgentSettingsService {
    db: Db,
    // `Arc` because accounts, connections, and agents share one credentials layer.
    credentials: Arc<CredentialService>,
    /// One image build at a time; a second is refused rather than queued.
    building: tokio::sync::Mutex<()>,
}

impl AgentSettingsService {
    pub fn new(db: Db, credentials: Arc<CredentialService>) -> Self {
        AgentSettingsService {
            db,
            credentials,
            building: tokio::sync::Mutex::new(()),
        }
    }

    pub async fn profile(&self) -> AppResult<AgentProfile> {
        self.db
            .call(|conn| get(conn, PROFILE_ID))
            .await?
            .ok_or_else(|| AppError::db("Settings → Agents has no Claude Code profile."))
    }

    pub async fn get(&self) -> AppResult<AgentSettings> {
        let profile = self.profile().await?;
        Ok(AgentSettings {
            missing: missing(&profile),
            profile,
            plan_offered: PLAN_OFFERED,
        })
    }

    /// Everything but the token or key. Choosing another engine forgets the
    /// image built on the old one.
    pub async fn save(&self, request: SaveAgentSettingsRequest) -> AppResult<AgentSettings> {
        // Held so a credential save cannot interleave with this one.
        let gate = self.credentials.gate(&credential_owner(PROFILE_ID)).await;
        let stored = self.profile().await?;
        let socket = match request.engine_socket.as_deref().map(str::trim) {
            None | Some("") => None,
            // The engine already chosen stays chosen while it is stopped.
            Some(s) if Some(s) == stored.engine_socket.as_deref() => Some(s.to_string()),
            Some(s) => Some(check_engine(s).await?),
        };
        let time_limit = in_range(
            "The time limit",
            request.time_limit_minutes,
            TIME_LIMIT_MINUTES,
        )?;
        let cpus = in_range("CPUs", request.cpus, CPUS)?;
        let memory = in_range("Memory in MiB", request.memory_mib, MEMORY_MIB)?;
        let workspace = in_range("The workspace in GiB", request.workspace_gib, WORKSPACE_GIB)?;
        let model = check_model(&request.model)?;
        let (expected, agreed, permissions) = (
            request.expected_version,
            request.sends_code_agreed,
            request.permissions,
        );
        let now = now_rfc3339();
        self.db
            .call(move |conn| {
                let tx = conn.transaction()?;
                let old: Option<String> = tx.query_row(
                    "SELECT h.socket FROM agent_profiles p JOIN agent_hosts h ON h.id = p.host_id
                         WHERE p.id = ?1",
                    [PROFILE_ID],
                    |r| r.get(0),
                )?;
                let changed = tx.execute(
                    "UPDATE agent_profiles SET sends_code_agreed = ?2, permissions = ?3,
                       time_limit_minutes = ?4, cpus = ?5, memory_mib = ?6, workspace_gib = ?7,
                       model = ?10, version = version + 1, updated_at = ?8
                     WHERE id = ?1 AND version = ?9",
                    params![
                        PROFILE_ID,
                        agreed,
                        permissions.as_str(),
                        time_limit,
                        cpus,
                        memory,
                        workspace,
                        now,
                        expected,
                        model
                    ],
                )?;
                if changed == 0 {
                    return Err(conflict());
                }
                if old != socket {
                    tx.execute(
                        "UPDATE agent_hosts SET socket = ?2, version = version + 1, updated_at = ?3
                         WHERE id = (SELECT host_id FROM agent_profiles WHERE id = ?1)",
                        params![PROFILE_ID, socket, now],
                    )?;
                    tx.execute(
                        "UPDATE agent_profiles SET image_id = NULL, image_recipe = NULL,
                           image_built_at = NULL
                         WHERE id = ?1",
                        [PROFILE_ID],
                    )?;
                }
                tx.commit()?;
                Ok(())
            })
            .await?;
        drop(gate);
        tracing::info!("agent settings saved");
        self.get().await
    }

    /// **Pay with**: a new token or key, or a new source for it. Changing
    /// how runs are paid for asks again to agree to send code.
    pub async fn save_credential(
        &self,
        request: SaveAgentCredentialRequest,
    ) -> AppResult<AgentSettings> {
        let payment = request.payment;
        if payment == AgentPayment::ClaudePlan && !PLAN_OFFERED {
            return Err(AppError::validation(
                "Paying with a Claude plan is not available in this version of Brainiac. Use an API key.",
            ));
        }
        let source = credential_source(&request.source)?;
        let owner = credential_owner(PROFILE_ID);
        let gate = self.credentials.gate(&owner).await;
        let stored = self.profile().await?;
        if stored.version != request.expected_version {
            return Err(conflict());
        }
        let pasted = match source {
            SecretSource::Store => request
                .secret
                .as_deref()
                .map(|text| normalize_credential(payment, text))
                .transpose()?,
            _ => None,
        };
        let old_source = stored.credential_source.clone();
        let old_pending = stored.credential.pending;
        // The Keychain item holds a value for the old payment: a new payment
        // needs a new paste.
        if source == SecretSource::Store
            && pasted.is_none()
            && (old_source != SecretSource::Store
                || old_pending == Some(CredentialPending::Save)
                || stored.payment != payment)
        {
            return Err(AppError::validation(match payment {
                AgentPayment::ClaudePlan => "Paste the token claude setup-token printed.",
                AgentPayment::ApiKey => "Paste the API key.",
            }));
        }
        // Every save binds anew, above any revision this run has seen.
        let revision = (stored.credential.revision + 1).max(self.credentials.next_revision(&owner));
        // The old item is deleted when this save moves away from it. A
        // cleanup already pending is kept for Retry, and a restored profile's
        // item on this Mac is only scheduled for it, never deleted by a save.
        let uncertain = source != SecretSource::Store
            && (old_source == SecretSource::Store || old_pending == Some(CredentialPending::Save));
        let pending_cleanup = source != SecretSource::Store
            && (uncertain || old_pending == Some(CredentialPending::Cleanup));
        let cleanup = uncertain && !stored.credential.needs_approval;
        let agreed = stored.sends_code_agreed && stored.payment == payment;

        if let Some(value) = &pasted {
            let expected = request.expected_version;
            self.db
                .call(move |conn| mark_pending(conn, CredentialPending::Save, expected))
                .await?;
            self.credentials.begin(&gate);
            self.credentials
                .write_item(&gate, SecretBytes::from_text(value))
                .await
                .map_err(partial_save)?;
        }
        let committed = {
            let (source, now) = (source.clone(), now_rfc3339());
            let pending = pending_cleanup.then_some(CredentialPending::Cleanup);
            self.db
                .call(move |conn| {
                    commit_credential(conn, payment, &source, revision, pending, agreed, &now)
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
            if let Err(e) = self.cleanup(&gate).await {
                tracing::warn!(error = %e, "the old agent Keychain item was not deleted");
            }
        }
        drop(gate);
        tracing::info!(payment = payment.as_str(), "agent credential saved");
        self.get().await
    }

    /// **Remove**: runs have no token or key until another is saved. The
    /// Keychain item, if any, is deleted after the row no longer names it.
    pub async fn remove_credential(&self, expected_version: i64) -> AppResult<AgentSettings> {
        let owner = credential_owner(PROFILE_ID);
        let gate = self.credentials.gate(&owner).await;
        let stored = self.profile().await?;
        if stored.version != expected_version {
            return Err(conflict());
        }
        let uncertain = stored.credential_source == SecretSource::Store
            || stored.credential.pending == Some(CredentialPending::Save);
        let pending = (uncertain || stored.credential.pending == Some(CredentialPending::Cleanup))
            .then_some(CredentialPending::Cleanup);
        let revision = (stored.credential.revision + 1).max(self.credentials.next_revision(&owner));
        let (payment, now) = (stored.payment, now_rfc3339());
        self.db
            .call(move |conn| {
                commit_credential(
                    conn,
                    payment,
                    &SecretSource::None,
                    revision,
                    pending,
                    false,
                    &now,
                )
            })
            .await?;
        self.credentials.commit(&gate, revision);
        if uncertain && !stored.credential.needs_approval {
            if let Err(e) = self.cleanup(&gate).await {
                tracing::warn!(error = %e, "the agent Keychain item was not deleted");
            }
        }
        drop(gate);
        tracing::info!("agent credential removed");
        self.get().await
    }

    /// Delete the old Keychain item after a move to another source.
    async fn cleanup(&self, gate: &OwnerGate) -> AppResult<()> {
        self.credentials.delete_item(gate).await?;
        self.db
            .call(|conn| clear_pending(conn, CredentialPending::Cleanup))
            .await
    }

    /// Settings → Secrets, **Retry**: finish a pending cleanup.
    pub async fn retry_cleanup(&self, id: &str) -> AppResult<()> {
        if id != PROFILE_ID {
            return Err(AppError::not_found("There is no such agent profile."));
        }
        let gate = self.credentials.gate(&credential_owner(id)).await;
        let profile = self.profile().await?;
        match profile.credential.pending {
            Some(CredentialPending::Cleanup)
                if profile.credential_source == SecretSource::Store =>
            {
                // Moving back to the Keychain wrote a new item over the old one.
                self.db
                    .call(|conn| clear_pending(conn, CredentialPending::Cleanup))
                    .await
            }
            Some(CredentialPending::Cleanup) | Some(CredentialPending::Removal) => {
                self.cleanup(&gate).await
            }
            Some(CredentialPending::Save) => Err(AppError::validation(
                "Paste the token or key again in Settings → Agents.",
            )),
            None => Ok(()),
        }
    }

    /// Confirm a restored setup (Settings → Agents, or **Allow This Source**
    /// in Settings → Secrets), for the revision the user was shown. Until
    /// then Brainiac does not read its credential, build its image, or start
    /// a run.
    pub async fn approve(&self, id: &str, revision: i64) -> AppResult<AgentSettings> {
        if id != PROFILE_ID {
            return Err(AppError::not_found("There is no such agent profile."));
        }
        let gate = self.credentials.gate(&credential_owner(id)).await;
        self.db
            .call(move |conn| {
                let changed = conn.execute(
                    "UPDATE agent_profiles SET source_approved = 1, version = version + 1
                     WHERE id = ?1 AND credential_revision = ?2",
                    params![PROFILE_ID, revision],
                )?;
                if changed == 0 {
                    return Err(conflict());
                }
                Ok(())
            })
            .await?;
        self.credentials.commit(&gate, revision);
        drop(gate);
        self.get().await
    }

    /// **Refresh**: the next run reads the token or key again.
    pub fn refresh(&self, id: &str) {
        self.credentials.invalidate(&credential_owner(id));
    }

    /// The binding a run resolves its credential through.
    pub fn binding(profile: &AgentProfile) -> Binding {
        Binding {
            owner: credential_owner(&profile.id),
            source: profile.credential_source.clone(),
            revision: profile.credential.revision,
            approved: !profile.credential.needs_approval,
            pending: profile.credential.pending,
            expires_at: None,
        }
    }

    /// **Build image** / **Rebuild…**: build Brainiac's Dockerfile on the
    /// chosen engine and record the image. The build takes minutes; the
    /// record is kept only if the engine is still the one it was built on.
    pub async fn build_image(&self) -> AppResult<AgentSettings> {
        let Ok(_building) = self.building.try_lock() else {
            return Err(AppError::validation("The image is already being built."));
        };
        let profile = self.profile().await?;
        if profile.credential.needs_approval {
            return Err(AppError::validation(
                "Confirm this setup first: it was restored from a backup.",
            ));
        }
        let socket = profile
            .engine_socket
            .clone()
            .ok_or_else(|| AppError::validation("Choose where runs execute first."))?;
        let built = image::build(Path::new(&socket)).await?;
        let now = now_rfc3339();
        let recorded = self
            .db
            .call(move |conn| {
                Ok(conn.execute(
                    "UPDATE agent_profiles SET image_id = ?2, image_recipe = ?3, image_built_at = ?4,
                       version = version + 1, updated_at = ?4
                     WHERE id = ?1
                       AND (SELECT socket FROM agent_hosts WHERE id = agent_profiles.host_id) = ?5",
                    params![PROFILE_ID, built.id, image::recipe(), now, socket],
                )?)
            })
            .await?;
        if recorded == 0 {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "The engine changed while the image was being built. Build it again.",
            ));
        }
        tracing::info!(image = %built.name, "agent image built");
        self.get().await
    }

    /// **Test** passed with this token or key, image, and engine: runs may
    /// start until one of them changes.
    pub async fn record_test(&self, profile: &AgentProfile) -> AppResult<AgentSettings> {
        let (revision, image, socket, now) = (
            profile.credential.revision,
            profile.image.as_ref().map(|i| i.id.clone()),
            profile.engine_socket.clone(),
            now_rfc3339(),
        );
        self.db
            .call(move |conn| {
                conn.execute(
                    "UPDATE agent_profiles SET test_passed_at = ?2, test_credential_revision = ?3,
                       test_image_id = ?4, test_engine_socket = ?5, version = version + 1
                     WHERE id = ?1",
                    params![PROFILE_ID, now, revision, image, socket],
                )?;
                Ok(())
            })
            .await?;
        self.get().await
    }

    /// Settings → Secrets: the profile when it has a token or key, or a
    /// cleanup still to do. Reads no secret.
    pub async fn secret_entries(&self) -> AppResult<Vec<SecretEntry>> {
        let p = self.profile().await?;
        if p.credential_source == SecretSource::None && p.credential.pending.is_none() {
            return Ok(Vec::new());
        }
        let owner = credential_owner(&p.id);
        Ok(vec![SecretEntry {
            owner: CredentialOwner::AgentProfile { id: p.id.clone() },
            label: "Claude Code runs".to_string(),
            destination: destination(p.payment).to_string(),
            input_required: false,
            last_test: self.credentials.last_test(&owner, p.credential.revision),
            source: p.credential_source,
            state: p.credential,
        }])
    }
}

/// A newly chosen engine: a socket that exists, of an engine runs were
/// tested on that answers.
async fn check_engine(socket: &str) -> AppResult<String> {
    let socket = check_socket(socket)?;
    let engine = super::engine::probe(Path::new(&socket)).await;
    if !(engine.reachable && engine.supported) {
        return Err(AppError::validation(format!(
            "{} cannot run agents: {}",
            engine.name,
            engine
                .problem
                .unwrap_or_else(|| "it did not answer.".to_string())
        )));
    }
    Ok(socket)
}

/// An engine's socket as chosen: an absolute path to a socket that exists.
fn check_socket(socket: &str) -> AppResult<String> {
    let path = Path::new(socket);
    if !path.is_absolute() {
        return Err(AppError::validation(
            "Choose the engine's socket by its full path.",
        ));
    }
    match std::fs::metadata(path) {
        Ok(m) if m.file_type().is_socket() => Ok(socket.to_string()),
        Ok(_) => Err(AppError::validation(format!("{socket} is not a socket."))),
        Err(_) => Err(AppError::not_found(format!(
            "There is no engine socket at {socket}. Is the engine running?"
        ))),
    }
}

/// What still stops a run from starting, in the order to do it.
fn missing(p: &AgentProfile) -> Vec<String> {
    let mut missing = Vec::new();
    if p.engine_socket.is_none() {
        missing.push("Choose where runs execute.".to_string());
    }
    if p.credential.needs_approval {
        missing.push("Confirm this setup: it was restored from a backup.".to_string());
    }
    if p.payment == AgentPayment::ClaudePlan && !PLAN_OFFERED {
        missing.push(
            "Use an API key: paying with a Claude plan is not available in this version."
                .to_string(),
        );
    } else if p.credential.pending == Some(CredentialPending::Save) {
        missing.push("Save the token or key again: the last save did not finish.".to_string());
    } else if p.credential_source == SecretSource::None {
        missing.push(match p.payment {
            AgentPayment::ClaudePlan => "Add the token from claude setup-token.".to_string(),
            AgentPayment::ApiKey => "Add an API key.".to_string(),
        });
    }
    if !p.sends_code_agreed {
        missing.push(format!(
            "Agree to send code and prompts to {}.",
            destination(p.payment)
        ));
    }
    match &p.image {
        None => missing.push("Build the image.".to_string()),
        Some(i) if !i.current => {
            missing.push("Rebuild the image: this version of Brainiac changed it.".to_string())
        }
        Some(_) => {}
    }
    if !p.test_current {
        missing.push(if p.test_passed_at.is_some() {
            "Test again: the token or key, the image, or the engine changed since the last test."
                .to_string()
        } else {
            "Pass a test run.".to_string()
        });
    }
    missing
}

const COLUMNS: &str = "p.id, h.socket, p.payment, p.secret_source, p.credential_revision,
    p.source_approved, p.credential_pending, p.credential_saved_at, p.sends_code_agreed,
    p.permissions, p.time_limit_minutes, p.cpus, p.memory_mib, p.workspace_gib, p.image_id,
    p.image_recipe, p.image_built_at, p.version, p.test_passed_at, p.test_credential_revision,
    p.test_image_id, p.test_engine_socket, p.model";

fn parse<T: serde::de::DeserializeOwned>(text: String) -> rusqlite::Result<T> {
    serde_json::from_value(serde_json::Value::String(text)).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })
}

fn parse_json<T: serde::de::DeserializeOwned>(text: String) -> rusqlite::Result<T> {
    serde_json::from_str(&text).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })
}

fn word<T: serde::Serialize>(value: &T) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(s)) => s,
        _ => String::new(),
    }
}

fn from_row(r: &Row<'_>) -> rusqlite::Result<AgentProfile> {
    let payment: AgentPayment = parse(r.get(2)?)?;
    let saved_at: Option<String> = r.get(7)?;
    let ageing = payment == AgentPayment::ClaudePlan
        && saved_at
            .as_deref()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .is_some_and(|t| chrono::Utc::now().signed_duration_since(t).num_days() >= AGEING_DAYS);
    let image_id: Option<String> = r.get(14)?;
    let recipe: Option<String> = r.get(15)?;
    let built_at: Option<String> = r.get(16)?;
    let image = match (image_id, recipe, built_at) {
        (Some(id), Some(recipe), Some(built_at)) => Some(AgentImage {
            name: image::name_for(&recipe),
            current: recipe == image::recipe(),
            id,
            built_at,
        }),
        _ => None,
    };
    let engine_socket: Option<String> = r.get(1)?;
    let revision: i64 = r.get(4)?;
    let test_passed_at: Option<String> = r.get(18)?;
    let test_revision: Option<i64> = r.get(19)?;
    let test_image: Option<String> = r.get(20)?;
    let test_socket: Option<String> = r.get(21)?;
    // The test counts while nothing it ran with changed.
    let test_current = test_passed_at.is_some()
        && test_revision == Some(revision)
        && test_image.is_some()
        && test_image == image.as_ref().map(|i| i.id.clone())
        && test_socket.is_some()
        && test_socket == engine_socket;
    Ok(AgentProfile {
        id: r.get(0)?,
        engine_socket,
        payment,
        credential_source: parse_json(r.get(3)?)?,
        credential: CredentialState {
            revision,
            needs_approval: !r.get::<_, bool>(5)?,
            pending: r.get::<_, Option<String>>(6)?.map(parse).transpose()?,
        },
        credential_saved_at: saved_at,
        credential_ageing: ageing,
        sends_code_agreed: r.get(8)?,
        permissions: parse(r.get(9)?)?,
        time_limit_minutes: r.get(10)?,
        cpus: r.get(11)?,
        memory_mib: r.get(12)?,
        model: r.get(22)?,
        workspace_gib: r.get(13)?,
        image,
        test_passed_at,
        test_current,
        version: r.get(17)?,
    })
}

fn get(conn: &Connection, id: &str) -> AppResult<Option<AgentProfile>> {
    Ok(conn
        .query_row(
            &format!(
                "SELECT {COLUMNS} FROM agent_profiles p JOIN agent_hosts h ON h.id = p.host_id
                 WHERE p.id = ?1"
            ),
            [id],
            from_row,
        )
        .optional()?)
}

fn mark_pending(conn: &Connection, pending: CredentialPending, expected: i64) -> AppResult<()> {
    let changed = conn.execute(
        "UPDATE agent_profiles SET credential_pending = ?2, version = version + 1
         WHERE id = ?1 AND version = ?3",
        params![PROFILE_ID, word(&pending), expected],
    )?;
    if changed == 0 {
        return Err(conflict());
    }
    Ok(())
}

/// The row after a credential save or removal: the payment and source, the
/// new revision, approved by this save, and what is still pending.
fn commit_credential(
    conn: &Connection,
    payment: AgentPayment,
    source: &SecretSource,
    revision: i64,
    pending: Option<CredentialPending>,
    agreed: bool,
    now: &str,
) -> AppResult<()> {
    let saved_at = (*source != SecretSource::None).then_some(now);
    conn.execute(
        "UPDATE agent_profiles SET payment = ?2, secret_source = ?3, credential_revision = ?4,
           source_approved = 1, credential_pending = ?5, credential_saved_at = ?6,
           sends_code_agreed = ?7, version = version + 1, updated_at = ?8
         WHERE id = ?1",
        params![
            PROFILE_ID,
            payment.as_str(),
            serde_json::to_string(source).map_err(|e| AppError::db(e.to_string()))?,
            revision,
            pending.as_ref().map(word),
            saved_at,
            agreed,
            now
        ],
    )?;
    Ok(())
}

fn clear_pending(conn: &Connection, which: CredentialPending) -> AppResult<()> {
    conn.execute(
        "UPDATE agent_profiles SET credential_pending = NULL
         WHERE id = ?1 AND credential_pending = ?2",
        params![PROFILE_ID, word(&which)],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAN: &str = "sk-ant-oat01-AbCdEfGhIjKlMnOpQrStUvWxYz0123456789_-AbCd";
    const KEY: &str = "sk-ant-api03-AbCdEfGhIjKlMnOpQrStUvWxYz0123456789_-AbCd";

    #[test]
    fn wrapped_tokens_are_joined_and_anything_else_refused() {
        let wrapped = format!("  {}\r\n{}  \n", &PLAN[..20], &PLAN[20..]);
        assert_eq!(
            normalize_credential(AgentPayment::ClaudePlan, &wrapped).unwrap(),
            PLAN
        );
        // Terminal pads a wrapped line with spaces before the break, and a
        // copy can carry a non-breaking or zero-width character.
        let padded = format!("{}   \n   {}\u{a0}\u{200b}", &PLAN[..30], &PLAN[30..]);
        assert_eq!(
            normalize_credential(AgentPayment::ClaudePlan, &padded).unwrap(),
            PLAN
        );
        let two = format!("{PLAN}\n{PLAN}");
        let err = normalize_credential(AgentPayment::ClaudePlan, &two).unwrap_err();
        assert!(err.message.contains("more than one"), "{err:?}");
        assert!(!err.message.contains("sk-ant"), "{err:?}");
        let sentence = format!("{PLAN} Store this token securely!");
        let err = normalize_credential(AgentPayment::ClaudePlan, &sentence).unwrap_err();
        assert!(err.message.contains("does not look like"), "{err:?}");
        assert!(normalize_credential(AgentPayment::ClaudePlan, "").is_err());
        assert!(normalize_credential(AgentPayment::ClaudePlan, "sk-ant-oat01-short").is_err());
    }

    #[test]
    fn a_key_and_a_plan_token_are_not_mixed_up() {
        let err = normalize_credential(AgentPayment::ClaudePlan, KEY).unwrap_err();
        assert!(err.message.contains("API key"), "{err:?}");
        let err = normalize_credential(AgentPayment::ApiKey, PLAN).unwrap_err();
        assert!(err.message.contains("Claude plan"), "{err:?}");
        assert_eq!(
            normalize_credential(AgentPayment::ApiKey, KEY).unwrap(),
            KEY
        );
        assert!(normalize_credential(
            AgentPayment::ApiKey,
            "xx-AbCdEfGhIjKlMnOpQrStUvWxYz0123456789"
        )
        .is_err());
    }
}
