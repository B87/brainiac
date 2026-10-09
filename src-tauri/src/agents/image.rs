//! The image Brainiac builds for runs (SPEC.md, Settings → Agents, Image):
//! a readable Dockerfile and entrypoint compiled into Brainiac, built on the
//! chosen engine through its API. The build context is those files (with
//! the npm lockfile that pins every package) and nothing else.
//!
//! The *recipe* is a hash of the files. An image is current while it was
//! built from this version's recipe; an update that changes them asks for a
//! rebuild.

use std::collections::VecDeque;
use std::path::Path;
use std::time::Duration;

use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::engine;
use crate::models::{AppError, AppResult};

/// The files of the build context, by name.
const FILES: &[(&str, &str)] = &[
    ("Dockerfile", include_str!("image/Dockerfile")),
    ("entrypoint.mjs", include_str!("image/entrypoint.mjs")),
    ("collector.mjs", include_str!("image/collector.mjs")),
    ("known.mjs", include_str!("image/known.mjs")),
    ("package.json", include_str!("image/package.json")),
    ("package-lock.json", include_str!("image/package-lock.json")),
    (
        "managed-settings.json",
        include_str!("image/managed-settings.json"),
    ),
    ("opencode.json", include_str!("image/opencode.json")),
];

/// A build installs packages from the network: give it time.
const BUILD_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// The last lines of build output kept for the details of a failure.
const KEEP_LINES: usize = 40;

/// The longest line of build output read; longer ones are skipped.
const MAX_LINE_BYTES: usize = 64 * 1024;

/// The Dockerfile, for **View Dockerfile**.
pub fn dockerfile() -> &'static str {
    FILES[0].1
}

/// The hash of the build context's files: 12 hex digits.
pub fn recipe() -> String {
    let mut hash = Sha256::new();
    for (name, text) in FILES {
        hash.update(name.as_bytes());
        hash.update([0]);
        hash.update((text.len() as u64).to_be_bytes());
        hash.update(text.as_bytes());
    }
    hash.finalize()
        .iter()
        .take(6)
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The image's name for a recipe.
pub fn name_for(recipe: &str) -> String {
    format!("brainiac-agents:{recipe}")
}

/// An image the engine built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltImage {
    pub name: String,
    /// The engine's image ID, `sha256:…`.
    pub id: String,
}

/// One line of the engine's build output.
#[derive(Deserialize)]
struct BuildLine {
    stream: Option<String>,
    error: Option<String>,
    aux: Option<Aux>,
}

#[derive(Deserialize)]
struct Aux {
    #[serde(rename = "ID")]
    id: Option<String>,
}

#[derive(Deserialize)]
struct Inspect {
    #[serde(rename = "Id")]
    id: String,
}

/// Build the image on the engine behind `socket`, pulling the base image
/// afresh, and return its ID as the engine reports it.
pub async fn build(socket: &Path) -> AppResult<BuiltImage> {
    let name = name_for(&recipe());
    let client = engine::client(socket, BUILD_TIMEOUT)?;
    let labels = serde_json::json!({
        "org.brainiac.role": "agent",
        "org.brainiac.recipe": recipe(),
    })
    .to_string();
    let url = reqwest::Url::parse_with_params(
        "http://docker/build",
        &[
            ("t", name.as_str()),
            ("labels", labels.as_str()),
            ("pull", "1"),
            ("rm", "1"),
            ("forcerm", "1"),
        ],
    )
    .map_err(|e| {
        AppError::io("The build request could not be made.").with_details(e.to_string())
    })?;
    let mut response = client
        .post(url)
        .header("Content-Type", "application/x-tar")
        .body(context())
        .send()
        .await
        .map_err(|e| {
            AppError::dependency("The engine is not answering. Is it running?")
                .with_details(e.to_string())
        })?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(
            AppError::io("The engine refused to build the image.").with_details(format!(
                "{status}: {}",
                body.chars().take(MAX_LINE_BYTES).collect::<String>()
            )),
        );
    }
    // Read as it arrives, keeping only what a result or an error needs: a
    // build can print a lot, and it is never held whole.
    let mut output = BuildOutput::default();
    while let Some(chunk) = response.chunk().await.map_err(|e| {
        AppError::dependency("The build stopped before it finished.").with_details(e.to_string())
    })? {
        output.feed(&chunk);
    }
    let id = output.finish()?;
    // The name now points at the image just built, unless something else
    // moved it since: then the build is not trusted.
    let inspect: Inspect = client
        .get(format!("http://docker/images/{name}/json"))
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| {
            AppError::io("The built image could not be found on the engine.")
                .with_details(e.to_string())
        })?
        .json()
        .await
        .map_err(|e| {
            AppError::io("The engine's answer could not be read.").with_details(e.to_string())
        })?;
    if id.as_deref().is_some_and(|id| id != inspect.id) {
        return Err(AppError::io(
            "The image name was changed during the build. Build it again.",
        ));
    }
    Ok(BuiltImage {
        name,
        id: inspect.id,
    })
}

