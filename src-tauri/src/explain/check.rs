//! The checker (SPEC.md, section 14, Checks; docs/architecture.md,
//! Explaining changes — v0.6): Brainiac shows only what it verified in the
//! agent's `.brainiac/explanation.json`.
//!
//! Two kinds of problem are told apart. A **failure** is the agent's to fix:
//! the file does not match the schema, a note is not on the change, or the
//! tour leaves a changed file out; it gets one follow-up turn with the errors.
//! A **drop** is Brainiac's to make: a source whose quote is not in its file,
//! a note left with no source, a concept's place that does not exist, a
//! disagreement without both quotes. Dropped claims are never shown, and
//! each is listed in the explanation's `checks`.
//!
//! After the follow-up turn (`Attempt::Last`), only the schema can fail the
//! file: a note still off the change is dropped, and a changed file the tour
//! still leaves out is added at its end with no role, so one stray note does
//! not cost the whole explanation.
//!
//! Checking is in two steps so that it stays a pure function: `parse` reads
//! the file and says which files it cites, the caller reads those files at
//! the subject's tip (`subject::read_files`), and `check` does the rest.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::store::{concept_key, KnownRef};
use super::subject::{Files, Subject};
use crate::models::{
    CitedQuote, Concept, ConceptKind, ConceptPlace, Disagreement, Explanation, ExplanationChecks,
    ExplanationNote, ExplanationQuestion, LeftOutConcept, TourStop,
};

/// A follow-up prompt lists at most this many errors.
pub const MAX_ERRORS: usize = 20;
const MAX_NOTES: usize = 200;
const MAX_SOURCES: usize = 10;
const MAX_CONCEPTS: usize = 60;
const MAX_KNOWN_USED: usize = 200;
const MAX_PLACES: usize = 20;
const MAX_QUESTIONS: usize = 5;
const MAX_DISAGREEMENTS: usize = 20;
const MAX_SOURCES_READ: usize = 50;

// --- The agent's file, as the prompt asks for it ----------------------------
//
// `#[derive(Deserialize)]` has serde write the code that reads JSON into
// these structs; a missing key or a value of the wrong type is an error that
// names the key, which is what the follow-up prompt passes on. Keys the
// prompt did not ask for are ignored. `#[serde(default)]` makes a key
// optional, empty when left out.

#[derive(Deserialize)]
struct RawExplanation {
    summary: String,
    #[serde(default)]
    sources_read: Vec<String>,
    tour: Vec<TourStop>,
    notes: Vec<RawNote>,
    concepts: Vec<RawConcept>,
    #[serde(default)]
    questions: Vec<ExplanationQuestion>,
    #[serde(default)]
    disagreements: Vec<RawDisagreement>,
    /// Known concepts the change relies on that the agent left out, by name.
    #[serde(default)]
    known_used: Vec<String>,
}

#[derive(Deserialize)]
struct RawNote {
    path: String,
    new_start: u32,
    new_end: u32,
    text: String,
    #[serde(default)]
    sources: Vec<RawQuote>,
}

#[derive(Deserialize)]
struct RawQuote {
    path: String,
    start: u32,
    end: u32,
    quote: String,
}

#[derive(Deserialize)]
struct RawConcept {
    name: String,
    /// Read by `concept_kind`, which also takes `project-pattern`.
    kind: String,
    explanation: String,
    #[serde(default)]
    appears: Vec<ConceptPlace>,
}

#[derive(Deserialize)]
struct RawDisagreement {
    claim: String,
    code: RawQuote,
    doc: RawQuote,
}

/// The agent's file, read but not yet checked.
pub struct Draft(RawExplanation);

/// What passed the checks; what was dropped or moved on the way is in its
/// `checks`.
#[derive(Debug)]
pub struct Checked {
    pub explanation: Explanation,
}

/// Which turn's file is checked: the first can still be sent back with its
/// errors, the last cannot (SPEC.md, Checks).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Attempt {
    First,
    Last,
}

