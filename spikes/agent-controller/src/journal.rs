//! Append-only trace on the controller's disk. Sequence numbers are the
//! replay cursor: a reconnect asks for `seq` greater than the last one it
//! stored, so the same beat is not shown twice.

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use crate::messages::Event;

pub struct Journal {
    path: PathBuf,
    next: u64,
}

impl Journal {
    pub fn open(path: PathBuf) -> std::io::Result<Self> {
        let next = next_seq(&path)?;
        Ok(Self { path, next })
    }

    pub fn cursor(&self) -> u64 {
        self.next.saturating_sub(1)
    }

    pub fn append(&mut self, kind: &str, text: &str) -> std::io::Result<Event> {
        let text = clip(text);
        let event = Event {
            seq: self.next,
            kind: kind.to_string(),
            text,
        };
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(
            file,
            "{}",
            serde_json::to_string(&event).expect("event serializes")
        )?;
        // A crash after this returns should still leave the line for replay.
        file.sync_data()?;
        self.next += 1;
        Ok(event)
    }

    /// At most `limit` events after `after`. `truncated` means the caller
    /// should ask again with the new cursor.
    pub fn replay_after(&self, after: u64, limit: usize) -> std::io::Result<(Vec<Event>, bool)> {
        if !self.path.exists() {
            return Ok((Vec::new(), false));
        }
        let file = File::open(&self.path)?;
        let mut out = Vec::new();
        let mut truncated = false;
        for line in BufReader::new(file).lines() {
            let line = line?;
            let Ok(event) = serde_json::from_str::<Event>(&line) else {
                continue;
            };
            if event.seq <= after {
                continue;
            }
            if out.len() == limit {
                truncated = true;
                break;
            }
            out.push(event);
        }
        Ok((out, truncated))
    }
}

fn next_seq(path: &Path) -> std::io::Result<u64> {
    if !path.exists() {
        return Ok(1);
    }
    let file = File::open(path)?;
    let mut max = 0u64;
    for line in BufReader::new(file).lines() {
        let line = line?;
        if let Ok(event) = serde_json::from_str::<Event>(&line) {
            max = max.max(event.seq);
        }
    }
    Ok(max + 1)
}

fn clip(text: &str) -> String {
    const MAX: usize = 4 * 1024;
    if text.len() <= MAX {
        return text.to_string();
    }
    let mut end = MAX;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "brainiac-spike-journal-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn replay_is_ordered_and_does_not_repeat_the_cursor() {
        let dir = scratch("order");
        let path = dir.join("journal.jsonl");
        let mut journal = Journal::open(path.clone()).unwrap();
        journal.append("beat", "one").unwrap();
        journal.append("beat", "two").unwrap();
        journal.append("follow", "marker").unwrap();
        drop(journal);

        let journal = Journal::open(path).unwrap();
        assert_eq!(journal.cursor(), 3);
        let (events, truncated) = journal.replay_after(1, 10).unwrap();
        assert!(!truncated);
        assert_eq!(
            events
                .iter()
                .map(|event| event.text.as_str())
                .collect::<Vec<_>>(),
            ["two", "marker"]
        );
        let (none, _) = journal.replay_after(3, 10).unwrap();
        assert!(none.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn replay_reports_a_truncated_page() {
        let dir = scratch("page");
        let mut journal = Journal::open(dir.join("journal.jsonl")).unwrap();
        journal.append("beat", "a").unwrap();
        journal.append("beat", "b").unwrap();
        journal.append("beat", "c").unwrap();
        let (events, truncated) = journal.replay_after(0, 2).unwrap();
        assert!(truncated);
        assert_eq!(events.len(), 2);
        assert_eq!(events[1].text, "b");
        let _ = std::fs::remove_dir_all(dir);
    }
}
