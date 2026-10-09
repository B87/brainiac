//! Save as note (SPEC.md, The explanation): an explanation as Markdown. The
//! summary, the tour as a list, then each note under its file's heading, as
//! a quote of the lines it explains with their numbers, its text, and its
//! sources as `path:lines`; then the concepts, the questions with their
//! answers, and the disagreements.

use crate::models::{CitedQuote, ConceptKind, Explanation};

fn lines(start: u32, end: u32) -> String {
    if start == end {
        start.to_string()
    } else {
        format!("{start}–{end}")
    }
}

fn source(q: &CitedQuote) -> String {
    format!("`{}:{}`", q.path, lines(q.start, q.end))
}

/// Each line of `text` as a Markdown quote.
fn quoted(text: &str) -> String {
    text.lines().map(|l| format!("> {l}\n")).collect::<String>()
}

pub fn render(title: &str, e: &Explanation) -> String {
    let mut md = format!("# {title}\n\n{}\n", e.summary.trim());
    if !e.tour.is_empty() {
        md.push_str("\n## Tour\n\n");
        for (i, stop) in e.tour.iter().enumerate() {
            if stop.role.trim().is_empty() {
                md.push_str(&format!("{}. `{}`\n", i + 1, stop.path));
            } else {
                md.push_str(&format!(
                    "{}. `{}` — {}\n",
                    i + 1,
                    stop.path,
                    stop.role.trim()
                ));
            }
        }
    }
    if !e.notes.is_empty() {
        md.push_str("\n## Notes\n");
        let mut current: Option<&str> = None;
        for note in &e.notes {
            if current != Some(note.path.as_str()) {
                md.push_str(&format!("\n### `{}`\n", note.path));
                current = Some(note.path.as_str());
            }
            md.push_str(&format!("\nLines {}\n\n", lines(note.start, note.end)));
            md.push_str(note.text.trim());
            md.push('\n');
            let sources: Vec<String> = note.sources.iter().map(source).collect();
            if !sources.is_empty() {
                md.push_str(&format!("\nSources: {}\n", sources.join(", ")));
            }
        }
    }
    if !e.concepts.is_empty() {
        md.push_str("\n## Concepts\n\n");
        for c in &e.concepts {
            let kind = match c.kind {
                ConceptKind::Language => "language",
                ConceptKind::Library => "library",
                ConceptKind::Protocol => "protocol",
                ConceptKind::Tool => "tool",
                ConceptKind::Technique => "technique",
                ConceptKind::ProjectPattern => "project pattern",
            };
            md.push_str(&format!(
                "- **{}** ({kind}): {}\n",
                c.name,
                c.explanation.trim()
            ));
        }
    }
    if !e.questions.is_empty() {
        md.push_str("\n## Check yourself\n");
        for q in &e.questions {
            md.push_str(&format!(
                "\n**{}**\n\n{}\n",
                q.question.trim(),
                q.answer.trim()
            ));
        }
    }
    if !e.disagreements.is_empty() {
        md.push_str("\n## Code and docs disagree\n");
        for d in &e.disagreements {
            md.push_str(&format!(
                "\n{}\n\nCode, {}:\n\n{}\nDocs, {}:\n\n{}",
                d.claim.trim(),
                source(&d.code),
                quoted(&d.code.quote),
                source(&d.doc),
                quoted(&d.doc.quote)
            ));
        }
    }
    md
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        Concept, ExplanationChecks, ExplanationNote, ExplanationQuestion, TourStop,
    };

    #[test]
    fn an_explanation_becomes_markdown_in_reading_order() {
        let quote = CitedQuote {
            path: "src/a.rs".into(),
            start: 3,
            end: 4,
            quote: "x".into(),
            moved: false,
        };
        let e = Explanation {
            summary: "Why.".into(),
            sources_read: vec![],
            tour: vec![TourStop {
                path: "src/a.rs".into(),
                role: "the rule".into(),
            }],
            notes: vec![ExplanationNote {
                path: "src/a.rs".into(),
                start: 3,
                end: 4,
                text: "A note.".into(),
                sources: vec![quote],
                sources_dropped: 0,
                lines_hash: String::new(),
            }],
            concepts: vec![Concept {
                name: "traits".into(),
                kind: ConceptKind::Language,
                explanation: "Shared behavior.".into(),
                appears: vec![],
            }],
            questions: vec![ExplanationQuestion {
                question: "Q?".into(),
                answer: "A.".into(),
            }],
            disagreements: vec![],
            checks: ExplanationChecks::default(),
            known_left_out: vec![],
        };
        let md = render("abc — Title", &e);
        assert!(md.starts_with("# abc — Title\n\nWhy.\n"));
        assert!(md.contains("1. `src/a.rs` — the rule"));
        assert!(md.contains("### `src/a.rs`\n\nLines 3–4\n\nA note.\n\nSources: `src/a.rs:3–4`"));
        assert!(md.contains("- **traits** (language): Shared behavior."));
        assert!(md.contains("**Q?**\n\nA."));
    }
}
