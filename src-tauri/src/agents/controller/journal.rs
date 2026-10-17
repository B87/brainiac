//! `TraceJournal`: a run's conversation on the controller's disk, appended
//! in order and replayed after a cursor (docs/architecture.md, Agent runs —
//! v0.5, Journal). What it stores is already filtered: the `Redactor`
//! removes the values the run was given before an event reaches it.

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use super::protocol::{Event, EventBody, MAX_PAGE_BYTES};

/// A run's journal stops growing here; the run is stopped and its work kept.
pub const MAX_JOURNAL_BYTES: u64 = 64 << 20;
/// The longest text one event keeps.
pub const MAX_TEXT_BYTES: usize = 16 << 10;
/// The largest event, as stored; a larger one is replaced by a notice, so
/// one event never fills a page a client cannot read.
pub const MAX_EVENT_BYTES: usize = 1 << 20;
/// The most locations of a tool, and entries of a plan, an event keeps.
const MAX_ITEMS: usize = 100;
/// Space kept for the events that end a run once the journal is full.
const CONTROL_RESERVE: u64 = 64 << 10;

pub struct TraceJournal {
    path: PathBuf,
    /// Where each event's line starts, by sequence - 1, so a replay seeks
    /// instead of reading the whole file.
    offsets: Vec<u64>,
    len: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Append {
    Stored(u64),
    /// Over `MAX_JOURNAL_BYTES`: the event was not stored.
    Full,
}

impl TraceJournal {
    /// Open a journal, reading back what an earlier controller wrote. A last
    /// line cut short by a crash is dropped.
    pub fn open(path: PathBuf) -> std::io::Result<Self> {
        let mut offsets = Vec::new();
        let mut len = 0u64;
        if path.exists() {
            let mut reader = BufReader::new(File::open(&path)?);
            let mut line = Vec::new();
            loop {
                line.clear();
                let n = reader.read_until(b'\n', &mut line)?;
                if n == 0 || line.last() != Some(&b'\n') {
                    break;
                }
                let Ok(event) = serde_json::from_slice::<Event>(&line) else {
                    break;
                };
                if event.seq != offsets.len() as u64 + 1 {
                    break;
                }
                offsets.push(len);
                len += n as u64;
            }
            // Anything after the last whole event is cut off, so the next
            // append starts on a line of its own.
            OpenOptions::new().write(true).open(&path)?.set_len(len)?;
        }
        Ok(Self { path, offsets, len })
    }

    /// The last sequence; 0 before the first event.
    pub fn cursor(&self) -> u64 {
        self.offsets.len() as u64
    }

