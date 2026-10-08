//! Explaining changes, v0.6 (SPEC.md, section 14; docs/architecture.md,
//! Explaining changes — v0.6): an agent run of its own kind writes
//! `.brainiac/explanation.json`, and Brainiac checks it before it is shown.
//!
//! - **`subject`** reads what an explanation is about from Brainiac's own
//!   bare repository: the changed files with their changed lines on the new
//!   side, and the text of the files the agent cites.
//! - **`check`** is the checker, a pure function of the agent's file and the
//!   subject: the schema, notes on changed lines, quotes found verbatim (and
//!   moved to where they are when the agent cited the wrong lines).
//! - **`prompt`** writes Brainiac's prompt and the follow-up turn's.
//! - **`store`** keeps explanations in `history.db`, and the answers, known
//!   concepts, and Settings → Explanations in `brainiac.db`.
//! - **`service`** is `ExplanationService`: the dialog, the explain run from
//!   start to checked file, and everything the panel and Settings change.

pub mod check;
pub mod markdown;
pub mod prompt;
pub mod service;
pub mod store;
pub mod subject;
