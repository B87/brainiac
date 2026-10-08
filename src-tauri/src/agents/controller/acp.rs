//! `AcpClient`: the one writer of a run's agent stream (docs/architecture.md,
//! Agent runs — v0.5, ACP). It sends the credential frame, waits for the
//! entrypoint, opens a session, and then turns the adapter's JSON-RPC into
//! the journal's events. It advertises no client filesystem or terminal and
//! passes no MCP servers; a request for either is refused.
//!
//! The session task owns the stream for the run's whole life. Clients reach
//! it only through `Command`s, so a client that disconnects or quits never
//! changes what the agent sees.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot};
use tokio::time::Instant;

use super::docker::Output;
use super::journal::{clip, Redactor};
use super::protocol::{Activity, EventBody, FileDiff, PendingPermission, PlanEntry};
use crate::models::{AppError, AppResult, ErrorCode, RunPermissions};

/// The entrypoint clones the run's start before it is ready; a large
/// history takes a while.
const BOOT_WAIT: Duration = Duration::from_secs(10 * 60);
/// A streamed reply is stored in pieces at least this far apart, or this large.
const FLUSH_AFTER: Duration = Duration::from_millis(150);
const FLUSH_BYTES: usize = 2048;
const TOOL_OUTPUT_BYTES: usize = 4096;
/// Permissions waiting at once; more are refused.
const MAX_PENDING: usize = 16;
/// Output that is not ACP, skipped; past this the run is stopped.
const MAX_SKIPPED_BYTES: usize = 64 << 20;
const TITLE_BYTES: usize = 1024;
/// An edit's text before or after, each, as the journal keeps any text; a
/// whole file written is often more.
const DIFF_TEXT_BYTES: usize = super::journal::MAX_TEXT_BYTES;
/// Edits kept from one tool update (a multi-site edit reports several).
const MAX_DIFFS: usize = 8;

/// What a client asks of the session.
pub enum Command {
    /// The next prompt. `first` is the run's own prompt, which waits for the
    /// session to open; any other is refused unless the agent is idle.
    /// `done` answers once the prompt is written to the agent.
    Prompt {
        command_id: String,
        text: String,
        first: bool,
        done: oneshot::Sender<AppResult<()>>,
    },
    Permit {
        permission_id: String,
        allow: bool,
        done: oneshot::Sender<AppResult<()>>,
    },
    /// The run is stopping: answer every pending permission cancelled, ask
    /// the agent to cancel its turn, and refuse everything after.
    Cancel { done: oneshot::Sender<()> },
}

/// Where the session reports. The run implements it; tests record it.
pub trait RunSink: Send + Sync + 'static {
    /// Store an event in the journal, filtered.
    fn record(&self, body: EventBody);
    /// The session is open, with the model the agent reported, if any.
    fn ready(&self, session_id: &str, model: Option<&str>);
    fn activity(&self, activity: Activity, turn: u32);
    fn asked(&self, permission: PendingPermission);
    fn answered(&self, permission_id: &str);
    /// The run cannot go on: stop it as failed, with this reason.
    fn failed(&self, message: String);
    /// The agent's stream ended.
    fn closed(&self);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// Waiting for the entrypoint's `bootstrap:ready`.
    Booting,
    Initializing,
    Opening,
    Idle,
    Working,
}

enum Call {
    Initialize,
    NewSession,
    Prompt { turn: u32 },
}

struct Parked {
    permission_id: String,
    rpc_id: Value,
    allow_option: Option<String>,
    reject_option: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StreamKind {
    Message,
    Thought,
}

pub struct Session<S: RunSink> {
    sink: Arc<S>,
    redactor: Arc<Redactor>,
    stdin: mpsc::UnboundedSender<Vec<u8>>,
    permissions: RunPermissions,
    state: State,
    next_rpc: u64,
    calls: HashMap<u64, Call>,
    session_id: Option<String>,
    turn: u32,
    parked: Vec<Parked>,
    next_permission: u32,
    cancelling: bool,
    failed: bool,
    /// The run's own prompt, waiting for the session to open.
    first: Option<(String, String, oneshot::Sender<AppResult<()>>)>,
    stream: Option<(StreamKind, String)>,
    last_chunk: Instant,
    /// The start of this turn's reply, to recognize a refused credential
    /// that comes back as an ordinary reply.
    reply_start: String,
    /// The session's cost the agent last reported, recorded only when it changes.
    cost: Option<(u64, String)>,
    /// The agent's name and version, from `initialize`.
    agent: (String, String),
    noticed: HashSet<String>,
    skipped: usize,
}

impl<S: RunSink> Session<S> {
    pub fn new(
        sink: Arc<S>,
        redactor: Arc<Redactor>,
        stdin: mpsc::UnboundedSender<Vec<u8>>,
        permissions: RunPermissions,
    ) -> Self {
        Self {
            sink,
            redactor,
            stdin,
            permissions,
            state: State::Booting,
            next_rpc: 1,
            calls: HashMap::new(),
            session_id: None,
            turn: 0,
            parked: Vec::new(),
            next_permission: 1,
            cancelling: false,
            failed: false,
            first: None,
            stream: None,
            last_chunk: Instant::now(),
            reply_start: String::new(),
            cost: None,
            agent: (String::new(), String::new()),
            noticed: HashSet::new(),
            skipped: 0,
        }
    }