    /// Append one event and flush it to disk before returning, so a
    /// controller that dies after this still replays it. `control` events
    /// (the run's own state) may use the space kept for them.
    pub fn append(
        &mut self,
        at: String,
        mut body: EventBody,
        control: bool,
    ) -> std::io::Result<Append> {
        match &mut body {
            EventBody::Tool { locations, .. } => locations.truncate(MAX_ITEMS),
            EventBody::Plan { entries, .. } => entries.truncate(MAX_ITEMS),
            _ => {}
        }
        for text in body.texts_mut() {
            clip(text, MAX_TEXT_BYTES);
        }
        let mut event = Event {
            seq: self.cursor() + 1,
            at,
            body,
        };
        let mut line = serde_json::to_vec(&event).map_err(std::io::Error::other)?;
        // Edits are what makes a tool or a permission large; without them a
        // permission can still be answered, where a notice could not.
        if line.len() > MAX_EVENT_BYTES && event.body.drop_diffs() {
            line = serde_json::to_vec(&event).map_err(std::io::Error::other)?;
        }
        if line.len() > MAX_EVENT_BYTES {
            event.body = EventBody::Notice {
                text: "An update from the agent over 1 MB was left out.".into(),
            };
            line = serde_json::to_vec(&event).map_err(std::io::Error::other)?;
        }
        line.push(b'\n');
        let limit = if control {
            MAX_JOURNAL_BYTES + CONTROL_RESERVE
        } else {
            MAX_JOURNAL_BYTES
        };
        if self.len + line.len() as u64 > limit {
            return Ok(Append::Full);
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        file.write_all(&line)?;
        file.sync_data()?;
        self.offsets.push(self.len);
        self.len += line.len() as u64;
        Ok(Append::Stored(event.seq))
    }

    /// Events after `after`, oldest first, at most `MAX_PAGE_BYTES` of them
    /// (always at least one when there is one).
    pub fn after(&self, after: u64) -> std::io::Result<Vec<Event>> {
        let Some(&start) = self.offsets.get(after as usize) else {
            return Ok(Vec::new());
        };
        let mut file = File::open(&self.path)?;
        file.seek(SeekFrom::Start(start))?;
        let mut reader = BufReader::new(file.take(self.len - start));
        let mut events = Vec::new();
        let mut bytes = 0usize;
        let mut line = Vec::new();
        loop {
            line.clear();
            let n = reader.read_until(b'\n', &mut line)?;
            if n == 0 || (!events.is_empty() && bytes + n > MAX_PAGE_BYTES) {
                break;
            }
            bytes += n;
            events.push(serde_json::from_slice(&line).map_err(std::io::Error::other)?);
        }
        Ok(events)
    }
}

/// Cut a text to at most `max` bytes, on a character boundary.
pub fn clip(text: &mut String, max: usize) {
    if text.len() <= max {
        return;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    text.push('…');
}

/// Removes the values a run was given (its token or key) from text before
/// it is stored, shown, or sent to the Mac. Best effort for known values
/// only: an encoded or transformed value, or a secret the agent found
/// elsewhere, is not recognized (SPEC.md, Credentials).
///
/// Matching is on decoded text: the adapter's JSON is parsed before this
/// sees it, so an escaped character cannot hide a value. Tokens and keys are
/// letters, digits, `-`, and `_`, which JSON never escapes anyway.
pub struct Redactor {
    values: Vec<(String, String)>,
}

impl Redactor {
    pub fn new() -> Self {
        Self { values: Vec::new() }
    }

    /// Register a value under the name that replaces it (`[ANTHROPIC_API_KEY]`).
    pub fn register(&mut self, value: &str, name: &str) {
        if !value.is_empty() {
            self.values.push((value.to_string(), format!("[{name}]")));
        }
    }

    /// An extra secret: the whole value, and for one that spans lines (a
    /// service account key) each long line too, since an agent that prints
    /// part of it still shows the part. Exact matching is a mitigation, not
    /// a guarantee against a transformed copy.
    ///
    /// A value that is JSON (a service account key) is also read as JSON:
    /// its strings are what a tool prints decoded (`jq -r .private_key`
    /// prints the PEM with real line breaks, which the file holds as `\n`),
    /// so each long line of each string counts too.
    pub fn register_secret(&mut self, value: &str, name: &str) {
        self.register(value, name);
        if value.contains('\n') {
            self.register_lines(value, name);
        }
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(value) {
            self.register_strings(&json, name);
        }
    }

    fn register_lines(&mut self, text: &str, name: &str) {
        for line in text.lines().map(str::trim) {
            if line.len() >= 24 {
                self.register(line, name);
            }
        }
    }

    fn register_strings(&mut self, value: &serde_json::Value, name: &str) {
        match value {
            serde_json::Value::String(s) => self.register_lines(s, name),
            serde_json::Value::Array(items) => {
                for item in items {
                    self.register_strings(item, name);
                }
            }
            serde_json::Value::Object(fields) => {
                for item in fields.values() {
                    self.register_strings(item, name);
                }
            }
            _ => {}
        }
    }

    pub fn redact(&self, text: &mut String) {
        for (value, name) in &self.values {
            if text.contains(value.as_str()) {
                *text = text.replace(value.as_str(), name);
            }
        }
    }

    /// A result file the agent wrote (a briefing, an explanation): JSON, so
    /// each string in it is redacted as decoded text, where an escaped line
    /// of a key file reads as the line itself. Text that is not JSON is
    /// redacted as it is; its check fails later anyway.
    pub fn redact_json(&self, text: &mut String) {
        if self.values.is_empty() {
            return;
        }
        match serde_json::from_str::<serde_json::Value>(text) {
            Ok(mut value) => {
                let mut changed = false;
                self.redact_value(&mut value, &mut changed);
                if changed {
                    if let Ok(redacted) = serde_json::to_string(&value) {
                        *text = redacted;
                    }
                }
            }
            Err(_) => self.redact(text),
        }
    }

    fn redact_value(&self, value: &mut serde_json::Value, changed: &mut bool) {
        match value {
            serde_json::Value::String(s) => {
                if self.values.iter().any(|(v, _)| s.contains(v.as_str())) {
                    self.redact(s);
                    *changed = true;
                }
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    self.redact_value(item, changed);
                }
            }
            serde_json::Value::Object(fields) => {
                for item in fields.values_mut() {
                    self.redact_value(item, changed);
                }
            }
            _ => {}
        }
    }

    pub fn redact_event(&self, body: &mut EventBody) {
        for text in body.texts_mut() {
            self.redact(text);
        }
    }

    /// How many bytes at the end of `text` could be the start of a value
    /// that the next piece of a streamed reply completes. Those bytes wait
    /// for the next piece, or for the end of the reply.
    pub fn held_suffix(&self, text: &str) -> usize {
        let mut held = 0;
        for (value, _) in &self.values {
            let longest = value.len().saturating_sub(1).min(text.len());
            for len in (held + 1..=longest).rev() {
                let start = text.len() - len;
                if text.is_char_boundary(start) && value.starts_with(&text[start..]) {
                    held = len;
                    break;
                }
            }
        }
        held
    }
}

impl Default for Redactor {
    fn default() -> Self {
        Self::new()
    }
}

// `Drop` runs when the run's redactor is let go: the values are overwritten
// before their memory is freed. Copies made by the allocator earlier cannot be.
impl Drop for Redactor {
    fn drop(&mut self) {
        for (value, _) in &mut self.values {
            let mut bytes = std::mem::take(value).into_bytes();
            bytes.fill(0);
        }
    }
}

/// The journal's file for a run's folder.
pub fn journal_path(run_dir: &Path) -> PathBuf {
    run_dir.join("journal.jsonl")
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_service_account_key_printed_decoded_is_still_taken_out() {
        let pem_line = "MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQC7";
        // In pieces, so secret scanning does not take this placeholder for a key.
        let (begin, end) = (
            concat!("-----BEGIN PRIVATE", " KEY-----"),
            concat!("-----END PRIVATE", " KEY-----"),
        );
        let key = serde_json::json!({
            "type": "service_account",
            "private_key": format!("{begin}\n{pem_line}\n{end}\n"),
        })
        .to_string();
        let mut redactor = Redactor::new();
        redactor.register_secret(&key, "GOOGLE_APPLICATION_CREDENTIALS");
        // What `jq -r .private_key` prints: real line breaks, no escapes.
        let mut printed = format!("{begin}\n{pem_line}\n{end}");
        redactor.redact(&mut printed);
        assert!(!printed.contains(pem_line), "{printed}");
    }

    #[test]
    fn a_result_file_loses_the_secrets_it_quotes_even_escaped_in_json() {
        let key = "{\n  \"type\": \"service_account\",\n  \"private_key\": \"MIIEvQIBADANBgkqhkiG9w0BAQEFAASC\"\n}";
        let mut redactor = Redactor::new();
        redactor.register("sk-ant-api03-secret-value", "ANTHROPIC_API_KEY");
        redactor.register_secret(key, "GOOGLE_APPLICATION_CREDENTIALS");
        // The agent quoted the key file in a finding: inside JSON, its lines
        // are escaped, and its token in the summary is plain.
        let briefing = serde_json::json!({
            "status": "findings",
            "summary": "Used sk-ant-api03-secret-value to look.",
            "findings": [{ "detail": format!("The key was:\n{key}") }],
        })
        .to_string();
        let mut text = briefing.clone();
        redactor.redact_json(&mut text);
        assert!(!text.contains("sk-ant-api03-secret-value"), "{text}");
        assert!(!text.contains("MIIEvQIBADANBgkqhkiG9w0BAQEFAASC"), "{text}");
        assert!(text.contains("[ANTHROPIC_API_KEY]"), "{text}");
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed["status"], "findings");

        // Text with nothing to take out is left as the agent wrote it.
        let mut clean = r#"{ "status": "ok", "summary": "Nothing new." }"#.to_string();
        let before = clean.clone();
        redactor.redact_json(&mut clean);
        assert_eq!(clean, before);

        // Text that is not JSON is redacted as it is.
        let mut broken = "not json: sk-ant-api03-secret-value".to_string();
        redactor.redact_json(&mut broken);
        assert_eq!(broken, "not json: [ANTHROPIC_API_KEY]");
    }

