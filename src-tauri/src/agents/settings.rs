//! Settings → Agents (SPEC.md, section 13): this Mac's engine and image,
//! and the profiles: each an agent (Claude Code or OpenCode) with one model
//! provider, how its runs are paid for, the agreement to send code to that
//! provider, and its new runs' defaults.
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

use super::controller::protocol::CredentialKey;
use super::image;
use crate::credentials::{check_source, Binding, CredentialService, OwnerGate, SecretBytes};
use crate::db::Db;
use crate::models::{
    now_rfc3339, AgentHost, AgentKind, AgentPayment, AgentProfile, AgentProvider, AgentSettings,
    AppError, AppResult, CredentialOwner, CredentialPending, CredentialState, ErrorCode,
    SaveAgentCredentialRequest, SaveAgentSettingsRequest, SecretEntry, SecretSource,
};

/// The Claude Code profile (created by migration 0009, with one OpenCode
/// profile per provider).
pub const CLAUDE_CODE: &str = "claude-code";

/// Whether this build offers paying with a Claude plan. Off in release
/// builds until Anthropic's answer on its terms is recorded (SPEC.md,
/// Settings → Agents; docs/design/agent-runs.md, Subscription token).
pub const PLAN_OFFERED: bool = cfg!(debug_assertions);

/// A plan token lasts a year; Settings warns from eleven months on.
const AGEING_DAYS: i64 = 335;

/// New run's time limit, in minutes: 30 minutes to 8 hours (SPEC.md, New run).
pub const TIME_LIMIT_MINUTES: (u32, u32) = (30, 480);
pub const CPUS: (u32, u32) = (1, 64);
/// The agents need a few GiB to run at all: the spike's Claude Code container needed 3 GiB.
pub const MEMORY_MIB: (u32, u32) = (3072, 256 * 1024);
pub const WORKSPACE_GIB: (u32, u32) = (1, 500);

/// The credential owner key of a profile, which is also its Keychain account.
pub fn credential_owner(id: &str) -> String {
    format!("agent:{id}")
}

/// A token or key as pasted or read from its source. Terminal wraps the
/// token `claude setup-token` prints, and copying the wrap brings line
/// breaks and the spaces that pad them, so every whitespace and invisible
/// character is removed and the pieces joined; two tokens, or anything that
/// still does not look like one of the provider's, are refused. The error
/// never repeats the text.
pub fn normalize_credential(
    provider: AgentProvider,
    payment: AgentPayment,
    text: &str,
) -> AppResult<String> {
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
    // What starts each of the provider's keys, so two pasted together show.
    let prefixes: &[&str] = match provider {
        AgentProvider::Anthropic => &["sk-ant-"],
        AgentProvider::Openai => &["sk-proj-", "sk-svcacct-", "sk-admin-"],
        AgentProvider::Openrouter => &["sk-or-"],
    };
    if prefixes
        .iter()
        .map(|p| joined.matches(p).count())
        .sum::<usize>()
        > 1
    {
        return Err(AppError::validation(format!(
            "That is more than one {what}. Paste only the {what} itself."
        )));
    }
    let name = provider.name();
    if !joined
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        || !(32..=512).contains(&joined.len())
    {
        return Err(AppError::validation(format!(
            "That does not look like an {name} {what}."
        )));
    }
    let anthropic = joined.starts_with("sk-ant-");
    let plan = joined.starts_with("sk-ant-oat");
    let openrouter = joined.starts_with("sk-or-");
    match (provider, payment) {
        (AgentProvider::Anthropic, AgentPayment::ClaudePlan) if !plan => {
            Err(AppError::validation(if anthropic {
                "That is an API key, not a Claude plan token. Choose API key, or paste what claude setup-token printed."
            } else {
                "That is not a Claude plan token: run claude setup-token in Terminal and paste what it prints."
            }))
        }
        (_, AgentPayment::ClaudePlan) if provider != AgentProvider::Anthropic => Err(
            AppError::validation("Only Claude Code is paid with a Claude plan."),
        ),
        (_, AgentPayment::ApiKey) if plan => Err(AppError::validation(
            "That is a Claude plan token, not an API key. Only Claude Code is paid with a Claude plan.",
        )),
        (AgentProvider::Anthropic, AgentPayment::ApiKey) if !anthropic => Err(
            AppError::validation(
                "That does not look like an Anthropic API key, which starts with sk-ant-.",
            ),
        ),
        (AgentProvider::Openrouter, _) if !openrouter => Err(AppError::validation(
            "That does not look like an OpenRouter API key, which starts with sk-or-.",
        )),
        (AgentProvider::Openai, _) if !joined.starts_with("sk-") || anthropic || openrouter => {
            Err(AppError::validation(
                "That does not look like an OpenAI API key, which starts with sk-.",
            ))
        }
        _ => Ok(joined),
    }
}