    /// Drive the session until the agent's stream ends. The frame goes first,
    /// once; it is never sent again, whoever reconnects.
    pub async fn run(
        mut self,
        frame: Vec<u8>,
        mut output: mpsc::Receiver<Output>,
        mut commands: mpsc::UnboundedReceiver<Command>,
    ) {
        if self.stdin.send(frame).is_err() {
            self.fail("The credential could not be written to the run's container.".into());
        }
        let boot_deadline = Instant::now() + BOOT_WAIT;
        let mut commands_open = true;
        loop {
            let booting = matches!(
                self.state,
                State::Booting | State::Initializing | State::Opening
            ) && !self.failed;
            let flush_at = self.stream.as_ref().map(|_| self.last_chunk + FLUSH_AFTER);
            tokio::select! {
                out = output.recv() => match out {
                    Some(Output::Line(line)) => self.line(&line.bytes),
                    Some(Output::Oversized) => {
                        self.fail("The agent sent a message over 16 MB, so the run was stopped.".into());
                    }
                    Some(Output::Closed) | None => break,
                },
                command = commands.recv(), if commands_open => match command {
                    Some(command) => self.command(command),
                    None => commands_open = false,
                },
                _ = tokio::time::sleep_until(flush_at.unwrap_or(boot_deadline)), if flush_at.is_some() => {
                    self.flush(false);
                }
                _ = tokio::time::sleep_until(boot_deadline), if booting => {
                    self.fail("The agent did not start within 10 minutes.".into());
                }
            }
        }
        self.flush(true);
        for parked in std::mem::take(&mut self.parked) {
            self.sink.record(EventBody::PermissionAnswered {
                permission_id: parked.permission_id.clone(),
                outcome: "cancelled".into(),
                by: "run".into(),
            });
            self.sink.answered(&parked.permission_id);
        }
        if let Some((_, _, done)) = self.first.take() {
            let _ = done.send(Err(stopping()));
        }
        self.sink.closed();
    }

    fn fail(&mut self, message: String) {
        if !self.failed {
            self.failed = true;
            self.sink.failed(message);
        }
    }

    fn send(&mut self, value: &Value) {
        let mut bytes = serde_json::to_vec(value).expect("JSON serializes");
        bytes.push(b'\n');
        if self.stdin.send(bytes).is_err() {
            self.fail("The run's container stopped reading.".into());
        }
    }

    fn call(&mut self, method: &str, params: Value, call: Call) {
        let id = self.next_rpc;
        self.next_rpc += 1;
        self.calls.insert(id, call);
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
    }

    fn notice_once(&mut self, key: &str, text: String) {
        if self.noticed.insert(key.to_string()) {
            self.sink.record(EventBody::Notice { text });
        }
    }

    fn line(&mut self, bytes: &[u8]) {
        if self.state == State::Booting {
            // Before the adapter starts, the entrypoint writes plain words.
            let text = String::from_utf8_lossy(bytes);
            let text = text.trim();
            if text == "bootstrap:ready" {
                self.state = State::Initializing;
                self.call(
                    "initialize",
                    json!({
                        "protocolVersion": 1,
                        "clientCapabilities": {
                            "fs": { "readTextFile": false, "writeTextFile": false },
                            "terminal": false
                        },
                        "clientInfo": { "name": "brainiac", "version": env!("CARGO_PKG_VERSION") }
                    }),
                    Call::Initialize,
                );
            } else if let Some(word) = text.strip_prefix("bootstrap:rejected ") {
                let reason = match word {
                    "clone" => "The run's start could not be cloned into its workspace.",
                    "workspace" => "The run's workspace was not empty.",
                    _ => "The run's container refused the credential frame.",
                };
                self.fail(reason.into());
            }
            return;
        }
        let Ok(message) = serde_json::from_slice::<Value>(bytes) else {
            self.skipped += bytes.len();
            if self.skipped > MAX_SKIPPED_BYTES {
                self.fail(
                    "The agent wrote over 64 MB that is not ACP, so the run was stopped.".into(),
                );
            }
            self.notice_once(
                "not-json",
                "The agent wrote something that is not ACP; it was skipped.".into(),
            );
            return;
        };
        let method = message
            .get("method")
            .and_then(Value::as_str)
            .map(str::to_string);
        let id = message.get("id").cloned();
        match (method, id) {
            (Some(method), Some(id)) => self.request(&method, id, &message),
            (Some(method), None) => self.notification(&method, &message),
            (None, Some(id)) => self.response(id, &message),
            (None, None) => {
                self.notice_once(
                    "malformed",
                    "The agent wrote a message with no method or ID; it was skipped.".into(),
                );
            }
        }
    }

