//! Command ids for follow-up prompts. The intent is stored before the text
//! is written to the agent, and the outcome is stored before the client is
//! told. A second delivery of the same id does not write to the agent again.
//! An intent with no outcome is uncertain: the turn is not repeated.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Begin {
    /// This id was not seen before. The caller may write it to the agent once.
    Fresh,
    /// Already written and acknowledged. Do not write it again.
    Delivered,
    /// Written, or about to be written, but the outcome was never stored.
    Uncertain,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct File {
    commands: BTreeMap<String, Command>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Command {
    outcome: Option<String>,
}

pub struct Ledger {
    path: PathBuf,
    file: File,
}

impl Ledger {
    pub fn open(path: PathBuf) -> std::io::Result<Self> {
        let file = if path.exists() {
            let text = fs::read_to_string(&path)?;
            serde_json::from_str(&text).unwrap_or_default()
        } else {
            File::default()
        };
        Ok(Self { path, file })
    }

    pub fn begin(&mut self, id: &str) -> std::io::Result<Begin> {
        if let Some(command) = self.file.commands.get(id) {
            return Ok(match command.outcome.as_deref() {
                Some("delivered") => Begin::Delivered,
                Some(_) | None => Begin::Uncertain,
            });
        }
        self.file
            .commands
            .insert(id.to_string(), Command { outcome: None });
        self.store()?;
        Ok(Begin::Fresh)
    }

    pub fn commit_delivered(&mut self, id: &str) -> std::io::Result<()> {
        let Some(command) = self.file.commands.get_mut(id) else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "command id was not begun",
            ));
        };
        command.outcome = Some("delivered".to_string());
        self.store()
    }

    /// A new session does not inherit command ids from the previous one.
    pub fn clear(&mut self) -> std::io::Result<()> {
        self.file.commands.clear();
        self.store()
    }

    pub fn outcome(&self, id: &str) -> Option<&str> {
        self.file
            .commands
            .get(id)
            .map(|command| command.outcome.as_deref().unwrap_or("uncertain"))
    }

    fn store(&self) -> std::io::Result<()> {
        let text = serde_json::to_string(&self.file).expect("ledger serializes");
        let tmp = self.path.with_extension("tmp");
        fs::write(&tmp, text)?;
        fs::rename(tmp, &self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_committed_id_is_not_fresh_again() {
        let path =
            std::env::temp_dir().join(format!("brainiac-spike-ledger-{}", std::process::id()));
        let _ = fs::remove_file(&path);
        let mut ledger = Ledger::open(path.clone()).unwrap();
        assert_eq!(ledger.begin("c1").unwrap(), Begin::Fresh);
        ledger.commit_delivered("c1").unwrap();
        drop(ledger);

        let mut ledger = Ledger::open(path.clone()).unwrap();
        assert_eq!(ledger.begin("c1").unwrap(), Begin::Delivered);
        assert_eq!(ledger.outcome("c1"), Some("delivered"));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn an_intent_without_an_outcome_is_uncertain() {
        let path = std::env::temp_dir().join(format!(
            "brainiac-spike-ledger-uncertain-{}",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        let mut ledger = Ledger::open(path.clone()).unwrap();
        assert_eq!(ledger.begin("c2").unwrap(), Begin::Fresh);
        drop(ledger);

        let mut ledger = Ledger::open(path.clone()).unwrap();
        assert_eq!(ledger.begin("c2").unwrap(), Begin::Uncertain);
        let _ = fs::remove_file(path);
    }
}
