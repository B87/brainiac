//! ACP over the container's stdio, after one credential frame.
//! The controller is the only client: it advertises no filesystem and no
//! terminal. Tool permission is cancelled unless this drive was started
//! with `allow_tools`, which selects one allow-once option. `hold_permissions`
//! does not answer the ask until a later permit arrives.

use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot};
use tokio::time::timeout;

use crate::engine::Line;

const READY_WAIT: Duration = Duration::from_secs(90);
const INIT_WAIT: Duration = Duration::from_secs(60);
const SESSION_WAIT: Duration = Duration::from_secs(180);
const PROMPT_WAIT: Duration = Duration::from_secs(240);

pub struct PromptJob {
    pub text: String,
    pub done: oneshot::Sender<Result<String, String>>,
}

pub enum Note {
    Log { kind: String, text: String },
    Closed,
}

/// A parked tool permission the client has now answered.
pub struct PermitChoice {
    pub id: String,
    pub allow: bool,
}

pub struct Drive {
    pub lines: mpsc::UnboundedReceiver<Line>,
    pub stdin: mpsc::UnboundedSender<Vec<u8>>,
    pub prompts: mpsc::UnboundedReceiver<PromptJob>,
    pub notes: mpsc::UnboundedSender<Note>,
    pub ready: oneshot::Sender<Result<String, String>>,
    pub allow_tools: bool,
    pub hold_permissions: bool,
    pub permits: mpsc::UnboundedReceiver<PermitChoice>,
}

pub fn bootstrap_frame(token: &str) -> anyhow::Result<Vec<u8>> {
    if !valid_token(token) {
        anyhow::bail!("credential is empty or not a single line");
    }
    let payload = serde_json::to_vec(&json!({ "CLAUDE_CODE_OAUTH_TOKEN": token }))?;
    if payload.len() > 8192 {
        anyhow::bail!("credential frame is too large");
    }
    let mut frame = Vec::with_capacity(8 + payload.len());
    frame.extend_from_slice(b"BRB1");
    frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    frame.extend_from_slice(&payload);
    Ok(frame)
}

pub fn valid_token(token: &str) -> bool {
    (16..=4096).contains(&token.len())
        && token
            .chars()
            .all(|ch| ch.is_ascii() && !ch.is_ascii_whitespace() && !ch.is_ascii_control())
}

pub fn redact(text: &str, secret: Option<&str>) -> String {
    let Some(secret) = secret else {
        return text.to_string();
    };
    if secret.is_empty() {
        return text.to_string();
    }
    text.replace(secret, "[credential]")
}

