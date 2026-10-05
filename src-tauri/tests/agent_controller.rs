//! The run controller (docs/architecture.md, Agent runs — v0.5): a run's
//! start, prompts, permissions, deadline, stop, and restart, through the
//! controller's socket as the app reaches it. The engine is a stand-in whose
//! "container" is a fake ACP agent in this process, so no Docker is needed;
//! `docker.rs` is the part this does not cover.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use brainiac_lib::agents::controller::docker::{
    Attached, CollectSpec, LaunchSpec, Output, Running, Workloads,
};
use brainiac_lib::agents::controller::protocol::{
    Activity, CollectManifest, Credential, CredentialKey, Delivery, Event, EventBody, Outcome,
    Phase, RunStatus, StartRun, StopReason,
};
use brainiac_lib::agents::controller::state::StateDir;
use brainiac_lib::agents::controller::{guard, serve, Config};
use brainiac_lib::agents::RunRuntime;
use brainiac_lib::models::{AppResult, ErrorCode, RunPermissions};
use serde_json::{json, Value};
use tokio::sync::mpsc;

const KEY: &str = "sk-ant-api03-FakeKeyForTests0123456789_-AbCdEfGh";

/// One fake container: its agent task, and what it was given.
struct Container {
    running: bool,
    output: mpsc::Sender<Output>,
    agent: tokio::task::JoinHandle<()>,
}

#[derive(Default)]
struct Engine {
    containers: HashMap<String, Container>,
    /// Where each run's "volume" is: a clone of its bundle on disk, which
    /// the fake agent edits and the real collector script reads.
    volumes: PathBuf,
    /// Each credential frame the agents read, as JSON.
    frames: Vec<Value>,
    /// Each prompt the agents were sent.
    prompts: Vec<String>,
    launches: usize,
    discarded: Vec<String>,
    /// The engine stops answering stops.
    unreachable: bool,
    /// How long making a container takes.
    launch_delay: Duration,
}

#[derive(Clone, Default)]
struct FakeEngine(Arc<Mutex<Engine>>);

impl FakeEngine {
    fn get(&self) -> std::sync::MutexGuard<'_, Engine> {
        self.0.lock().unwrap()
    }
}

impl Workloads for FakeEngine {
    async fn launch(&self, spec: LaunchSpec) -> AppResult<Attached> {
        assert!(spec.bundle.is_file());
        let delay = self.get().launch_delay;
        let mut cancel = spec.cancel.clone();
        tokio::select! {
            _ = tokio::time::sleep(delay) => {}
            _ = cancel.wait_for(|stop| *stop) => {
                return Err(brainiac_lib::models::AppError::new(ErrorCode::Cancelled, "stopped"));
            }
        }
        // The volume: the bundle cloned, as the entrypoint does it.
        let work = self.get().volumes.join(&spec.run_id);
        let _ = std::fs::remove_dir_all(&work);
        let cloned = std::process::Command::new("git")
            .args(["clone", "--quiet", "--branch", "start"])
            .arg(&spec.bundle)
            .arg(&work)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .status()
            .unwrap();
        assert!(cloned.success(), "the fake engine clones the bundle");
        let (stdin, stdin_rx) = mpsc::unbounded_channel();
        let (output_tx, output) = mpsc::channel(64);
        let agent = tokio::spawn(fake_agent(self.clone(), work, stdin_rx, output_tx.clone()));
        let mut engine = self.get();
        engine.launches += 1;
        engine.containers.insert(
            spec.run_id.clone(),
            Container {
                running: true,
                output: output_tx,
                agent,
            },
        );
        Ok(Attached {
            container_id: format!("container-{}", spec.run_id),
            output,
            stdin,
        })
    }

    async fn stop(&self, _: &str, _: &str, run_id: &str) -> AppResult<()> {
        let output = {
            let mut engine = self.get();
            if engine.unreachable {
                return Err(brainiac_lib::models::AppError::dependency(
                    "The engine is not answering.",
                ));
            }
            match engine.containers.get_mut(run_id) {
                Some(container) if container.running => {
                    container.running = false;
                    container.agent.abort();
                    Some(container.output.clone())
                }
                _ => None,
            }
        };
        if let Some(output) = output {
            let _ = output.send(Output::Closed).await;
        }
        Ok(())
    }

    async fn running(&self, _: &str, _: &str, run_id: &str) -> AppResult<Running> {
        let engine = self.get();
        if engine.unreachable {
            return Err(brainiac_lib::models::AppError::dependency(
                "The engine is not answering.",
            ));
        }
        Ok(match engine.containers.get(run_id) {
            Some(c) if c.running => Running::Yes,
            _ => Running::No,
        })
    }

