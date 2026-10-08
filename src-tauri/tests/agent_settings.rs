//! Settings → Agents (SPEC.md, section 13): the profiles, each an agent and
//! a provider; the token or key through the credentials layer (the Keychain
//! is in memory here), what still stops a run, a payment change asking again
//! to agree to send code, an interrupted save staying blocked, cleanup of an
//! old item, confirmation of a restored setup, and an engine change
//! forgetting the image.

use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::sync::Arc;

use brainiac_lib::agents::settings::run_missing;
use brainiac_lib::agents::AgentSettingsService;
use brainiac_lib::credentials::{CommandRunner, CredentialService, MemoryStore, Resolution};
use brainiac_lib::db::{self, Db};
use brainiac_lib::models::{
    AgentKind, AgentPayment, AgentProfile, AgentProvider, AgentSettings, CredentialOwner,
    CredentialPending, ErrorCode, RunPermissions, SaveAgentCredentialRequest,
    SaveAgentSettingsRequest, SecretSource,
};
use brainiac_lib::secrets::SecretsService;

const KEY: &str = "sk-ant-api03-AbCdEfGhIjKlMnOpQrStUvWxYz0123456789_-AbCd";
const PLAN: &str = "sk-ant-oat01-AbCdEfGhIjKlMnOpQrStUvWxYz0123456789_-AbCd";
const ITEM: &str = "agent:claude-code";
const CLAUDE_CODE: &str = "claude-code";
const OPENROUTER_KEY: &str = "sk-or-v1-0123456789abcdef0123456789abcdef0123456789abcdef";

/// The Claude Code profile, as Settings shows it.
fn claude(s: &AgentSettings) -> &AgentProfile {
    profile(s, CLAUDE_CODE)
}

fn profile<'a>(s: &'a AgentSettings, id: &str) -> &'a AgentProfile {
    s.profiles.iter().find(|p| p.id == id).unwrap()
}

/// What stops a Claude Code run on this Mac.
fn missing(s: &AgentSettings) -> Vec<String> {
    run_missing(claude(s), &s.hosts[0])
}

struct Harness {
    tmp: tempfile::TempDir,
    core: Db,
    store: Arc<MemoryStore>,
    credentials: Arc<CredentialService>,
    agents: Arc<AgentSettingsService>,
}

impl Harness {
    fn new() -> Harness {
        let tmp = tempfile::tempdir().unwrap();
        let core = Db::open(&tmp.path().join(db::CORE_FILE)).unwrap();
        let store = Arc::new(MemoryStore::default());
        let credentials = Arc::new(CredentialService::new(
            store.clone(),
            CommandRunner::new(tmp.path().join("commands")),
        ));
        let agents = Arc::new(AgentSettingsService::new(
            core.clone(),
            Arc::clone(&credentials),
        ));
        Harness {
            tmp,
            core,
            store,
            credentials,
            agents,
        }
    }

    async fn sql(&self, sql: &'static str) {
        self.core
            .call(move |conn| Ok(conn.execute_batch(sql)?))
            .await
            .unwrap();
    }

    /// What a run would be handed, read through the credentials layer.
    async fn credential(&self) -> Option<String> {
        let profile = self.agents.profile(CLAUDE_CODE).await.unwrap();
        match self
            .credentials
            .resolve(&AgentSettingsService::binding(&profile))
            .await
            .ok()?
        {
            Resolution::Secret(lease) => {
                Some(String::from_utf8_lossy(lease.bytes().expose()).into_owned())
            }
            _ => None,
        }
    }
}

fn credential(
    settings: &AgentSettings,
    payment: AgentPayment,
    source: SecretSource,
    secret: Option<&str>,
) -> SaveAgentCredentialRequest {
    credential_for(settings, CLAUDE_CODE, payment, source, secret)
}

fn credential_for(
    settings: &AgentSettings,
    id: &str,
    payment: AgentPayment,
    source: SecretSource,
    secret: Option<&str>,
) -> SaveAgentCredentialRequest {
    SaveAgentCredentialRequest {
        profile_id: id.to_string(),
        expected_version: profile(settings, id).version,
        payment,
        source,
        secret: secret.map(str::to_string),
    }
}