    use super::*;
    use crate::agents::controller::protocol::PlanEntry;

    fn message(text: &str) -> EventBody {
        EventBody::Message {
            turn: 1,
            text: text.into(),
        }
    }

    #[test]
    fn replay_starts_after_the_cursor_and_survives_reopening() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal.jsonl");
        let mut journal = TraceJournal::open(path.clone()).unwrap();
        for text in ["one", "two", "three"] {
            journal.append("t".into(), message(text), false).unwrap();
        }
        drop(journal);
        let journal = TraceJournal::open(path).unwrap();
        assert_eq!(journal.cursor(), 3);
        let events = journal.after(1).unwrap();
        assert_eq!(events.iter().map(|e| e.seq).collect::<Vec<_>>(), vec![2, 3]);
        assert!(journal.after(3).unwrap().is_empty());
    }

    #[test]
    fn a_line_cut_short_by_a_crash_is_dropped_and_appending_continues() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal.jsonl");
        let mut journal = TraceJournal::open(path.clone()).unwrap();
        journal.append("t".into(), message("whole"), false).unwrap();
        drop(journal);
        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(br#"{"seq":2,"at":"t","type":"mess"#)
            .unwrap();
        drop(file);
        let mut journal = TraceJournal::open(path).unwrap();
        assert_eq!(journal.cursor(), 1);
        assert_eq!(
            journal.append("t".into(), message("next"), false).unwrap(),
            Append::Stored(2)
        );
        assert_eq!(journal.after(0).unwrap().len(), 2);
    }