/// The engine's build output, a JSON object per line, read as it arrives.
/// It keeps the image ID, the first error, and the last lines printed.
#[derive(Default)]
struct BuildOutput {
    /// The end of a line not yet complete; a line longer than
    /// `MAX_LINE_BYTES` is dropped.
    partial: Vec<u8>,
    overlong: bool,
    id: Option<String>,
    error: Option<String>,
    tail: VecDeque<String>,
}

impl BuildOutput {
    fn feed(&mut self, mut bytes: &[u8]) {
        while let Some(end) = bytes.iter().position(|b| *b == b'\n') {
            self.push(&bytes[..end]);
            bytes = &bytes[end + 1..];
        }
        self.push_partial(bytes);
    }

    fn push_partial(&mut self, bytes: &[u8]) {
        if self.partial.len() + bytes.len() > MAX_LINE_BYTES {
            self.partial.clear();
            self.overlong = true;
        } else if !self.overlong {
            self.partial.extend_from_slice(bytes);
        }
    }

    /// The end of a line arrived.
    fn push(&mut self, end: &[u8]) {
        self.push_partial(end);
        let line = std::mem::take(&mut self.partial);
        if std::mem::take(&mut self.overlong) || line.is_empty() {
            return;
        }
        let Ok(line) = serde_json::from_slice::<BuildLine>(&line) else {
            return;
        };
        if let Some(text) = line.stream {
            for l in text.lines() {
                if self.tail.len() == KEEP_LINES {
                    self.tail.pop_front();
                }
                self.tail.push_back(l.to_string());
            }
        }
        if let Some(error) = line.error {
            self.error.get_or_insert(error);
        }
        if let Some(found) = line.aux.and_then(|a| a.id) {
            self.id = Some(found);
        }
    }

    /// The image ID the build reported, or the error it ended with, with the
    /// last lines of its output as details.
    fn finish(mut self) -> AppResult<Option<String>> {
        self.push(&[]);
        if let Some(error) = self.error {
            let tail: Vec<String> = self.tail.into();
            return Err(AppError::io("The image could not be built.")
                .with_details(format!("{}\n{error}", tail.join("\n"))));
        }
        Ok(self.id)
    }
}

/// The build context: the files as a tar archive (POSIX ustar). Names are
/// under 100 bytes, so no long-name extension is needed.
pub(crate) fn context() -> Vec<u8> {
    let mut tar = Vec::new();
    for (name, text) in FILES {
        tar.extend_from_slice(&header(name, text.len()));
        tar.extend_from_slice(text.as_bytes());
        tar.resize(tar.len().div_ceil(512) * 512, 0);
    }
    // Two empty blocks end the archive.
    tar.resize(tar.len() + 1024, 0);
    tar
}