/// The variable the container's agent reads the token or key from.
pub fn credential_key(profile: &AgentProfile) -> CredentialKey {
    match (profile.provider, profile.payment) {
        (_, AgentPayment::ClaudePlan) => CredentialKey::ClaudeCodeOauthToken,
        (AgentProvider::Anthropic, AgentPayment::ApiKey) => CredentialKey::AnthropicApiKey,
        (AgentProvider::Openai, AgentPayment::ApiKey) => CredentialKey::OpenaiApiKey,
        (AgentProvider::Openrouter, AgentPayment::ApiKey) => CredentialKey::OpenrouterApiKey,
    }
}

/// A profile's name, as Settings and New run show it: "Claude Code", or
/// "OpenCode · OpenRouter".
pub fn profile_name(p: &AgentProfile) -> String {
    match p.agent {
        AgentKind::ClaudeCode => p.agent.name().to_string(),
        AgentKind::Opencode => format!("{} · {}", p.agent.name(), p.provider.name()),
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

/// A model as typed: for Claude Code an alias such as `sonnet` or
/// `opus[1m]`, or a full name, and empty for its default; for OpenCode the
/// provider's model name, which OpenRouter's carry a slash in
/// (`anthropic/claude-sonnet-5-5`). Only the characters model names use, so
/// the value is safe in the container's environment.
pub fn check_model(agent: AgentKind, text: &str) -> AppResult<String> {
    let model = text.trim();
    let plain =
        |c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':' | '[' | ']' | '/');
    if model.len() > 96 || !model.chars().all(plain) {
        return Err(AppError::validation(match agent {
            AgentKind::ClaudeCode => {
                "The model is an alias such as sonnet, or a full model name, with no spaces."
            }
            AgentKind::Opencode => "The model is the provider's model name, with no spaces.",
        }));
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
fn destination(provider: AgentProvider, payment: AgentPayment) -> String {
    match payment {
        AgentPayment::ClaudePlan => "Anthropic, under your Claude plan".to_string(),
        AgentPayment::ApiKey => format!("{}, with an API key", provider.name()),
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

    pub async fn profile(&self, id: &str) -> AppResult<AgentProfile> {
        let id = id.to_string();
        self.db
            .call(move |conn| get(conn, &id))
            .await?
            .ok_or_else(|| AppError::not_found("There is no such agent profile."))
    }

    pub async fn profiles(&self) -> AppResult<Vec<AgentProfile>> {
        self.db.call(|conn| list(conn)).await
    }

    pub async fn get(&self) -> AppResult<AgentSettings> {
        let profiles = self.profiles().await?;
        let (engine_socket, hosts) = self
            .db
            .call(|conn| Ok((local_socket(conn)?, super::hosts::list(conn)?)))
            .await?;
        Ok(AgentSettings {
            profiles,
            engine_socket,
            plan_offered: PLAN_OFFERED,
            hosts,
        })
    }

    /// One profile's agreement and new runs' defaults: everything but its
    /// token or key.
    pub async fn save(&self, request: SaveAgentSettingsRequest) -> AppResult<AgentSettings> {
        let id = request.profile_id.clone();
        // Held so a credential save cannot interleave with this one.
        let gate = self.credentials.gate(&credential_owner(&id)).await;
        let stored = self.profile(&id).await?;
        let time_limit = in_range(
            "The time limit",
            request.time_limit_minutes,
            TIME_LIMIT_MINUTES,
        )?;
        let cpus = in_range("CPUs", request.cpus, CPUS)?;
        let memory = in_range("Memory in MiB", request.memory_mib, MEMORY_MIB)?;
        let workspace = in_range("The workspace in GiB", request.workspace_gib, WORKSPACE_GIB)?;
        let model = check_model(stored.agent, &request.model)?;
        let (expected, agreed, permissions) = (
            request.expected_version,
            request.sends_code_agreed,
            request.permissions,
        );
        let now = now_rfc3339();
        self.db
            .call(move |conn| {
                let changed = conn.execute(
                    "UPDATE agent_profiles SET sends_code_agreed = ?2, permissions = ?3,
                       time_limit_minutes = ?4, cpus = ?5, memory_mib = ?6, workspace_gib = ?7,
                       model = ?10, version = version + 1, updated_at = ?8
                     WHERE id = ?1 AND version = ?9",
                    params![
                        id,
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
                Ok(())
            })
            .await?;
        drop(gate);
        tracing::info!(profile = %request.profile_id, "agent settings saved");
        self.get().await
    }

    /// This Mac's engine. Choosing another one forgets the image built on
    /// the old one, and with it every profile's test there.
    pub async fn choose_engine(&self, socket: Option<String>) -> AppResult<AgentSettings> {
        let stored = self.db.call(|conn| local_socket(conn)).await?;
        let socket = match socket.as_deref().map(str::trim) {
            None | Some("") => None,
            // The engine already chosen stays chosen while it is stopped.
            Some(s) if Some(s) == stored.as_deref() => Some(s.to_string()),
            Some(s) => Some(check_engine(s).await?),
        };
        if socket != stored {
            let now = now_rfc3339();
            self.db
                .call(move |conn| {
                    conn.execute(
                        "UPDATE agent_hosts SET socket = ?2, image_id = NULL, image_recipe = NULL,
                           image_built_at = NULL, version = version + 1, updated_at = ?3
                         WHERE id = ?1",
                        params![super::hosts::LOCAL_ID, socket, now],
                    )?;
                    Ok(())
                })
                .await?;
            tracing::info!("agent engine chosen");
        }
        self.get().await
    }

    /// **Pay with**: a new token or key, or a new source for it. Changing
    /// how runs are paid for asks again to agree to send code.
    pub async fn save_credential(
        &self,
        request: SaveAgentCredentialRequest,
    ) -> AppResult<AgentSettings> {
        let payment = request.payment;
        let id = request.profile_id.clone();
        if payment == AgentPayment::ClaudePlan && !PLAN_OFFERED {
            return Err(AppError::validation(
                "Paying with a Claude plan is not available in this version of Brainiac. Use an API key.",
            ));
        }
        let source = credential_source(&request.source)?;
        let owner = credential_owner(&id);
        let gate = self.credentials.gate(&owner).await;
        let stored = self.profile(&id).await?;
        if stored.version != request.expected_version {
            return Err(conflict());
        }
        if payment == AgentPayment::ClaudePlan && stored.agent != AgentKind::ClaudeCode {
            return Err(AppError::validation(
                "Only Claude Code is paid with a Claude plan.",
            ));
        }
        let provider = stored.provider;
        let pasted = match source {
            SecretSource::Store => request
                .secret
                .as_deref()
                .map(|text| normalize_credential(provider, payment, text))
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
            let (expected, id) = (request.expected_version, id.clone());
            self.db
                .call(move |conn| mark_pending(conn, &id, CredentialPending::Save, expected))
                .await?;
            self.credentials.begin(&gate);
            self.credentials
                .write_item(&gate, SecretBytes::from_text(value))
                .await
                .map_err(partial_save)?;
        }
        let committed = {
            let (source, now, id) = (source.clone(), now_rfc3339(), id.clone());
            let pending = pending_cleanup.then_some(CredentialPending::Cleanup);
            self.db
                .call(move |conn| {
                    commit_credential(conn, &id, payment, &source, revision, pending, agreed, &now)
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
                tracing::warn!(error = %e, "the old agent Keychain item was not deleted");
            }
        }
        drop(gate);
        tracing::info!(profile = %id, payment = payment.as_str(), "agent credential saved");
        self.get().await
    }

    /// **Remove**: runs have no token or key until another is saved. The
    /// Keychain item, if any, is deleted after the row no longer names it.
    pub async fn remove_credential(
        &self,
        id: &str,
        expected_version: i64,
    ) -> AppResult<AgentSettings> {
        let owner = credential_owner(id);
        let gate = self.credentials.gate(&owner).await;
        let stored = self.profile(id).await?;
        if stored.version != expected_version {
            return Err(conflict());
        }
        let uncertain = stored.credential_source == SecretSource::Store
            || stored.credential.pending == Some(CredentialPending::Save);
        let pending = (uncertain || stored.credential.pending == Some(CredentialPending::Cleanup))
            .then_some(CredentialPending::Cleanup);
        let revision = (stored.credential.revision + 1).max(self.credentials.next_revision(&owner));
        let (payment, now, profile_id) = (stored.payment, now_rfc3339(), id.to_string());
        self.db
            .call(move |conn| {
                commit_credential(
                    conn,
                    &profile_id,
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
            if let Err(e) = self.cleanup(&gate, id).await {
                tracing::warn!(error = %e, "the agent Keychain item was not deleted");
            }
        }
        drop(gate);
        tracing::info!("agent credential removed");
        self.get().await
    }

    /// Delete the old Keychain item after a move to another source.
    async fn cleanup(&self, gate: &OwnerGate, id: &str) -> AppResult<()> {
        self.credentials.delete_item(gate).await?;
        let id = id.to_string();
        self.db
            .call(move |conn| clear_pending(conn, &id, CredentialPending::Cleanup))
            .await
    }

    /// Settings → Secrets, **Retry**: finish a pending cleanup.
    pub async fn retry_cleanup(&self, id: &str) -> AppResult<()> {
        let profile = self.profile(id).await?;
        let gate = self.credentials.gate(&credential_owner(id)).await;
        match profile.credential.pending {
            Some(CredentialPending::Cleanup)
                if profile.credential_source == SecretSource::Store =>
            {
                // Moving back to the Keychain wrote a new item over the old one.
                let id = id.to_string();
                self.db
                    .call(move |conn| clear_pending(conn, &id, CredentialPending::Cleanup))
                    .await
            }
            Some(CredentialPending::Cleanup) | Some(CredentialPending::Removal) => {
                self.cleanup(&gate, id).await
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
        self.profile(id).await?;
        let gate = self.credentials.gate(&credential_owner(id)).await;
        let profile_id = id.to_string();
        self.db
            .call(move |conn| {
                let changed = conn.execute(
                    "UPDATE agent_profiles SET source_approved = 1, version = version + 1
                     WHERE id = ?1 AND credential_revision = ?2",
                    params![profile_id, revision],
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

    /// **Build image** / **Rebuild…** on this Mac: build Brainiac's
    /// Dockerfile on the chosen engine and record the image. The build takes
    /// minutes; the record is kept only if the engine is still the one it was
    /// built on.
    pub async fn build_image(&self) -> AppResult<AgentSettings> {
        let Ok(_building) = self.building.try_lock() else {
            return Err(AppError::validation("The image is already being built."));
        };
        let socket = self
            .db
            .call(|conn| local_socket(conn))
            .await?
            .ok_or_else(|| AppError::validation("Choose where runs execute first."))?;
        let built = image::build(Path::new(&socket)).await?;
        let now = now_rfc3339();
        let recorded =
            self.db
                .call(move |conn| {
                    Ok(conn.execute(
                    "UPDATE agent_hosts SET image_id = ?2, image_recipe = ?3, image_built_at = ?4,
                       version = version + 1, updated_at = ?4
                     WHERE id = ?1 AND socket = ?5",
                    params![super::hosts::LOCAL_ID, built.id, image::recipe(), now, socket],
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

    /// **Test** of a profile passed on a host with this token or key, image,
    /// and engine: its runs may start there until one of them changes.
    pub async fn record_test(
        &self,
        host_id: &str,
        profile: &AgentProfile,
        image_id: &str,
        engine_socket: Option<&str>,
    ) -> AppResult<()> {
        let (host, id, revision, image, socket, now) = (
            host_id.to_string(),
            profile.id.clone(),
            profile.credential.revision,
            image_id.to_string(),
            engine_socket.map(str::to_string),
            now_rfc3339(),
        );
        self.db
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO agent_tests (host_id, profile_id, passed_at, credential_revision,
                       image_id, engine_socket)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT (host_id, profile_id) DO UPDATE SET passed_at = ?3,
                       credential_revision = ?4, image_id = ?5, engine_socket = ?6",
                    params![host, id, now, revision, image, socket],
                )?;
                Ok(())
            })
            .await
    }

    /// Settings → Secrets: each profile with a token or key, or a cleanup
    /// still to do. Reads no secret.
    pub async fn secret_entries(&self) -> AppResult<Vec<SecretEntry>> {
        Ok(self
            .profiles()
            .await?
            .into_iter()
            .filter(|p| p.credential_source != SecretSource::None || p.credential.pending.is_some())
            .map(|p| {
                let owner = credential_owner(&p.id);
                SecretEntry {
                    owner: CredentialOwner::AgentProfile { id: p.id.clone() },
                    label: format!("{} runs", profile_name(&p)),
                    destination: destination(p.provider, p.payment),
                    input_required: false,
                    last_test: self.credentials.last_test(&owner, p.credential.revision),
                    source: p.credential_source.clone(),
                    state: p.credential.clone(),
                }
            })
            .collect())
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

/// What stops a run of this profile on this host, in the order to do it:
/// the profile's own setup, the host's, then a passed test of the profile
/// there that still matches (SPEC.md, Settings → Agents, Before the first run).
pub fn run_missing(profile: &AgentProfile, host: &AgentHost) -> Vec<String> {
    let mut missing: Vec<String> = profile
        .missing
        .iter()
        .chain(&host.missing)
        .cloned()
        .collect();
    let name = profile_name(profile);
    match host.tests.iter().find(|t| t.profile_id == profile.id) {
        None => missing.push(format!("Pass a test of {name} on {}.", host.name)),
        Some(t) if !t.current => missing.push(format!(
            "Test {name} on {} again: the token or key, the image, or the engine changed since.",
            host.name
        )),
        Some(_) => {}
    }
    missing
}

/// What stops a profile's runs on any host: the restored setup, the token
/// or key, the agreement, and OpenCode's model (SPEC.md, Settings → Agents).
/// A host's own engine, image, and test are on its page.
fn profile_missing(p: &AgentProfile) -> Vec<String> {
    let mut missing = Vec::new();
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
            AgentPayment::ApiKey => format!("Add an {} API key.", p.provider.name()),
        });
    }
    if !p.sends_code_agreed {
        missing.push(format!(
            "Agree to send code and prompts to {}.",
            destination(p.provider, p.payment)
        ));
    }
    if p.agent == AgentKind::Opencode && p.model.is_empty() {
        missing.push("Choose the model new runs use.".to_string());
    }
    missing
}

const COLUMNS: &str = "id, agent, provider, payment, secret_source, credential_revision,
    source_approved, credential_pending, credential_saved_at, sends_code_agreed, permissions,
    time_limit_minutes, cpus, memory_mib, workspace_gib, model, version";

/// Claude Code first, then OpenCode by provider.
const ORDER: &str = "CASE agent WHEN 'claude_code' THEN 0 ELSE 1 END, provider";

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
    let payment: AgentPayment = parse(r.get(3)?)?;
    let saved_at: Option<String> = r.get(8)?;
    let ageing = payment == AgentPayment::ClaudePlan
        && saved_at
            .as_deref()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .is_some_and(|t| chrono::Utc::now().signed_duration_since(t).num_days() >= AGEING_DAYS);
    let mut profile = AgentProfile {
        id: r.get(0)?,
        agent: parse(r.get(1)?)?,
        provider: parse(r.get(2)?)?,
        payment,
        credential_source: parse_json(r.get(4)?)?,
        credential: CredentialState {
            revision: r.get(5)?,
            needs_approval: !r.get::<_, bool>(6)?,
            pending: r.get::<_, Option<String>>(7)?.map(parse).transpose()?,
        },
        credential_saved_at: saved_at,
        credential_ageing: ageing,
        sends_code_agreed: r.get(9)?,
        permissions: parse(r.get(10)?)?,
        time_limit_minutes: r.get(11)?,
        cpus: r.get(12)?,
        memory_mib: r.get(13)?,
        workspace_gib: r.get(14)?,
        model: r.get(15)?,
        missing: Vec::new(),
        version: r.get(16)?,
    };
    profile.missing = profile_missing(&profile);
    Ok(profile)
}

fn get(conn: &Connection, id: &str) -> AppResult<Option<AgentProfile>> {
    Ok(conn
        .query_row(
            &format!("SELECT {COLUMNS} FROM agent_profiles WHERE id = ?1"),
            [id],
            from_row,
        )
        .optional()?)
}

fn list(conn: &Connection) -> AppResult<Vec<AgentProfile>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM agent_profiles ORDER BY {ORDER}"
    ))?;
    let rows = stmt.query_map([], from_row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// This Mac's engine socket, `None` until chosen.
fn local_socket(conn: &Connection) -> AppResult<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT socket FROM agent_hosts WHERE id = ?1",
            [super::hosts::LOCAL_ID],
            |r| r.get(0),
        )
        .optional()?
        .flatten())
}

fn mark_pending(
    conn: &Connection,
    id: &str,
    pending: CredentialPending,
    expected: i64,
) -> AppResult<()> {
    let changed = conn.execute(
        "UPDATE agent_profiles SET credential_pending = ?2, version = version + 1
         WHERE id = ?1 AND version = ?3",
        params![id, word(&pending), expected],
    )?;
    if changed == 0 {
        return Err(conflict());
    }
    Ok(())
}

/// The row after a credential save or removal: the payment and source, the
/// new revision, approved by this save, and what is still pending.
// Every argument is one column of the row; a struct would only rename them.
#[allow(clippy::too_many_arguments)]
fn commit_credential(
    conn: &Connection,
    id: &str,
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
            id,
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

fn clear_pending(conn: &Connection, id: &str, which: CredentialPending) -> AppResult<()> {
    conn.execute(
        "UPDATE agent_profiles SET credential_pending = NULL
         WHERE id = ?1 AND credential_pending = ?2",
        params![id, word(&which)],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAN: &str = "sk-ant-oat01-AbCdEfGhIjKlMnOpQrStUvWxYz0123456789_-AbCd";
    const KEY: &str = "sk-ant-api03-AbCdEfGhIjKlMnOpQrStUvWxYz0123456789_-AbCd";
    const OPENAI: &str = "sk-proj-AbCdEfGhIjKlMnOpQrStUvWxYz0123456789_-AbCdEfGh";
    const OPENROUTER: &str = "sk-or-v1-0123456789abcdef0123456789abcdef0123456789abcdef";

    fn plan(text: &str) -> AppResult<String> {
        normalize_credential(AgentProvider::Anthropic, AgentPayment::ClaudePlan, text)
    }

    fn key(provider: AgentProvider, text: &str) -> AppResult<String> {
        normalize_credential(provider, AgentPayment::ApiKey, text)
    }

    #[test]
    fn wrapped_tokens_are_joined_and_anything_else_refused() {
        let wrapped = format!("  {}\r\n{}  \n", &PLAN[..20], &PLAN[20..]);
        assert_eq!(plan(&wrapped).unwrap(), PLAN);
        // Terminal pads a wrapped line with spaces before the break, and a
        // copy can carry a non-breaking or zero-width character.
        let padded = format!("{}   \n   {}\u{a0}\u{200b}", &PLAN[..30], &PLAN[30..]);
        assert_eq!(plan(&padded).unwrap(), PLAN);
        let two = format!("{PLAN}\n{PLAN}");
        let err = plan(&two).unwrap_err();
        assert!(err.message.contains("more than one"), "{err:?}");
        assert!(!err.message.contains("sk-ant"), "{err:?}");
        let sentence = format!("{PLAN} Store this token securely!");
        let err = plan(&sentence).unwrap_err();
        assert!(err.message.contains("does not look like"), "{err:?}");
        assert!(plan("").is_err());
        assert!(plan("sk-ant-oat01-short").is_err());
    }

    #[test]
    fn a_key_and_a_plan_token_are_not_mixed_up() {
        let err = plan(KEY).unwrap_err();
        assert!(err.message.contains("API key"), "{err:?}");
        let err = key(AgentProvider::Anthropic, PLAN).unwrap_err();
        assert!(err.message.contains("Claude plan"), "{err:?}");
        assert_eq!(key(AgentProvider::Anthropic, KEY).unwrap(), KEY);
        assert!(key(
            AgentProvider::Anthropic,
            "xx-AbCdEfGhIjKlMnOpQrStUvWxYz0123456789"
        )
        .is_err());
    }

    #[test]
    fn each_provider_takes_only_its_own_keys() {
        assert_eq!(key(AgentProvider::Openai, OPENAI).unwrap(), OPENAI);
        assert_eq!(
            key(AgentProvider::Openrouter, OPENROUTER).unwrap(),
            OPENROUTER
        );
        for (provider, wrong) in [
            (AgentProvider::Openai, KEY),
            (AgentProvider::Openai, OPENROUTER),
            (AgentProvider::Openrouter, OPENAI),
            (AgentProvider::Anthropic, OPENROUTER),
        ] {
            let err = key(provider, wrong).unwrap_err();
            assert!(err.message.contains("does not look like"), "{err:?}");
            assert!(err.message.contains(provider.name()), "{err:?}");
        }
        let two = format!("{OPENROUTER}{OPENROUTER}");
        assert!(key(AgentProvider::Openrouter, &two)
            .unwrap_err()
            .message
            .contains("more than one"));
        let err = normalize_credential(AgentProvider::Openai, AgentPayment::ClaudePlan, PLAN)
            .unwrap_err();
        assert!(err.message.contains("Only Claude Code"), "{err:?}");
    }

    #[test]
    fn an_openrouter_model_keeps_its_slash() {
        assert_eq!(
            check_model(AgentKind::Opencode, " anthropic/claude-sonnet-5-5 ").unwrap(),
            "anthropic/claude-sonnet-5-5"
        );
        assert!(check_model(AgentKind::Opencode, "gpt 6").is_err());
        assert!(check_model(AgentKind::ClaudeCode, "opus[1m]").is_ok());
    }
}