/// Why the file failed, in words for the agent's follow-up turn and for the
/// user. At most `MAX_ERRORS`, the last saying how many more there were.
#[derive(Debug, PartialEq, Eq)]
pub struct Failed {
    pub errors: Vec<String>,
}

impl Failed {
    fn from(mut errors: Vec<String>) -> Self {
        if errors.len() > MAX_ERRORS {
            let more = errors.len() - (MAX_ERRORS - 1);
            errors.truncate(MAX_ERRORS - 1);
            errors.push(format!("…and {more} more errors like these."));
        }
        Failed { errors }
    }
}

/// Read the agent's file against the schema.
pub fn parse(text: &str) -> Result<Draft, Failed> {
    let raw: RawExplanation = serde_json::from_str(text).map_err(|e| Failed {
        errors: vec![format!("The file does not match the schema: {e}.")],
    })?;
    let mut errors = Vec::new();
    if raw.summary.trim().is_empty() {
        errors.push("\"summary\" is empty.".to_string());
    }
    for (i, note) in raw.notes.iter().enumerate() {
        if note.new_start == 0 || note.new_end < note.new_start {
            errors.push(format!(
                "notes[{i}]: new_start and new_end must be line numbers from 1, with new_end not before new_start (got {}–{}).",
                note.new_start, note.new_end
            ));
        }
        if note.text.trim().is_empty() {
            errors.push(format!("notes[{i}]: \"text\" is empty."));
        }
    }
    for (i, concept) in raw.concepts.iter().enumerate() {
        if concept_kind(&concept.kind).is_none() {
            errors.push(format!(
                "concepts[{i}]: \"kind\" must be language, library, system, or project_pattern (got {:?}).",
                concept.kind
            ));
        }
    }
    if errors.is_empty() {
        Ok(Draft(raw))
    } else {
        Err(Failed::from(errors))
    }
}

impl Draft {
    /// The files whose text `check` needs: those the notes are on, and those
    /// every quote and concept place cites.
    pub fn cited_paths(&self) -> BTreeSet<String> {
        let raw = &self.0;
        let notes = raw
            .notes
            .iter()
            .flat_map(|n| std::iter::once(&n.path).chain(n.sources.iter().map(|q| &q.path)));
        let places = raw
            .concepts
            .iter()
            .flat_map(|c| c.appears.iter().map(|p| &p.path));
        let disagreements = raw
            .disagreements
            .iter()
            .flat_map(|d| [&d.code.path, &d.doc.path]);
        notes.chain(places).chain(disagreements).cloned().collect()
    }
}