pub async fn drive(drive: Drive) {
    let Drive {
        mut lines,
        stdin,
        mut prompts,
        notes,
        ready,
        allow_tools,
        hold_permissions,
        mut permits,
    } = drive;
    let mut ready = Some(ready);
    let mut next_id = 1u64;
    let mut next_perm = 1u64;
    let mut answer = String::new();

    let init_id = next_id;
    next_id += 1;
    if !wait_ready(&mut lines, &notes).await {
        signal(
            &mut ready,
            Err("adapter bootstrap did not become ready".into()),
        );
        finish(&notes);
        return;
    }
    if send(
        &stdin,
        &json!({
            "jsonrpc": "2.0",
            "id": init_id,
            "method": "initialize",
            "params": {
                "protocolVersion": 1,
                "clientCapabilities": {},
                "clientInfo": { "name": "brainiac-spike", "version": "0.0.0" }
            }
        }),
    )
    .is_err()
    {
        signal(&mut ready, Err("could not write initialize".into()));
        finish(&notes);
        return;
    }
    let init = match roundtrip(
        &mut lines,
        &stdin,
        &notes,
        &mut permits,
        hold_permissions,
        &mut next_perm,
        init_id,
        INIT_WAIT,
        &mut answer,
        allow_tools,
    )
    .await
    {
        Ok(value) => value,
        Err(err) => {
            signal(&mut ready, Err(err));
            finish(&notes);
            return;
        }
    };
    let agent = init
        .pointer("/agentInfo/name")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let version = init
        .pointer("/agentInfo/version")
        .and_then(Value::as_str)
        .unwrap_or("");
    log(&notes, "acp", &format!("agent {agent} {version}"));

    let session_rpc = next_id;
    next_id += 1;
    if send(
        &stdin,
        &json!({
            "jsonrpc": "2.0",
            "id": session_rpc,
            "method": "session/new",
            "params": { "cwd": "/workspace", "mcpServers": [] }
        }),
    )
    .is_err()
    {
        signal(&mut ready, Err("could not write session/new".into()));
        finish(&notes);
        return;
    }
    let created = match roundtrip(
        &mut lines,
        &stdin,
        &notes,
        &mut permits,
        hold_permissions,
        &mut next_perm,
        session_rpc,
        SESSION_WAIT,
        &mut answer,
        allow_tools,
    )
    .await
    {
        Ok(value) => value,
        Err(err) => {
            signal(&mut ready, Err(err));
            finish(&notes);
            return;
        }
    };
    let Some(session_id) = created.get("sessionId").and_then(Value::as_str) else {
        signal(
            &mut ready,
            Err("session/new did not return a session id".into()),
        );
        finish(&notes);
        return;
    };
    let session_id = session_id.to_string();
    log(&notes, "acp", &format!("session {session_id}"));
    signal(&mut ready, Ok(session_id.clone()));

    loop {
        tokio::select! {
            job = prompts.recv() => {
                let Some(job) = job else {
                    break;
                };
                if let Err(err) = run_prompt(
                    &mut lines,
                    &stdin,
                    &notes,
                    &mut permits,
                    &session_id,
                    &mut next_id,
                    &mut next_perm,
                    &mut answer,
                    allow_tools,
                    hold_permissions,
                    job,
                ).await {
                    log(&notes, "acp", &format!("error {err}"));
                    break;
                }
            }
            line = lines.recv() => {
                match line {
                    Some(Line::Stdout(text)) => log(&notes, "acp", &format!("raw {text}")),
                    Some(Line::Closed { stderr_bytes, stderr_tail }) => {
                        log_close(&notes, stderr_bytes, &stderr_tail);
                        break;
                    }
                    None => break,
                }
            }
        }
    }
    finish(&notes);
}

#[allow(clippy::too_many_arguments)]
async fn run_prompt(
    lines: &mut mpsc::UnboundedReceiver<Line>,
    stdin: &mpsc::UnboundedSender<Vec<u8>>,
    notes: &mpsc::UnboundedSender<Note>,
    permits: &mut mpsc::UnboundedReceiver<PermitChoice>,
    session_id: &str,
    next_id: &mut u64,
    next_perm: &mut u64,
    answer: &mut String,
    allow_tools: bool,
    hold_permissions: bool,
    job: PromptJob,
) -> Result<(), String> {
    answer.clear();
    let prompt_id = *next_id;
    *next_id += 1;
    log(notes, "acp", &format!("prompt {prompt_id}"));
    if send(
        stdin,
        &json!({
            "jsonrpc": "2.0",
            "id": prompt_id,
            "method": "session/prompt",
            "params": {
                "sessionId": session_id,
                "prompt": [{ "type": "text", "text": job.text }]
            }
        }),
    )
    .is_err()
    {
        let _ = job.done.send(Err("agent stdin closed".into()));
        return Err("agent stdin closed".into());
    }
    let budget = if hold_permissions {
        Duration::from_secs(360)
    } else {
        PROMPT_WAIT
    };
    match roundtrip(
        lines,
        stdin,
        notes,
        permits,
        hold_permissions,
        next_perm,
        prompt_id,
        budget,
        answer,
        allow_tools,
    )
    .await
    {
        Ok(result) => {
            let stop = result
                .get("stopReason")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            if !answer.is_empty() {
                log(notes, "acp", &format!("answer {answer}"));
            }
            log(notes, "acp", &format!("stop {stop}"));
            let _ = job.done.send(Ok(stop.to_string()));
            Ok(())
        }
        Err(err) => {
            if !answer.is_empty() {
                log(notes, "acp", &format!("answer {answer}"));
            }
            let _ = job.done.send(Err(err.clone()));
            Err(err)
        }
    }
}

