//! A routine's schedule as the app does it (docs/design/routines-ux.md,
//! Schedules): it is set, then approved, which sends the run host the agent's
//! key and the code; the host fires it by itself; the app later takes the
//! briefing and the run; a change to what it was approved for turns it off
//! on the host; pause and remove reach the host or say they could not. The
//! host's controller runs in this process, reached as if over SSH.

mod fake_engine;

use std::sync::Arc;
use std::time::Duration;

use brainiac_lib::agents::controller::state::StateDir;
use brainiac_lib::agents::controller::{serve, Config};
use brainiac_lib::agents::routines::RoutineService;
use brainiac_lib::agents::{
    image, AgentRunService, AgentSettingsService, RepositoryLookup, RunArtifacts, RunRuntime,
};
use brainiac_lib::credentials::{CommandRunner, CredentialService, MemoryStore};
use brainiac_lib::db::Stores;
use brainiac_lib::git::GitService;
use brainiac_lib::models::{
    AgentPayment, AgentProvider, AnswerCodeSharingRequest, BriefingState, Routine,
    SaveAgentCredentialRequest, SaveRoutineRequest, ScheduleDaysKind, ScheduleState, ScheduleWhen,
    SecretSource, SetScheduleRequest, Settings,
};
use brainiac_lib::sharing::CodeSharingService;
use brainiac_lib::workspaces::RepositoryService;
use chrono::{Duration as Span, Local};
use fake_engine::{fake_creds, git, FakeEngine, KEY};

const CLAUDE_CODE: &str = "claude-code";
const HOST: &str = "host-1";
const BRIEFING: &str = r#"{"status":"findings","summary":"One error.",
  "memory":"Covered SENTRY-1.",
  "findings":[{"key":"SENTRY-1","title":"Boom","severity":"high"}]}"#;

struct Harness {
    repository_id: String,
    engine: FakeEngine,
    runtime: Arc<RunRuntime>,
    runs: Arc<AgentRunService>,
    routines: Arc<RoutineService>,
    _server: tokio::task::JoinHandle<()>,
    _tmp: tempfile::TempDir,
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
        // A run host reached over SSH, with its image built.
        let recipe = image::recipe();
        core.call(move |conn| {
            conn.execute_batch(&format!(
                "INSERT INTO machines (id, name, ssh_user, ssh_host, ssh_port, fingerprint,
                   created_at, updated_at)
                 VALUES ('m1', 'runner.example', 'brainiac', 'runner.example', 22, 'SHA256:x',
                   '2026-10-05T00:00:00Z', '2026-10-05T00:00:00Z');
                 INSERT INTO agent_hosts (id, kind, machine_id, installation, loop_devices,
                   image_id, image_recipe, image_built_at, created_at, updated_at)
                 VALUES ('{HOST}', 'ssh', 'm1', 'install', 1, 'sha256:abc', '{recipe}',
                   '2026-10-05T00:00:00Z', '2026-10-05T00:00:00Z', '2026-10-05T00:00:00Z');"
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

        let engine = FakeEngine::default();
        engine.get().volumes = tmp.path().join("volumes");
        let state = StateDir::new(tmp.path().join("c"));
        let socket = tmp.path().join("c").join("runner.sock");
        let config = Config {
            state: state.clone(),
            socket: socket.clone(),
            idle_exit: None,
            spawn_guard: false,
            creds_program: Some(fake_creds(tmp.path())),
            service: true,
        };
        let serving = engine.clone();
        let server = tokio::spawn(async move {
            serve(config, serving).await.unwrap();
        });
        let runtime = Arc::new(RunRuntime::attach(state, socket));
        for _ in 0..100 {
            if runtime.running().await.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
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
                StateDir::new(tmp.path().join("local")),
                tmp.path().join("local").join("runner.sock"),
            )),
            Arc::clone(&repositories) as Arc<dyn RepositoryLookup>,
            Arc::new(move |event| {
                let _ = sender.send(event.run_id.clone());
            }),
        );
        runs.use_runtime_for_host(HOST, Arc::clone(&runtime));
        let routines = RoutineService::new(core, history, Arc::clone(&runs), settings, changes);
        Harness {
            repository_id,
            engine,
            runtime,
            runs,
            routines,
            _server: server,
            _tmp: tmp,
        }
    }

