//! The run controller's protocol (docs/architecture.md, Agent runs — v0.5):
//! one JSON object per line over the controller's socket, a request and its
//! response. It is internal to Brainiac and not exported to TypeScript.
//!
//! The controller only answers; it never pushes. A client that stopped
//! reading (a sleeping Mac, a quit app) is not in any loop the agent's
//! stream depends on. Live updates are a long poll: `Events` returns as soon
//! as there is something after the client's cursor.

use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::models::{AppError, RunPermissions};

/// Bumped when a request or response changes shape. A controller with
/// another protocol is replaced only when it has no live runs.
/// 2 adds bundle upload and download, remote image build, and the engine
/// probe (docs/architecture.md, Remote hosts).
pub const PROTOCOL: u32 = 2;

/// One piece of a bundle copied to or from the controller. Hex doubles it,
/// and the line must stay under [`MAX_LINE_BYTES`].
pub const TRANSFER_CHUNK: usize = 256 * 1024;
/// The same ceiling as a Git bundle the collector will accept.
pub const MAX_TRANSFER: u64 = (1 << 33) - 1;

/// The longest request or response line. A prompt is at most
/// `MAX_PROMPT_BYTES`; a page of events is bounded by `MAX_PAGE_BYTES`.
pub const MAX_LINE_BYTES: usize = 8 << 20;
pub const MAX_PROMPT_BYTES: usize = 100 << 10;
pub const MAX_PAGE_BYTES: usize = 4 << 20;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    /// The first line of every connection.
    Hello { token: String, protocol: u32 },
    /// Accept a run and prepare it in the background. Repeating the same run
    /// ID and attempt returns the run as it is; it never starts another.
    Start(StartRun),
    /// Every run this controller knows.
    Status,
    /// Events after `after`, waiting up to `wait_ms` for the first one.
    Events {
        run_id: String,
        after: u64,
        wait_ms: u64,
    },
    /// The next prompt, only while the run is idle.
    Prompt {
        run_id: String,
        command_id: String,
        text: String,
    },
    /// Answer one pending permission: allow once, or reject.
    Permit {
        run_id: String,
        command_id: String,
        permission_id: String,
        allow: bool,
    },
    /// Stop the run and keep its container and volume. Repeating it is
    /// harmless: a run stops once.
    Stop { run_id: String, reason: StopReason },
    /// Build the snapshot of a stopped run's working tree: the collector
    /// runs against its volume and `result.bundle` is written into
    /// `out_dir`. `include` names left-out files to collect this time.
    /// `image` is the current image's name: the run's own image serves
    /// its collector and workspace helper, and this one stands in when the
    /// run's image is no longer on the engine.
    Collect {
        run_id: String,
        include: Vec<String>,
        out_dir: PathBuf,
        #[serde(default)]
        image: Option<String>,
    },
    /// Remove a stopped run's container, volume, and records here. `image`
    /// is as for `Collect`.
    Discard {
        run_id: String,
        #[serde(default)]
        image: Option<String>,
    },
    /// Exit, only when no run is live: a newer Brainiac replaces it.
    Shutdown,
    /// Write the run's input bundle into the controller's state directory,
    /// one chunk at a time. `offset` is how many bytes are already there.
    PutBundle {
        run_id: String,
        offset: u64,
        total: u64,
        hex: String,
    },
    /// Read `result.bundle` after [`Request::CollectHere`].
    ReadBundle {
        run_id: String,
        offset: u64,
        max: u32,
    },
    /// Collect into the controller's state directory, not a path on the Mac.
    CollectHere {
        run_id: String,
        include: Vec<String>,
        #[serde(default)]
        image: Option<String>,
    },
    /// What the engine on this host is, and whether it can attach loop devices.
    ProbeEngine {
        socket: String,
        #[serde(default)]
        image: Option<String>,
    },
    /// Build the run image from a tar of Brainiac's Dockerfile and its files.
    BuildImage {
        socket: String,
        tag: String,
        hex: String,
    },
    /// Stop every live run's containers and keep them. The guard's socket
    /// answers this when the Mac cannot.
    EmergencyStop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// Finish and collect: only while the run is idle.
    Finish,
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CredentialKey {
    #[serde(rename = "CLAUDE_CODE_OAUTH_TOKEN")]
    ClaudeCodeOauthToken,
    #[serde(rename = "ANTHROPIC_API_KEY")]
    AnthropicApiKey,
}

impl CredentialKey {
    pub fn as_str(self) -> &'static str {
        match self {
            CredentialKey::ClaudeCodeOauthToken => "CLAUDE_CODE_OAUTH_TOKEN",
            CredentialKey::AnthropicApiKey => "ANTHROPIC_API_KEY",
        }
    }
}

/// The one value a run's container gets. It crosses the controller's socket
/// once, at the start, and stays in the controller's memory.
#[derive(Clone, Serialize, Deserialize)]
pub struct Credential {
    pub key: CredentialKey,
    pub value: String,
}

