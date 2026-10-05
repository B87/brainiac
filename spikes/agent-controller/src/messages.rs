//! One JSON object per line. The controller never pushes data: a client
//! that has stopped reading (a sleeping Mac) cannot stall the agent stream.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Event {
    pub seq: u64,
    pub kind: String,
    pub text: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    Status,
    /// `deadline_secs` is the run limit, counted from when the controller
    /// accepts the start. It is not extended when the client disconnects.
    Start {
        deadline_secs: u64,
    },
    /// Start the pinned Claude adapter. The token is delivered once and is
    /// not stored in the journal or the ledger. `allow_tools` selects an
    /// allow-once option when the agent asks; otherwise the ask is cancelled.
    /// `hold_permissions` leaves the ask unanswered until `Permit`, including
    /// after this client disconnects, and it takes precedence over `allow_tools`.
    StartClaude {
        token: String,
        allow_tools: bool,
        #[serde(default)]
        hold_permissions: bool,
    },
    /// Events with `seq` greater than `after`, oldest first.
    Replay {
        after: u64,
    },
    Follow {
        id: String,
        text: String,
    },
    Resolve {
        id: String,
    },
    /// Answer the one pending permission. `choice` is `allow` or `cancel`.
    /// A repeated id returns the recorded outcome and is not sent again.
    Permit {
        id: String,
        choice: String,
    },
    /// Read the ended run's workspace with the read-only collector. It can be
    /// repeated; the workload is not started again.
    Collect,
    /// Remove the ended run's container and workspace volume. This is the
    /// only request that deletes a run's work.
    Discard,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Response {
    Status {
        run_id: Option<String>,
        running: bool,
        cursor: u64,
        tty: Option<bool>,
        log_driver: Option<String>,
        interrupted: bool,
        /// `idle` or `permission` while the run is live.
        waiting: Option<String>,
        permission_id: Option<String>,
        expired: bool,
        /// The ended run's stopped container and workspace are still kept.
        #[serde(default)]
        kept: bool,
    },
    Started {
        run_id: String,
        cursor: u64,
        tty: bool,
        log_driver: String,
    },
    Replay {
        run_id: Option<String>,
        cursor: u64,
        truncated: bool,
        events: Vec<Event>,
    },
    Ack {
        id: String,
        outcome: String,
    },
    /// One JSON object per workspace entry, from the collector.
    Collected {
        run_id: String,
        entries: Vec<String>,
    },
    Err {
        message: String,
    },
}