    async fn routine(&self) -> Routine {
        self.routines
            .save(SaveRoutineRequest {
                id: None,
                expected_version: 0,
                name: "Morning check".into(),
                repository_id: self.repository_id.clone(),
                profile_id: CLAUDE_CODE.into(),
                host_id: HOST.into(),
                start: "main".into(),
                prompt: "Check production.".into(),
                time_limit_minutes: 30,
                memory: "Ignore SENTRY-9.".into(),
                previous_results: 3,
            })
            .await
            .unwrap()
    }

    fn when(&self, minutes_ahead: i64) -> ScheduleWhen {
        let at = Local::now() + Span::minutes(minutes_ahead);
        ScheduleWhen {
            days: ScheduleDaysKind::Daily,
            weekday: 0,
            time: at.format("%H:%M").to_string(),
            late_start: false,
            cost_ceiling_micros: 1_000_000,
        }
    }

    async fn schedule(&self, routine: &Routine) -> brainiac_lib::models::RoutineSchedule {
        self.routines
            .overview()
            .await
            .unwrap()
            .into_iter()
            .find(|i| i.routine.id == routine.id)
            .unwrap()
            .schedule
            .expect("a schedule")
    }
}

#[tokio::test]
async fn a_schedule_is_off_until_approved_then_the_host_fires_it_and_the_app_takes_the_briefing() {
    let h = Harness::new().await;
    let routine = h.routine().await;
    // A routine on this Mac cannot be scheduled.
    assert!(h
        .routines
        .set_schedule(SetScheduleRequest {
            routine_id: "nope".into(),
            when: h.when(1),
        })
        .await
        .is_err());

    // A schedule starts from a named branch or a commit, not from HEAD.
    let moving = h
        .routines
        .save(SaveRoutineRequest {
            id: Some(routine.id.clone()),
            expected_version: routine.version,
            name: routine.name.clone(),
            repository_id: routine.repository_id.clone(),
            profile_id: routine.profile_id.clone(),
            host_id: routine.host_id.clone(),
            start: "HEAD".into(),
            prompt: routine.prompt.clone(),
            time_limit_minutes: routine.time_limit_minutes,
            memory: routine.memory.clone(),
            previous_results: routine.previous_results,
        })
        .await
        .unwrap();
    let refused = h
        .routines
        .set_schedule(SetScheduleRequest {
            routine_id: moving.id.clone(),
            when: h.when(1),
        })
        .await
        .unwrap_err();
    assert!(refused.message.contains("not HEAD"), "{refused:?}");
    let routine = h
        .routines
        .save(SaveRoutineRequest {
            id: Some(moving.id.clone()),
            expected_version: moving.version,
            name: moving.name.clone(),
            repository_id: moving.repository_id.clone(),
            profile_id: moving.profile_id.clone(),
            host_id: moving.host_id.clone(),
            start: "main".into(),
            prompt: moving.prompt.clone(),
            time_limit_minutes: moving.time_limit_minutes,
            memory: moving.memory.clone(),
            previous_results: moving.previous_results,
        })
        .await
        .unwrap();

    // Due in the next minute: off, and nothing on the host, until approved.
    h.routines
        .set_schedule(SetScheduleRequest {
            routine_id: routine.id.clone(),
            when: h.when(1),
        })
        .await
        .unwrap();
    assert_eq!(
        h.schedule(&routine).await.state,
        ScheduleState::NeedsApproval
    );
    assert!(h.runtime.schedules().await.unwrap().0.is_empty());

    h.engine.get().briefings.push_back(BRIEFING.into());
    h.routines.approve(&routine.id).await.unwrap();
    let schedule = h.schedule(&routine).await;
    assert_eq!(schedule.state, ScheduleState::Active, "{schedule:?}");
    assert!(schedule.changed.is_empty());
    let (on_host, _) = h.runtime.schedules().await.unwrap();
    assert_eq!(on_host.len(), 1);
    assert!(on_host[0].spec.enabled);
    assert!(on_host[0].credential);
    assert!(on_host[0].bundle_commit.is_some());
    assert!(schedule.code_commit.is_some());

    // The host fires it by itself; the app takes what it left.
    let mut taken = None;
    for _ in 0..170 {
        h.routines.sync_host(HOST).await.unwrap();
        let results = h.routines.results(&routine.id).await.unwrap();
        if let Some(r) = results.first() {
            taken = Some(r.clone());
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let result = taken.expect("the host never fired the schedule");
    assert_eq!(result.state, BriefingState::Ready, "{result:?}");
    assert_eq!(result.findings[0].key, "SENTRY-1");
    // The run is in Runs, tagged with the routine; the memory came back.
    let run = h.runs.get(result.run_id.as_ref().unwrap()).await.unwrap();
    assert_eq!(run.routine_id.as_deref(), Some(routine.id.as_str()));
    assert_eq!(run.title, "Morning check");
    let after = h.routines.overview().await.unwrap().remove(0).routine;
    assert_eq!(after.memory, "Covered SENTRY-1.");
    // Taken once: another sync does not bring it again.
    h.routines.sync_host(HOST).await.unwrap();
    assert_eq!(h.routines.results(&routine.id).await.unwrap().len(), 1);
}

#[tokio::test]
async fn a_change_turns_the_schedule_off_on_the_host_and_pause_and_remove_reach_it() {
    let h = Harness::new().await;
    let routine = h.routine().await;
    h.routines
        .set_schedule(SetScheduleRequest {
            routine_id: routine.id.clone(),
            // Far from now: it does not fire during this test.
            when: h.when(300),
        })
        .await
        .unwrap();
    h.routines.approve(&routine.id).await.unwrap();
    assert_eq!(h.schedule(&routine).await.state, ScheduleState::Active);

    // A new prompt is something it was not approved for.
    let routine = h.routines.overview().await.unwrap().remove(0).routine;
    let changed = h
        .routines
        .save(SaveRoutineRequest {
            id: Some(routine.id.clone()),
            expected_version: routine.version,
            name: routine.name.clone(),
            repository_id: routine.repository_id.clone(),
            profile_id: routine.profile_id.clone(),
            host_id: routine.host_id.clone(),
            start: routine.start.clone(),
            prompt: "Check production and the database.".into(),
            time_limit_minutes: routine.time_limit_minutes,
            memory: routine.memory.clone(),
            previous_results: routine.previous_results,
        })
        .await
        .unwrap();
    let schedule = h.schedule(&changed).await;
    assert_eq!(schedule.state, ScheduleState::NeedsApproval);
    assert_eq!(schedule.changed, ["the prompt"]);
    assert!(!h.runtime.schedules().await.unwrap().0[0].spec.enabled);
    // It cannot be resumed without approval.
    assert!(h.routines.set_paused(&routine.id, false).await.is_err());

    // Approved again, then paused and resumed: the host follows each time.
    h.routines.approve(&routine.id).await.unwrap();
    assert!(h.runtime.schedules().await.unwrap().0[0].spec.enabled);
    h.routines.set_paused(&routine.id, true).await.unwrap();
    assert!(!h.runtime.schedules().await.unwrap().0[0].spec.enabled);
    assert_eq!(h.schedule(&routine).await.state, ScheduleState::Paused);
    h.routines.set_paused(&routine.id, false).await.unwrap();
    assert!(h.runtime.schedules().await.unwrap().0[0].spec.enabled);

    // Removing it removes it there.
    h.routines.remove_schedule(&routine.id).await.unwrap();
    assert!(h.runtime.schedules().await.unwrap().0.is_empty());
    let overview = h.routines.overview().await.unwrap();
    assert!(overview[0].schedule.is_none());
}