/// Check a parsed file against the subject and the text of the files it
/// cites (at the subject's tip).
pub fn check(
    draft: Draft,
    subject: &Subject,
    files: &Files,
    known: &[KnownRef],
    attempt: Attempt,
) -> Result<Checked, Failed> {
    let raw = draft.0;
    let texts: BTreeMap<&str, Text> = files
        .iter()
        .map(|(path, text)| (path.as_str(), Text::new(text)))
        .collect();
    // On the last attempt a misplaced note is dropped rather than sent back.
    let mut errors = Vec::new();
    let mut dropped = Vec::new();
    let mut moved = 0;
    let misplaced = |errors: &mut Vec<String>, dropped: &mut Vec<String>, why: String| match attempt
    {
        Attempt::First => errors.push(why),
        Attempt::Last => dropped.push(why),
    };

    // The tour: every changed file, each once; a path outside the change is dropped.
    let mut tour: Vec<TourStop> = Vec::new();
    for stop in raw.tour {
        if subject.file(&stop.path).is_none() {
            dropped.push(format!("tour: {} is not part of the change", stop.path));
        } else if !tour.iter().any(|s| s.path == stop.path) {
            tour.push(stop);
        }
    }
    let missing: Vec<&str> = subject
        .files
        .iter()
        .filter(|f| !tour.iter().any(|s| s.path == f.path))
        .map(|f| f.path.as_str())
        .collect();
    if !missing.is_empty() {
        match attempt {
            Attempt::First => errors.push(format!(
                "\"tour\" leaves out changed files: {}. List every changed file once.",
                missing.join(", ")
            )),
            Attempt::Last => {
                dropped.push(format!(
                    "tour: the agent left out {}, added at the end",
                    missing.join(", ")
                ));
                let missing: Vec<String> = missing.into_iter().map(String::from).collect();
                tour.extend(missing.into_iter().map(|path| TourStop {
                    path,
                    role: String::new(),
                }));
            }
        }
    }

    // Notes: on changed lines of the new side, each with at least one source found.
    let mut notes = Vec::new();
    for (i, note) in raw.notes.into_iter().enumerate() {
        let (start, end) = (note.new_start, note.new_end);
        let at = format!("notes[{i}] ({} {start}–{end})", note.path);
        let Some(file) = subject.file(&note.path) else {
            misplaced(
                &mut errors,
                &mut dropped,
                format!("{at}: {} is not a file this change touches.", note.path),
            );
            continue;
        };
        if file.deleted {
            misplaced(&mut errors, &mut dropped, format!(
                "{at}: {} is deleted by this change, so it has no new side; put the note on the lines that replace it, or leave it out.",
                note.path
            ));
            continue;
        }
        if !file.changed.iter().any(|c| c.touches(start, end)) {
            let changed: Vec<String> = file.changed.iter().take(10).map(|c| c.describe()).collect();
            misplaced(
                &mut errors,
                &mut dropped,
                if changed.is_empty() {
                    format!("{at}: this file has no changed lines to put a note on.")
                } else {
                    format!(
                        "{at}: no line there is changed on the new side. Changed lines in this file: {}.",
                        changed.join(", ")
                    )
                },
            );
            continue;
        }
        let Some(text) = texts.get(note.path.as_str()) else {
            dropped.push(format!(
                "{at}: the file is too large or not text, so it cannot be checked"
            ));
            continue;
        };
        let Some(lines) = text.span(start, end) else {
            misplaced(
                &mut errors,
                &mut dropped,
                format!(
                    "{at}: the file has only {} lines on the new side.",
                    text.line_count()
                ),
            );
            continue;
        };
        let lines_hash = format!("{:x}", Sha256::digest(lines.as_bytes()));
        let mut sources = Vec::new();
        let mut sources_dropped = 0;
        for quote in note.sources {
            match find(&quote, &texts) {
                Some(found) => {
                    moved += u32::from(found.moved);
                    if sources.len() < MAX_SOURCES {
                        sources.push(found);
                    }
                }
                None => {
                    sources_dropped += 1;
                    dropped.push(format!("{at}: a quote not found in {}", quote.path));
                }
            }
        }
        if sources.is_empty() {
            dropped.push(format!("{at}: no source could be verified"));
            continue;
        }
        notes.push(ExplanationNote {
            path: note.path,
            start,
            end,
            text: note.text,
            sources,
            sources_dropped,
            lines_hash,
        });
    }
    if !errors.is_empty() {
        return Err(Failed::from(errors));
    }
    if notes.len() > MAX_NOTES {
        dropped.push(format!(
            "{} notes past the first {MAX_NOTES}",
            notes.len() - MAX_NOTES
        ));
        notes.truncate(MAX_NOTES);
    }

    // Concepts keep only the places that exist.
    let mut concepts = Vec::new();
    for concept in raw.concepts.into_iter().take(MAX_CONCEPTS) {
        if concept.name.trim().is_empty() {
            dropped.push("a concept with no name".to_string());
            continue;
        }
        let appears: Vec<ConceptPlace> = concept
            .appears
            .into_iter()
            .filter(|p| {
                texts
                    .get(p.path.as_str())
                    .is_some_and(|t| p.line >= 1 && p.line <= t.line_count())
            })
            .take(MAX_PLACES)
            .collect();
        let Some(kind) = concept_kind(&concept.kind) else {
            continue;
        };
        concepts.push(Concept {
            name: concept.name.trim().to_string(),
            kind,
            explanation: concept.explanation,
            appears,
        });
    }

    // A disagreement needs both quotes.
    let mut disagreements = Vec::new();
    for (i, d) in raw.disagreements.into_iter().enumerate() {
        match (find(&d.code, &texts), find(&d.doc, &texts)) {
            (Some(code), Some(doc)) => {
                moved += u32::from(code.moved) + u32::from(doc.moved);
                if disagreements.len() < MAX_DISAGREEMENTS {
                    disagreements.push(Disagreement {
                        claim: d.claim,
                        code,
                        doc,
                    });
                }
            }
            _ => dropped.push(format!("disagreements[{i}]: a quote not found")),
        }
    }

    let questions = raw
        .questions
        .into_iter()
        .filter(|q| !q.question.trim().is_empty() && !q.answer.trim().is_empty())
        .take(MAX_QUESTIONS)
        .collect();
    let mut sources_read = raw.sources_read;
    sources_read.truncate(MAX_SOURCES_READ);
    let known_left_out = confirm_known(&raw.known_used, known);

    Ok(Checked {
        explanation: Explanation {
            summary: raw.summary,
            sources_read,
            tour,
            notes,
            concepts,
            questions,
            disagreements,
            checks: ExplanationChecks {
                moved,
                left_out: dropped,
            },
            known_left_out,
        },
    })
}