fn settings_request(settings: &AgentSettings) -> SaveAgentSettingsRequest {
    settings_request_for(settings, CLAUDE_CODE)
}

fn settings_request_for(settings: &AgentSettings, id: &str) -> SaveAgentSettingsRequest {
    let p = profile(settings, id);
    SaveAgentSettingsRequest {
        profile_id: id.to_string(),
        expected_version: p.version,
        sends_code_agreed: p.sends_code_agreed,
        permissions: p.permissions,
        time_limit_minutes: p.time_limit_minutes,
        cpus: p.cpus,
        memory_mib: p.memory_mib,
        workspace_gib: p.workspace_gib,
        model: p.model.clone(),
    }
}

/// A stand-in Docker engine on a socket: it answers `/info` and `/version`
/// as an engine named `os` would, one connection at a time.
fn fake_engine(socket: &std::path::Path, os: &'static str) {
    let listener = UnixListener::bind(socket).unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let mut request = Vec::new();
            let mut buf = [0u8; 1024];
            while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                match stream.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => request.extend_from_slice(&buf[..n]),
                }
            }
            let body = if request.starts_with(b"GET /version") {
                r#"{"ApiVersion":"1.51"}"#.to_string()
            } else {
                format!(r#"{{"OperatingSystem":"{os}","ServerVersion":"28.3.2","NCPU":8}}"#)
            };
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
}

fn command(program: &str) -> SecretSource {
    SecretSource::Command {
        program: program.to_string(),
        args: vec![],
    }
}

#[tokio::test]
async fn a_new_setup_lists_everything_a_run_still_needs() {
    let h = Harness::new();
    let s = h.agents.get().await.unwrap();
    // Claude Code, then OpenCode with each provider.
    let profiles: Vec<_> = s
        .profiles
        .iter()
        .map(|p| (p.id.as_str(), p.agent, p.provider))
        .collect();
    assert_eq!(
        profiles,
        [
            (CLAUDE_CODE, AgentKind::ClaudeCode, AgentProvider::Anthropic),
            (
                "opencode-anthropic",
                AgentKind::Opencode,
                AgentProvider::Anthropic
            ),
            (
                "opencode-openai",
                AgentKind::Opencode,
                AgentProvider::Openai
            ),
            (
                "opencode-openrouter",
                AgentKind::Opencode,
                AgentProvider::Openrouter
            ),
        ]
    );
    let p = claude(&s);
    assert_eq!(p.payment, AgentPayment::ApiKey);
    assert_eq!(p.credential_source, SecretSource::None);
    assert_eq!(p.permissions, RunPermissions::Ask);
    assert_eq!(p.time_limit_minutes, 60);
    assert!(s.hosts[0].image.is_none());
    let missing = missing(&s).join(" | ");
    for want in [
        "Add an Anthropic API key",
        "Agree to send",
        "Choose where",
        "Build the image",
        "Pass a test of Claude Code on This Mac",
    ] {
        assert!(missing.contains(want), "{missing}");
    }
    // OpenCode has no model of its own to fall back to.
    let openrouter = profile(&s, "opencode-openrouter");
    assert!(openrouter.missing.iter().any(|m| m.contains("model")));
    assert!(
        openrouter.missing[0].contains("OpenRouter API key"),
        "{openrouter:?}"
    );
    // Nothing to list in Settings → Secrets until there is a key.
    assert!(h.agents.secret_entries().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_pasted_key_goes_to_the_keychain_and_reaches_a_run() {
    let h = Harness::new();
    let s = h.agents.get().await.unwrap();
    let wrapped = format!("{}\n{}\n", &KEY[..30], &KEY[30..]);
    let request = credential(
        &s,
        AgentPayment::ApiKey,
        SecretSource::Store,
        Some(&wrapped),
    );
    assert!(!format!("{request:?}").contains("sk-ant"));
    let saved = h.agents.save_credential(request).await.unwrap();
    assert_eq!(h.store.text(ITEM).as_deref(), Some(KEY));
    assert_eq!(claude(&saved).credential_source, SecretSource::Store);
    assert!(claude(&saved).credential_saved_at.is_some());
    assert!(claude(&saved).credential.revision > claude(&s).credential.revision);
    assert_eq!(h.credential().await.as_deref(), Some(KEY));
    assert!(!missing(&saved).iter().any(|m| m.contains("Add an")));

    // Saving over an older version of the pane is refused.
    let stale = credential(&s, AgentPayment::ApiKey, SecretSource::Store, Some(KEY));
    assert_eq!(
        h.agents.save_credential(stale).await.unwrap_err().code,
        ErrorCode::Conflict
    );

    let entries = h.agents.secret_entries().await.unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].owner,
        CredentialOwner::AgentProfile {
            id: "claude-code".into()
        }
    );
}