async fn wait_ready(
    lines: &mut mpsc::UnboundedReceiver<Line>,
    notes: &mpsc::UnboundedSender<Note>,
) -> bool {
    let deadline = Instant::now() + READY_WAIT;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return false;
        }
        let line = match timeout(remaining, lines.recv()).await {
            Ok(Some(line)) => line,
            _ => return false,
        };
        match line {
            Line::Stdout(text) if text == "bootstrap:ready" => {
                log(notes, "acp", "bootstrap:ready");
                return true;
            }
            Line::Stdout(text) => log(notes, "acp", &text),
            Line::Closed {
                stderr_bytes,
                stderr_tail,
            } => {
                log_close(notes, stderr_bytes, &stderr_tail);
                return false;
            }
        }
    }
}

struct Parked {
    id: String,
    rpc_id: Value,
    option_id: Option<String>,
}

enum Incoming {
    Line(Option<Line>),
    Permit(Option<PermitChoice>),
}

#[allow(clippy::too_many_arguments)]
async fn roundtrip(
    lines: &mut mpsc::UnboundedReceiver<Line>,
    stdin: &mpsc::UnboundedSender<Vec<u8>>,
    notes: &mpsc::UnboundedSender<Note>,
    permits: &mut mpsc::UnboundedReceiver<PermitChoice>,
    hold_permissions: bool,
    next_perm: &mut u64,
    expected: u64,
    budget: Duration,
    answer: &mut String,
    allow_tools: bool,
) -> Result<Value, String> {
    let deadline = Instant::now() + budget;
    let mut parked: Option<Parked> = None;
    let mut permits_open = true;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(format!("timed out waiting for ACP response {expected}"));
        }
        let incoming = timeout(remaining, async {
            tokio::select! {
                line = lines.recv() => Incoming::Line(line),
                choice = permits.recv(), if permits_open => Incoming::Permit(choice),
            }
        })
        .await;
        match incoming {
            Ok(Incoming::Permit(None)) => permits_open = false,
            Ok(Incoming::Permit(Some(choice))) => {
                answer_parked(stdin, notes, &mut parked, choice)?;
            }
            Ok(Incoming::Line(Some(Line::Stdout(line)))) => {
                if let Some(done) = take_response(
                    stdin,
                    notes,
                    &line,
                    &mut parked,
                    hold_permissions,
                    next_perm,
                    expected,
                    answer,
                    allow_tools,
                )? {
                    return Ok(done);
                }
            }
            Ok(Incoming::Line(Some(Line::Closed {
                stderr_bytes,
                stderr_tail,
            }))) => {
                log_close(notes, stderr_bytes, &stderr_tail);
                return Err("agent attach closed".into());
            }
            Ok(Incoming::Line(None)) => return Err("agent attach closed".into()),
            Err(_) => return Err(format!("timed out waiting for ACP response {expected}")),
        }
    }
}