/// The known concepts the agent says it left out, each confirmed against the
/// ledger by its exact name (folded as the ledger folds it). A name the
/// ledger does not hold is not counted, so the line can never claim more than
/// the reader's own ledger says; one concept is listed once however it was
/// spelled or merged.
fn confirm_known(named: &[String], known: &[KnownRef]) -> Vec<LeftOutConcept> {
    let mut out: Vec<LeftOutConcept> = Vec::new();
    for name in named.iter().take(MAX_KNOWN_USED) {
        let key = concept_key(strip_kind(name));
        if key.is_empty() {
            continue;
        }
        let Some(found) = known.iter().find(|k| k.key == key) else {
            continue;
        };
        if out.iter().all(|o| o.id != found.id) {
            out.push(LeftOutConcept {
                id: found.id.clone(),
                name: found.name.clone(),
                kind: found.kind,
            });
        }
    }
    out
}

/// The prompt lists known concepts as "name (kind)"; an agent that copies
/// that form gets its kind word dropped, and nothing else.
fn strip_kind(name: &str) -> &str {
    let name = name.trim();
    if let Some(open) = name.rfind('(') {
        let inner = name[open + 1..].trim_end_matches(')').trim().to_lowercase();
        if name.ends_with(')')
            && matches!(
                inner.as_str(),
                "language" | "library" | "system" | "project pattern"
            )
        {
            return name[..open].trim_end();
        }
    }
    name
}

/// A concept's kind, with `-` read as `_` (the spike's prompt asked for
/// `project-pattern`).
fn concept_kind(kind: &str) -> Option<ConceptKind> {
    match kind.trim().replace('-', "_").as_str() {
        "language" => Some(ConceptKind::Language),
        "library" => Some(ConceptKind::Library),
        "system" => Some(ConceptKind::System),
        "project_pattern" => Some(ConceptKind::ProjectPattern),
        _ => None,
    }
}

/// Find a quote verbatim (line endings aside, and the spaces around it): at
/// its cited lines, else at its first place in the same file.
fn find(quote: &RawQuote, texts: &BTreeMap<&str, Text>) -> Option<CitedQuote> {
    let text = texts.get(quote.path.as_str())?;
    let wanted = quote.quote.replace("\r\n", "\n");
    let wanted = wanted.trim();
    if wanted.is_empty() {
        return None;
    }
    let cited = text
        .span(quote.start, quote.end)
        .is_some_and(|lines| lines.contains(wanted));
    let (start, end) = if cited {
        (quote.start, quote.end)
    } else {
        text.locate(wanted)?
    };
    Some(CitedQuote {
        path: quote.path.clone(),
        start,
        end,
        quote: wanted.to_string(),
        moved: !cited,
    })
}