    /// The real collector script, run on this Mac against the run's folder
    /// instead of in a container.
    async fn collect(&self, spec: CollectSpec) -> AppResult<CollectManifest> {
        let work = self.get().volumes.join(&spec.run_id);
        let scratch = self.get().volumes.join(format!("{}-collect", spec.run_id));
        let _ = std::fs::remove_dir_all(&scratch);
        let input = scratch.join("input");
        let out = scratch.join("out");
        std::fs::create_dir_all(&input).unwrap();
        std::fs::copy(&spec.bundle, input.join("input.bundle")).unwrap();
        std::fs::write(
            input.join("collect.json"),
            json!({ "start": spec.start_commit, "include": spec.include }).to_string(),
        )
        .unwrap();
        let script =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/agents/image/collector.mjs");
        let status = tokio::process::Command::new("node")
            .arg(script)
            .env("BRAINIAC_WORK", &work)
            .env("BRAINIAC_INPUT", &input)
            .env("BRAINIAC_OUT", &out)
            .env("BRAINIAC_SCRATCH", scratch.join("scratch"))
            .status()
            .await
            .unwrap();
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(out.join("manifest.json")).unwrap()).unwrap();
        if let Some(error) = manifest["error"].as_str() {
            return Err(brainiac_lib::models::AppError::io(error));
        }
        assert!(status.success());
        let manifest: CollectManifest = serde_json::from_value(manifest).unwrap();
        if manifest.result != manifest.start {
            std::fs::rename(
                out.join("result.bundle"),
                spec.out_dir.join("result.bundle"),
            )
            .unwrap();
        }
        Ok(manifest)
    }

    async fn discard(&self, _: &str, _: &str, run_id: &str, _: &str) -> AppResult<()> {
        let mut engine = self.get();
        engine.containers.remove(run_id);
        engine.discarded.push(run_id.to_string());
        Ok(())
    }
}

/// A stand-in for the entrypoint and the Claude ACP adapter. Prompts steer it:
/// "ask" asks a permission, "hang" works until cancelled, "die" exits,
/// "fail-auth" is refused, "echo-secret" repeats the key in two pieces,
/// "edit" changes files in the workspace.
async fn fake_agent(
    engine: FakeEngine,
    work: PathBuf,
    mut stdin: mpsc::UnboundedReceiver<Vec<u8>>,
    out: mpsc::Sender<Output>,
) {
    let mut buffer = Vec::new();
    // The frame: BRB1, a length, the JSON.
    while buffer.len() < 8
        || buffer.len() < 8 + u32::from_be_bytes(buffer[4..8].try_into().unwrap()) as usize
    {
        let Some(chunk) = stdin.recv().await else {
            return;
        };
        buffer.extend_from_slice(&chunk);
    }
    assert_eq!(&buffer[..4], b"BRB1");
    let len = u32::from_be_bytes(buffer[4..8].try_into().unwrap()) as usize;
    let frame: Value = serde_json::from_slice(&buffer[8..8 + len]).unwrap();
    let key = frame
        .get("ANTHROPIC_API_KEY")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    engine.get().frames.push(frame);
    buffer.drain(..8 + len);
    let say = |value: Value| {
        let out = out.clone();
        async move {
            let _ = out
                .send(Output::Line(serde_json::to_vec(&value).unwrap().into()))
                .await;
        }
    };
    let update = |text: &str| {
        json!({ "jsonrpc": "2.0", "method": "session/update", "params": { "sessionId": "sess-1",
            "update": { "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": text } } } })
    };
    let _ = out
        .send(Output::Line(b"bootstrap:ready".to_vec().into()))
        .await;
    let mut prompt_id: Option<Value> = None;
    let mut asked: Option<(Value, Value)> = None;
    loop {
        while let Some(pos) = buffer.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = buffer.drain(..=pos).collect();
            let message: Value = serde_json::from_slice(&line).unwrap();
            let id = message.get("id").cloned();
            match message.get("method").and_then(Value::as_str) {
                Some("initialize") => {
                    assert_eq!(message["params"]["clientCapabilities"]["terminal"], false);
                    say(json!({ "jsonrpc": "2.0", "id": id, "result": {
                        "protocolVersion": 1, "agentInfo": { "name": "fake-agent", "version": "1.0" } } }))
                    .await;
                }
                Some("session/new") => {
                    assert_eq!(message["params"]["mcpServers"], json!([]));
                    say(json!({ "jsonrpc": "2.0", "id": id, "result": { "sessionId": "sess-1" } }))
                        .await;
                }
                Some("session/prompt") => {
                    let text = message["params"]["prompt"][0]["text"]
                        .as_str()
                        .unwrap()
                        .to_string();
                    engine.get().prompts.push(text.clone());
                    if text.contains("ask") {
                        // "ask-secret" asks to run a command that holds the key.
                        let command = if text.contains("ask-secret") {
                            format!("curl -H 'x-api-key: {key}' example.com")
                        } else {
                            "touch notes.txt".to_string()
                        };
                        say(json!({ "jsonrpc": "2.0", "method": "session/update", "params": { "sessionId": "sess-1",
                            "update": { "sessionUpdate": "tool_call", "toolCallId": "t1", "title": command,
                                "kind": "execute", "status": "pending" } } }))
                        .await;
                        let ask = json!({ "jsonrpc": "2.0", "id": "perm-1", "method": "session/request_permission",
                            "params": { "sessionId": "sess-1",
                                "toolCall": { "toolCallId": "t1", "title": command, "kind": "execute",
                                    "rawInput": { "command": command } },
                                "options": [
                                    { "optionId": "allow", "name": "Allow", "kind": "allow_once" },
                                    { "optionId": "always", "name": "Always", "kind": "allow_always" },
                                    { "optionId": "reject", "name": "Reject", "kind": "reject_once" } ] } });
                        say(ask).await;
                        asked = Some((id.clone().unwrap(), json!("perm-1")));
                    } else if text.contains("hang") {
                        prompt_id = id;
                    } else if text.contains("die") {
                        let _ = out.send(Output::Closed).await;
                        return;
                    } else if text.contains("fail-auth") {
                        say(json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32000, "message": "Authentication required" } }))
                            .await;
                    } else if text.contains("edit") {
                        // Edits the agent never committed, a new file, a
                        // deletion, and a file its ignore rules would hide.
                        std::fs::write(work.join("readme.txt"), "edited by the agent\n").unwrap();
                        std::fs::write(work.join("agent.txt"), "new\n").unwrap();
                        std::fs::remove_file(work.join("old.txt")).unwrap();
                        std::fs::write(work.join("notes.tmp"), "scratch\n").unwrap();
                        std::fs::write(work.join(".gitignore"), "*\n").unwrap();
                        say(update("edited")).await;
                        say(json!({ "jsonrpc": "2.0", "id": id, "result": { "stopReason": "end_turn" } })).await;
                    } else if text.contains("echo-secret") {
                        let (a, b) = key.split_at(10);
                        say(update(&format!("the key is {a}"))).await;
                        say(update(&format!("{b}, there"))).await;
                        say(json!({ "jsonrpc": "2.0", "id": id, "result": { "stopReason": "end_turn" } })).await;
                    } else {
                        say(update("you said: ")).await;
                        say(update(&text)).await;
                        say(json!({ "jsonrpc": "2.0", "id": id, "result": { "stopReason": "end_turn" } })).await;
                    }
                }
                Some("session/cancel") => {
                    if let Some(id) = prompt_id.take() {
                        say(json!({ "jsonrpc": "2.0", "id": id, "result": { "stopReason": "cancelled" } })).await;
                    }
                }
                Some(_) => {}
                // An answer to the permission.
                None if id == Some(json!("perm-1")) => {
                    let (prompt, _) = asked.take().unwrap();
                    let outcome = &message["result"]["outcome"];
                    let word = match outcome["outcome"].as_str() {
                        Some("selected") => outcome["optionId"].as_str().unwrap().to_string(),
                        other => other.unwrap_or("none").to_string(),
                    };
                    say(update(&format!("answer: {word}"))).await;
                    say(json!({ "jsonrpc": "2.0", "id": prompt, "result": {
                        "stopReason": if word == "cancelled" { "cancelled" } else { "end_turn" } } }))
                    .await;
                }
                None => {}
            }
        }
        let Some(chunk) = stdin.recv().await else {
            return;
        };
        buffer.extend_from_slice(&chunk);
    }
}