/// `Some` when this line is the response the turn is waiting for.
#[allow(clippy::too_many_arguments)]
fn take_response(
    stdin: &mpsc::UnboundedSender<Vec<u8>>,
    notes: &mpsc::UnboundedSender<Note>,
    line: &str,
    parked: &mut Option<Parked>,
    hold_permissions: bool,
    next_perm: &mut u64,
    expected: u64,
    answer: &mut String,
    allow_tools: bool,
) -> Result<Option<Value>, String> {
    if line.is_empty() {
        return Ok(None);
    }
    let Ok(message) = serde_json::from_str::<Value>(line) else {
        log(notes, "acp", &format!("raw {line}"));
        return Ok(None);
    };
    if message.get("method").is_some() && message.get("id").is_some() {
        if hold_permissions
            && message.get("method").and_then(Value::as_str) == Some("session/request_permission")
        {
            if parked.is_some() {
                answer_client(stdin, notes, &message, false)?;
                return Ok(None);
            }
            let id = format!("perm-{next_perm}");
            *next_perm += 1;
            log(notes, "permission", &format!("perm:pending:{id}"));
            *parked = Some(Parked {
                id,
                rpc_id: message.get("id").cloned().unwrap_or(Value::Null),
                option_id: allow_once(&message),
            });
            return Ok(None);
        }
        answer_client(stdin, notes, &message, allow_tools)?;
        return Ok(None);
    }
    if message.get("method").is_some() {
        absorb_update(notes, answer, &message);
        return Ok(None);
    }
    let Some(id) = message.get("id").and_then(Value::as_u64) else {
        log(notes, "acp", "unmatched message");
        return Ok(None);
    };
    if id != expected {
        log(notes, "acp", &format!("unmatched response {id}"));
        return Ok(None);
    }
    if let Some(error) = message.get("error") {
        let text = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("ACP error");
        log(notes, "acp", &format!("error {text}"));
        return Err(text.to_string());
    }
    Ok(Some(message.get("result").cloned().unwrap_or(Value::Null)))
}

fn answer_parked(
    stdin: &mpsc::UnboundedSender<Vec<u8>>,
    notes: &mpsc::UnboundedSender<Note>,
    parked: &mut Option<Parked>,
    choice: PermitChoice,
) -> Result<(), String> {
    let Some(pending) = parked.take() else {
        log(notes, "permission", "permit with nothing pending");
        return Ok(());
    };
    if choice.id != pending.id {
        log(
            notes,
            "permission",
            &format!("permit {} does not match {}", choice.id, pending.id),
        );
        *parked = Some(pending);
        return Ok(());
    }
    let option_id = choice.allow.then_some(pending.option_id.clone()).flatten();
    let reply = match option_id.as_deref() {
        Some(option_id) => permission_reply(&pending.rpc_id, option_id),
        None => cancelled(&pending.rpc_id),
    };
    send(stdin, &reply)?;
    let word = if option_id.is_some() {
        "allowed"
    } else {
        "cancelled"
    };
    log(notes, "permission", &format!("perm:{word}:{}", pending.id));
    Ok(())
}

fn permission_reply(id: &Value, option_id: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": { "outcome": { "outcome": "selected", "optionId": option_id } }
    })
}

fn absorb_update(notes: &mpsc::UnboundedSender<Note>, answer: &mut String, message: &Value) {
    let kind = message
        .pointer("/params/update/sessionUpdate")
        .and_then(Value::as_str)
        .unwrap_or("update");
    let text = message
        .pointer("/params/update/content/text")
        .and_then(Value::as_str)
        .unwrap_or("");
    let status = message
        .pointer("/params/update/status")
        .and_then(Value::as_str)
        .unwrap_or("");
    let title = message
        .pointer("/params/update/title")
        .and_then(Value::as_str)
        .unwrap_or("");
    if kind == "agent_message_chunk" && !text.is_empty() && answer.len() < 8 * 1024 {
        answer.push_str(text);
    }
    let mut line = format!("update {kind}");
    if !status.is_empty() {
        line.push(' ');
        line.push_str(status);
    }
    if !title.is_empty() {
        line.push(' ');
        line.push_str(title);
    }
    if !text.is_empty() {
        line.push(' ');
        line.push_str(text);
    }
    log(notes, "acp", &line);
}

fn allow_once(message: &Value) -> Option<String> {
    let options = message.pointer("/params/options")?.as_array()?;
    let mut always = None;
    for option in options {
        let Some(id) = option.get("optionId").and_then(Value::as_str) else {
            continue;
        };
        match option.get("kind").and_then(Value::as_str).unwrap_or("") {
            "allow_once" => return Some(id.to_string()),
            "allow_always" if always.is_none() => always = Some(id.to_string()),
            _ => {}
        }
    }
    always
}

