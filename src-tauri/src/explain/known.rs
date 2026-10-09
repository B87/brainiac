//! The reader's known concepts as a file an explain run reads
//! (docs/design/code-explanations.md, How the ledger reaches the run): one
//! concept per line, tab-separated: its folded name, its name, its kind, and
//! the words an earlier explanation of this repository used. The container
//! gets it as `/opt/brainiac/input/known-concepts.tsv` and the `known` script
//! in the image reads it, so the prompt names the file and a count, never
//! the list.

use std::collections::BTreeSet;

use super::store::KnownRef;
use crate::agents::controller::protocol::MAX_KNOWN_CONCEPTS_BYTES;
use crate::models::ConceptKind;

/// Where the container keeps the file (`agents/image/known.mjs` reads it).
pub const PATH: &str = "/opt/brainiac/input/known-concepts.tsv";

/// At most this many concepts are written, the newest first.
const MAX_LINES: usize = 5000;
/// A description is cut to this many bytes, on a character boundary.
const MAX_DESCRIPTION_BYTES: usize = 300;
/// A name or key longer than this is left out. Cutting a name would make
/// `known` print a name that no longer folds to the ledger's key.
const MAX_COLUMN_BYTES: usize = 4 << 10;

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
        ConceptKind::Protocol => "protocol",
        ConceptKind::Tool => "tool",
        ConceptKind::ProjectPattern => "project pattern",
    }
}

/// Tabs and line breaks become spaces, so a name or description cannot
/// start another column or line.
fn flatten(text: &str) -> String {
    let flat: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    flat.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `text` cut to at most `max_bytes`, without splitting a character.
fn truncate_bytes(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_string()
}

/// The file for the known concepts of one repository, `None` when there are
/// none. A key and a kind are written once (an alias has its own key and its
/// target's name, kind, and words). A name that cannot fit, or a line that
/// would pass [`MAX_KNOWN_CONCEPTS_BYTES`], is left out: the start is not
/// failed by a file the app itself wrote.
pub fn file(refs: &[KnownRef]) -> Option<KnownFile> {
    let mut seen = BTreeSet::new();
    let mut text = String::new();
    let mut count = 0;
    for r in refs {
        if count == MAX_LINES || text.len() == MAX_KNOWN_CONCEPTS_BYTES {
            break;
        }
        // The key is the ledger's fold. Rewriting it would make `known` miss.
        if r.key.is_empty()
            || r.key.len() > MAX_COLUMN_BYTES
            || r.key.chars().any(|c| c.is_control())
        {
            continue;
        }
        let name = flatten(&r.name);
        if name.is_empty() || name.len() > MAX_COLUMN_BYTES {
            continue;
        }
        if !seen.insert((r.key.as_str(), kind_word(r.kind))) {
            continue;
        }
        let description = truncate_bytes(&flatten(&r.description), MAX_DESCRIPTION_BYTES);
        let line = format!(
            "{}\t{}\t{}\t{}\n",
            r.key,
            name,
            kind_word(r.kind),
            description
        );
        // One concept that cannot fit on its own is left out. Once the file
        // is full, stop: an older smaller concept does not take a newer
        // one's place.
        if line.len() > MAX_KNOWN_CONCEPTS_BYTES {
            continue;
        }
        if text.len() + line.len() > MAX_KNOWN_CONCEPTS_BYTES {
            break;
        }
        text.push_str(&line);
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
    fn two_kinds_of_one_name_are_both_written() {
        let mut library = known("result", "Result", "the crate");
        library.kind = ConceptKind::Library;
        let f = file(&[known("result", "Result", "the type"), library]).unwrap();
        assert_eq!(f.count, 2);
        assert!(f.text.contains("result\tResult\tlanguage\tthe type\n"));
        assert!(f.text.contains("result\tResult\tlibrary\tthe crate\n"));
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
        assert!(f.text.len() <= MAX_KNOWN_CONCEPTS_BYTES);
    }

    #[test]
    fn a_name_that_cannot_fit_is_left_out() {
        let huge = "a".repeat(MAX_COLUMN_BYTES + 1);
        let f = file(&[known(&huge, &huge, ""), known("dkim", "DKIM", "signs")]).unwrap();
        assert_eq!(f.count, 1);
        assert!(f.text.starts_with("dkim\t"));
        assert!(f.text.len() <= MAX_KNOWN_CONCEPTS_BYTES);
    }

    #[test]
    fn the_file_stops_at_the_byte_ceiling_the_controller_checks() {
        let name = "n".repeat(200);
        let description = "d".repeat(MAX_DESCRIPTION_BYTES);
        let refs: Vec<KnownRef> = (0..MAX_LINES)
            .map(|i| {
                let key = format!("k{i:04}");
                KnownRef {
                    id: key.clone(),
                    name: format!("{name}{i}"),
                    kind: ConceptKind::Language,
                    key,
                    description: description.clone(),
                }
            })
            .collect();
        let f = file(&refs).unwrap();
        assert!(f.count < MAX_LINES);
        assert!(f.count > 1000);
        assert!(f.text.len() <= MAX_KNOWN_CONCEPTS_BYTES);
        assert!(f.text.starts_with("k0000\t"));
    }

    #[test]
    fn a_description_is_cut_on_a_character_boundary() {
        let f = file(&[known("a", "A", &"é".repeat(200))]).unwrap();
        let description = f.text.split('\t').nth(3).unwrap().trim_end_matches('\n');
        assert!(description.len() <= MAX_DESCRIPTION_BYTES);
        assert_eq!(description.chars().count(), MAX_DESCRIPTION_BYTES / 2);
    }
}
