//! A run host fires a routine's schedule with no Mac (docs/design/
//! routines-ux.md, Schedules): the controller keeps the schedule, the code and
//! the sealed model credential; starts the run at the time; follows it;
//! reads and checks the briefing; and keeps what it left until the Mac takes
//! it. A slot the host missed is recorded, not forgotten. The controller runs
//! in this process, on the fake engine.

mod fake_engine;

use std::time::Duration;

use brainiac_lib::agents::controller::protocol::{
    CredentialKey, PutSchedule, ScheduleDays, ScheduleSpec, ScheduleStatus, ScheduledState,
};
use brainiac_lib::agents::controller::state::StateDir;
use brainiac_lib::agents::controller::{serve, Config};
use brainiac_lib::agents::RunRuntime;
use brainiac_lib::models::{AgentKind, AgentProvider, BriefingStatus};
use chrono::Local;
use fake_engine::{fake_creds, git, FakeEngine, KEY};

const ROUTINE: &str = "11111111-2222-3333-4444-555555555555";
const COMMIT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const BRIEFING: &str = r#"{"status":"findings","summary":"One error.",
  "memory":"Covered SENTRY-1.",
  "findings":[{"key":"SENTRY-1","title":"Boom","severity":"high"}],
  "proposals":[{"kind":"task","title":"Fix it"}]}"#;

struct Harness {
    tmp: tempfile::TempDir,
    engine: FakeEngine,
    runtime: RunRuntime,
    _server: tokio::task::JoinHandle<()>,
}

impl Harness {
    async fn new() -> Harness {
        Harness::seeded(|_| {}).await
    }