fn cancelled(id: &Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": { "outcome": { "outcome": "cancelled" } }
    })
}

fn answer_client(
    stdin: &mpsc::UnboundedSender<Vec<u8>>,
    notes: &mpsc::UnboundedSender<Note>,
    message: &Value,
    allow_tools: bool,
) -> Result<(), String> {
    let id = message.get("id").cloned().unwrap_or(Value::Null);
    let method = message.get("method").and_then(Value::as_str).unwrap_or("");
    log(notes, "acp", &format!("client-request {method}"));
    let response = if method == "session/request_permission" {
        if allow_tools {
            if let Some(option_id) = allow_once(message) {
                log(notes, "acp", &format!("permission allow-once {option_id}"));
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "outcome": { "outcome": "selected", "optionId": option_id }
                    }
                })
            } else {
                log(notes, "acp", "permission cancel no-allow-option");
                cancelled(&id)
            }
        } else {
            cancelled(&id)
        }
    } else {
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {
                "code": -32601,
                "message": "client filesystem and terminal are not available"
            }
        })
    };
    send(stdin, &response)
}

fn send(stdin: &mpsc::UnboundedSender<Vec<u8>>, value: &Value) -> Result<(), String> {
    let mut bytes = serde_json::to_vec(value).map_err(|err| err.to_string())?;
    bytes.push(b'\n');
    stdin
        .send(bytes)
        .map_err(|_| "agent stdin closed".to_string())
}

fn log(notes: &mpsc::UnboundedSender<Note>, kind: &str, text: &str) {
    let _ = notes.send(Note::Log {
        kind: kind.to_string(),
        text: text.to_string(),
    });
}

fn log_close(notes: &mpsc::UnboundedSender<Note>, stderr_bytes: u64, stderr_tail: &str) {
    log(
        notes,
        "session",
        &format!("attach closed stderr_bytes={stderr_bytes}"),
    );
    if !stderr_tail.is_empty() {
        log(notes, "acp", &format!("stderr {stderr_tail}"));
    }
}

fn signal(
    ready: &mut Option<oneshot::Sender<Result<String, String>>>,
    result: Result<String, String>,
) {
    if let Some(sender) = ready.take() {
        let _ = sender.send(result);
    }
}

fn finish(notes: &mpsc::UnboundedSender<Note>) {
    let _ = notes.send(Note::Closed);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_carries_only_the_subscription_token() {
        let frame = bootstrap_frame("sk-ant-oat01-example-token-value").unwrap();
        assert_eq!(&frame[..4], b"BRB1");
        let len = u32::from_be_bytes(frame[4..8].try_into().unwrap()) as usize;
        assert_eq!(frame.len(), 8 + len);
        let value: Value = serde_json::from_slice(&frame[8..]).unwrap();
        let object = value.as_object().unwrap();
        assert_eq!(object.len(), 1);
        assert!(object.contains_key("CLAUDE_CODE_OAUTH_TOKEN"));
        assert!(!object.contains_key("ANTHROPIC_API_KEY"));
    }

    #[test]
    fn frame_rejects_a_token_with_whitespace() {
        assert!(bootstrap_frame("sk-ant-oat01 example").is_err());
        assert!(bootstrap_frame("short").is_err());
    }

    #[test]
    fn held_allow_selects_the_option() {
        let reply = permission_reply(&json!(7), "allow-once");
        assert_eq!(reply["id"], 7);
        assert_eq!(reply["result"]["outcome"]["outcome"], "selected");
        assert_eq!(reply["result"]["outcome"]["optionId"], "allow-once");
    }

    #[test]
    fn redact_replaces_the_in_memory_secret() {
        assert_eq!(
            redact(
                "token sk-ant-oat01-example-token-value end",
                Some("sk-ant-oat01-example-token-value")
            ),
            "token [credential] end"
        );
        assert_eq!(redact("plain", None), "plain");
    }
}