    fn response(&mut self, id: Value, message: &Value) {
        let Some(call) = id.as_u64().and_then(|id| self.calls.remove(&id)) else {
            return;
        };
        let error = message
            .pointer("/error/message")
            .and_then(Value::as_str)
            .map(|m| m.to_string());
        let result = message.get("result").cloned().unwrap_or(Value::Null);
        match call {
            Call::Initialize => {
                if let Some(error) = error {
                    return self.fail(format!("The agent did not start: {}", self.safe(&error)));
                }
                let agent = result
                    .pointer("/agentInfo/name")
                    .and_then(Value::as_str)
                    .unwrap_or("agent")
                    .to_string();
                let version = result
                    .pointer("/agentInfo/version")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                self.state = State::Opening;
                // No MCP servers, and the adapter's default setting sources:
                // the repository's CLAUDE.md and .claude settings apply.
                self.call(
                    "session/new",
                    json!({ "cwd": "/workspace", "mcpServers": [] }),
                    Call::NewSession,
                );
                self.agent = (agent, version);
            }
            Call::NewSession => {
                if let Some(error) = error {
                    let reason = if credential_refused(&error) {
                        refused_credential()
                    } else {
                        format!("The agent could not open a session: {}", self.safe(&error))
                    };
                    return self.fail(reason);
                }
                let Some(session_id) = result.get("sessionId").and_then(Value::as_str) else {
                    return self.fail("The agent opened a session without an ID.".into());
                };
                let (agent, version) = self.agent.clone();
                // The agent reports the model the session opened with, a
                // name it could not use showing as its default: Claude
                // Code's adapter in ACP's `models`, OpenCode as the current
                // value of its `model` configuration option.
                let model = result
                    .pointer("/models/currentModelId")
                    .and_then(Value::as_str)
                    .or_else(|| {
                        result
                            .get("configOptions")?
                            .as_array()?
                            .iter()
                            .find(|o| o.get("id").and_then(Value::as_str) == Some("model"))?
                            .get("currentValue")?
                            .as_str()
                    })
                    .map(str::to_string);
                self.session_id = Some(session_id.to_string());
                self.state = State::Idle;
                self.sink.record(EventBody::Ready {
                    session_id: session_id.to_string(),
                    agent,
                    version,
                    model: model.clone(),
                });
                self.sink.ready(session_id, model.as_deref());
                if let Some((command_id, text, done)) = self.first.take() {
                    let _ = done.send(self.prompt(command_id, text));
                }
            }
            Call::Prompt { turn } => {
                self.flush(true);
                for parked in std::mem::take(&mut self.parked) {
                    // The turn ended with a permission unanswered: it no longer waits.
                    self.sink.record(EventBody::PermissionAnswered {
                        permission_id: parked.permission_id.clone(),
                        outcome: "cancelled".into(),
                        by: "run".into(),
                    });
                    self.sink.answered(&parked.permission_id);
                }
                self.state = State::Idle;
                let reply = std::mem::take(&mut self.reply_start);
                match error {
                    Some(error) => {
                        let message = self.safe(&error);
                        self.sink.record(EventBody::TurnEnded {
                            turn,
                            reason: "error".into(),
                            message: Some(message.clone()),
                        });
                        if credential_refused(&error) {
                            self.fail(refused_credential());
                        } else if usage_limit(&error) {
                            self.sink.activity(Activity::PlanLimit, turn);
                        } else {
                            self.sink.activity(Activity::Idle, turn);
                        }
                    }
                    None => {
                        let reason = result
                            .get("stopReason")
                            .and_then(Value::as_str)
                            .unwrap_or("end_turn")
                            .to_string();
                        self.sink.record(EventBody::TurnEnded {
                            turn,
                            reason,
                            message: None,
                        });
                        // A rejected token can come back as the agent's reply.
                        if credential_refused(&reply) && reply.contains("API Error") {
                            self.fail(refused_credential());
                        } else if usage_limit(&reply) && reply.len() < 512 {
                            self.sink.activity(Activity::PlanLimit, turn);
                        } else {
                            self.sink.activity(Activity::Idle, turn);
                        }
                    }
                }
            }
        }
    }

    /// An error from the agent, filtered and cut short: it may quote anything.
    fn safe(&self, text: &str) -> String {
        let mut text = text.to_string();
        self.redactor.redact(&mut text);
        clip(&mut text, 500);
        text
    }

