//! Briefings as the app does them (docs/design/scheduled-runs.md): Run now
//! starts a run on the fake engine, the file the agent writes is checked and
//! kept as a result, a bad file gets one follow-up turn and then fails, a
//! run that writes nothing is a failed briefing and never "nothing found",
//! and a finding seen yesterday says so. The controller runs in this process.

mod fake_engine;

use std::sync::Arc;
use std::time::Duration;

use brainiac_lib::agents::briefings::BriefingService;
use brainiac_lib::agents::controller::state::StateDir;
use brainiac_lib::agents::controller::{serve, Config};
use brainiac_lib::agents::{
    image, AgentRunService, AgentSettingsService, RepositoryLookup, RunArtifacts, RunRuntime,
};
use brainiac_lib::credentials::{CommandRunner, CredentialService, MemoryStore};
use brainiac_lib::db::Stores;
use brainiac_lib::git::GitService;
use brainiac_lib::models::{
    AgentPayment, AgentProvider, AnswerCodeSharingRequest, Briefing, BriefingResult, BriefingState,
    BriefingStatus, SaveAgentCredentialRequest, SaveBriefingRequest, SecretSource, Settings,
};
use brainiac_lib::sharing::CodeSharingService;
use brainiac_lib::workspaces::RepositoryService;
use fake_engine::{git, FakeEngine, KEY};

const SOCKET: &str = "/fake/docker.sock";
const CLAUDE_CODE: &str = "claude-code";

struct Harness {
    _tmp: tempfile::TempDir,
    repository_id: String,
    engine: FakeEngine,
    _server: tokio::task::JoinHandle<()>,
    briefings: Arc<BriefingService>,
}

