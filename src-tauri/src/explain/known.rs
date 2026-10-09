//! The reader's known concepts as a file an explain run reads
//! (docs/design/code-explanations.md, How the ledger reaches the run): one
//! concept per line, tab-separated: its folded name, its name, its kind, and
//! the words an earlier explanation used for it. The container gets it as
//! `/opt/brainiac/input/known-concepts.tsv` and the `known` script in the
//! image reads it, so the prompt names the file and a count, never the list.

use super::store::KnownRef;
use crate::models::ConceptKind;

/// Where the container keeps the file (`agents/image/known.mjs` reads it).
pub const PATH: &str = "/opt/brainiac/input/known-concepts.tsv";

/// At most this many concepts are written, the newest first.
const MAX_LINES: usize = 5000;
/// A description is cut to this many characters.
const MAX_DESCRIPTION: usize = 300;

/// The file's text and how many concepts it holds.
#[derive(Debug, PartialEq, Eq)]
pub struct KnownFile {
    pub text: String,
    pub count: usize,
}

/// The kind as the prompt and the file say it.
fn kind_word(kind: ConceptKind) -> &'static str {
    match kind {
        ConceptKind::Language => "language",
        ConceptKind::Library => "library",
        ConceptKind::System => "system",
        ConceptKind::ProjectPattern => "project pattern",
    }
}

/// One line of text: tabs and line breaks are spaces, so a name or
/// description cannot start another column or line.
fn one_line(text: &str, limit: usize) -> String {
    let flat: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    flat.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(limit)
        .collect()
}

/// The file for the known concepts of one repository, `None` when there are
/// none. A key is written once (an alias has its own key and its target's
/// name, kind, and words).
pub fn file(refs: &[KnownRef]) -> Option<KnownFile> {
    let mut seen = std::collections::BTreeSet::new();
    let mut text = String::new();
    let mut count = 0;
    for r in refs {
        if count == MAX_LINES {
            break;
        }
        if r.key.is_empty() || !seen.insert(r.key.as_str()) {
            continue;
        }
        text.push_str(&format!(
            "{}\t{}\t{}\t{}\n",
            r.key,
            one_line(&r.name, 200),
            kind_word(r.kind),
            one_line(&r.description, MAX_DESCRIPTION)
        ));
        count += 1;
    }
    (count > 0).then_some(KnownFile { text, count })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known(key: &str, name: &str, description: &str) -> KnownRef {
        KnownRef {
            id: key.to_string(),
            name: name.to_string(),
            kind: ConceptKind::Language,
            key: key.to_string(),
            description: description.to_string(),
        }
    }

    #[test]
    fn the_file_has_one_line_a_concept_in_four_columns() {
        let f = file(&[
            known("go embed", "go:embed", "Puts a file\tinto the\nbinary."),
            known("go embed", "again", "a second row with the same key"),
            known("dkim", "DKIM", ""),
        ])
        .unwrap();
        assert_eq!(f.count, 2);
        assert_eq!(
            f.text,
            "go embed\tgo:embed\tlanguage\tPuts a file into the binary.\ndkim\tDKIM\tlanguage\t\n"
        );
    }

    #[test]
    fn no_known_concepts_make_no_file() {
        assert_eq!(file(&[]), None);
    }

    #[test]
    fn the_file_is_capped_newest_first() {
        let refs: Vec<KnownRef> = (0..MAX_LINES + 10)
            .map(|i| known(&format!("c{i}"), &format!("C{i}"), ""))
            .collect();
        let f = file(&refs).unwrap();
        assert_eq!(f.count, MAX_LINES);
        assert!(f.text.starts_with("c0\t"));
        assert!(!f.text.contains(&format!("c{MAX_LINES}\t")));
    }
}
