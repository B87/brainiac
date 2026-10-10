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
        ConceptKind::Technique => "technique",
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

    // The `known` script ships in the agents image, but what it reads (this
    // file's columns) and how it folds a name (`store::concept_key`) are
    // explain's, so its tests live here and read the script from the image.
    const SCRIPT: &str = include_str!("../agents/image/known.mjs");

    /// `known` is plain Node. The test fails when Node is not installed:
    /// a silent skip would hide a script that no longer matches the ledger.
    fn require_node() {
        let version = std::process::Command::new("node")
            .arg("--version")
            .output()
            .unwrap_or_else(|error| panic!("node is required to test the known script: {error}"));
        assert!(
            version.status.success(),
            "node --version failed: {version:?}"
        );
    }

    fn run_known(
        script: &std::path::Path,
        list: &std::path::Path,
        stdin: Option<&str>,
        args: &[&str],
    ) -> String {
        let mut command = std::process::Command::new("node");
        command
            .arg(script)
            .args(args)
            .env("BRAINIAC_KNOWN_FILE", list)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        if stdin.is_some() {
            command.stdin(std::process::Stdio::piped());
        }
        let mut child = command.spawn().unwrap();
        if let Some(text) = stdin {
            use std::io::Write;
            child
                .stdin
                .take()
                .unwrap()
                .write_all(text.as_bytes())
                .unwrap();
        }
        let out = child.wait_with_output().unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }

    #[test]
    fn known_says_which_names_are_exact_matches_of_the_list() {
        require_node();
        let dir = tempfile::tempdir().unwrap();
        let list = dir.path().join("known-concepts.tsv");
        std::fs::write(
            &list,
            "go embed\tgo:embed\tlanguage\tPuts a file into the binary.\n\
go embed\tgo:embed\tlibrary\tThe crate.\n\
dkim\tDKIM\tsystem\t\n",
        )
        .unwrap();
        let script = dir.path().join("known.mjs");
        std::fs::write(&script, SCRIPT).unwrap();
        let out = run_known(
            &script,
            &list,
            None,
            &[
                "GO:EMBED (language)",
                "dkim",
                "go",
                "go:embed directive",
                "spf",
            ],
        );
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(
            lines[0],
            "known: go:embed (language) - Puts a file into the binary."
        );
        assert_eq!(lines[1], "known: go:embed (library) - The crate.");
        assert_eq!(lines[2], "known: DKIM (system)");
        // Exact names only: a shorter or longer name is new, followed by what
        // is similar to it (both kinds of go:embed).
        assert_eq!(lines[3], "new: go");
        assert!(lines[4].starts_with("similar: go:embed"), "{out}");
        assert!(lines[5].starts_with("similar: go:embed"), "{out}");
        assert_eq!(lines[6], "new: go:embed directive");
        assert!(lines[7].starts_with("similar: go:embed"), "{out}");
        assert!(lines[8].starts_with("similar: go:embed"), "{out}");
        assert_eq!(lines[9], "new: spf");
        assert_eq!(lines.len(), 10, "{out}");
        // No list: every name is new, and it says why.
        std::fs::remove_file(&list).unwrap();
        assert!(run_known(&script, &list, None, &["dkim"]).contains("treat every concept as new"));
    }

    /// A name with no exact match is `new:` and then lists known concepts
    /// with the same words, closest first, at most three: a plural, a missing
    /// small word, or a longer name that contains this one.
    #[test]
    fn known_lists_similar_names_after_a_new_one() {
        require_node();
        let dir = tempfile::tempdir().unwrap();
        let list = dir.path().join("known-concepts.tsv");
        let mut tsv = String::from(
            "closures\tClosures\ttechnique\tAnonymous functions.\n\
result the operator\tResult and the ? operator\tlanguage\t\n\
borrow checker\tBorrow checker\tlanguage\t\n",
        );
        for i in 0..5 {
            tsv.push_str(&format!("arc {i}\tArc {i}\tlibrary\t\n"));
        }
        std::fs::write(&list, &tsv).unwrap();
        let script = dir.path().join("known.mjs");
        std::fs::write(&script, SCRIPT).unwrap();
        let out = run_known(
            &script,
            &list,
            Some("Closure\nResult and ?\nArc\nborrowing\nBorrow checker\n"),
            &[],
        );
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "new: Closure");
        assert_eq!(
            lines[1],
            "similar: Closures (technique) - Anonymous functions."
        );
        assert_eq!(lines[2], "new: Result and ?");
        assert_eq!(lines[3], "similar: Result and the ? operator (language)");
        // Five names contain "Arc", and three are shown.
        assert_eq!(lines[4], "new: Arc");
        assert_eq!(
            lines[5..8]
                .iter()
                .filter(|l| l.starts_with("similar: Arc "))
                .count(),
            3
        );
        // Different words are not similar, and an exact name has no similar lines.
        assert_eq!(lines[8], "new: borrowing");
        assert_eq!(lines[9], "known: Borrow checker (language)");
        assert_eq!(lines.len(), 10, "{out}");
    }

    /// The script's fold is the ledger's fold, including names a shell would
    /// split, and names come one per line on stdin.
    #[test]
    fn known_folds_names_as_the_ledger_does_and_reads_stdin() {
        require_node();
        let dir = tempfile::tempdir().unwrap();
        let names = ["İstanbul", "AΣ", "Arc<Mutex<_>>", "go:embed", "Limit rule"];
        let mut tsv = String::new();
        for name in names {
            let key = super::super::store::concept_key(name);
            tsv.push_str(&format!("{key}\t{name}\tlanguage\t\n"));
        }
        let list = dir.path().join("known-concepts.tsv");
        std::fs::write(&list, &tsv).unwrap();
        let script = dir.path().join("known.mjs");
        std::fs::write(&script, SCRIPT).unwrap();
        let out = run_known(
            &script,
            &list,
            Some(&format!("{}\n", names.join("\n"))),
            &[],
        );
        for name in names {
            assert!(out.contains(&format!("known: {name} (language)")), "{out}");
        }
        let usage = std::process::Command::new("node")
            .arg(&script)
            .env("BRAINIAC_KNOWN_FILE", &list)
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap();
        assert_eq!(usage.status.code(), Some(2));
    }
}