    fn request(&mut self, method: &str, id: Value, message: &Value) {
        if method != "session/request_permission" {
            self.notice_once(
                method,
                format!("The agent asked for {method}, which runs do not offer; it was refused."),
            );
            self.send(&json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32601, "message": "Runs do not offer files or terminals outside the container." }
            }));
            return;
        }
        if self.cancelling || self.state != State::Working {
            // A request after Cancel, or outside a turn, has no effect.
            self.send(&cancelled(&id));
            return;
        }
        if self.parked.len() >= MAX_PENDING {
            self.notice_once(
                "too-many-permissions",
                format!("The agent asked for more than {MAX_PENDING} permissions at once; the rest were refused."),
            );
            self.send(&cancelled(&id));
            return;
        }
        self.flush(true);
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        let tool = params.get("toolCall").cloned().unwrap_or(Value::Null);
        let options = params
            .get("options")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let option = |kind: &str| {
            options
                .iter()
                .find(|o| o.get("kind").and_then(Value::as_str) == Some(kind))
                .and_then(|o| o.get("optionId").and_then(Value::as_str))
                .map(str::to_string)
        };
        let permission_id = format!("p{}", self.next_permission);
        self.next_permission += 1;
        let permission = PendingPermission {
            permission_id: permission_id.clone(),
            turn: self.turn,
            title: clipped(
                tool.get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("An action"),
                TITLE_BYTES,
            ),
            kind: tool
                .get("kind")
                .and_then(Value::as_str)
                .map(|k| clipped(k, 64)),
            detail: detail(&tool, &self.redactor),
            asked_at: super::now(),
            diffs: tool_diffs(&tool, &self.redactor),
        };
        let parked = Parked {
            permission_id: permission_id.clone(),
            rpc_id: id,
            allow_option: option("allow_once"),
            reject_option: option("reject_once"),
        };
        self.sink.record(EventBody::Permission(permission.clone()));
        match self.permissions {
            RunPermissions::Act => {
                let outcome = if parked.allow_option.is_some() {
                    "allowed"
                } else {
                    "rejected"
                };
                if parked.allow_option.is_none() {
                    self.notice_once(
                        "no-allow-once",
                        "The agent asked for a permission that cannot be allowed just once; it was refused.".into(),
                    );
                }
                self.reply(&parked, true);
                self.sink.record(EventBody::PermissionAnswered {
                    permission_id,
                    outcome: outcome.into(),
                    by: "auto".into(),
                });
            }
            RunPermissions::Ask => {
                self.parked.push(parked);
                self.sink.asked(permission);
                self.sink.activity(Activity::Permission, self.turn);
            }
        }
    }

    /// Allow once when allowed and the agent offered it; otherwise reject
    /// once, or cancel when there is no reject option.
    fn reply(&mut self, parked: &Parked, allow: bool) {
        let option = if allow {
            parked
                .allow_option
                .clone()
                .or_else(|| parked.reject_option.clone())
        } else {
            parked.reject_option.clone()
        };
        let reply = match option {
            Some(option) => json!({
                "jsonrpc": "2.0",
                "id": parked.rpc_id,
                "result": { "outcome": { "outcome": "selected", "optionId": option } }
            }),
            None => cancelled(&parked.rpc_id),
        };
        self.send(&reply);
    }

    fn notification(&mut self, method: &str, message: &Value) {
        // The adapter reports which account it uses; that is not whether
        // the token or key works, and the run already knows the account.
        if method == "_auth/status_update" {
            return;
        }
        if method != "session/update" {
            self.notice_once(
                method,
                format!("The agent sent {method}, which runs do not show; it was skipped."),
            );
            return;
        }
        let Some(update) = message.pointer("/params/update") else {
            return;
        };
        let kind = update
            .get("sessionUpdate")
            .and_then(Value::as_str)
            .unwrap_or("");
        match kind {
            "agent_message_chunk" | "agent_thought_chunk" => {
                let text = update
                    .pointer("/content/text")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let stream = if kind == "agent_message_chunk" {
                    StreamKind::Message
                } else {
                    StreamKind::Thought
                };
                if stream == StreamKind::Message && self.reply_start.len() < 512 {
                    self.reply_start.push_str(text);
                }
                self.chunk(stream, text);
            }
            "tool_call" | "tool_call_update" => {
                self.flush(true);
                let field =
                    |name: &str| update.get(name).and_then(Value::as_str).map(str::to_string);
                let locations = update
                    .get("locations")
                    .and_then(Value::as_array)
                    .map(|l| {
                        l.iter()
                            .filter_map(|l| {
                                l.get("path").and_then(Value::as_str).map(str::to_string)
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                self.sink.record(EventBody::Tool {
                    turn: self.turn,
                    tool_id: field("toolCallId").unwrap_or_default(),
                    title: field("title"),
                    kind: field("kind"),
                    status: field("status"),
                    locations,
                    output: tool_output(update, &self.redactor),
                    diffs: tool_diffs(update, &self.redactor),
                });
            }
            "plan" => {
                self.flush(true);
                let entries = update
                    .get("entries")
                    .and_then(Value::as_array)
                    .map(|entries| {
                        entries
                            .iter()
                            .map(|e| PlanEntry {
                                content: e
                                    .get("content")
                                    .and_then(Value::as_str)
                                    .unwrap_or("")
                                    .to_string(),
                                status: e.get("status").and_then(Value::as_str).map(str::to_string),
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                self.sink.record(EventBody::Plan {
                    turn: self.turn,
                    entries,
                });
            }
            // Echoes of the prompt, and the adapter's own bookkeeping.
            "user_message_chunk"
            | "available_commands_update"
            | "current_mode_update"
            | "config_option_update"
            | "session_info_update" => {}
            // The session's cost so far, when the agent reports one.
            "usage_update" => {
                let amount = update.pointer("/cost/amount").and_then(Value::as_f64);
                let currency = update
                    .pointer("/cost/currency")
                    .and_then(Value::as_str)
                    .unwrap_or("USD");
                if let Some(amount) = amount.filter(|a| a.is_finite() && *a >= 0.0) {
                    // Millionths, so the journal stays whole numbers.
                    let micros = (amount * 1_000_000.0).round() as u64;
                    let cost = (micros, currency.chars().take(8).collect::<String>());
                    if self.cost.as_ref() != Some(&cost) {
                        self.cost = Some(cost.clone());
                        self.sink.record(EventBody::Usage {
                            turn: self.turn,
                            cost_micros: cost.0,
                            currency: cost.1,
                        });
                    }
                }
            }
            other => {
                let other = other.to_string();
                self.notice_once(
                    &format!("update:{other}"),
                    format!(
                        "The agent sent an update the run does not show ({other}); it was skipped."
                    ),
                );
            }
        }
    }

    /// A piece of a streamed reply. It is stored once enough has come, or
    /// when something else happens; the end of it waits while it could be
    /// the start of a value the next piece completes.
    fn chunk(&mut self, kind: StreamKind, text: &str) {
        if self.stream.as_ref().is_some_and(|(k, _)| *k != kind) {
            self.flush(true);
        }
        let (_, buffer) = self.stream.get_or_insert_with(|| (kind, String::new()));
        buffer.push_str(text);
        self.last_chunk = Instant::now();
        if buffer.len() >= FLUSH_BYTES {
            self.flush(false);
        }
    }

    fn flush(&mut self, all: bool) {
        let Some((kind, mut buffer)) = self.stream.take() else {
            return;
        };
        let keep = if all {
            0
        } else {
            self.redactor.held_suffix(&buffer)
        };
        let held = buffer.split_off(buffer.len() - keep);
        if !buffer.is_empty() {
            let turn = self.turn;
            self.sink.record(match kind {
                StreamKind::Message => EventBody::Message { turn, text: buffer },
                StreamKind::Thought => EventBody::Thought { turn, text: buffer },
            });
        }
        if !held.is_empty() {
            self.stream = Some((kind, held));
        }
    }

    fn command(&mut self, command: Command) {
        match command {
            Command::Prompt {
                command_id,
                text,
                first,
                done,
            } => {
                if self.cancelling || self.failed {
                    let _ = done.send(Err(stopping()));
                } else if first
                    && matches!(
                        self.state,
                        State::Booting | State::Initializing | State::Opening
                    )
                {
                    self.first = Some((command_id, text, done));
                } else if self.state != State::Idle {
                    let _ = done.send(Err(AppError::new(
                        ErrorCode::Conflict,
                        "The agent is working; send the next prompt when its turn ends.",
                    )));
                } else {
                    let _ = done.send(self.prompt(command_id, text));
                }
            }
            Command::Permit {
                permission_id,
                allow,
                done,
            } => {
                let Some(index) = self
                    .parked
                    .iter()
                    .position(|p| p.permission_id == permission_id)
                else {
                    let _ = done.send(Err(AppError::not_found(
                        "That permission is no longer waiting.",
                    )));
                    return;
                };
                let parked = self.parked.remove(index);
                self.reply(&parked, allow);
                self.sink.record(EventBody::PermissionAnswered {
                    permission_id: permission_id.clone(),
                    outcome: if allow { "allowed" } else { "rejected" }.into(),
                    by: "user".into(),
                });
                self.sink.answered(&permission_id);
                if self.parked.is_empty() {
                    self.sink.activity(Activity::Working, self.turn);
                }
                let _ = done.send(Ok(()));
            }
            Command::Cancel { done } => {
                self.cancelling = true;
                self.flush(true);
                for parked in std::mem::take(&mut self.parked) {
                    self.send(&cancelled(&parked.rpc_id));
                    self.sink.record(EventBody::PermissionAnswered {
                        permission_id: parked.permission_id.clone(),
                        outcome: "cancelled".into(),
                        by: "run".into(),
                    });
                    self.sink.answered(&parked.permission_id);
                }
                if self.state == State::Working {
                    if let Some(session_id) = self.session_id.clone() {
                        self.send(&json!({
                            "jsonrpc": "2.0",
                            "method": "session/cancel",
                            "params": { "sessionId": session_id }
                        }));
                    }
                }
                if let Some((_, _, done)) = self.first.take() {
                    let _ = done.send(Err(stopping()));
                }
                let _ = done.send(());
            }
        }
    }

    /// Write a prompt; the turn is recorded first. An error means nothing
    /// was written.
    fn prompt(&mut self, command_id: String, text: String) -> AppResult<()> {
        let Some(session_id) = self.session_id.clone() else {
            return Err(stopping());
        };
        self.turn += 1;
        self.reply_start.clear();
        self.sink.record(EventBody::Prompt {
            turn: self.turn,
            command_id,
            text: text.clone(),
        });
        self.state = State::Working;
        self.sink.activity(Activity::Working, self.turn);
        let turn = self.turn;
        self.call(
            "session/prompt",
            json!({ "sessionId": session_id, "prompt": [{ "type": "text", "text": text }] }),
            Call::Prompt { turn },
        );
        // Once written, the prompt counts as delivered even if the run
        // fails right after: it is never written again.
        Ok(())
    }
}

fn stopping() -> AppError {
    AppError::new(ErrorCode::Conflict, "The run is stopping.")
}

fn cancelled(id: &Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": { "outcome": { "outcome": "cancelled" } } })
}

fn refused_credential() -> String {
    "The provider did not accept the token or key. Update it in Settings → Agents and start another run.".into()
}

/// How the agents word a refused credential. Claude Code's adapter: an ACP
/// error "Authentication required", or a reply "Failed to authenticate. API
/// Error: 401 …" (docs/design/agent-runs.md, Spike record: Claude
/// subscription adapter). OpenCode: the prompt's error, with the provider's
/// own words ("API key is invalid.", "Incorrect API key provided", "No auth
/// credentials found", "User not found."), or "Authentication required"
/// for a provider with no key (Record: OpenCode in a run's container).
pub(crate) fn credential_refused(text: &str) -> bool {
    let text = text.to_lowercase();
    text.contains("authentication required")
        || text.contains("failed to authenticate")
        || text.contains("invalid x-api-key")
        || text.contains("oauth access token is invalid")
        || text.contains("api key is invalid")
        || text.contains("invalid api key")
        || text.contains("incorrect api key")
        || text.contains("invalid_api_key")
        || text.contains("no auth credentials found")
        || text.contains("user not found.")
}

/// A plan's usage limit, by its wording; Brainiac does not claim to know when
/// it resets.
fn usage_limit(text: &str) -> bool {
    let text = text.to_lowercase();
    text.contains("usage limit") || text.contains("limit reached")
}

fn clipped(text: &str, max: usize) -> String {
    let mut text = text.to_string();
    clip(&mut text, max);
    text
}

/// Filtered, then cut short: a known value that straddles the cut is
/// recognized whole, never kept as all but its end.
fn redacted_clip(mut text: String, redactor: &Redactor, max: usize) -> String {
    redactor.redact(&mut text);
    clip(&mut text, max);
    text
}

/// The command or file a permission is about, when the tool says.
fn detail(tool: &Value, redactor: &Redactor) -> Option<String> {
    let input = tool.get("rawInput")?;
    let value = ["command", "file_path", "path", "url"]
        .iter()
        .find_map(|key| input.get(*key).and_then(Value::as_str))?;
    Some(redacted_clip(
        value.to_string(),
        redactor,
        TOOL_OUTPUT_BYTES,
    ))
}

/// The text a tool update carries, cut short.
fn tool_output(update: &Value, redactor: &Redactor) -> Option<String> {
    let content = update.get("content")?.as_array()?;
    let text: String = content
        .iter()
        .filter_map(|item| item.pointer("/content/text").and_then(Value::as_str))
        .collect();
    if text.is_empty() {
        return None;
    }
    Some(redacted_clip(text, redactor, TOOL_OUTPUT_BYTES))
}

/// The edits a tool call or update reports: ACP's `diff` content items, cut
/// short. They say what the agent meant to change, not what the workspace holds.
fn tool_diffs(update: &Value, redactor: &Redactor) -> Vec<FileDiff> {
    let Some(content) = update.get("content").and_then(Value::as_array) else {
        return Vec::new();
    };
    content
        .iter()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("diff"))
        .filter_map(|item| {
            let path = item.get("path").and_then(Value::as_str)?;
            let new_text = item.get("newText").and_then(Value::as_str)?.to_string();
            let old_text = item
                .get("oldText")
                .and_then(Value::as_str)
                .map(str::to_string);
            let truncated = new_text.len() > DIFF_TEXT_BYTES
                || old_text.as_ref().is_some_and(|t| t.len() > DIFF_TEXT_BYTES);
            Some(FileDiff {
                path: redacted_clip(path.to_string(), redactor, TITLE_BYTES),
                old_text: old_text.map(|t| redacted_clip(t, redactor, DIFF_TEXT_BYTES)),
                new_text: redacted_clip(new_text, redactor, DIFF_TEXT_BYTES),
                truncated,
            })
        })
        .take(MAX_DIFFS)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Recorder {
        events: Mutex<Vec<EventBody>>,
        failed: Mutex<Option<String>>,
        activity: Mutex<Option<Activity>>,
    }

    impl RunSink for Recorder {
        fn record(&self, body: EventBody) {
            self.events.lock().unwrap().push(body);
        }
        fn ready(&self, _: &str, _: Option<&str>) {}
        fn activity(&self, activity: Activity, _: u32) {
            *self.activity.lock().unwrap() = Some(activity);
        }
        fn asked(&self, _: PendingPermission) {}
        fn answered(&self, _: &str) {}
        fn failed(&self, message: String) {
            *self.failed.lock().unwrap() = Some(message);
        }
        fn closed(&self) {}
    }

    fn session(
        redactor: Redactor,
    ) -> (
        Session<Recorder>,
        Arc<Recorder>,
        mpsc::UnboundedReceiver<Vec<u8>>,
    ) {
        let sink = Arc::new(Recorder::default());
        let (stdin, rx) = mpsc::unbounded_channel();
        let mut session = Session::new(
            Arc::clone(&sink),
            Arc::new(redactor),
            stdin,
            RunPermissions::Ask,
        );
        session.state = State::Working;
        session.session_id = Some("s1".into());
        session.turn = 1;
        (session, sink, rx)
    }

    fn chunk(text: &str) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": { "sessionId": "s1", "update": {
                "sessionUpdate": "agent_message_chunk",
                "content": { "type": "text", "text": text }
            }}
        }))
        .unwrap()
    }

    #[test]
    fn a_value_split_across_pieces_is_never_released_whole() {
        let mut redactor = Redactor::new();
        redactor.register("sk-ant-api03-abcdefgh", "ANTHROPIC_API_KEY");
        let (mut session, sink, _rx) = session(redactor);
        session.line(&chunk("the key is sk-ant-ap"));
        session.flush(false);
        // The possible start of the value waits for the next piece.
        assert_eq!(session.stream.as_ref().unwrap().1, "sk-ant-ap");
        session.line(&chunk("i03-abcdefgh, done"));
        session.flush(true);
        let mut redactor = Redactor::new();
        redactor.register("sk-ant-api03-abcdefgh", "ANTHROPIC_API_KEY");
        // The sink filters each piece it stores; together they never held the value.
        let stored: String = sink
            .events
            .lock()
            .unwrap()
            .iter()
            .map(|e| match e {
                EventBody::Message { text, .. } => {
                    let mut text = text.clone();
                    redactor.redact(&mut text);
                    text
                }
                _ => String::new(),
            })
            .collect();
        assert_eq!(stored, "the key is [ANTHROPIC_API_KEY], done");
    }

    #[test]
    fn requests_for_mac_files_or_terminals_are_refused() {
        let (mut session, sink, mut rx) = session(Redactor::new());
        session.line(
            &serde_json::to_vec(&json!({
                "jsonrpc": "2.0", "id": 9, "method": "fs/read_text_file",
                "params": { "path": "/etc/passwd" }
            }))
            .unwrap(),
        );
        let reply: Value = serde_json::from_slice(&rx.try_recv().unwrap()).unwrap();
        assert_eq!(reply["id"], 9);
        assert_eq!(reply["error"]["code"], -32601);
        assert!(matches!(
            sink.events.lock().unwrap()[0],
            EventBody::Notice { .. }
        ));
    }

    #[test]
    fn a_refused_token_that_comes_back_as_a_reply_fails_the_run() {
        let (mut session, sink, _rx) = session(Redactor::new());
        session.calls.insert(4, Call::Prompt { turn: 1 });
        session.line(&chunk(
            "Failed to authenticate. API Error: 401 OAuth access token is invalid.",
        ));
        session.line(
            &serde_json::to_vec(
                &json!({ "jsonrpc": "2.0", "id": 4, "result": { "stopReason": "end_turn" } }),
            )
            .unwrap(),
        );
        assert!(sink
            .failed
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .contains("did not accept"));
    }

    #[test]
    fn a_permission_after_cancel_has_no_effect() {
        let (mut session, sink, mut rx) = session(Redactor::new());
        let (done, _) = oneshot::channel();
        session.command(Command::Cancel { done });
        while rx.try_recv().is_ok() {}
        session.line(
            &serde_json::to_vec(&json!({
                "jsonrpc": "2.0", "id": 5, "method": "session/request_permission",
                "params": { "sessionId": "s1", "toolCall": { "toolCallId": "t1", "title": "rm -rf /" },
                    "options": [{ "optionId": "a", "kind": "allow_once", "name": "Allow" }] }
            }))
            .unwrap(),
        );
        let reply: Value = serde_json::from_slice(&rx.try_recv().unwrap()).unwrap();
        assert_eq!(reply["result"]["outcome"]["outcome"], "cancelled");
        assert!(sink
            .events
            .lock()
            .unwrap()
            .iter()
            .all(|e| !matches!(e, EventBody::Permission(_))));
    }

    #[test]
    fn an_edit_carries_the_change_the_agent_reports() {
        let (mut session, sink, _rx) = session(Redactor::new());
        let long = "x".repeat(DIFF_TEXT_BYTES + 10);
        session.line(
            &serde_json::to_vec(&json!({
                "jsonrpc": "2.0", "method": "session/update",
                "params": { "sessionId": "s1", "update": {
                    "sessionUpdate": "tool_call", "toolCallId": "t1", "title": "Edit a.txt",
                    "kind": "edit", "status": "pending",
                    "locations": [{ "path": "/workspace/a.txt" }],
                    "content": [
                        { "type": "diff", "path": "/workspace/a.txt", "oldText": "one", "newText": "two" },
                        { "type": "diff", "path": "/workspace/b.txt", "oldText": null, "newText": long },
                        { "type": "content", "content": { "type": "text", "text": "done" } }
                    ]
                }}
            }))
            .unwrap(),
        );
        let events = sink.events.lock().unwrap();
        let EventBody::Tool { diffs, output, .. } = &events[0] else {
            panic!("a tool event");
        };
        assert_eq!(output.as_deref(), Some("done"));
        assert_eq!(diffs.len(), 2);
        assert_eq!(diffs[0].old_text.as_deref(), Some("one"));
        assert_eq!(diffs[0].new_text, "two");
        assert!(!diffs[0].truncated);
        assert_eq!(diffs[1].old_text, None);
        assert!(diffs[1].truncated);
        assert!(diffs[1].new_text.len() <= DIFF_TEXT_BYTES + '…'.len_utf8());
    }

    #[test]
    fn a_permission_to_edit_shows_the_edit() {
        let (mut session, sink, _rx) = session(Redactor::new());
        session.line(
            &serde_json::to_vec(&json!({
                "jsonrpc": "2.0", "id": 5, "method": "session/request_permission",
                "params": { "sessionId": "s1",
                    "toolCall": { "toolCallId": "t1", "title": "Edit a.txt", "kind": "edit",
                        "rawInput": { "file_path": "/workspace/a.txt" },
                        "content": [{ "type": "diff", "path": "/workspace/a.txt", "oldText": "one", "newText": "two" }] },
                    "options": [{ "optionId": "a", "kind": "allow_once", "name": "Allow" }] }
            }))
            .unwrap(),
        );
        let events = sink.events.lock().unwrap();
        let Some(EventBody::Permission(p)) = events.last() else {
            panic!("a permission event");
        };
        assert_eq!(p.detail.as_deref(), Some("/workspace/a.txt"));
        assert_eq!(p.diffs.len(), 1);
        assert_eq!(p.diffs[0].new_text, "two");
    }

    #[test]
    fn a_key_across_the_cut_of_an_edit_is_removed_whole() {
        let key = "sk-ant-api03-abcdefghijklmnop";
        let mut redactor = Redactor::new();
        redactor.register(key, "ANTHROPIC_API_KEY");
        let (mut session, sink, _rx) = session(redactor);
        // The key starts 10 bytes before the cut.
        let text = format!("{}{key}\n", "x".repeat(DIFF_TEXT_BYTES - 10));
        session.line(
            &serde_json::to_vec(&json!({
                "jsonrpc": "2.0", "method": "session/update",
                "params": { "sessionId": "s1", "update": {
                    "sessionUpdate": "tool_call", "toolCallId": "t1", "title": "Write .env",
                    "kind": "edit", "status": "pending",
                    "content": [
                        { "type": "diff", "path": "/workspace/.env", "oldText": null, "newText": text },
                        { "type": "content", "content": { "type": "text", "text": text } }
                    ]
                }}
            }))
            .unwrap(),
        );
        let events = sink.events.lock().unwrap();
        let EventBody::Tool { diffs, output, .. } = &events[0] else {
            panic!("a tool event");
        };
        assert!(diffs[0].truncated);
        for stored in [&diffs[0].new_text, output.as_ref().unwrap()] {
            assert!(
                !stored.contains("sk-ant-api03-abc"),
                "{}",
                &stored[stored.len() - 40..]
            );
        }
    }
}