// A hand-written `Debug` so a logged request never shows the value.
impl fmt::Debug for Credential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credential")
            .field("key", &self.key)
            .field("value", &"[redacted]")
            .finish()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartRun {
    /// Brainiac's ID for the run: letters, digits, and dashes.
    pub run_id: String,
    pub attempt: u32,
    /// The engine's socket, chosen in Settings.
    pub engine_socket: String,
    /// The image name, `brainiac-claude:<recipe>`.
    pub image: String,
    pub cpus: u32,
    pub memory_mib: u32,
    /// The workspace's size: a filesystem of exactly this size.
    pub workspace_gib: u32,
    pub time_limit_secs: u64,
    pub permissions: RunPermissions,
    /// The commit the run starts from: the snapshot's parent at collection.
    pub start_commit: String,
    /// `agent-runs/<run-id>/input.bundle` in the data folder.
    pub bundle: PathBuf,
    pub prompt_id: String,
    pub prompt: String,
    pub credential: Credential,
    /// The model to ask Claude Code for, as `ANTHROPIC_MODEL` in the
    /// container's environment; empty for its default. Not a secret.
    #[serde(default)]
    pub model: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Response {
    Welcome {
        protocol: u32,
        /// The installation this controller belongs to: labels on its
        /// containers carry it.
        installation: String,
        /// The Brainiac version the controller runs.
        build: String,
        pid: u32,
    },
    Run {
        run: RunStatus,
    },
    Runs {
        runs: Vec<RunStatus>,
    },
    Events {
        page: EventPage,
    },
    /// A command's recorded outcome. Delivered means handed to the agent's
    /// input, in order, once; not that the agent read it or that its turn
    /// ended. Uncertain means it may have been handed over: it is never
    /// sent again.
    Ack {
        command_id: String,
        outcome: Delivery,
    },
    Collected {
        manifest: CollectManifest,
    },
    Done,
    /// The input bundle is in place on this host. `path` is where Start reads it.
    Uploaded {
        path: String,
        size: u64,
    },
    /// A slice of `result.bundle`. `hex` is empty when `offset == total`.
    Chunk {
        offset: u64,
        total: u64,
        hex: String,
    },
    Engine {
        report: EngineReport,
    },
    Image {
        id: String,
    },
    Error {
        error: AppError,
    },
}

/// What a host's Docker engine said (SPEC.md, Remote hosts).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineReport {
    pub name: String,
    pub supported: bool,
    pub problem: Option<String>,
    /// `losetup` worked in a privileged container of the run image.
    pub loop_devices: bool,
}

/// What the collector found (docs/architecture.md, Agent runs — v0.5,
/// Artifacts). `result` equals `start` when nothing changed, and then no
/// bundle was written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollectManifest {
    pub start: String,
    pub result: String,
    pub changed_files: u32,
    pub left_out: Vec<LeftOut>,
    /// Left-out files beyond the ones listed.
    #[serde(default)]
    pub left_out_more: u32,
}

/// A new file the snapshot does not include, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeftOut {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Delivery {
    Delivered,
    Uncertain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Accepted: the container starts, the start is cloned, the session opens.
    Preparing,
    Running,
    /// A stop was asked for and the engine has not confirmed it.
    Stopping,
    Ended,
}