    #[test]
    fn no_event_is_larger_than_a_page_can_carry() {
        let dir = tempfile::tempdir().unwrap();
        let mut journal = TraceJournal::open(dir.path().join("journal.jsonl")).unwrap();
        let body = EventBody::Tool {
            turn: 1,
            tool_id: "t1".into(),
            title: None,
            kind: None,
            status: None,
            locations: vec!["x".repeat(MAX_TEXT_BYTES); 900],
            output: None,
            diffs: vec![],
        };
        journal.append("t".into(), body, false).unwrap();
        let body = EventBody::Plan {
            turn: 1,
            entries: (0..5000)
                .map(|i| PlanEntry {
                    content: format!("step {i}"),
                    status: None,
                })
                .collect(),
        };
        journal.append("t".into(), body, false).unwrap();
        let events = journal.after(0).unwrap();
        assert_eq!(events.len(), 2);
        for event in &events {
            assert!(serde_json::to_vec(event).unwrap().len() <= MAX_EVENT_BYTES);
        }
        assert!(
            matches!(&events[1].body, EventBody::Plan { entries, .. } if entries.len() == MAX_ITEMS)
        );
    }

    #[test]
    fn long_text_is_clipped_on_a_character_boundary() {
        let mut text = "é".repeat(10);
        clip(&mut text, 5);
        assert_eq!(text, "éé…");
    }

    #[test]
    fn known_values_are_replaced_by_their_name() {
        let mut redactor = Redactor::new();
        redactor.register("sk-ant-api03-secret-value", "ANTHROPIC_API_KEY");
        let mut body = EventBody::Tool {
            turn: 1,
            tool_id: "t1".into(),
            title: Some("echo sk-ant-api03-secret-value".into()),
            kind: None,
            status: None,
            locations: vec![],
            output: Some("sk-ant-api03-secret-value\n".into()),
            // An edit that writes the key into a file.
            diffs: vec![super::super::protocol::FileDiff {
                path: "/workspace/.env".into(),
                old_text: None,
                new_text: "KEY=sk-ant-api03-secret-value\n".into(),
                truncated: false,
            }],
        };
        redactor.redact_event(&mut body);
        let text = serde_json::to_string(&body).unwrap();
        assert!(!text.contains("secret-value"));
        assert!(text.contains("[ANTHROPIC_API_KEY]"));
    }

    #[test]
    fn a_streamed_piece_holds_back_only_a_possible_start_of_a_value() {
        let mut redactor = Redactor::new();
        redactor.register("sk-ant-oat01-abcdef", "CLAUDE_CODE_OAUTH_TOKEN");
        assert_eq!(redactor.held_suffix("plain text"), 0);
        assert_eq!(redactor.held_suffix("the token is sk-an"), 5);
        assert_eq!(redactor.held_suffix("ends with s"), 1);
        // The whole value is not held: it is complete and gets replaced.
        assert_eq!(redactor.held_suffix("x sk-ant-oat01-abcdef"), 0);
    }

    #[test]
    fn a_permission_too_large_with_its_edits_keeps_the_request() {
        let dir = tempfile::tempdir().unwrap();
        let mut journal = TraceJournal::open(dir.path().join("journal.jsonl")).unwrap();
        // Control characters are written six times as long in JSON.
        let heavy = "\u{1}".repeat(MAX_TEXT_BYTES);
        let diff = super::super::protocol::FileDiff {
            path: "/workspace/a".into(),
            old_text: Some(heavy.clone()),
            new_text: heavy,
            truncated: false,
        };
        let body = EventBody::Permission(super::super::protocol::PendingPermission {
            permission_id: "p1".into(),
            turn: 1,
            title: "Edit a".into(),
            kind: Some("edit".into()),
            detail: Some("/workspace/a".into()),
            asked_at: "t".into(),
            diffs: vec![diff; 8],
        });
        journal.append("t".into(), body, false).unwrap();
        let events = journal.after(0).unwrap();
        let EventBody::Permission(p) = &events[0].body else {
            panic!("the permission, not a notice: {:?}", events[0].body);
        };
        assert_eq!(p.permission_id, "p1");
        assert!(p.diffs.is_empty());
    }
}
