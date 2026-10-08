//! Shared by the agent run tests: a stand-in engine whose "container" is a
//! fake ACP agent in this process, so no Docker is needed, and Git in a
//! test's own folder.
#![allow(dead_code)]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use brainiac_lib::agents::controller::docker::{
    Attached, CollectSpec, LaunchSpec, Output, Running, Workloads,
};
use brainiac_lib::agents::controller::protocol::CollectManifest;
use brainiac_lib::models::{AppResult, ErrorCode};
use serde_json::{json, Value};
use tokio::sync::mpsc;

pub const KEY: &str = "sk-ant-api03-FakeKeyForTests0123456789_-AbCdEfGh";

/// One fake container: its agent task, and what it was given.
pub struct Container {
    pub running: bool,
    pub output: mpsc::Sender<Output>,
    pub agent: tokio::task::JoinHandle<()>,
}

#[derive(Default)]
pub struct Engine {
    pub containers: HashMap<String, Container>,
    /// Where each run's "volume" is: a clone of its bundle on disk, which
    /// the fake agent edits and the real collector script reads.
    pub volumes: PathBuf,
    /// Each credential frame the agents read, as JSON.
    pub frames: Vec<Value>,
    /// Each prompt the agents were sent.
    pub prompts: Vec<String>,
    pub launches: usize,
    /// The container environment of each launch, in order.
    pub envs: Vec<Vec<String>>,
    pub discarded: Vec<String>,
    /// The current image each collection and discard named, in order.
    pub fallback_images: Vec<Option<String>>,
    /// The agent answers the prompt with a refused key's text as an ordinary
    /// reply, as Claude Code did with a malformed token.
    pub refusal_as_reply: bool,
    /// The engine stops answering stops.
    pub unreachable: bool,
    /// How long making a container takes.
    pub launch_delay: Duration,
    /// The run image is gone from the engine, as after a prune.
    pub image_missing: bool,
    /// Collections of a running run's workspace (Changes so far).
    pub previews: usize,
    /// How long a preview's container takes to be made.
    pub preview_delay: Duration,
    /// Previews whose container was never started: the run was stopping.
    pub previews_cancelled: usize,
}

#[derive(Clone, Default)]
pub struct FakeEngine(pub Arc<Mutex<Engine>>);

impl FakeEngine {
    pub fn get(&self) -> std::sync::MutexGuard<'_, Engine> {
        self.0.lock().unwrap()
    }
}

impl Workloads for FakeEngine {
    async fn check_image(&self, _: &str, _: &str) -> AppResult<()> {
        if self.get().image_missing {
            return Err(brainiac_lib::models::AppError::dependency(
                "The run's image is no longer on the engine.",
            ));
        }
        Ok(())
    }

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
        // OpenCode answers as itself (`launch_env` says which agent starts).
        let opencode = spec.env.iter().any(|e| e == "BRAINIAC_AGENT=opencode");
        let agent = tokio::spawn(fake_agent(
            self.clone(),
            work,
            stdin_rx,
            output_tx.clone(),
            opencode,
        ));
        let mut engine = self.get();
        engine.launches += 1;
        engine.envs.push(spec.env.clone());
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
        if spec.live {
            self.get().previews += 1;
            // Making the preview's container takes a while; a stop meanwhile
            // means it is never started.
            let delay = self.get().preview_delay;
            tokio::time::sleep(delay).await;
            if spec.cancel.as_ref().is_some_and(|c| *c.borrow()) {
                self.get().previews_cancelled += 1;
                return Err(brainiac_lib::models::AppError::new(
                    ErrorCode::Conflict,
                    "The run started stopping while its changes were being read.",
                ));
            }
        } else {
            self.get().fallback_images.push(spec.fallback_image.clone());
        }
        let work = self.get().volumes.join(&spec.run_id);
        let scratch = self.get().volumes.join(format!(
            "{}-{}",
            spec.run_id,
            if spec.live { "preview" } else { "collect" }
        ));
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

    async fn discard(
        &self,
        _: &str,
        _: &str,
        run_id: &str,
        _: &str,
        _: &str,
        fallback_image: Option<&str>,
    ) -> AppResult<()> {
        let mut engine = self.get();
        engine.containers.remove(run_id);
        engine.discarded.push(run_id.to_string());
        engine
            .fallback_images
            .push(fallback_image.map(str::to_string));
        Ok(())
    }
}

