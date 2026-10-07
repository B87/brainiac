//! Agent runs as the app sees them (SPEC.md, section 13): starting a run
//! from a repository, following it through the run controller, the journal
//! mirrored to the Mac, prompts and permissions, Finish and collect with
//! the snapshot reviewed and exported, Cancel while the controller cannot be
//! reached, an interrupted run's Collect work, Delete, and Settings' Test.
//! The engine is the fake one of `fake_engine`; the controller runs in this
//! process.

mod fake_engine;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use brainiac_lib::agents::controller::protocol::{Credential, CredentialKey, StartRun};
use brainiac_lib::agents::controller::state::StateDir;
use brainiac_lib::agents::controller::{serve, Config};
use brainiac_lib::agents::runs::Located;
use brainiac_lib::agents::{
    image, AgentRunService, AgentSettingsService, RepositoryLookup, RunArtifacts, RunRuntime,
    StartError,
};
use brainiac_lib::credentials::{CommandRunner, CredentialService, MemoryStore};
use brainiac_lib::db::{self, Db};
use brainiac_lib::git::GitService;
use brainiac_lib::models::{
    AgentPayment, AgentRun, AgentRunChangedEvent, ErrorCode, RunActivity, RunCollection,
    RunDiffRequest, RunEventBody, RunOutcome, RunPermissions, RunPhase, SaveAgentCredentialRequest,
    SecretSource, StartRunRequest,
};
use fake_engine::{git, FakeEngine, KEY};

const SOCKET: &str = "/fake/docker.sock";

/// The one repository the tests start runs from.
struct Repos(PathBuf);

impl RepositoryLookup for Repos {
    fn locate(&self, repository_id: &str) -> Located<'_> {
        let found = if repository_id == "repo-1" {
            Ok(("example".to_string(), self.0.clone()))
        } else {
            Err(brainiac_lib::models::AppError::not_found(
                "no such repository",
            ))
        };
        Box::pin(async move { found })
    }
}

struct Harness {
    tmp: tempfile::TempDir,
    engine: FakeEngine,
    server: tokio::task::JoinHandle<()>,
    runs: Arc<AgentRunService>,
    settings: Arc<AgentSettingsService>,
    events: Arc<Mutex<Vec<AgentRunChangedEvent>>>,
    start: String,
}

impl Harness {
    async fn new() -> Harness {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("data");
        std::fs::create_dir_all(&data).unwrap();
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q"]);
        std::fs::write(repo.join("readme.txt"), "start\n").unwrap();
        std::fs::write(repo.join("old.txt"), "old\n").unwrap();
        std::fs::write(repo.join(".gitignore"), "*.tmp\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "Start"]);
        let start = git(&repo, &["rev-parse", "HEAD"]);

        let core = Db::open(&data.join(db::CORE_FILE)).unwrap();
        let history = Db::open_store(&data.join(db::HISTORY_FILE), &db::HISTORY).unwrap();
        let credentials = Arc::new(CredentialService::new(
            Arc::new(MemoryStore::default()),
            CommandRunner::new(data.join("commands")),
        ));
        let settings = Arc::new(AgentSettingsService::new(
            core.clone(),
            Arc::clone(&credentials),
        ));
        // Settings → Agents as a user would leave it: a key saved, the engine
        // and the image chosen (written straight into the rows: the real
        // engine check asks a socket), the agreement, and a passed test.
        let profile = settings.get().await.unwrap().profile;
        settings
            .save_credential(SaveAgentCredentialRequest {
                expected_version: profile.version,
                payment: AgentPayment::ApiKey,
                source: SecretSource::Store,
                secret: Some(KEY.to_string()),
            })
            .await
            .unwrap();
        let revision = settings.get().await.unwrap().profile.credential.revision;
        let recipe = image::recipe();
        core.call(move |conn| {
            conn.execute_batch(&format!(
                "UPDATE agent_hosts SET socket = '{SOCKET}';
                 UPDATE agent_profiles SET sends_code_agreed = 1, image_id = 'sha256:abc',
                   image_recipe = '{recipe}', image_built_at = '2026-10-05T00:00:00Z',
                   test_passed_at = '2026-10-05T00:00:00Z', test_credential_revision = {revision},
                   test_image_id = 'sha256:abc', test_engine_socket = '{SOCKET}';"
            ))?;
            Ok(())
        })
        .await
        .unwrap();
        assert!(settings.get().await.unwrap().missing.is_empty());

        let engine = FakeEngine::default();
        engine.get().volumes = tmp.path().join("volumes");
        let artifacts = Arc::new(RunArtifacts::new(
            Some(GitService::detect().await.unwrap()),
            &data,
        ));
        let events: Arc<Mutex<Vec<AgentRunChangedEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&events);
        let mut harness = Harness {
            server: tokio::spawn(async {}),
            runs: AgentRunService::new(
                history,
                Arc::clone(&settings),
                credentials,
                artifacts,
                Arc::new(RunRuntime::attach(
                    StateDir::new(tmp.path().join("c")),
                    tmp.path().join("c").join("runner.sock"),
                )),
                Arc::new(Repos(repo.clone())),
                Arc::new(move |event| seen.lock().unwrap().push(event)),
            ),
            settings,
            events,
            tmp,
            engine,
            start,
        };
        harness.restart().await;
        harness
    }

