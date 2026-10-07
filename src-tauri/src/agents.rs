//! Agent runs, v0.5 (SPEC.md, section 13; docs/architecture.md, Agent runs —
//! v0.5): a coding agent in a container on this Mac's Docker engine, started
//! from a repository and reviewed before its work leaves Brainiac.
//!
//! - **`AgentSettingsService`** (`settings.rs`) is Settings → Agents: the
//!   engine, how runs are paid for (a Claude plan token or an API key, read
//!   through the credentials layer under the owner `agent:<profile id>`),
//!   the agreement to send code to the provider, new runs' defaults, and the
//!   image.
//! - **`engine`** finds this Mac's Docker engines by their sockets and asks
//!   each what it is.
//! - **`image`** holds the readable Dockerfile and entrypoint Brainiac builds,
//!   and builds them on the chosen engine.
//! - **`RunArtifacts`** (`repository.rs`) reads a repository to start a run:
//!   it refuses what a run cannot copy completely, resolves the start once,
//!   and copies that commit and its history into Brainiac's own bare
//!   repository and a bundle, without writing to the user's repository.
//! - **`RunController`** (`controller.rs`) is `brainiac runner`, the process
//!   that owns each run's container, agent session, deadline, and journal,
//!   and outlives the app; **`RunGuard`** (`controller/guard.rs`) stops its
//!   containers if it dies.
//! - **`RunRuntime`** (`runtime.rs`) is how the app reaches the controller,
//!   starting it when none answers.
//! - **`AgentRunService`** (`runs.rs`) is runs as the app sees them: the
//!   start, following the controller and mirroring its journal, the user's
//!   actions, collection and review, Delete and retention, and Settings'
//!   Test.
//! - **`AgentHostService`** (`hosts.rs`) approves remote hosts and installs,
//!   upgrades, and removes their run controller; **`HostJobService`**
//!   (`host_jobs.rs`) runs those steps in the background as jobs the window
//!   follows.

pub mod controller;
pub mod engine;
pub mod host_jobs;
pub mod hosts;
pub mod image;
pub mod repository;
pub mod runner_binary;
pub mod runs;
pub mod runtime;
pub mod settings;
pub mod ssh;

pub use repository::{ExportedStart, RunArtifacts};
pub use runs::{AgentRunService, RepositoryLookup, RunEmitter};
pub use runtime::{RunRuntime, StartError};
pub use settings::{credential_owner, normalize_credential, AgentSettingsService, PLAN_OFFERED};