/// A file's text with `\r\n` read as `\n`, and where each line starts.
struct Text {
    text: String,
    /// The byte offset of each line's start; line n starts at `starts[n - 1]`.
    starts: Vec<usize>,
}

impl Text {
    fn new(raw: &str) -> Self {
        let text = raw.replace("\r\n", "\n");
        let mut starts = Vec::new();
        if !text.is_empty() {
            starts.push(0);
            starts.extend(
                text.match_indices('\n')
                    .map(|(at, _)| at + 1)
                    .filter(|&at| at < text.len()),
            );
        }
        Text { text, starts }
    }

    fn line_count(&self) -> u32 {
        u32::try_from(self.starts.len()).unwrap_or(u32::MAX)
    }

    /// Lines `start..=end` without the last line break, or `None` when they
    /// are not all in the file.
    fn span(&self, start: u32, end: u32) -> Option<&str> {
        if start == 0 || end < start || end > self.line_count() {
            return None;
        }
        let from = self.starts[start as usize - 1];
        let to = match self.starts.get(end as usize) {
            Some(&next) => next - 1,
            None => self.text.len() - usize::from(self.text.ends_with('\n')),
        };
        Some(&self.text[from..to])
    }

    /// The lines a quote's first place covers.
    fn locate(&self, quote: &str) -> Option<(u32, u32)> {
        let at = self.text.find(quote)?;
        // `partition_point` counts the line starts at or before `at`: the line number.
        let start = u32::try_from(self.starts.partition_point(|&s| s <= at)).ok()?;
        let lines = u32::try_from(quote.matches('\n').count()).ok()?;
        Some((start, start + lines))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};

    use super::*;
    use crate::explain::subject::{Changed, ChangedFile};

    const RULES: &str =
        "fn main() {\n    let limit = 3;\n    if limit > 2 {\n        run();\n    }\n}\n";
    const DOCS: &str = "# Rules\r\n\r\nThe limit is three.\r\nIt is checked on start.\r\n";

    fn subject() -> Subject {
        let file = |path: &str, changed: Vec<Changed>, deleted: bool| ChangedFile {
            path: path.to_string(),
            old_path: None,
            deleted,
            binary: false,
            changed,
        };
        Subject {
            files: vec![
                file("src/main.rs", vec![Changed { start: 2, count: 2 }], false),
                file("docs/rules.md", vec![Changed { start: 3, count: 1 }], false),
                file("old.txt", vec![Changed { start: 0, count: 0 }], true),
            ],
        }
    }

    fn files() -> Files {
        Files::from([
            ("src/main.rs".to_string(), RULES.to_string()),
            ("docs/rules.md".to_string(), DOCS.to_string()),
        ])
    }

    fn quote(path: &str, start: u32, end: u32, quote: &str) -> Value {
        json!({ "path": path, "start": start, "end": end, "quote": quote })
    }

    fn file() -> Value {
        json!({
            "summary": "The limit moves from the docs into the code.",
            "sources_read": ["src/main.rs", "docs/rules.md"],
            "tour": [
                { "path": "docs/rules.md", "role": "the rule" },
                { "path": "src/main.rs", "role": "the code" },
                { "path": "old.txt", "role": "removed" },
            ],
            "notes": [{
                "path": "src/main.rs", "new_start": 2, "new_end": 3,
                "text": "The limit the docs promise.",
                "sources": [
                    quote("src/main.rs", 2, 2, "let limit = 3;"),
                    quote("docs/rules.md", 3, 3, "The limit is three."),
                ],
            }],
            "concepts": [{
                "name": " let binding ", "kind": "language", "explanation": "Names a value.",
                "appears": [{ "path": "src/main.rs", "line": 2 }, { "path": "src/main.rs", "line": 99 }],
            }],
            "questions": [{ "question": "What is the limit?", "answer": "Three." }],
            "disagreements": [],
        })
    }

    fn run(value: Value) -> Result<Checked, Failed> {
        let draft = parse(&value.to_string())?;
        check(draft, &subject(), &files(), &[], Attempt::First)
    }