/// What the agent is doing while the run is running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Activity {
    Preparing,
    Working,
    /// Waiting for the user to answer a permission.
    Permission,
    /// Ready for the next prompt.
    Idle,
    /// The plan's usage limit stopped the turn; the session is still open.
    PlanLimit,
    Stopping,
    Ended,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Finished,
    Cancelled,
    Expired,
    Failed,
    /// The controller, the engine, or the container stopped unexpectedly.
    Interrupted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingPermission {
    pub permission_id: String,
    pub turn: u32,
    pub title: String,
    pub kind: Option<String>,
    /// The command or the file, when the agent said which.
    pub detail: Option<String>,
    pub asked_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunStatus {
    pub run_id: String,
    pub attempt: u32,
    pub phase: Phase,
    pub activity: Activity,
    pub turn: u32,
    pub outcome: Option<Outcome>,
    /// The engine confirmed that nothing of the run is running.
    pub stop_confirmed: bool,
    /// The run's stopped container and workspace volume are on the engine.
    pub kept: bool,
    pub permissions: Vec<PendingPermission>,
    pub session_id: Option<String>,
    /// The model the agent reported when its session opened.
    #[serde(default)]
    pub model: Option<String>,
    pub accepted_at: String,
    /// When the time limit ends the run, by the wall clock at acceptance.
    pub deadline_at: String,
    pub ended_at: Option<String>,
    /// The deadline passed while this Mac slept.
    pub expired_asleep: bool,
    /// The last journal sequence.
    pub cursor: u64,
    /// Why the run failed or could not stop, in words safe to show.
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventPage {
    pub run_id: String,
    pub events: Vec<Event>,
    /// The run's last sequence; more pages follow while it is past the last event.
    pub cursor: u64,
}

/// One entry of a run's journal. Sequences start at 1 and never repeat.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    pub seq: u64,
    pub at: String,
    #[serde(flatten)]
    pub body: EventBody,
}

/// What the journal records: normalized, filtered, never raw protocol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventBody {
    Accepted {
        deadline_at: String,
        permissions: RunPermissions,
    },
    /// The session is open.
    Ready {
        session_id: String,
        agent: String,
        version: String,
        /// The model the agent says the session opened with.
        #[serde(default)]
        model: Option<String>,
    },
    Prompt {
        turn: u32,
        command_id: String,
        text: String,
    },
    /// A piece of the agent's reply; consecutive pieces of one turn join.
    Message {
        turn: u32,
        text: String,
    },
    Thought {
        turn: u32,
        text: String,
    },
    Plan {
        turn: u32,
        entries: Vec<PlanEntry>,
    },
    /// A tool call, or a change to one: fields the update did not carry are absent.
    Tool {
        turn: u32,
        tool_id: String,
        title: Option<String>,
        kind: Option<String>,
        status: Option<String>,
        locations: Vec<String>,
        output: Option<String>,
    },
    Permission(PendingPermission),
    PermissionAnswered {
        permission_id: String,
        /// `allowed`, `rejected`, or `cancelled`.
        outcome: String,
        /// `user`, `auto` (Act without asking), or `run` (the run ended).
        by: String,
    },
    TurnEnded {
        turn: u32,
        /// The agent's stop reason (`end_turn`, `cancelled`, …) or `error`.
        reason: String,
        message: Option<String>,
    },
    /// Something the user should know that is not the agent's: a skipped
    /// message, a refused request from the agent.
    Notice {
        text: String,
    },
    Stopping {
        outcome: Outcome,
    },
    Ended {
        outcome: Outcome,
        message: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanEntry {
    pub content: String,
    pub status: Option<String>,
}

impl EventBody {
    /// Every text the event carries, for filtering before it is stored.
    pub fn texts_mut(&mut self) -> Vec<&mut String> {
        match self {
            EventBody::Accepted { .. } | EventBody::Stopping { .. } => Vec::new(),
            EventBody::Ready {
                session_id,
                agent,
                version,
                model,
            } => {
                let mut texts = vec![session_id, agent, version];
                texts.extend(model.as_mut());
                texts
            }
            EventBody::Prompt {
                command_id, text, ..
            } => vec![command_id, text],
            EventBody::Message { text, .. }
            | EventBody::Thought { text, .. }
            | EventBody::Notice { text } => vec![text],
            EventBody::Plan { entries, .. } => entries
                .iter_mut()
                .flat_map(|e| std::iter::once(&mut e.content).chain(e.status.as_mut()))
                .collect(),
            EventBody::Tool {
                tool_id,
                title,
                kind,
                status,
                locations,
                output,
                ..
            } => std::iter::once(tool_id)
                .chain(title.as_mut())
                .chain(kind.as_mut())
                .chain(status.as_mut())
                .chain(locations.iter_mut())
                .chain(output.as_mut())
                .collect(),
            EventBody::Permission(p) => std::iter::once(&mut p.permission_id)
                .chain(std::iter::once(&mut p.title))
                .chain(p.kind.as_mut())
                .chain(p.detail.as_mut())
                .collect(),
            EventBody::PermissionAnswered {
                permission_id,
                outcome,
                by,
            } => vec![permission_id, outcome, by],
            EventBody::TurnEnded {
                reason, message, ..
            } => std::iter::once(reason).chain(message.as_mut()).collect(),
            EventBody::Ended { message, .. } => message.iter_mut().collect(),
        }
    }
}

/// IDs become file names and labels: letters, digits, dashes, underscores.
pub fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0xf) as usize] as char);
    }
    out
}

pub fn decode_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() / 2);
    let mut i = 0;
    while i < bytes.len() {
        let hi = from_hex(bytes[i])?;
        let lo = from_hex(bytes[i + 1])?;
        out.push((hi << 4) | lo);
        i += 2;
    }
    Some(out)
}

fn from_hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_start_request_never_shows_its_credential_in_debug() {
        let credential = Credential {
            key: CredentialKey::AnthropicApiKey,
            value: "sk-ant-api03-example-value".into(),
        };
        let text = format!("{credential:?}");
        assert!(!text.contains("example-value"));
        assert!(text.contains("AnthropicApiKey"));
    }

    #[test]
    fn events_are_one_flat_object() {
        let event = Event {
            seq: 3,
            at: "2026-10-05T12:00:00Z".into(),
            body: EventBody::Message {
                turn: 1,
                text: "hello".into(),
            },
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "message");
        assert_eq!(json["seq"], 3);
        let back: Event = serde_json::from_value(json).unwrap();
        assert_eq!(back, event);
    }

    #[test]
    fn ids_are_names_not_paths() {
        assert!(valid_id("run-1_a"));
        assert!(!valid_id("../x"));
        assert!(!valid_id(""));
        assert!(!valid_id(&"a".repeat(65)));
    }
}