    /// A controller started on a state folder `seed` wrote to first, as one
    /// restarted after a stop finds it.
    async fn seeded(seed: impl FnOnce(&std::path::Path)) -> Harness {
        let tmp = tempfile::tempdir().unwrap();
        seed(&tmp.path().join("controller"));
        let state = StateDir::new(tmp.path().join("controller"));
        let socket = tmp.path().join("controller").join("runner.sock");
        let config = Config {
            state: state.clone(),
            socket: socket.clone(),
            idle_exit: None,
            spawn_guard: false,
            creds_program: Some(fake_creds(tmp.path())),
            service: true,
        };
        let engine = FakeEngine::default();
        engine.get().volumes = tmp.path().join("volumes");
        let serving = engine.clone();
        let server = tokio::spawn(async move {
            serve(config, serving).await.unwrap();
        });
        let runtime = RunRuntime::attach(state, socket);
        for _ in 0..100 {
            if runtime.running().await.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        Harness {
            tmp,
            engine,
            runtime,
            _server: server,
        }
    }

    /// A schedule due `ago_secs` seconds ago (rounded down to the minute), counted from `since_secs` ago.
    fn spec(&self, ago_secs: i64, late_start: bool) -> ScheduleSpec {
        spec_due(ago_secs, late_start)
    }

    async fn send_code(&self) {
        // A real bundle with the branch `start`, as a run's start is.
        let repo = self.tmp.path().join("code");
        let file = self.tmp.path().join("code.bundle");
        if !repo.exists() {
            std::fs::create_dir_all(&repo).unwrap();
            git(&repo, &["init", "-q", "-b", "start"]);
            std::fs::write(repo.join("README.md"), "# Service\n").unwrap();
            git(&repo, &["add", "."]);
            git(&repo, &["commit", "-q", "-m", "First"]);
        }
        git(
            &repo,
            &["bundle", "create", "-q", file.to_str().unwrap(), "start"],
        );
        self.runtime
            .send_schedule_bundle(ROUTINE, &file)
            .await
            .unwrap();
    }

    async fn put(&self, spec: ScheduleSpec, since_secs: i64) {
        self.runtime
            .put_schedule(PutSchedule {
                spec,
                memory: Some("Ignore SENTRY-9.".into()),
                bundle_commit: Some(COMMIT.into()),
                since: Some((Local::now() - chrono::Duration::seconds(since_secs)).to_rfc3339()),
            })
            .await
            .unwrap();
    }

    async fn status(&self) -> ScheduleStatus {
        let (list, _) = self.runtime.schedules().await.unwrap();
        list.into_iter().next().expect("a schedule")
    }

    /// Wait for the schedule to have a result.
    async fn first_result(&self) -> ScheduleStatus {
        for _ in 0..600 {
            let status = self.status().await;
            if !status.results.is_empty() {
                return status;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("the schedule never left a result");
    }
}

/// A schedule due `ago_secs` seconds ago, to the minute.
fn spec_due(ago_secs: i64, late_start: bool) -> ScheduleSpec {
    let slot = Local::now() - chrono::Duration::seconds(ago_secs);
    ScheduleSpec {
        routine_id: ROUTINE.into(),
        name: "Morning check".into(),
        days: ScheduleDays::Daily,
        time: slot.format("%H:%M").to_string(),
        late_start,
        enabled: true,
        profile_id: "claude-code".into(),
        credential_key: CredentialKey::AnthropicApiKey,
        agent: AgentKind::ClaudeCode,
        provider: AgentProvider::Anthropic,
        model: String::new(),
        image: "brainiac-agents:test".into(),
        cpus: 2,
        memory_mib: 2048,
        workspace_gib: 1,
        time_limit_secs: 600,
        host_secrets: Vec::new(),
        prompt: "Check production.".into(),
        previous_results: 2,
    }
}

#[tokio::test]
async fn the_host_fires_a_due_slot_reads_the_briefing_and_keeps_it_until_acknowledged() {
    let h = Harness::new().await;
    h.runtime
        .put_model_credential("claude-code", CredentialKey::AnthropicApiKey, KEY.into())
        .await
        .unwrap();
    h.send_code().await;
    h.engine.get().briefings.push_back(BRIEFING.into());
    h.put(h.spec(70, false), 600).await;

    let status = h.first_result().await;
    let result = &status.results[0];
    assert_eq!(result.state, ScheduledState::Ready, "{result:?}");
    let draft = result.draft.as_ref().unwrap();
    assert_eq!(draft.status, Some(BriefingStatus::Findings));
    assert_eq!(draft.findings[0].key, "SENTRY-1");
    // The host's memory is the agent's new one; the earlier prompt carried the old.
    assert_eq!(status.memory, "Covered SENTRY-1.");
    let prompts = h.engine.get().prompts.clone();
    assert!(prompts[0].starts_with("Check production."));
    assert!(prompts[0].contains("Ignore SENTRY-9."));
    assert!(prompts[0].contains(".brainiac/briefing.json"));
    // The key reached the run, and the model credential is not a secret a run names.
    assert!(status.credential);
    assert!(h.runtime.list_secrets().await.unwrap().is_empty());
    assert!(status.next_fire.is_some());

    // Taken, it is not sent again as new.
    h.runtime
        .ack_schedule_results(ROUTINE, vec![result.id.clone()])
        .await
        .unwrap();
    let after = h.status().await;
    assert!(after.results.iter().all(|r| r.acked));

    // It fired once for the slot, not again on the next look: nothing new
    // waits (what was taken is not sent again).
    tokio::time::sleep(Duration::from_secs(6)).await;
    assert!(h.status().await.results.is_empty());
}

#[tokio::test]
async fn a_slot_the_host_missed_is_recorded_and_a_late_start_runs_once() {
    let h = Harness::new().await;
    h.runtime
        .put_model_credential("claude-code", CredentialKey::AnthropicApiKey, KEY.into())
        .await
        .unwrap();
    h.send_code().await;

    // Due three hours ago, counted from six hours ago: missed, said so.
    h.put(h.spec(3 * 3600, false), 6 * 3600).await;
    let status = h.first_result().await;
    assert_eq!(status.results[0].state, ScheduledState::Missed);
    assert!(status.results[0].draft.is_none());
    assert!(status.results[0]
        .error
        .as_deref()
        .unwrap()
        .contains("does not start late"));
    assert!(status.live_run.is_none());
    h.runtime.delete_schedule(ROUTINE).await.unwrap();
    assert!(h.runtime.schedules().await.unwrap().0.is_empty());

    // The same with a late start: one run, now. Removing the schedule took
    // the code and the agent's key with it.
    h.runtime
        .put_model_credential("claude-code", CredentialKey::AnthropicApiKey, KEY.into())
        .await
        .unwrap();
    h.send_code().await;
    h.engine.get().briefings.push_back(BRIEFING.into());
    h.put(h.spec(3 * 3600, true), 6 * 3600).await;
    let status = h.first_result().await;
    assert_eq!(
        status.results[0].state,
        ScheduledState::Ready,
        "{:?}",
        status.results
    );
}

#[tokio::test]
async fn a_firing_that_cannot_start_says_why_and_a_bad_schedule_is_refused() {
    let h = Harness::new().await;
    // No model credential on the host: the firing fails with the reason.
    h.send_code().await;
    h.put(h.spec(70, false), 600).await;
    let status = h.first_result().await;
    assert_eq!(status.results[0].state, ScheduledState::Failed);
    assert!(status.results[0]
        .error
        .as_deref()
        .unwrap()
        .contains("agent's key"));
    assert!(!status.credential);

    // Not a time, and a secret name that is a model credential's.
    let mut bad = h.spec(70, false);
    bad.time = "7am".into();
    let put = |spec| PutSchedule {
        spec,
        memory: None,
        bundle_commit: None,
        since: None,
    };
    assert!(h.runtime.put_schedule(put(bad)).await.is_err());
    let mut named = h.spec(70, false);
    named.host_secrets = vec!["MODELKEY_CLAUDE_CODE".into()];
    assert!(h.runtime.put_schedule(put(named)).await.is_err());
}

#[tokio::test]
async fn resuming_does_not_owe_the_time_it_was_off_and_the_agents_key_cannot_be_reached_as_a_secret(
) {
    let h = Harness::new().await;
    h.runtime
        .put_model_credential("claude-code", CredentialKey::AnthropicApiKey, KEY.into())
        .await
        .unwrap();
    h.send_code().await;

    // Off, with a slot three hours ago inside the time it was off.
    let mut off = h.spec(3 * 3600, false);
    off.enabled = false;
    h.put(off, 6 * 3600).await;
    // Turned on: the hours it was off are not owed, so nothing is "missed".
    h.put(h.spec(3 * 3600, false), 6 * 3600).await;
    tokio::time::sleep(Duration::from_secs(7)).await;
    let status = h.status().await;
    assert!(status.results.is_empty(), "{:?}", status.results);
    assert!(status.next_fire.is_some());

    // The sealed key is not a secret a client can set or remove by name.
    let put = h
        .runtime
        .put_secret("MODELKEY_CLAUDE_CODE", "12345678abcd".into(), false)
        .await;
    assert!(put.is_err());
    assert!(h
        .runtime
        .delete_secret("MODELKEY_CLAUDE_CODE")
        .await
        .is_err());

    // It goes with the last schedule that used it.
    assert!(h.status().await.credential);
    h.runtime.delete_schedule(ROUTINE).await.unwrap();
    h.send_code().await;
    h.put(h.spec(70, false), 600).await;
    assert!(!h.status().await.credential);
}

/// A schedule's record as a controller stopped halfway through starting a
/// run for the slot `slot_ago_secs` ago leaves it: the cursor past the slot,
/// and the run it was starting named, with no run of that name.
fn left_mid_start(dir: &std::path::Path, slot_ago_secs: i64, late_start: bool) {
    let slot = Local::now() - chrono::Duration::seconds(slot_ago_secs);
    let record = serde_json::json!({
        "spec": spec_due(slot_ago_secs, late_start),
        "memory": "",
        "memory_updated_at": null,
        "bundle_commit": null,
        "bundle_at": null,
        "cursor": Local::now().to_rfc3339(),
        "results": [],
        "followed": [],
        "firing": {
            "slot": slot.to_rfc3339(),
            "run_id": "99999999-8888-7777-6666-555555555555"
        }
    });
    let schedules = dir.join("schedules");
    std::fs::create_dir_all(&schedules).unwrap();
    std::fs::write(
        schedules.join(format!("{ROUTINE}.json")),
        serde_json::to_vec(&record).unwrap(),
    )
    .unwrap();
}

/// What the controller saved about the start it was making.
fn firing_on_disk(h: &Harness) -> serde_json::Value {
    let path = h
        .tmp
        .path()
        .join("controller")
        .join("schedules")
        .join(format!("{ROUTINE}.json"));
    let record: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    record["firing"].clone()
}

#[tokio::test]
async fn a_start_a_stop_cut_short_is_tried_again_while_it_is_on_time() {
    // Stopped 70 seconds after the slot, before the run's record was written.
    let h = Harness::seeded(|dir| left_mid_start(dir, 70, false)).await;
    let status = h.first_result().await;
    // Tried again for that slot: it fails here only because this host has no
    // code yet, which is the start's own reason, not a lost slot.
    let result = &status.results[0];
    assert_eq!(result.state, ScheduledState::Failed, "{result:?}");
    assert!(
        result
            .error
            .as_deref()
            .unwrap()
            .contains("no copy of the code"),
        "{result:?}"
    );
    assert_eq!(status.results.len(), 1);
    assert!(firing_on_disk(&h).is_null());
}

#[tokio::test]
async fn a_start_a_stop_cut_short_too_long_ago_says_it_did_not_start() {
    // Three hours later, and the schedule does not start late.
    let h = Harness::seeded(|dir| left_mid_start(dir, 3 * 3600, false)).await;
    let status = h.first_result().await;
    let result = &status.results[0];
    assert_eq!(result.state, ScheduledState::Failed, "{result:?}");
    assert!(
        result
            .error
            .as_deref()
            .unwrap()
            .contains("stopped while it was starting"),
        "{result:?}"
    );
    assert!(status.live_run.is_none());
    assert!(firing_on_disk(&h).is_null());
}