    fn run_last(value: Value) -> Result<Checked, Failed> {
        let draft = parse(&value.to_string())?;
        check(draft, &subject(), &files(), &[], Attempt::Last)
    }

    #[test]
    fn a_file_whose_notes_and_quotes_check_out_is_kept() {
        let checked = run(file()).unwrap();
        let e = &checked.explanation;
        assert_eq!(e.tour.len(), 3);
        assert_eq!(e.notes.len(), 1);
        let note = &e.notes[0];
        assert_eq!((note.start, note.end), (2, 3));
        assert_eq!(note.sources.len(), 2);
        assert!(note.sources.iter().all(|s| !s.moved));
        let lines = "    let limit = 3;\n    if limit > 2 {";
        assert_eq!(
            note.lines_hash,
            format!("{:x}", Sha256::digest(lines.as_bytes()))
        );
        assert_eq!(e.concepts[0].name, "let binding");
        assert_eq!(
            e.concepts[0].appears,
            vec![ConceptPlace {
                path: "src/main.rs".into(),
                line: 2
            }]
        );
        assert_eq!(e.questions.len(), 1);
        assert_eq!(checked.explanation.checks.moved, 0);
        assert!(
            checked.explanation.checks.left_out.is_empty(),
            "{:?}",
            checked.explanation.checks.left_out
        );
    }

    #[test]
    fn a_quote_at_the_wrong_lines_is_moved_and_one_not_found_is_dropped() {
        let mut value = file();
        value["notes"][0]["sources"] = json!([
            quote("src/main.rs", 5, 6, "if limit > 2 {\n        run();"),
            quote("docs/rules.md", 1, 1, "The limit is four."),
            quote("missing.md", 1, 1, "anything"),
        ]);
        let checked = run(value).unwrap();
        let sources = &checked.explanation.notes[0].sources;
        assert_eq!(sources.len(), 1);
        assert_eq!(
            (sources[0].start, sources[0].end, sources[0].moved),
            (3, 4, true)
        );
        assert_eq!(checked.explanation.checks.moved, 1);
        assert_eq!(checked.explanation.checks.left_out.len(), 2);
        assert_eq!(checked.explanation.notes[0].sources_dropped, 2);
    }

    #[test]
    fn a_note_with_no_source_found_is_dropped() {
        let mut value = file();
        value["notes"][0]["sources"] = json!([quote("src/main.rs", 1, 1, "not in the file")]);
        let checked = run(value).unwrap();
        assert!(checked.explanation.notes.is_empty());
        assert!(checked
            .explanation
            .checks
            .left_out
            .iter()
            .any(|d| d.contains("no source")));
    }

    #[test]
    fn a_note_off_the_change_fails_with_the_changed_lines() {
        let mut value = file();
        value["notes"][0]["new_start"] = json!(5);
        value["notes"][0]["new_end"] = json!(6);
        let failed = run(value).unwrap_err();
        assert_eq!(
            failed.errors,
            vec!["notes[0] (src/main.rs 5–6): no line there is changed on the new side. Changed lines in this file: 2–3."]
        );
    }

    #[test]
    fn notes_on_other_files_deleted_files_or_past_the_end_fail() {
        let mut value = file();
        let note = value["notes"][0].clone();
        let with = |path: &str, start: u32, end: u32| {
            let mut n = note.clone();
            n["path"] = json!(path);
            n["new_start"] = json!(start);
            n["new_end"] = json!(end);
            n
        };
        value["notes"] = json!([
            with("README.md", 1, 1),
            with("old.txt", 1, 1),
            with("src/main.rs", 3, 40),
        ]);
        let errors = run(value).unwrap_err().errors;
        assert_eq!(errors.len(), 3, "{errors:?}");
        assert!(errors[0].contains("not a file this change touches"));
        assert!(errors[1].contains("deleted by this change"));
        assert!(errors[2].contains("only 6 lines"));
    }