#[tokio::test]
async fn a_new_payment_needs_a_new_paste_and_a_new_agreement() {
    let h = Harness::new();
    let s = h.agents.get().await.unwrap();
    let s = h
        .agents
        .save_credential(credential(
            &s,
            AgentPayment::ApiKey,
            SecretSource::Store,
            Some(KEY),
        ))
        .await
        .unwrap();
    let mut agree = settings_request(&s);
    agree.sends_code_agreed = true;
    let s = h.agents.save(agree).await.unwrap();
    assert!(claude(&s).sends_code_agreed);

    // A key pasted as a plan token is refused without echoing it.
    let err = h
        .agents
        .save_credential(credential(
            &s,
            AgentPayment::ClaudePlan,
            SecretSource::Store,
            Some(KEY),
        ))
        .await
        .unwrap_err();
    assert!(
        err.message.contains("API key") && !err.message.contains("sk-ant"),
        "{err:?}"
    );
    // The item holds a key, not a plan token: switching needs a paste.
    let err = h
        .agents
        .save_credential(credential(
            &s,
            AgentPayment::ClaudePlan,
            SecretSource::Store,
            None,
        ))
        .await
        .unwrap_err();
    assert!(err.message.contains("claude setup-token"), "{err:?}");

    let s = h
        .agents
        .save_credential(credential(
            &s,
            AgentPayment::ClaudePlan,
            SecretSource::Store,
            Some(PLAN),
        ))
        .await
        .unwrap();
    assert_eq!(claude(&s).payment, AgentPayment::ClaudePlan);
    assert!(!claude(&s).sends_code_agreed);
    assert!(!claude(&s).credential_ageing);
    assert_eq!(h.credential().await.as_deref(), Some(PLAN));

    // Saved eleven months ago: Settings warns.
    h.sql("UPDATE agent_profiles SET credential_saved_at = '2020-01-01T00:00:00Z'")
        .await;
    assert!(claude(&h.agents.get().await.unwrap()).credential_ageing);
}

#[tokio::test]
async fn an_interrupted_keychain_save_stays_blocked_until_saved_again() {
    let h = Harness::new();
    let s = h.agents.get().await.unwrap();
    h.store.fail_writes(true);
    let err = h
        .agents
        .save_credential(credential(
            &s,
            AgentPayment::ApiKey,
            SecretSource::Store,
            Some(KEY),
        ))
        .await
        .unwrap_err();
    assert!(err.message.contains("saved again"), "{err:?}");
    let s = h.agents.get().await.unwrap();
    assert_eq!(claude(&s).credential.pending, Some(CredentialPending::Save));
    assert!(missing(&s).iter().any(|m| m.contains("did not finish")));
    assert_eq!(h.credential().await, None);

    h.store.fail_writes(false);
    let s = h
        .agents
        .save_credential(credential(
            &s,
            AgentPayment::ApiKey,
            SecretSource::Store,
            Some(KEY),
        ))
        .await
        .unwrap();
    assert_eq!(claude(&s).credential.pending, None);
    assert_eq!(h.credential().await.as_deref(), Some(KEY));
}