impl Harness {
    async fn new() -> Harness {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("data");
        std::fs::create_dir_all(&data).unwrap();
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        std::fs::write(repo.join("README.md"), "# Service\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "First"]);

        let stores = Stores::open(&data).unwrap();
        let (core, history) = (stores.core.clone(), stores.history.clone());
        let repositories = Arc::new(RepositoryService::new(
            core.clone(),
            GitService::detect().await,
            Settings::default(),
            Arc::new(|_| {}),
        ));
        let repository_id = repositories.register(&repo).await.unwrap().id;
        let credentials = Arc::new(CredentialService::new(
            Arc::new(MemoryStore::default()),
            CommandRunner::new(data.join("commands")),
        ));
        let settings = Arc::new(AgentSettingsService::new(
            core.clone(),
            Arc::clone(&credentials),
        ));
        let recipe = image::recipe();
        core.call(move |conn| {
            conn.execute_batch(&format!(
                "UPDATE agent_hosts SET socket = '{SOCKET}', image_id = 'sha256:abc',
                   image_recipe = '{recipe}', image_built_at = '2026-10-05T00:00:00Z'
                 WHERE id = 'local';"
            ))?;
            Ok(())
        })
        .await
        .unwrap();
        let profile = settings.profile(CLAUDE_CODE).await.unwrap();
        settings
            .save_credential(SaveAgentCredentialRequest {
                profile_id: CLAUDE_CODE.into(),
                expected_version: profile.version,
                payment: AgentPayment::ApiKey,
                source: SecretSource::Store,
                secret: Some(KEY.to_string()),
            })
            .await
            .unwrap();
        let revision = settings
            .profile(CLAUDE_CODE)
            .await
            .unwrap()
            .credential
            .revision;
        core.call(move |conn| {
            conn.execute(
                "INSERT OR REPLACE INTO agent_tests (host_id, profile_id, passed_at,
                   credential_revision, image_id, engine_socket)
                 VALUES ('local', ?1, '2026-10-05T00:00:00Z', ?2, 'sha256:abc', ?3)",
                rusqlite::params![CLAUDE_CODE, revision, SOCKET],
            )?;
            Ok(())
        })
        .await
        .unwrap();

        let engine = FakeEngine::default();
        engine.get().volumes = tmp.path().join("volumes");
        let config = Config {
            state: StateDir::new(tmp.path().join("c")),
            socket: tmp.path().join("c").join("runner.sock"),
            idle_exit: None,
            spawn_guard: false,
            creds_program: None,
            service: false,
        };
        let serving = engine.clone();
        let server = tokio::spawn(async move {
            serve(config, serving).await.unwrap();
        });
        let artifacts = Arc::new(RunArtifacts::new(
            Some(GitService::detect().await.unwrap()),
            &data,
        ));
        let (changes, _) = tokio::sync::broadcast::channel::<String>(256);
        let sender = changes.clone();
        let sharing = CodeSharingService::new(core.clone(), Arc::new(|_| {}));
        sharing
            .answer(AnswerCodeSharingRequest {
                repository_id: repository_id.clone(),
                provider: AgentProvider::Anthropic,
                allowed: true,
                workspace_id: None,
            })
            .await
            .unwrap();
        let runs = AgentRunService::new(
            history.clone(),
            Arc::clone(&settings),
            sharing,
            credentials,
            artifacts,
            Arc::new(RunRuntime::attach(
                StateDir::new(tmp.path().join("c")),
                tmp.path().join("c").join("runner.sock"),
            )),
            Arc::clone(&repositories) as Arc<dyn RepositoryLookup>,
            Arc::new(move |event| {
                let _ = sender.send(event.run_id.clone());
            }),
        );
        for _ in 0..100 {
            if runs.controller_status().await.running {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let briefings = BriefingService::new(core, history, runs, settings, changes);
        Harness {
            _tmp: tmp,
            repository_id,
            engine,
            _server: server,
            briefings,
        }
    }

    async fn briefing(&self) -> Briefing {
        self.briefings
            .save(SaveBriefingRequest {
                id: None,
                expected_version: 0,
                name: "Morning check".into(),
                repository_id: self.repository_id.clone(),
                profile_id: CLAUDE_CODE.into(),
                host_id: String::new(),
                start: "main".into(),
                prompt: "Check production.".into(),
                time_limit_minutes: 30,
            })
            .await
            .unwrap()
    }

    /// Run now and wait for the result to leave Working.
    async fn run(&self, briefing: &Briefing) -> BriefingResult {
        let started = self.briefings.run_now(&briefing.id).await.unwrap();
        for _ in 0..400 {
            let result = self.briefings.result(&started.id).await.unwrap();
            if result.state != BriefingState::Working {
                return result;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("the briefing never finished");
    }
}

const FINDINGS: &str = r#"{"status":"findings","summary":"One error.","findings":[
    {"key":"SENTRY-1","title":"TypeError in checkout","severity":"high","link":"https://sentry.example/i/1"}],
  "proposals":[{"key":"fix-1","kind":"task","title":"Fix the checkout error","finding_key":"SENTRY-1"}]}"#;

#[tokio::test]
async fn run_now_keeps_the_checked_briefing_and_remembers_yesterdays_findings() {
    let h = Harness::new().await;
    let briefing = h.briefing().await;
    h.engine.get().briefings.push_back(FINDINGS.to_string());
    let first = h.run(&briefing).await;
    assert_eq!(first.state, BriefingState::Ready, "{first:?}");
    assert_eq!(first.status, Some(BriefingStatus::Findings));
    assert_eq!(first.findings[0].key, "SENTRY-1");
    assert!(!first.findings[0].seen_before);
    // The prompt carried the user's words and the file contract.
    let prompts = h.engine.get().prompts.clone();
    assert!(prompts[0].starts_with("Check production."));
    assert!(prompts[0].contains(".brainiac/briefing.json"));
    assert!(h
        .engine
        .get()
        .reads
        .iter()
        .all(|p| p == "/workspace/.brainiac/briefing.json"));

    // The next day the same finding is not new.
    h.engine.get().briefings.push_back(FINDINGS.to_string());
    let second = h.run(&briefing).await;
    assert_eq!(second.state, BriefingState::Ready);
    assert!(second.findings[0].seen_before);

    // A task is made once from a proposal.
    let marked = h
        .briefings
        .mark_task(&second.id, "fix-1", "task-1")
        .await
        .unwrap();
    assert_eq!(marked.proposals[0].task_id.as_deref(), Some("task-1"));
    assert!(h
        .briefings
        .mark_task(&second.id, "fix-1", "task-2")
        .await
        .is_err());

    let overview = h.briefings.overview().await.unwrap();
    assert_eq!(overview.len(), 1);
    assert_eq!(overview[0].latest.as_ref().unwrap().id, second.id);
}

#[tokio::test]
async fn a_bad_file_gets_one_follow_up_then_fails_and_no_file_is_not_nothing_found() {
    let h = Harness::new().await;
    let briefing = h.briefing().await;

    // Bad first, good second: the follow-up turn names what was wrong.
    {
        let mut engine = h.engine.get();
        engine
            .briefings
            .push_back(r#"{"status":"fine","summary":"x"}"#.into());
        engine.briefings.push_back(FINDINGS.into());
    }
    let result = h.run(&briefing).await;
    assert_eq!(result.state, BriefingState::Ready, "{result:?}");
    let prompts = h.engine.get().prompts.clone();
    assert_eq!(prompts.len(), 2);
    assert!(prompts[1].contains("\"status\" must be"));

    // Bad twice: failed, with the reason.
    h.engine.get().prompts.clear();
    {
        let mut engine = h.engine.get();
        engine.briefings.push_back("not json".into());
        engine.briefings.push_back("still not json".into());
    }
    let failed = h.run(&briefing).await;
    assert_eq!(failed.state, BriefingState::Failed);
    assert!(failed.status.is_none());
    assert!(failed.error.as_deref().unwrap().contains("usable briefing"));

    // Nothing written at all: failed, never "nothing found".
    let none = h.run(&briefing).await;
    assert_eq!(none.state, BriefingState::Failed);
    assert!(none.status.is_none());
    assert!(none.error.as_deref().unwrap().contains("briefing.json"));
}

#[tokio::test]
async fn a_start_that_is_refused_is_kept_as_a_failed_result() {
    let h = Harness::new().await;
    let mut briefing = h.briefing().await;
    briefing = h
        .briefings
        .save(SaveBriefingRequest {
            id: Some(briefing.id.clone()),
            expected_version: briefing.version,
            name: briefing.name.clone(),
            repository_id: briefing.repository_id.clone(),
            profile_id: briefing.profile_id.clone(),
            host_id: String::new(),
            start: "no-such-branch".into(),
            prompt: briefing.prompt.clone(),
            time_limit_minutes: briefing.time_limit_minutes,
        })
        .await
        .unwrap();
    let result = h.briefings.run_now(&briefing.id).await.unwrap();
    assert_eq!(result.state, BriefingState::Failed);
    assert!(result.error.as_deref().unwrap().contains("did not start"));
    // A stale version is refused when saving.
    assert!(h
        .briefings
        .save(SaveBriefingRequest {
            id: Some(briefing.id.clone()),
            expected_version: 1,
            name: "x".into(),
            repository_id: briefing.repository_id,
            profile_id: briefing.profile_id,
            host_id: String::new(),
            start: "main".into(),
            prompt: "y".into(),
            time_limit_minutes: 30,
        })
        .await
        .is_err());
}