/// A ustar header for a regular file readable by everyone.
pub(crate) fn header(name: &str, size: usize) -> [u8; 512] {
    let mut h = [0u8; 512];
    let mut field = |offset: usize, value: &[u8]| {
        h[offset..offset + value.len()].copy_from_slice(value);
    };
    field(0, name.as_bytes());
    field(100, b"0000644\0");
    field(108, b"0000000\0");
    field(116, b"0000000\0");
    field(124, format!("{size:011o}\0").as_bytes());
    field(136, b"00000000000\0");
    field(156, b"0");
    field(257, b"ustar\0");
    field(263, b"00");
    // The checksum is the sum of the header's bytes with its own field as spaces.
    h[148..156].copy_from_slice(b"        ");
    let sum: u32 = h.iter().map(|b| u32::from(*b)).sum();
    h[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_context_is_a_tar_of_the_files() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("context.tar");
        std::fs::write(&file, context()).unwrap();
        let out = std::process::Command::new("tar")
            .arg("-xf")
            .arg(&file)
            .arg("-C")
            .arg(dir.path())
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        for (name, text) in FILES {
            assert_eq!(
                &std::fs::read_to_string(dir.path().join(name)).unwrap(),
                text
            );
        }
    }

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
        std::fs::write(&script, include_str!("image/known.mjs")).unwrap();
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
        std::fs::write(&script, include_str!("image/known.mjs")).unwrap();
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
            let key = crate::explain::store::concept_key(name);
            tsv.push_str(&format!("{key}\t{name}\tlanguage\t\n"));
        }
        let list = dir.path().join("known-concepts.tsv");
        std::fs::write(&list, &tsv).unwrap();
        let script = dir.path().join("known.mjs");
        std::fs::write(&script, include_str!("image/known.mjs")).unwrap();
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

    #[test]
    fn the_recipe_names_the_image() {
        let recipe = recipe();
        assert_eq!(recipe.len(), 12);
        assert_eq!(name_for(&recipe), format!("brainiac-agents:{recipe}"));
        assert!(dockerfile().contains("ENTRYPOINT"));
    }

    /// Opt in with a real engine's socket: `BRAINIAC_TEST_DOCKER_SOCKET=… cargo
    /// test -- --ignored builds_on_a_real_engine`. It pulls and installs packages.
    #[tokio::test]
    #[ignore]
    async fn builds_on_a_real_engine() {
        let socket =
            std::env::var("BRAINIAC_TEST_DOCKER_SOCKET").expect("BRAINIAC_TEST_DOCKER_SOCKET");
        let built = build(Path::new(&socket)).await.unwrap();
        assert_eq!(built.name, name_for(&recipe()));
        assert!(built.id.starts_with("sha256:"), "{built:?}");
    }

    /// The output split anywhere across chunks, as the engine sends it.
    fn read(body: &[u8], chunk: usize) -> AppResult<Option<String>> {
        let mut output = BuildOutput::default();
        body.chunks(chunk).for_each(|c| output.feed(c));
        output.finish()
    }

    #[test]
    fn build_output_gives_the_id_or_the_error() {
        let ok = b"{\"stream\":\"Step 1/9\\n\"}\n{\"aux\":{\"ID\":\"sha256:abc\"}}\n";
        for chunk in [1, 3, ok.len()] {
            assert_eq!(read(ok, chunk).unwrap().as_deref(), Some("sha256:abc"));
        }
        let failed = b"{\"stream\":\"npm ERR! 404\\n\"}\n{\"error\":\"returned a non-zero code\"}";
        for chunk in [2, failed.len()] {
            let err = read(failed, chunk).unwrap_err();
            let details = err.details.unwrap();
            assert!(details.contains("npm ERR! 404") && details.contains("non-zero"));
        }
    }

    #[test]
    fn build_output_is_kept_bounded() {
        let mut output = BuildOutput::default();
        for i in 0..1000 {
            output.feed(format!("{{\"stream\":\"line {i}\\n\"}}\n").as_bytes());
        }
        // One line far longer than any the engine prints is skipped whole.
        output.feed(b"{\"stream\":\"");
        output.feed(&vec![b'x'; MAX_LINE_BYTES * 2]);
        output.feed(b"\"}\n{\"aux\":{\"ID\":\"sha256:def\"}}\n");
        assert_eq!(output.tail.len(), KEEP_LINES);
        assert_eq!(output.tail.back().map(String::as_str), Some("line 999"));
        assert!(output.partial.is_empty());
        assert_eq!(output.finish().unwrap().as_deref(), Some("sha256:def"));
    }
}