#[tokio::test]
async fn moving_to_a_command_deletes_the_old_item_and_a_failed_delete_is_retried() {
    let h = Harness::new();
    let s = h.agents.get().await.unwrap();
    let s = h
        .agents
        .save_credential(credential(
            &s,
            AgentPayment::ApiKey,
            SecretSource::Store,
            Some(KEY),
        ))
        .await
        .unwrap();
    let program = h.tmp.path().join("print-key");
    std::fs::write(&program, format!("#!/bin/sh\necho {KEY}\n")).unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();

    h.store.fail_deletes(true);
    let s = h
        .agents
        .save_credential(credential(
            &s,
            AgentPayment::ApiKey,
            command(program.to_str().unwrap()),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(
        claude(&s).credential.pending,
        Some(CredentialPending::Cleanup)
    );
    assert!(h.store.text(ITEM).is_some());
    // The command is the source now; the old item is never a fallback.
    assert_eq!(h.credential().await.as_deref(), Some(KEY));

    h.store.fail_deletes(false);
    let secrets = SecretsService::new(
        Arc::clone(&h.credentials),
        Arc::new(brainiac_lib::forge::AccountService::new(
            h.core.clone(),
            Arc::clone(&h.credentials),
            brainiac_lib::forge::http::Http::new().unwrap(),
            brainiac_lib::forge::Endpoints::production(),
        )),
        Arc::new(brainiac_lib::databases::ConnectionService::new(
            h.core.clone(),
            Arc::clone(&h.credentials),
        )),
        Arc::clone(&h.agents),
    );
    let owner = CredentialOwner::AgentProfile {
        id: "claude-code".into(),
    };
    secrets.retry(&owner).await.unwrap();
    assert!(h.store.text(ITEM).is_none());
    assert_eq!(
        claude(&h.agents.get().await.unwrap()).credential.pending,
        None
    );

    // Remove: no key at all.
    let s = h.agents.get().await.unwrap();
    let s = h
        .agents
        .remove_credential(CLAUDE_CODE, claude(&s).version)
        .await
        .unwrap();
    assert_eq!(claude(&s).credential_source, SecretSource::None);
    assert!(missing(&s)
        .iter()
        .any(|m| m.contains("Add an Anthropic API key")));
}

#[tokio::test]
async fn a_restored_setup_is_not_read_until_confirmed() {
    let h = Harness::new();
    let s = h.agents.get().await.unwrap();
    h.agents
        .save_credential(credential(
            &s,
            AgentPayment::ApiKey,
            SecretSource::Store,
            Some(KEY),
        ))
        .await
        .unwrap();
    // What a restore does (backup.rs).
    h.sql("UPDATE agent_profiles SET source_approved = 0").await;
    h.credentials.invalidate(ITEM);
    let s = h.agents.get().await.unwrap();
    assert!(claude(&s).credential.needs_approval);
    assert!(missing(&s).iter().any(|m| m.contains("Confirm")));
    assert_eq!(h.credential().await, None);

    let s = h
        .agents
        .approve(CLAUDE_CODE, claude(&s).credential.revision)
        .await
        .unwrap();
    assert!(!claude(&s).credential.needs_approval);
    assert_eq!(h.credential().await.as_deref(), Some(KEY));
}

#[tokio::test]
async fn limits_are_checked_and_another_engine_forgets_the_image() {
    let h = Harness::new();
    let s = h.agents.get().await.unwrap();
    let mut short = settings_request(&s);
    short.time_limit_minutes = 10;
    assert!(h
        .agents
        .save(short)
        .await
        .unwrap_err()
        .message
        .contains("time limit"));

    // The model: an alias or a name, or empty for Claude Code's default.
    let mut spaced = settings_request(&s);
    spaced.model = "claude sonnet".into();
    let err = h.agents.save(spaced).await.unwrap_err();
    assert!(err.message.contains("model"), "{err:?}");
    let mut alias = settings_request(&s);
    alias.model = " opus[1m] ".into();
    let s = h.agents.save(alias).await.unwrap();
    assert_eq!(claude(&s).model, "opus[1m]");
    let mut none = settings_request(&s);
    none.model = String::new();
    let s = h.agents.save(none).await.unwrap();
    assert_eq!(claude(&s).model, "");

    let path = |name: &str| Some(h.tmp.path().join(name).display().to_string());
    assert_eq!(
        h.agents
            .choose_engine(path("none.sock"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );

    let socket = h.tmp.path().join("a.sock");
    // An engine runs were not tested on is not offered.
    fake_engine(&h.tmp.path().join("untested.sock"), "Ubuntu 24.04");
    let err = h
        .agents
        .choose_engine(path("untested.sock"))
        .await
        .unwrap_err();
    assert!(err.message.contains("cannot run agents"), "{err:?}");

    fake_engine(&socket, "OrbStack");
    let s = h.agents.choose_engine(path("a.sock")).await.unwrap();
    assert_eq!(s.engine_socket.as_deref(), socket.to_str());
    assert!(!missing(&s).iter().any(|m| m.contains("Choose where")));
    let mut chosen = settings_request(&s);
    chosen.permissions = RunPermissions::Act;
    chosen.time_limit_minutes = 120;
    let s = h.agents.save(chosen).await.unwrap();
    assert_eq!(claude(&s).permissions, RunPermissions::Act);

    // The chosen engine stopped: it stays chosen.
    let stopped = h.tmp.path().join("stopped.sock");
    fake_engine(&stopped, "OrbStack");
    h.agents.choose_engine(path("stopped.sock")).await.unwrap();
    std::fs::remove_file(&stopped).unwrap();
    let s = h.agents.choose_engine(path("stopped.sock")).await.unwrap();
    assert_eq!(s.engine_socket.as_deref(), stopped.to_str());
    h.agents.choose_engine(path("a.sock")).await.unwrap();

    h.sql(
        "UPDATE agent_hosts SET image_id = 'sha256:1', image_recipe = 'old', image_built_at = '2026-10-05T00:00:00Z' WHERE id = 'local'",
    )
    .await;
    let s = h.agents.get().await.unwrap();
    assert!(s.hosts[0].image.as_ref().is_some_and(|i| !i.current));
    assert!(missing(&s).iter().any(|m| m.contains("Rebuild")));

    // The same engine again keeps it; another one does not.
    let s = h.agents.choose_engine(path("a.sock")).await.unwrap();
    assert!(s.hosts[0].image.is_some());
    fake_engine(&h.tmp.path().join("b.sock"), "Docker Desktop");
    let s = h.agents.choose_engine(path("b.sock")).await.unwrap();
    assert!(s.hosts[0].image.is_none());
}

#[tokio::test]
async fn each_opencode_profile_takes_its_own_providers_key_and_a_model() {
    let h = Harness::new();
    let s = h.agents.get().await.unwrap();
    let id = "opencode-openrouter";
    // Another provider's key, or a plan token, is refused without echoing it.
    let err = h
        .agents
        .save_credential(credential_for(
            &s,
            id,
            AgentPayment::ApiKey,
            SecretSource::Store,
            Some(KEY),
        ))
        .await
        .unwrap_err();
    assert!(
        err.message.contains("OpenRouter") && !err.message.contains("sk-ant"),
        "{err:?}"
    );
    let err = h
        .agents
        .save_credential(credential_for(
            &s,
            id,
            AgentPayment::ClaudePlan,
            SecretSource::Store,
            Some(PLAN),
        ))
        .await
        .unwrap_err();
    assert!(err.message.contains("Claude"), "{err:?}");
    let s = h
        .agents
        .save_credential(credential_for(
            &s,
            id,
            AgentPayment::ApiKey,
            SecretSource::Store,
            Some(OPENROUTER_KEY),
        ))
        .await
        .unwrap();
    // Its own Keychain item: Claude Code's is untouched.
    assert_eq!(
        h.store.text("agent:opencode-openrouter").as_deref(),
        Some(OPENROUTER_KEY)
    );
    assert!(h.store.text(ITEM).is_none());
    let mut agree = settings_request_for(&s, id);
    agree.sends_code_agreed = true;
    agree.model = "anthropic/claude-sonnet-5-5".into();
    let s = h.agents.save(agree).await.unwrap();
    assert_eq!(profile(&s, id).missing, Vec::<String>::new());
    assert_eq!(profile(&s, id).model, "anthropic/claude-sonnet-5-5");
    let entries = h.agents.secret_entries().await.unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].label, "OpenCode · OpenRouter runs");
    assert_eq!(entries[0].destination, "OpenRouter, with an API key");
}