    #[test]
    fn on_the_last_attempt_misplaced_notes_and_missing_tour_stops_are_dropped() {
        let mut value = file();
        let mut off = value["notes"][0].clone();
        off["new_start"] = json!(5);
        off["new_end"] = json!(6);
        value["notes"].as_array_mut().unwrap().push(off);
        value["tour"] = json!([{ "path": "src/main.rs", "role": "the code" }]);
        assert!(run(value.clone()).is_err());

        let checked = run_last(value).unwrap();
        let e = &checked.explanation;
        assert_eq!(e.notes.len(), 1);
        let tour: Vec<(&str, &str)> = e
            .tour
            .iter()
            .map(|s| (s.path.as_str(), s.role.as_str()))
            .collect();
        assert_eq!(
            tour,
            [
                ("src/main.rs", "the code"),
                ("docs/rules.md", ""),
                ("old.txt", "")
            ]
        );
        assert_eq!(e.checks.left_out.len(), 2, "{:?}", e.checks.left_out);
        assert!(e.checks.left_out[0].contains("left out docs/rules.md, old.txt"));
        assert!(e.checks.left_out[1].contains("no line there is changed"));
    }

    #[test]
    fn the_last_attempt_still_fails_a_file_off_the_schema() {
        let mut value = file();
        value.as_object_mut().unwrap().remove("notes");
        assert!(run_last(value).is_err());
    }

    #[test]
    fn the_tour_must_list_every_changed_file_and_only_those() {
        let mut value = file();
        value["tour"] = json!([
            { "path": "src/main.rs", "role": "the code" },
            { "path": "src/main.rs", "role": "again" },
            { "path": "elsewhere.rs", "role": "not changed" },
        ]);
        let errors = run(value.clone()).unwrap_err().errors;
        assert_eq!(
            errors,
            vec!["\"tour\" leaves out changed files: docs/rules.md, old.txt. List every changed file once."]
        );
        value["tour"].as_array_mut().unwrap().extend([
            json!({ "path": "docs/rules.md", "role": "" }),
            json!({ "path": "old.txt", "role": "" }),
        ]);
        let checked = run(value).unwrap();
        let tour: Vec<&str> = checked
            .explanation
            .tour
            .iter()
            .map(|s| s.path.as_str())
            .collect();
        assert_eq!(tour, ["src/main.rs", "docs/rules.md", "old.txt"]);
        assert_eq!(
            checked.explanation.checks.left_out,
            ["tour: elsewhere.rs is not part of the change"]
        );
    }

    #[test]
    fn a_file_off_the_schema_fails_naming_what_is_wrong() {
        let mut value = file();
        value.as_object_mut().unwrap().remove("tour");
        let errors = run(value).unwrap_err().errors;
        assert!(errors[0].contains("missing field `tour`"), "{errors:?}");

        let mut value = file();
        value["concepts"][0]["kind"] = json!("folklore");
        assert!(run(value).unwrap_err().errors[0].contains("folklore"));

        let mut value = file();
        value["concepts"][0]["kind"] = json!("project-pattern");
        value.as_object_mut().unwrap().remove("questions");
        value["extra"] = json!(true);
        let checked = run(value).unwrap();
        assert_eq!(
            checked.explanation.concepts[0].kind,
            ConceptKind::ProjectPattern
        );
        assert!(checked.explanation.questions.is_empty());

        assert!(parse("not json").is_err());
        let mut value = file();
        value["summary"] = json!("  ");
        value["notes"][0]["new_end"] = json!(1);
        assert_eq!(run(value).unwrap_err().errors.len(), 2);
    }

    #[test]
    fn a_disagreement_needs_both_quotes() {
        let mut value = file();
        value["disagreements"] = json!([
            {
                "claim": "The docs say three.",
                "code": quote("src/main.rs", 2, 2, "let limit = 3;"),
                "doc": quote("docs/rules.md", 9, 9, "It is checked on start."),
            },
            {
                "claim": "Made up.",
                "code": quote("src/main.rs", 2, 2, "let limit = 3;"),
                "doc": quote("docs/rules.md", 3, 3, "The limit is five."),
            },
        ]);
        let checked = run(value).unwrap();
        let kept = &checked.explanation.disagreements;
        assert_eq!(kept.len(), 1);
        assert_eq!((kept[0].doc.start, kept[0].doc.moved), (4, true));
        assert_eq!(
            checked.explanation.checks.left_out,
            ["disagreements[1]: a quote not found"]
        );
    }