struct Harness {
    tmp: tempfile::TempDir,
    engine: FakeEngine,
    runtime: RunRuntime,
    server: tokio::task::JoinHandle<()>,
    bundle: PathBuf,
    /// The repository the bundle came from, and its start commit.
    repo: PathBuf,
    start: String,
}

/// Git in a test's own folder, with nothing of the user's configuration.
fn git(dir: &std::path::Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .args([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("HOME", dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

impl Harness {
    async fn new() -> Harness {
        let tmp = tempfile::tempdir().unwrap();
        // A real repository and bundle: the fake engine clones it, and the
        // collector reads it back.
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q"]);
        std::fs::write(repo.join("readme.txt"), "start\n").unwrap();
        std::fs::write(repo.join("old.txt"), "old\n").unwrap();
        std::fs::write(repo.join(".gitignore"), "*.tmp\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "Start"]);
        git(&repo, &["branch", "start"]);
        let start = git(&repo, &["rev-parse", "start"]);
        let bundle = tmp.path().join("input.bundle");
        git(
            &repo,
            &[
                "bundle",
                "create",
                "-q",
                bundle.to_str().unwrap(),
                "refs/heads/start",
            ],
        );
        let engine = FakeEngine::default();
        engine.get().volumes = tmp.path().join("volumes");
        let mut harness = Harness {
            server: tokio::spawn(async {}),
            runtime: RunRuntime::attach(StateDir::new(tmp.path()), PathBuf::new()),
            tmp,
            engine,
            bundle,
            repo,
            start,
        };
        harness.restart().await;
        harness
    }

    fn state(&self) -> StateDir {
        StateDir::new(self.tmp.path().join("controller"))
    }

    fn socket(&self) -> PathBuf {
        self.tmp.path().join("controller").join("runner.sock")
    }

    /// Start the controller, or start it again as if the old one were killed.
    async fn restart(&mut self) {
        self.server.abort();
        let _ = (&mut self.server).await;
        let config = Config {
            state: self.state(),
            socket: self.socket(),
            idle_exit: None,
            spawn_guard: false,
        };
        let engine = self.engine.clone();
        self.server = tokio::spawn(async move {
            serve(config, engine).await.unwrap();
        });
        self.runtime = RunRuntime::attach(self.state(), self.socket());
        for _ in 0..100 {
            if self.runtime.running().await.is_some() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("the controller did not start");
    }

    fn start_request(&self, run_id: &str, prompt: &str, permissions: RunPermissions) -> StartRun {
        StartRun {
            run_id: run_id.into(),
            attempt: 1,
            engine_socket: "/fake/docker.sock".into(),
            image: "brainiac-claude:test".into(),
            cpus: 2,
            memory_mib: 2048,
            time_limit_secs: 3600,
            permissions,
            start_commit: self.start.clone(),
            bundle: self.bundle.clone(),
            prompt_id: format!("{run_id}-prompt"),
            prompt: prompt.into(),
            credential: Credential {
                key: CredentialKey::AnthropicApiKey,
                value: KEY.into(),
            },
        }
    }

    async fn start(&self, run_id: &str, prompt: &str, permissions: RunPermissions) -> RunStatus {
        self.runtime
            .start(self.start_request(run_id, prompt, permissions))
            .await
            .unwrap()
    }

    async fn status(&self, run_id: &str) -> RunStatus {
        self.runtime
            .runs()
            .await
            .unwrap()
            .into_iter()
            .find(|r| r.run_id == run_id)
            .expect("the run is listed")
    }

    async fn wait(&self, run_id: &str, what: &str, done: impl Fn(&RunStatus) -> bool) -> RunStatus {
        for _ in 0..300 {
            let status = self.status(run_id).await;
            if done(&status) {
                return status;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("{what}: {:?}", self.status(run_id).await);
    }

    async fn events(&self, run_id: &str) -> Vec<Event> {
        let mut events = Vec::new();
        loop {
            let after = events.last().map(|e: &Event| e.seq).unwrap_or(0);
            let page = self
                .runtime
                .events(run_id, after, Duration::ZERO)
                .await
                .unwrap();
            if page.events.is_empty() {
                return events;
            }
            events.extend(page.events);
        }
    }

    async fn reply(&self, run_id: &str, turn: u32) -> String {
        self.events(run_id)
            .await
            .into_iter()
            .filter_map(|e| match e.body {
                EventBody::Message { turn: t, text } if t == turn => Some(text),
                _ => None,
            })
            .collect()
    }
}

fn idle(status: &RunStatus) -> bool {
    status.phase == Phase::Running && status.activity == Activity::Idle
}

fn ended(status: &RunStatus) -> bool {
    status.phase == Phase::Ended
}

#[tokio::test]
async fn a_run_takes_its_prompt_and_a_follow_up_once() {
    let h = Harness::new().await;
    let accepted = h.start("run-1", "hello", RunPermissions::Act).await;
    assert_eq!(accepted.phase, Phase::Preparing);
    let status = h.wait("run-1", "the first turn ends", idle).await;
    assert_eq!(status.turn, 1);
    assert_eq!(status.session_id.as_deref(), Some("sess-1"));
    assert_eq!(h.reply("run-1", 1).await, "you said: hello");

    assert_eq!(
        h.runtime.prompt("run-1", "c2", "again").await.unwrap(),
        Delivery::Delivered
    );
    h.wait("run-1", "the second turn ends", |s| idle(s) && s.turn == 2)
        .await;
    // The same command again gets the first answer and is not sent again.
    assert_eq!(
        h.runtime.prompt("run-1", "c2", "again").await.unwrap(),
        Delivery::Delivered
    );
    assert_eq!(h.engine.get().prompts, vec!["hello", "again"]);
    assert_eq!(h.reply("run-1", 2).await, "you said: again");

    // A repeated start of the same run and attempt starts nothing.
    let again = h.start("run-1", "hello", RunPermissions::Act).await;
    assert_eq!(again.accepted_at, accepted.accepted_at);
    assert_eq!(h.engine.get().launches, 1);
}

#[tokio::test]
async fn the_credential_reaches_the_container_once_and_never_the_journal() {
    let h = Harness::new().await;
    h.start("run-1", "echo-secret", RunPermissions::Act).await;
    h.wait("run-1", "the turn ends", idle).await;
    let frames = h.engine.get().frames.clone();
    assert_eq!(frames, vec![json!({ "ANTHROPIC_API_KEY": KEY })]);
    assert_eq!(
        h.reply("run-1", 1).await,
        "the key is [ANTHROPIC_API_KEY], there"
    );
    for entry in walk(h.tmp.path()) {
        let bytes = std::fs::read(&entry).unwrap();
        assert!(
            !String::from_utf8_lossy(&bytes).contains(KEY),
            "{} holds the key",
            entry.display()
        );
    }
}

fn walk(dir: &std::path::Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            files.extend(walk(&path));
        } else if path.is_file() {
            files.push(path);
        }
    }
    files
}

#[tokio::test]
async fn ask_holds_a_permission_until_the_user_answers_once() {
    let h = Harness::new().await;
    h.start("run-1", "ask first", RunPermissions::Ask).await;
    let waiting = h
        .wait("run-1", "the permission is pending", |s| {
            s.activity == Activity::Permission
        })
        .await;
    assert_eq!(waiting.permissions.len(), 1);
    let permission = &waiting.permissions[0];
    assert_eq!(permission.title, "touch notes.txt");
    assert_eq!(permission.detail.as_deref(), Some("touch notes.txt"));

    // Nothing answers it while no one asks: it is still pending later.
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(h.status("run-1").await.activity, Activity::Permission);

    let id = permission.permission_id.clone();
    assert_eq!(
        h.runtime.permit("run-1", "a1", &id, true).await.unwrap(),
        Delivery::Delivered
    );
    assert_eq!(
        h.runtime.permit("run-1", "a1", &id, true).await.unwrap(),
        Delivery::Delivered
    );
    h.wait("run-1", "the turn ends", idle).await;
    assert_eq!(h.reply("run-1", 1).await, "answer: allow");
    let answered: Vec<_> = h
        .events("run-1")
        .await
        .into_iter()
        .filter(|e| matches!(e.body, EventBody::PermissionAnswered { .. }))
        .collect();
    assert_eq!(answered.len(), 1);
}

#[tokio::test]
async fn act_allows_once_without_asking() {
    let h = Harness::new().await;
    h.start("run-1", "ask first", RunPermissions::Act).await;
    h.wait("run-1", "the turn ends", idle).await;
    assert_eq!(h.reply("run-1", 1).await, "answer: allow");
}

#[tokio::test]
async fn cancel_answers_the_pending_permission_and_keeps_the_work() {
    let h = Harness::new().await;
    h.start("run-1", "ask first", RunPermissions::Ask).await;
    let waiting = h
        .wait("run-1", "the permission is pending", |s| {
            s.activity == Activity::Permission
        })
        .await;
    let id = waiting.permissions[0].permission_id.clone();
    h.runtime.stop("run-1", StopReason::Cancel).await.unwrap();
    let status = h.wait("run-1", "the run ends", ended).await;
    assert_eq!(status.outcome, Some(Outcome::Cancelled));
    assert!(status.stop_confirmed && status.kept);
    assert!(h.events("run-1").await.iter().any(|e| matches!(
        &e.body,
        EventBody::PermissionAnswered { outcome, by, .. } if outcome == "cancelled" && by == "run"
    )));
    // A late answer has no effect.
    let late = h
        .runtime
        .permit("run-1", "a1", &id, true)
        .await
        .unwrap_err();
    assert_eq!(late.code, ErrorCode::Conflict);
    // Stopping again changes nothing.
    assert_eq!(
        h.runtime
            .stop("run-1", StopReason::Cancel)
            .await
            .unwrap()
            .outcome,
        Some(Outcome::Cancelled)
    );

    h.runtime.discard("run-1").await.unwrap();
    assert_eq!(h.engine.get().discarded, vec!["run-1"]);
    assert!(h.runtime.runs().await.unwrap().is_empty());
}

#[tokio::test]
async fn finish_waits_for_the_agent_to_be_idle() {
    let h = Harness::new().await;
    h.start("run-1", "hang", RunPermissions::Act).await;
    h.wait("run-1", "the turn starts", |s| {
        s.activity == Activity::Working
    })
    .await;
    let refused = h
        .runtime
        .stop("run-1", StopReason::Finish)
        .await
        .unwrap_err();
    assert_eq!(refused.code, ErrorCode::Conflict);
    let busy = h.runtime.prompt("run-1", "c2", "more").await.unwrap_err();
    assert_eq!(busy.code, ErrorCode::Conflict);
    // A refused command may be sent again later.
    h.runtime.stop("run-1", StopReason::Cancel).await.unwrap();
    let status = h.wait("run-1", "the run ends", ended).await;
    assert_eq!(status.outcome, Some(Outcome::Cancelled));
}

#[tokio::test]
async fn finish_while_idle_ends_the_run_finished() {
    let h = Harness::new().await;
    h.start("run-1", "hello", RunPermissions::Act).await;
    h.wait("run-1", "the turn ends", idle).await;
    h.runtime.stop("run-1", StopReason::Finish).await.unwrap();
    let status = h.wait("run-1", "the run ends", ended).await;
    assert_eq!(status.outcome, Some(Outcome::Finished));
    assert!(status.error.is_none());
}

#[tokio::test]
async fn the_deadline_ends_a_run_no_client_is_watching() {
    let h = Harness::new().await;
    let mut start = h.start_request("run-1", "hang", RunPermissions::Act);
    start.time_limit_secs = 1;
    h.runtime.start(start).await.unwrap();
    tokio::time::sleep(Duration::from_millis(2500)).await;
    let status = h.wait("run-1", "the run expires", ended).await;
    assert_eq!(status.outcome, Some(Outcome::Expired));
    assert!(!status.expired_asleep);
    assert!(status.stop_confirmed);
}

#[tokio::test]
async fn a_refused_credential_fails_the_run_and_keeps_its_work() {
    let h = Harness::new().await;
    h.start("run-1", "fail-auth", RunPermissions::Act).await;
    let status = h.wait("run-1", "the run fails", ended).await;
    assert_eq!(status.outcome, Some(Outcome::Failed));
    assert!(status
        .error
        .unwrap()
        .contains("did not accept the token or key"));
    assert!(status.kept);
}

#[tokio::test]
async fn an_agent_that_exits_on_its_own_interrupts_the_run() {
    let h = Harness::new().await;
    h.start("run-1", "hello", RunPermissions::Act).await;
    h.wait("run-1", "the first turn ends", idle).await;
    h.runtime.prompt("run-1", "c2", "die").await.unwrap();
    let status = h.wait("run-1", "the run ends", ended).await;
    assert_eq!(status.outcome, Some(Outcome::Interrupted));
}

#[tokio::test]
async fn a_restarted_controller_interrupts_live_runs_and_keeps_their_journal() {
    let mut h = Harness::new().await;
    h.start("run-1", "hello", RunPermissions::Act).await;
    h.wait("run-1", "the first turn ends", idle).await;
    let before = h.events("run-1").await;
    // The controller is killed; its container keeps running until the new
    // controller stops it.
    h.restart().await;
    let status = h.wait("run-1", "the run is interrupted", ended).await;
    assert_eq!(status.outcome, Some(Outcome::Interrupted));
    assert!(status.stop_confirmed && status.kept);
    assert!(!h.engine.get().containers["run-1"].running);
    let after = h.events("run-1").await;
    assert_eq!(&after[..before.len()], &before[..]);
    // Nothing of the killed controller wrote after it: the file's sequences
    // are whole and in order.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let journal =
        std::fs::read_to_string(h.state().run_dir("run-1").join("journal.jsonl")).unwrap();
    let seqs: Vec<u64> = journal
        .lines()
        .map(|l| serde_json::from_str::<Event>(l).unwrap().seq)
        .collect();
    assert_eq!(seqs, (1..=seqs.len() as u64).collect::<Vec<_>>());
    // It never takes another prompt, and the credential was never asked for again.
    assert!(h.runtime.prompt("run-1", "c2", "again").await.is_err());
    assert_eq!(h.engine.get().frames.len(), 1);
}

#[tokio::test]
async fn a_stop_the_engine_cannot_confirm_stays_stopping() {
    let h = Harness::new().await;
    h.start("run-1", "hello", RunPermissions::Act).await;
    h.wait("run-1", "the first turn ends", idle).await;
    h.engine.get().unreachable = true;
    h.runtime.stop("run-1", StopReason::Cancel).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let status = h.status("run-1").await;
    assert_eq!(status.phase, Phase::Stopping);
    assert!(!status.stop_confirmed);
    // Discard is refused while the stop is not confirmed.
    assert!(h.runtime.discard("run-1").await.is_err());
    h.engine.get().unreachable = false;
    // The controller tries again by itself.
    for _ in 0..150 {
        if h.status("run-1").await.phase == Phase::Ended {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the stop was not retried");
}

#[tokio::test]
async fn events_wait_for_the_next_one() {
    let h = Harness::new().await;
    h.start("run-1", "hello", RunPermissions::Act).await;
    h.wait("run-1", "the first turn ends", idle).await;
    let cursor = h.status("run-1").await.cursor;
    let runtime = &h.runtime;
    let waiting = runtime.events("run-1", cursor, Duration::from_secs(5));
    let prompt = async {
        tokio::time::sleep(Duration::from_millis(200)).await;
        runtime.prompt("run-1", "c2", "again").await.unwrap()
    };
    let (page, _) = tokio::join!(waiting, prompt);
    let page = page.unwrap();
    assert_eq!(page.events[0].seq, cursor + 1);
}

#[tokio::test]
async fn a_client_without_the_token_is_refused() {
    let h = Harness::new().await;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let stream = tokio::net::UnixStream::connect(h.socket()).await.unwrap();
    let (read, mut write) = stream.into_split();
    write
        .write_all(b"{\"op\":\"hello\",\"token\":\"wrong\",\"protocol\":1}\n{\"op\":\"status\"}\n")
        .await
        .unwrap();
    let mut lines = BufReader::new(read).lines();
    let first: Value = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
    assert_eq!(first["op"], "error");
    // The connection is closed: the status request gets nothing.
    assert!(lines.next_line().await.unwrap().is_none());
}

#[tokio::test]
async fn the_guard_stops_what_a_dead_controller_left() {
    let mut h = Harness::new().await;
    h.start("run-1", "hello", RunPermissions::Act).await;
    h.wait("run-1", "the first turn ends", idle).await;
    // Stand in for a killed controller: stop serving, keep the container.
    h.server.abort();
    let _ = (&mut h.server).await;
    let mut child = std::process::Command::new("true").spawn().unwrap();
    let dead = child.id();
    child.wait().unwrap();
    tokio::time::timeout(
        Duration::from_secs(5),
        guard::watch(h.state(), dead, h.engine.clone()),
    )
    .await
    .expect("the guard acts once the controller is gone");
    assert!(!h.engine.get().containers["run-1"].running);
    // Stopped, never removed.
    assert!(h.engine.get().discarded.is_empty());
}

/// The real engine, the real image, and the Claude adapter, with a fake key
/// that Anthropic refuses: the container starts non-TTY with logging off,
/// the start is cloned into the workspace, the session opens, nothing of the
/// key shows in the container's settings, the refusal fails the run and the
/// stop keeps the container, and Discard removes it and its volume. It
/// reaches Anthropic's API and takes about three minutes: Claude Code
/// retries a refused key before it gives up. Opt-in: set BRAINIAC_TEST_DOCKER_SOCKET
/// to an engine's socket and run `cargo test --test agent_controller --
/// --ignored real_engine`. It builds the image when it is missing.
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn a_run_on_a_real_engine() {
    use bollard::query_parameters::{InspectContainerOptions, ListContainersOptionsBuilder};
    use brainiac_lib::agents::controller::docker::{DockerEngine, LABEL_RUN};

    let socket = std::env::var("BRAINIAC_TEST_DOCKER_SOCKET").expect("BRAINIAC_TEST_DOCKER_SOCKET");
    let image = brainiac_lib::agents::image::build(std::path::Path::new(&socket))
        .await
        .unwrap();

    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    let git = |args: &[&str]| {
        let status = std::process::Command::new("git")
            .args([
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "-c",
                "init.defaultBranch=main",
            ])
            .args(args)
            .current_dir(tmp.path())
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    };
    git(&["init", "-q", repo.to_str().unwrap()]);
    std::fs::write(repo.join("README.md"), "# Example\n").unwrap();
    let repo_arg = repo.to_str().unwrap();
    git(&["-C", repo_arg, "add", "README.md"]);
    git(&["-C", repo_arg, "commit", "-q", "-m", "Start"]);
    git(&["-C", repo_arg, "branch", "start"]);
    let start_commit = String::from_utf8(
        std::process::Command::new("git")
            .args(["-C", repo_arg, "rev-parse", "start"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .to_string();
    let bundle = tmp.path().join("input.bundle");
    git(&[
        "-C",
        repo_arg,
        "bundle",
        "create",
        "-q",
        bundle.to_str().unwrap(),
        "start",
    ]);

    // A short folder: a socket path is limited to 104 bytes.
    let state = StateDir::new(tmp.path().join("c"));
    let config = Config {
        state: state.clone(),
        socket: tmp.path().join("c").join("runner.sock"),
        idle_exit: None,
        spawn_guard: false,
    };
    let server = tokio::spawn(async move { serve(config, DockerEngine::new()).await.unwrap() });
    let runtime = RunRuntime::attach(state, tmp.path().join("c").join("runner.sock"));
    for _ in 0..100 {
        if runtime.running().await.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let run_id = format!("test-{}", std::process::id());
    let start = StartRun {
        run_id: run_id.clone(),
        attempt: 1,
        engine_socket: socket.clone(),
        image: image.name.clone(),
        cpus: 2,
        memory_mib: 3072,
        time_limit_secs: 600,
        permissions: RunPermissions::Ask,
        start_commit: start_commit.clone(),
        bundle,
        prompt_id: "p1".into(),
        prompt: "Reply with the word ready.".into(),
        credential: Credential {
            key: CredentialKey::AnthropicApiKey,
            value: KEY.into(),
        },
    };
    runtime.start(start).await.unwrap();

    // The session opens only after the entrypoint cloned the start.
    let mut ready = false;
    let mut after = 0;
    for _ in 0..240 {
        let page = runtime
            .events(&run_id, after, Duration::from_millis(500))
            .await
            .unwrap();
        for event in &page.events {
            after = event.seq;
            match &event.body {
                EventBody::Ready { agent, .. } => {
                    ready = true;
                    eprintln!("session open: {agent}");
                }
                EventBody::Ended { outcome, message } => {
                    panic!("the run ended: {outcome:?} {message:?}")
                }
                _ => {}
            }
        }
        if ready {
            break;
        }
    }
    assert!(ready, "the session did not open");

    let docker =
        bollard::Docker::connect_with_unix(&socket, 60, bollard::API_DEFAULT_VERSION).unwrap();
    let mut filters = HashMap::new();
    filters.insert("label".to_string(), vec![format!("{LABEL_RUN}={run_id}")]);
    let listed = docker
        .list_containers(Some(
            ListContainersOptionsBuilder::new()
                .all(true)
                .filters(&filters)
                .build(),
        ))
        .await
        .unwrap();
    assert_eq!(listed.len(), 1);
    let id = listed[0].id.clone().unwrap();
    let inspected = docker
        .inspect_container(&id, None::<InspectContainerOptions>)
        .await
        .unwrap();
    let text = serde_json::to_string(&inspected).unwrap();
    assert!(
        !text.contains(KEY),
        "the key is in the container's settings"
    );
    assert_eq!(inspected.config.as_ref().unwrap().tty, Some(false));
    let host = inspected.host_config.as_ref().unwrap();
    assert_eq!(
        host.log_config.as_ref().unwrap().typ.as_deref(),
        Some("none")
    );
    assert!(host.binds.as_ref().is_none_or(|b| b.is_empty()));

    // Anthropic refuses the fake key: the run fails rather than ending its
    // turn as if the refusal were a reply.
    let mut failed = false;
    after = 0;
    for i in 0..480 {
        if i % 60 == 0 {
            let run = runtime
                .runs()
                .await
                .unwrap()
                .into_iter()
                .find(|r| r.run_id == run_id)
                .unwrap();
            eprintln!(
                "status: {:?} {:?} turn {}",
                run.phase, run.activity, run.turn
            );
        }
        let page = runtime
            .events(&run_id, after, Duration::from_millis(500))
            .await
            .unwrap();
        for event in &page.events {
            after = event.seq;
            eprintln!("{:?}", event.body);
            if matches!(
                event.body,
                EventBody::Stopping {
                    outcome: Outcome::Failed
                }
            ) {
                failed = true;
            }
        }
        if failed {
            break;
        }
    }
    assert!(failed, "the refused key did not fail the run");

    runtime.stop(&run_id, StopReason::Cancel).await.unwrap();
    let mut status = None;
    for _ in 0..300 {
        let run = runtime
            .runs()
            .await
            .unwrap()
            .into_iter()
            .find(|r| r.run_id == run_id)
            .unwrap();
        if run.phase == Phase::Ended {
            status = Some(run);
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let status = status.expect("the run stopped");
    assert_eq!(status.outcome, Some(Outcome::Failed));
    assert!(
        status
            .error
            .as_deref()
            .unwrap_or("")
            .contains("did not accept"),
        "{status:?}"
    );
    assert!(status.stop_confirmed && status.kept);
    let inspected = docker
        .inspect_container(&id, None::<InspectContainerOptions>)
        .await
        .unwrap();
    assert_eq!(
        inspected.state.unwrap().running,
        Some(false),
        "stopped, and kept"
    );
    docker
        .inspect_volume(&format!("brainiac-run-{run_id}"))
        .await
        .expect("the workspace is kept");

    // The collector runs against the kept volume, offline and read only:
    // the agent changed nothing, so the result is the start itself.
    let out = tmp.path().join("out");
    std::fs::create_dir_all(&out).unwrap();
    let manifest = runtime.collect(&run_id, Vec::new(), &out).await.unwrap();
    eprintln!("collected: {manifest:?}");
    assert_eq!(manifest.result, start_commit);
    assert_eq!(manifest.changed_files, 0);
    assert!(!out.join("result.bundle").exists());

    runtime.discard(&run_id).await.unwrap();
    assert!(docker
        .inspect_container(&id, None::<InspectContainerOptions>)
        .await
        .is_err());
    assert!(docker
        .inspect_volume(&format!("brainiac-run-{run_id}"))
        .await
        .is_err());
    server.abort();
}

#[tokio::test]
async fn a_pending_permission_is_filtered_before_status_shows_it() {
    let h = Harness::new().await;
    h.start("run-1", "ask-secret", RunPermissions::Ask).await;
    let waiting = h
        .wait("run-1", "the permission is pending", |s| {
            s.activity == Activity::Permission
        })
        .await;
    let shown = serde_json::to_string(&waiting.permissions).unwrap();
    assert!(!shown.contains(KEY), "{shown}");
    assert!(shown.contains("[ANTHROPIC_API_KEY]"));
}

#[tokio::test]
async fn cancel_while_the_container_is_made_starts_nothing() {
    let h = Harness::new().await;
    h.engine.get().launch_delay = Duration::from_secs(30);
    h.start("run-1", "hello", RunPermissions::Act).await;
    h.runtime.stop("run-1", StopReason::Cancel).await.unwrap();
    let status = h.wait("run-1", "the run ends", ended).await;
    assert_eq!(status.outcome, Some(Outcome::Cancelled));
    assert!(status.stop_confirmed);
    assert!(h.engine.get().prompts.is_empty());
    assert!(h.engine.get().frames.is_empty());
}

#[tokio::test]
async fn a_repeat_of_a_command_on_its_way_gets_its_outcome() {
    let h = Harness::new().await;
    h.engine.get().launch_delay = Duration::from_millis(500);
    h.start("run-1", "hello", RunPermissions::Act).await;
    // The run's own prompt is still waiting for its session: a repeat of its
    // ID waits too, then gets Delivered, and nothing is sent twice.
    let repeat = h
        .runtime
        .prompt("run-1", "run-1-prompt", "hello")
        .await
        .unwrap();
    assert_eq!(repeat, Delivery::Delivered);
    h.wait("run-1", "the first turn ends", idle).await;
    assert_eq!(h.engine.get().prompts, vec!["hello"]);
}

#[tokio::test]
async fn the_guard_leaves_a_newer_controllers_runs_alone() {
    let h = Harness::new().await;
    h.start("run-1", "hello", RunPermissions::Act).await;
    h.wait("run-1", "the first turn ends", idle).await;
    // An old controller's guard: its controller is gone, and the running one
    // holds the lock.
    let mut child = std::process::Command::new("true").spawn().unwrap();
    let dead = child.id();
    child.wait().unwrap();
    tokio::time::timeout(
        Duration::from_secs(5),
        guard::watch(h.state(), dead, h.engine.clone()),
    )
    .await
    .expect("the guard exits");
    assert!(h.engine.get().containers["run-1"].running);
    assert!(idle(&h.status("run-1").await));
}

#[tokio::test]
async fn a_finished_run_is_collected_as_one_snapshot_on_the_start() {
    let h = Harness::new().await;
    h.start("run-1", "edit", RunPermissions::Act).await;
    h.wait("run-1", "the turn ends", idle).await;
    // Too early: the run is live.
    let out = h.tmp.path().join("out");
    std::fs::create_dir_all(&out).unwrap();
    let err = h
        .runtime
        .collect("run-1", Vec::new(), &out)
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Conflict);

    h.runtime.stop("run-1", StopReason::Finish).await.unwrap();
    let status = h
        .wait("run-1", "the stop is confirmed", |s| {
            ended(s) && s.stop_confirmed
        })
        .await;
    assert_eq!(status.outcome, Some(Outcome::Finished));

    let manifest = h.runtime.collect("run-1", Vec::new(), &out).await.unwrap();
    assert_eq!(manifest.start, h.start);
    assert_ne!(manifest.result, h.start);
    // readme.txt edited, agent.txt new, old.txt deleted, .gitignore edited;
    // notes.tmp left out by the start's rules, not hidden by the agent's "*".
    assert_eq!(manifest.changed_files, 4);
    assert_eq!(
        manifest
            .left_out
            .iter()
            .map(|l| l.path.as_str())
            .collect::<Vec<_>>(),
        ["notes.tmp"]
    );
    assert!(manifest.left_out[0].reason.contains("*.tmp"));
    let bundle = out.join("result.bundle");
    assert!(bundle.is_file());
    assert!(h
        .runtime
        .collect("run-1", vec!["../x".into()], &out)
        .await
        .is_err());

    // The bundle's one commit sits on the start, and is what the agent left.
    git(
        &h.repo,
        &[
            "fetch",
            "-q",
            bundle.to_str().unwrap(),
            "refs/heads/result:refs/heads/result",
        ],
    );
    assert_eq!(git(&h.repo, &["rev-parse", "result"]), manifest.result);
    assert_eq!(git(&h.repo, &["rev-parse", "result^"]), h.start);
    assert_eq!(
        git(&h.repo, &["show", "result:readme.txt"]),
        "edited by the agent"
    );
    assert_eq!(git(&h.repo, &["show", "result:agent.txt"]), "new");
    assert!(git(&h.repo, &["ls-tree", "--name-only", "result"])
        .lines()
        .all(|l| l != "old.txt" && l != "notes.tmp"));

    // Choosing the left-out file collects it too; the volume is unchanged.
    let manifest = h
        .runtime
        .collect("run-1", vec!["notes.tmp".into()], &out)
        .await
        .unwrap();
    assert_eq!(manifest.changed_files, 5);
    assert!(manifest.left_out.is_empty());
}