/// A stand-in for the entrypoint and the agent: Claude Code's ACP adapter,
/// or OpenCode, which reports its model as a configuration option. Prompts
/// steer it:
/// "ask" asks a permission, "hang" works until cancelled, "die" exits,
/// "fail-auth" is refused, "echo-secret" repeats the key in two pieces,
/// "edit" changes files in the workspace ("edit and report" also reports
/// the edit as a diff).
pub async fn fake_agent(
    engine: FakeEngine,
    work: PathBuf,
    mut stdin: mpsc::UnboundedReceiver<Vec<u8>>,
    out: mpsc::Sender<Output>,
    opencode: bool,
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
    // Exactly one key, whichever the agent reads.
    let object = frame.as_object().unwrap();
    assert_eq!(object.len(), 1, "one credential per frame");
    let key = object
        .values()
        .next()
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
                Some("session/new") if opencode => {
                    assert_eq!(message["params"]["mcpServers"], json!([]));
                    // OpenCode names the model in use as a configuration option.
                    say(json!({ "jsonrpc": "2.0", "id": id, "result": { "sessionId": "sess-1",
                        "configOptions": [{ "id": "model", "category": "model", "type": "select",
                            "currentValue": "openrouter/anthropic/claude-sonnet-5-5", "options": [] }] } }))
                        .await;
                }
                Some("session/new") => {
                    assert_eq!(message["params"]["mcpServers"], json!([]));
                    // The adapter names the models it offers and the one in use.
                    say(json!({ "jsonrpc": "2.0", "id": id, "result": { "sessionId": "sess-1",
                        "models": { "currentModelId": "claude-sonnet-4-5",
                            "availableModels": [{ "modelId": "claude-sonnet-4-5", "name": "Sonnet" }] } } }))
                        .await;
                }
                Some("session/prompt") => {
                    let text = message["params"]["prompt"][0]["text"]
                        .as_str()
                        .unwrap()
                        .to_string();
                    engine.get().prompts.push(text.clone());
                    let refusal_as_reply = engine.get().refusal_as_reply;
                    if refusal_as_reply {
                        say(update(
                            "Failed to authenticate. API Error: 401 {\"type\":\"error\"}",
                        ))
                        .await;
                        say(json!({ "jsonrpc": "2.0", "id": id, "result": { "stopReason": "end_turn" } })).await;
                    } else if text.contains("ask") {
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
                        // "edit and report" also says what it edited, as
                        // the adapter's Edit tool does.
                        if text.contains("report") {
                            say(json!({ "jsonrpc": "2.0", "method": "session/update", "params": { "sessionId": "sess-1",
                                "update": { "sessionUpdate": "tool_call", "toolCallId": "e1", "title": "Edit readme.txt",
                                    "kind": "edit", "status": "completed",
                                    "locations": [{ "path": "/workspace/readme.txt" }],
                                    "content": [{ "type": "diff", "path": "/workspace/readme.txt",
                                        "oldText": "start", "newText": "edited by the agent" }] } } }))
                            .await;
                        }
                        // Edits the agent never committed, a new file, a
                        // deletion, and a file its ignore rules would hide.
                        std::fs::write(work.join("readme.txt"), "edited by the agent\n").unwrap();
                        std::fs::write(work.join("agent.txt"), "new\n").unwrap();
                        std::fs::remove_file(work.join("old.txt")).unwrap();
                        std::fs::write(work.join("notes.tmp"), "scratch\n").unwrap();
                        // Kept by the start's negated rule, whatever the agent's rewrite says.
                        std::fs::write(work.join("important.log"), "keep\n").unwrap();
                        std::fs::write(work.join(".gitignore"), "*\n").unwrap();
                        // A fresh ext4 filesystem's own folder, and a folder the
                        // agent made unreadable.
                        std::fs::create_dir_all(work.join("lost+found")).unwrap();
                        std::fs::create_dir_all(work.join("secret")).unwrap();
                        std::fs::write(work.join("secret/x.txt"), "x\n").unwrap();
                        std::fs::set_permissions(
                            work.join("secret"),
                            std::os::unix::fs::PermissionsExt::from_mode(0o000),
                        )
                        .unwrap();
                        say(update("edited")).await;
                        say(json!({ "jsonrpc": "2.0", "id": id, "result": { "stopReason": "end_turn" } })).await;
                    } else if text.contains("odd files") {
                        // What a text diff gets wrong: bytes that are not
                        // text, a link, executable bits (on a new file and
                        // as the only change to a tracked one), and the
                        // ignore file deleted with an ignored file made.
                        std::fs::write(work.join("data.bin"), [0u8, 159, 146, 150, 0, 255, 10])
                            .unwrap();
                        std::os::unix::fs::symlink("readme.txt", work.join("link")).unwrap();
                        std::fs::write(work.join("run.sh"), "#!/bin/sh\necho hi\n").unwrap();
                        for file in ["run.sh", "readme.txt"] {
                            std::fs::set_permissions(
                                work.join(file),
                                std::os::unix::fs::PermissionsExt::from_mode(0o755),
                            )
                            .unwrap();
                        }
                        std::fs::remove_file(work.join(".gitignore")).unwrap();
                        std::fs::write(work.join("debug.log"), "noise\n").unwrap();
                        say(update("made odd files")).await;
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

/// Git in a test's own folder, with nothing of the user's configuration.
pub fn git(dir: &std::path::Path, args: &[&str]) -> String {
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