    fn data(&self) -> PathBuf {
        self.tmp.path().join("data")
    }

    /// Another client of the same controller, as another Brainiac would be.
    fn runtime(&self) -> RunRuntime {
        RunRuntime::attach(
            StateDir::new(self.tmp.path().join("c")),
            self.tmp.path().join("c").join("runner.sock"),
        )
    }

    /// The agent's unreadable folder, readable again so it can be collected
    /// and the temporary folder removed.
    fn open_secret(&self, run_id: &str) {
        std::fs::set_permissions(
            self.tmp.path().join("volumes").join(run_id).join("secret"),
            std::os::unix::fs::PermissionsExt::from_mode(0o755),
        )
        .unwrap();
    }

    /// Start the controller, or start it again as if the old one were killed.
    async fn restart(&mut self) {
        if !self.server.is_finished() {
            self.server.abort();
            let _ = (&mut self.server).await;
        }
        let config = Config {
            state: StateDir::new(self.tmp.path().join("c")),
            socket: self.tmp.path().join("c").join("runner.sock"),
            idle_exit: None,
            spawn_guard: false,
            service: false,
        };
        let engine = self.engine.clone();
        self.server = tokio::spawn(async move {
            serve(config, engine).await.unwrap();
        });
        for _ in 0..100 {
            if self.runs.controller_status().await.running {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("the controller did not start");
    }

    async fn start(&self, prompt: &str, permissions: RunPermissions) -> AgentRun {
        self.runs
            .start(StartRunRequest {
                repository_id: "repo-1".into(),
                start_commit: self.start.clone(),
                prompt: prompt.into(),
                permissions,
                time_limit_minutes: 60,
                cpus: 2,
                memory_mib: 2048,
                workspace_gib: 20,
                model: "sonnet".into(),
                host_id: String::new(),
            })
            .await
            .unwrap()
    }

    async fn wait(&self, id: &str, what: &str, done: impl Fn(&AgentRun) -> bool) -> AgentRun {
        for _ in 0..500 {
            let run = self.runs.get(id).await.unwrap();
            if done(&run) {
                return run;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("{what}: {:?}", self.runs.get(id).await.unwrap());
    }

    /// The journal, once it holds an event `found` accepts: an action
    /// answers from the controller's status before the mirror catches up.
    async fn wait_events(
        &self,
        id: &str,
        what: &str,
        found: impl Fn(&RunEventBody) -> bool,
    ) -> Vec<brainiac_lib::models::RunEvent> {
        for _ in 0..500 {
            let events = self.runs.events(id, 0).await.unwrap().events;
            if events.iter().any(|e| found(&e.body)) {
                return events;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("{what}: not in the journal");
    }

    /// The agent's reply in `turn`, once the journal has the turn's end:
    /// an action answers from the controller's status before the mirror
    /// catches up.
    async fn reply(&self, id: &str, turn: u32) -> String {
        for _ in 0..500 {
            let events = self.runs.events(id, 0).await.unwrap().events;
            if events
                .iter()
                .any(|e| matches!(&e.body, RunEventBody::TurnEnded { turn: t, .. } if *t == turn))
            {
                return events
                    .into_iter()
                    .filter_map(|e| match e.body {
                        RunEventBody::Message { turn: t, text } if t == turn => Some(text),
                        _ => None,
                    })
                    .collect();
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("turn {turn} did not end in the journal");
    }
}

fn idle(run: &AgentRun) -> bool {
    run.phase == RunPhase::Running && run.activity == RunActivity::Idle
}

#[tokio::test]
async fn a_run_is_started_followed_finished_collected_and_reviewed() {
    let h = Harness::new().await;
    let run = h.start("edit\nsecond line", RunPermissions::Act).await;
    assert_eq!(run.title, "edit");
    assert_eq!(run.repository_name, "example");
    assert_eq!(run.start_commit, h.start);
    assert_eq!(run.start_subject, "Start");
    assert_eq!(run.payment, AgentPayment::ApiKey);
    assert!(run.credential_source.contains("Keychain"));
    assert_eq!(run.image_name, image::name_for(&image::recipe()));
    assert!(run.deadline_at.is_some());
    // The key went to the agent, once, and the model it was asked for
    // went with it; the session reported the one it opened with.
    let run = h.wait(&run.id, "the first turn ends", idle).await;
    assert_eq!(run.turn, 1);
    assert_eq!(h.engine.get().frames.len(), 1);
    assert_eq!(h.engine.get().models, vec!["sonnet".to_string()]);
    assert_eq!(run.model, "sonnet");
    assert_eq!(run.model_used.as_deref(), Some("claude-sonnet-4-5"));

    // The journal is mirrored to the Mac, filtered, in order.
    let page = h.runs.events(&run.id, 0).await.unwrap();
    let kinds: Vec<&str> = page
        .events
        .iter()
        .map(|e| match &e.body {
            RunEventBody::Accepted { .. } => "accepted",
            RunEventBody::Ready { model, .. } => {
                assert_eq!(model.as_deref(), Some("claude-sonnet-4-5"));
                "ready"
            }
            RunEventBody::Prompt { text, .. } => {
                assert_eq!(text, "edit\nsecond line");
                "prompt"
            }
            RunEventBody::Message { .. } => "message",
            RunEventBody::TurnEnded { .. } => "turn_ended",
            _ => "other",
        })
        .collect();
    assert_eq!(
        kinds,
        ["accepted", "ready", "prompt", "message", "turn_ended"]
    );
    assert!(page.events.windows(2).all(|w| w[1].seq == w[0].seq + 1));
    assert_eq!(page.cursor, run.cursor);
    let trace = std::fs::read_to_string(
        h.data()
            .join("agent-runs")
            .join(&run.id)
            .join("trace.jsonl"),
    )
    .unwrap();
    assert_eq!(trace.lines().count(), 5);
    assert!(!trace.contains(KEY));

    // A follow-up on the same session, without the key again.
    let run = h.runs.prompt(&run.id, "hello").await.unwrap();
    let run = h
        .wait(&run.id, "the second turn ends", |r| idle(r) && r.turn == 2)
        .await;
    assert_eq!(h.reply(&run.id, 2).await, "you said: hello");
    assert_eq!(h.engine.get().frames.len(), 1);
    assert!(h.runs.delete(&run.id).await.unwrap_err().code == ErrorCode::Conflict);

    // Finish: the stop is confirmed, then the work is collected on its own.
    let run = h.runs.finish(&run.id).await.unwrap();
    assert!(matches!(run.phase, RunPhase::Stopping | RunPhase::Ended));
    let run = h
        .wait(&run.id, "the work is collected", |r| {
            r.collection == RunCollection::Ready
        })
        .await;
    assert_eq!(run.outcome, Some(RunOutcome::Finished));
    assert!(run.stop_confirmed);
    assert_eq!(run.changed_files, Some(5));
    assert_eq!(run.left_out.len(), 2);
    assert!(run.left_out.iter().any(|l| l.path == "notes.tmp"));
    // A left-out file waits for a decision: the container is kept.
    assert!(run.kept && !run.snapshot_accepted);
    assert!(h.engine.get().discarded.is_empty());

    // Choose files to add…: the left-out folder is named as the list shows
    // it, with its slash, and the volume is unchanged.
    h.open_secret(&run.id);
    let run = h
        .runs
        .collect(&run.id, vec!["secret/".into()])
        .await
        .unwrap();
    assert_eq!(run.collection, RunCollection::Ready);
    assert_eq!(run.changed_files, Some(6));
    assert_eq!(run.left_out.len(), 1, "{:?}", run.left_out);
    assert_eq!(run.left_out[0].path, "notes.tmp");
    assert!(run.kept && !run.snapshot_accepted);

    let changes = h.runs.changes(&run.id).await.unwrap();
    let mut paths: Vec<&str> = changes.files.iter().map(|f| f.path.as_str()).collect();
    paths.sort();
    assert_eq!(
        paths,
        [
            ".gitignore",
            "agent.txt",
            "important.log",
            "old.txt",
            "readme.txt",
            "secret/x.txt"
        ]
    );
    let diff = h
        .runs
        .diff(RunDiffRequest {
            run_id: run.id.clone(),
            path: "readme.txt".into(),
            old_path: None,
            options: Default::default(),
        })
        .await
        .unwrap();
    assert!(serde_json::to_string(&diff.content)
        .unwrap()
        .contains("edited by the agent"));

    // Copy patch confirms the left-out list, as Keep this snapshot would.
    let patch = h.runs.patch(&run.id, None).await.unwrap();
    assert!(patch.contains("+edited by the agent") && patch.contains("-old"));
    let run = h.wait(&run.id, "the container goes", |r| !r.kept).await;
    assert!(run.snapshot_accepted);
    assert_eq!(h.engine.get().discarded, vec![run.id.clone()]);
    let saved = h.tmp.path().join("run.patch");
    h.runs.patch(&run.id, Some(&saved)).await.unwrap();
    assert_eq!(std::fs::read_to_string(&saved).unwrap(), patch);

    // Delete removes the conversation, the result, and the row.
    h.runs.delete(&run.id).await.unwrap();
    assert!(h.runs.get(&run.id).await.is_err());
    assert!(!h.data().join("agent-runs").join(&run.id).exists());
    assert!(h.runs.list().await.unwrap().runs.is_empty());
    assert!(h
        .events
        .lock()
        .unwrap()
        .iter()
        .any(|e| e.run_id == run.id && e.deleted));
    // Every collection and the cleanup named the current image, for a run
    // whose own image is no longer on the engine.
    let current = h.settings.get().await.unwrap().profile.image.unwrap().name;
    let named = h.engine.get().fallback_images.clone();
    assert_eq!(named.len(), 3, "{named:?}");
    assert!(named.iter().all(|i| i.as_deref() == Some(current.as_str())));
}

#[tokio::test]
async fn the_settings_test_fails_when_a_refused_key_comes_back_as_a_reply() {
    let h = Harness::new().await;
    h.engine.get().refusal_as_reply = true;
    let before = h.settings.get().await.unwrap().profile.test_passed_at;
    let result = h.runs.test().await.unwrap();
    assert!(!result.passed, "{result:?}");
    let step = result
        .steps
        .iter()
        .find(|s| s.name == "The agent answers a prompt")
        .unwrap();
    assert!(!step.passed, "{result:?}");
    let detail = step.detail.as_deref().unwrap_or("").to_lowercase();
    assert!(
        detail.contains("did not accept") || detail.contains("refused"),
        "{result:?}"
    );
    // Nothing is recorded, and the test's run is discarded.
    assert_eq!(
        h.settings.get().await.unwrap().profile.test_passed_at,
        before
    );
    assert_eq!(h.engine.get().discarded.len(), 1);
}

#[tokio::test]
async fn a_run_whose_image_is_gone_fails_with_nothing_to_collect() {
    let h = Harness::new().await;
    h.engine.get().image_missing = true;
    let run = h.start("hello", RunPermissions::Act).await;
    let run = h
        .wait(&run.id, "the run fails", |r| {
            r.phase == RunPhase::Ended && r.stop_confirmed
        })
        .await;
    assert_eq!(run.outcome, Some(RunOutcome::Failed));
    assert!(!run.kept);
    assert_eq!(run.collection, RunCollection::None);
    // Once its end is mirrored, the controller forgets it; the engine was
    // never asked to remove anything.
    for _ in 0..500 {
        if h.runtime().runs().await.unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(h.runtime().runs().await.unwrap().is_empty());
    assert!(h.engine.get().discarded.is_empty());
    assert!(h.runs.collect(&run.id, Vec::new()).await.is_err());
    h.runs.delete(&run.id).await.unwrap();
    assert!(h.runs.get(&run.id).await.is_err());
}

#[tokio::test]
async fn a_kept_run_the_controller_forgot_can_still_be_deleted() {
    let h = Harness::new().await;
    let run = h.start("edit", RunPermissions::Act).await;
    h.wait(&run.id, "the turn ends", idle).await;
    h.runs.finish(&run.id).await.unwrap();
    let run = h
        .wait(&run.id, "the work is collected", |r| {
            r.collection == RunCollection::Ready
        })
        .await;
    assert!(run.kept);
    // The controller's record goes without the row hearing of it: a discard
    // whose answer was lost, or the controller's folder wiped.
    h.runtime().discard(&run.id, None).await.unwrap();
    assert!(h.runs.get(&run.id).await.unwrap().kept);
    h.open_secret(&run.id);
    h.runs.delete(&run.id).await.unwrap();
    assert!(h.runs.get(&run.id).await.is_err());
}

#[tokio::test]
async fn a_start_the_controller_answers_with_an_error_is_a_refusal() {
    let mut h = Harness::new().await;
    let start = |bundle: PathBuf| StartRun {
        run_id: "run-x".into(),
        attempt: 1,
        engine_socket: SOCKET.into(),
        image: "brainiac-claude:test".into(),
        cpus: 2,
        memory_mib: 2048,
        workspace_gib: 1,
        time_limit_secs: 600,
        permissions: RunPermissions::Act,
        start_commit: h.start.clone(),
        bundle,
        prompt_id: "p1".into(),
        prompt: "hello".into(),
        credential: Credential {
            key: CredentialKey::AnthropicApiKey,
            value: KEY.into(),
        },
        model: String::new(),
    };
    // A bundle that is not there: the controller answers, and made nothing.
    let missing = h.tmp.path().join("missing.bundle");
    match h.runtime().start(start(missing.clone())).await {
        Err(StartError::Refused(e)) => assert_eq!(e.code, ErrorCode::NotFound, "{e:?}"),
        other => panic!("{other:?}"),
    }
    assert!(h.runtime().runs().await.unwrap().is_empty());
    // No controller: the start may or may not have been seen.
    h.server.abort();
    let _ = (&mut h.server).await;
    match h.runtime().start(start(missing)).await {
        Err(StartError::Unanswered(_)) => {}
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn a_permission_waits_for_the_user_and_is_answered_once() {
    let h = Harness::new().await;
    let run = h.start("ask", RunPermissions::Ask).await;
    let run = h
        .wait(&run.id, "the permission is asked", |r| {
            r.activity == RunActivity::Permission
        })
        .await;
    assert_eq!(run.pending_permissions.len(), 1);
    let permission = &run.pending_permissions[0];
    assert_eq!(permission.title, "touch notes.txt");
    let page = h.runs.events(&run.id, 0).await.unwrap();
    assert!(page
        .events
        .iter()
        .any(|e| matches!(&e.body, RunEventBody::Permission(p) if p.permission_id == permission.permission_id)));

    h.runs
        .permit(&run.id, &permission.permission_id, true)
        .await
        .unwrap();
    let run = h.wait(&run.id, "the turn ends", idle).await;
    assert!(run.pending_permissions.is_empty());
    assert_eq!(h.reply(&run.id, 1).await, "answer: allow");
    // The same answer again is not a second answer.
    assert!(h
        .runs
        .permit(&run.id, &permission.permission_id, true)
        .await
        .is_ok());
    assert_eq!(h.reply(&run.id, 1).await, "answer: allow");
}

#[tokio::test]
async fn cancel_waits_for_the_controller_and_an_interrupted_run_is_collected_by_hand() {
    let mut h = Harness::new().await;
    let run = h.start("hello", RunPermissions::Act).await;
    let run = h.wait(&run.id, "the turn ends", idle).await;

    // The controller is gone: Cancel is remembered, not refused.
    h.server.abort();
    let _ = (&mut h.server).await;
    let run = h.runs.cancel(&run.id).await.unwrap();
    assert!(run.cancel_requested);
    assert!(!run.connected);

    // A new controller finds the run live in its records and interrupts it.
    h.restart().await;
    let run = h
        .wait(&run.id, "the run is interrupted", |r| {
            r.phase == RunPhase::Ended && r.stop_confirmed && !r.cancel_requested
        })
        .await;
    assert_eq!(run.outcome, Some(RunOutcome::Interrupted));
    assert!(run.kept);
    assert_eq!(run.collection, RunCollection::None);
    h.wait_events(&run.id, "the run ended as interrupted", |b| {
        matches!(
            b,
            RunEventBody::Ended {
                outcome: RunOutcome::Interrupted,
                ..
            }
        )
    })
    .await;

    // Collect work: nothing changed, nothing is left out, so the container goes.
    let run = h.runs.collect(&run.id, Vec::new()).await.unwrap();
    assert_eq!(run.collection, RunCollection::NoChanges);
    assert_eq!(run.result_commit.as_deref(), Some(h.start.as_str()));
    assert!(run.snapshot_accepted);
    let run = h.wait(&run.id, "the container goes", |r| !r.kept).await;
    assert!(run.cleanup_pending.is_none());
    let changes = h.runs.changes(&run.id).await.unwrap();
    assert!(changes.files.is_empty());
    assert_eq!(
        h.runs.patch(&run.id, None).await.unwrap_err().code,
        ErrorCode::Validation
    );
}

#[tokio::test]
async fn the_settings_test_runs_prompts_cancels_and_collects() {
    let h = Harness::new().await;
    h.settings.get().await.unwrap();
    let before = h.runs.list().await.unwrap().runs.len();
    let result = h.runs.test().await.unwrap();
    let names: Vec<&str> = result.steps.iter().map(|s| s.name.as_str()).collect();
    assert!(result.passed, "{result:?}");
    assert_eq!(
        names,
        [
            "Start a run",
            "The agent answers a prompt",
            "Cancel stops the container",
            "Collect the work"
        ]
    );
    assert!(result.passed, "{result:?}");
    assert_eq!(names.len(), 4, "{result:?}");
    assert_eq!(
        result.steps[1].detail.as_deref(),
        Some("you said: Reply with the single word ready and nothing else.")
    );
    // The test's run is not a run of the user's, and leaves nothing behind.
    assert_eq!(h.runs.list().await.unwrap().runs.len(), before);
    assert_eq!(h.engine.get().discarded.len(), 1);
    assert!(h.settings.get().await.unwrap().profile.test_current);
}

#[tokio::test]
async fn a_start_the_settings_do_not_allow_is_refused_before_anything_is_read() {
    let h = Harness::new().await;
    h.settings
        .remove_credential(h.settings.get().await.unwrap().profile.version)
        .await
        .unwrap();
    let err = h
        .runs
        .start(StartRunRequest {
            repository_id: "repo-1".into(),
            start_commit: h.start.clone(),
            prompt: "hello".into(),
            permissions: RunPermissions::Act,
            time_limit_minutes: 60,
            cpus: 2,
            memory_mib: 2048,
            workspace_gib: 20,
            model: String::new(),
            host_id: String::new(),
        })
        .await
        .unwrap_err();
    assert!(err.message.contains("Add an API key"), "{err:?}");
    assert!(h.runs.list().await.unwrap().runs.is_empty());
    assert_eq!(h.engine.get().launches, 0);
}