    #[test]
    fn errors_past_the_limit_are_counted() {
        let mut value = file();
        let note = value["notes"][0].clone();
        value["notes"] = Value::Array(
            (0..30)
                .map(|_| {
                    let mut n = note.clone();
                    n["path"] = json!("nowhere.rs");
                    n
                })
                .collect(),
        );
        let errors = run(value).unwrap_err().errors;
        assert_eq!(errors.len(), MAX_ERRORS);
        assert_eq!(errors.last().unwrap(), "…and 11 more errors like these.");
    }

    #[test]
    fn the_cited_paths_are_those_the_checks_read() {
        let mut value = file();
        value["disagreements"] = json!([{
            "claim": "c",
            "code": quote("a.rs", 1, 1, "x"),
            "doc": quote("b.md", 1, 1, "y"),
        }]);
        let draft = parse(&value.to_string()).unwrap();
        let paths: Vec<String> = draft.cited_paths().into_iter().collect();
        assert_eq!(paths, ["a.rs", "b.md", "docs/rules.md", "src/main.rs"]);
    }

    #[test]
    fn spans_and_places_read_lines_from_one() {
        let text = Text::new("a\r\nb\nc");
        assert_eq!(text.line_count(), 3);
        assert_eq!(text.span(2, 3), Some("b\nc"));
        assert_eq!(text.span(1, 1), Some("a"));
        assert_eq!(text.span(3, 4), None);
        assert_eq!(text.locate("b\nc"), Some((2, 3)));
        assert_eq!(Text::new("").line_count(), 0);
        assert_eq!(Text::new("x\n").span(1, 1), Some("x"));
    }
    fn known(id: &str, name: &str, kind: ConceptKind) -> KnownRef {
        KnownRef {
            id: id.to_string(),
            name: name.to_string(),
            kind,
            key: concept_key(name),
        }
    }

    #[test]
    fn a_known_concept_the_agent_left_out_is_confirmed_against_the_ledger() {
        let ledger = [
            known("1", "go:embed", ConceptKind::Language),
            known("2", "DKIM", ConceptKind::System),
        ];
        let named = [
            "GO:EMBED".to_string(),      // case does not matter
            "DKIM (system)".to_string(), // the prompt's own form
            "go embed".to_string(),      // the same folded name: listed once
            "SPF".to_string(),           // not in the ledger: not counted
            "  ".to_string(),
        ];
        let left = confirm_known(&named, &ledger);
        assert_eq!(
            left.iter().map(|l| l.id.as_str()).collect::<Vec<_>>(),
            ["1", "2"]
        );
        assert_eq!(left[0].name, "go:embed");
    }

    #[test]
    fn a_name_that_only_looks_like_a_known_one_is_not_counted() {
        let ledger = [known("1", "go:embed", ConceptKind::Language)];
        // Exact name only: no prefix, substring, or related name.
        let named = ["go".to_string(), "go:embed directive".to_string()];
        assert!(confirm_known(&named, &ledger).is_empty());
    }

    #[test]
    fn the_check_reports_what_the_ledger_confirms() {
        let mut value = file();
        value["known_used"] = json!(["let binding", "nothing like it"]);
        let draft = parse(&value.to_string()).unwrap();
        let ledger = [known("7", "Let Binding", ConceptKind::Language)];
        let checked = check(draft, &subject(), &files(), &ledger, Attempt::First).unwrap();
        assert_eq!(checked.explanation.known_left_out.len(), 1);
        assert_eq!(checked.explanation.known_left_out[0].id, "7");
    }

    #[test]
    fn a_file_without_known_used_still_passes() {
        let checked = run(file()).unwrap();
        assert!(checked.explanation.known_left_out.is_empty());
    }
}
