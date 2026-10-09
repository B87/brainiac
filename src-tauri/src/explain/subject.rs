//! What an explanation is about, read from Brainiac's own bare repository
//! (docs/architecture.md, Explaining changes — v0.6, Checks): the files a
//! range changes, each with its changed lines on the new side, and the text
//! of the files an explanation cites at the range's tip.
//!
//! The subject has been imported into Brainiac's repository for its run, so
//! these reads never touch the user's repository. Every Git call goes
//! through `GitService::run_isolated`.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use crate::git::{validate_repo_path, validate_revision, GitService};
use crate::models::{AppError, AppResult};

/// Git's empty tree, the base of a root commit (SHA-1 repositories only, as
/// for runs).
pub const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

/// At most this many files are read for one check.
pub const MAX_FILES: usize = 200;

/// A file larger than this is not read, so its quotes cannot be verified.
pub const MAX_FILE_BYTES: usize = 1024 * 1024;

/// One run of changed lines on the new side, as a hunk header of
/// `git diff -U0` gives it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Changed {
    /// The first changed line, from 1. When `count` is 0, lines were only
    /// removed, after this line (0 when at the top).
    pub start: u32,
    pub count: u32,
}

impl Changed {
    /// Whether lines `start..=end` of the new side touch this change. A
    /// removal touches the lines on either side of it.
    pub fn touches(&self, start: u32, end: u32) -> bool {
        if self.count == 0 {
            start <= self.start + 1 && end >= self.start.max(1)
        } else {
            start < self.start + self.count && end >= self.start
        }
    }

    /// The change in words for a follow-up prompt: "40–52", "80", or
    /// "removed after 79".
    pub fn describe(&self) -> String {
        match self.count {
            0 => format!("removed after {}", self.start),
            1 => self.start.to_string(),
            n => format!("{}–{}", self.start, self.start + n - 1),
        }
    }
}

/// A file the subject changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedFile {
    /// The path at the tip, or, for a deleted file, the path it had.
    pub path: String,
    /// The path before a rename.
    pub old_path: Option<String>,
    pub deleted: bool,
    /// Git compared it as binary, so it has no lines.
    pub binary: bool,
    pub changed: Vec<Changed>,
}

/// What an explanation is about: the files a range changes, in Git's order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Subject {
    pub files: Vec<ChangedFile>,
}

impl Subject {
    pub fn file(&self, path: &str) -> Option<&ChangedFile> {
        self.files.iter().find(|f| f.path == path)
    }
}

/// Text of files at the subject's tip, by path. A path missing here is not
/// in the tip, or is binary, not UTF-8, or larger than `MAX_FILE_BYTES`.
pub type Files = BTreeMap<String, String>;

/// The options both reads of a range share, so they list the same files in
/// the same order.
const DIFF_OPTIONS: [&str; 4] = [
    "--no-ext-diff",
    "--no-textconv",
    "--find-renames",
    "--ignore-submodules=none",
];

/// Read the files that `base..tip` changes, with their changed lines. `base`
/// is the tip's parent for a commit, `EMPTY_TREE` for a root commit, and the
/// merge base or the run's start for a range.
pub async fn read_subject(
    git: &GitService,
    repo: &Path,
    home: &Path,
    base: &str,
    tip: &str,
    timeout: Duration,
) -> AppResult<Subject> {
    validate_revision(base)?;
    validate_revision(tip)?;
    let read =
        |format: &'static [&'static str]| {
            let mut args: Vec<&str> = vec!["diff"];
            args.extend_from_slice(format);
            args.extend_from_slice(&DIFF_OPTIONS);
            args.extend_from_slice(&["--end-of-options", base, tip, "--"]);
            async move {
                let out = git.run_isolated(repo, &args, home, timeout).await?;
                if !out.success() || out.truncated {
                    return Err(AppError::io("Git could not read the change.")
                        .with_details(format!("git {}\n{}", args.join(" "), out.stderr)));
                }
                Ok(out.stdout)
            }
        };
    let names = read(&["--name-status", "-z"]).await?;
    let patch = read(&["-U0", "--no-color"]).await?;
    parse_subject(&names, &patch)
}

/// Build a subject from `git diff --name-status -z` and `git diff -U0` of
/// the same range. Both list the files in the same order, so the n-th
/// `diff --git` header of the patch is the n-th file of the list. Only
/// headers are read from the patch: a line of content starts with `+`, `-`,
/// a space, or `\`, so it cannot be taken for one.
pub fn parse_subject(names: &[u8], patch: &[u8]) -> AppResult<Subject> {
    let mut files = parse_names(names)?;
    let mut index: Option<usize> = None;
    let mut last_header: &[u8] = &[];
    for line in patch.split(|&b| b == b'\n') {
        if line.starts_with(b"diff --git ") {
            // A path whose type changed (a file becomes a symlink) is one
            // entry in the name list but two identical headers in the patch.
            if index.is_some() && line == last_header {
                continue;
            }
            last_header = line;
            let next = index.map_or(0, |i| i + 1);
            if next >= files.len() {
                return Err(mismatch());
            }
            index = Some(next);
        } else if let Some(i) = index {
            if line.starts_with(b"@@ ") {
                files[i].changed.push(parse_hunk(line)?);
            } else if line.starts_with(b"Binary files ") || line.starts_with(b"GIT binary patch") {
                files[i].binary = true;
            }
        }
    }
    if index.map_or(0, |i| i + 1) != files.len() {
        return Err(mismatch());
    }
    Ok(Subject { files })
}

fn mismatch() -> AppError {
    AppError::io("Git could not read the change.")
        .with_details("The file list and the patch name different numbers of files.")
}

/// `M\0path\0`, `D\0path\0`, or `R100\0old\0new\0` (copies are not asked
/// for, but read the same way).
fn parse_names(bytes: &[u8]) -> AppResult<Vec<ChangedFile>> {
    let mut fields = bytes
        .split(|&b| b == 0)
        .map(|f| String::from_utf8_lossy(f).into_owned());
    let mut files = Vec::new();
    while let Some(status) = fields.next() {
        if status.is_empty() {
            continue;
        }
        let mut path = || {
            fields
                .next()
                .filter(|p| !p.is_empty())
                .ok_or_else(|| AppError::io("Git could not read the change."))
        };
        let file = match status.as_bytes()[0] {
            b'R' | b'C' => {
                let old = path()?;
                ChangedFile {
                    path: path()?,
                    old_path: Some(old),
                    deleted: false,
                    binary: false,
                    changed: Vec::new(),
                }
            }
            kind => ChangedFile {
                path: path()?,
                old_path: None,
                deleted: kind == b'D',
                binary: false,
                changed: Vec::new(),
            },
        };
        files.push(file);
    }
    Ok(files)
}

/// The new side of `@@ -a,b +c,d @@`; a missing `,d` means one line.
fn parse_hunk(line: &[u8]) -> AppResult<Changed> {
    let bad = || AppError::io("Git could not read the change.");
    let text = std::str::from_utf8(line).map_err(|_| bad())?;
    let new = text
        .split(' ')
        .find(|part| part.starts_with('+'))
        .ok_or_else(bad)?;
    let (start, count) = match new[1..].split_once(',') {
        Some((start, count)) => (start, count),
        None => (&new[1..], "1"),
    };
    Ok(Changed {
        start: start.parse().map_err(|_| bad())?,
        count: count.parse().map_err(|_| bad())?,
    })
}

/// Read the text of `paths` at `tip`, skipping a path that is not there or
/// cannot be quoted (binary, not UTF-8, too large). At most `MAX_FILES`
/// paths are read; a path that could leave the repository is skipped.
pub async fn read_files<'a>(
    git: &GitService,
    repo: &Path,
    home: &Path,
    tip: &str,
    paths: impl IntoIterator<Item = &'a String>,
    timeout: Duration,
) -> AppResult<Files> {
    validate_revision(tip)?;
    let mut files = Files::new();
    for path in paths.into_iter().take(MAX_FILES) {
        if validate_repo_path(path).is_err() {
            continue;
        }
        let object = format!("{tip}:{path}");
        let size = git
            .run_isolated(repo, &["cat-file", "-s", &object], home, timeout)
            .await?;
        let fits = size.success()
            && String::from_utf8_lossy(&size.stdout)
                .trim()
                .parse::<usize>()
                .is_ok_and(|n| n <= MAX_FILE_BYTES);
        if !fits {
            continue;
        }
        let out = git
            .run_isolated(repo, &["cat-file", "blob", &object], home, timeout)
            .await?;
        if !out.success() || out.truncated || out.stdout.contains(&0) {
            continue;
        }
        // `String::from_utf8` takes the bytes and gives them back as text
        // only when they are valid UTF-8; `if let Ok` skips the file otherwise.
        if let Ok(text) = String::from_utf8(out.stdout) {
            files.insert(path.clone(), text);
        }
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_patch_and_its_file_list_make_a_subject() {
        let names = b"M\0src/a.rs\0R087\0old.md\0docs/new.md\0D\0gone.txt\0A\0logo.png\0";
        let patch = b"diff --git a/src/a.rs b/src/a.rs\n\
index 1..2 100644\n\
--- a/src/a.rs\n\
+++ b/src/a.rs\n\
@@ -10,0 +11,3 @@ fn main() {\n\
+a\n+b\n+c\n\
@@ -40 +43 @@\n\
-x\n+y\n\
@@ -60,2 +62,0 @@\n\
--- removed line that looks like a header\n-z\n\
diff --git a/old.md b/docs/new.md\n\
similarity index 87%\n\
rename from old.md\n\
rename to docs/new.md\n\
@@ -3 +3 @@\n\
-@@ not a hunk\n+@@ still not a hunk\n\
diff --git a/gone.txt b/gone.txt\n\
deleted file mode 100644\n\
@@ -1,2 +0,0 @@\n\
-1\n-2\n\
diff --git a/logo.png b/logo.png\n\
new file mode 100644\n\
Binary files /dev/null and b/logo.png differ\n";
        let subject = parse_subject(names, patch).unwrap();
        let a = subject.file("src/a.rs").unwrap();
        assert_eq!(
            a.changed,
            vec![
                Changed {
                    start: 11,
                    count: 3
                },
                Changed {
                    start: 43,
                    count: 1
                },
                Changed {
                    start: 62,
                    count: 0
                },
            ]
        );
        let renamed = subject.file("docs/new.md").unwrap();
        assert_eq!(renamed.old_path.as_deref(), Some("old.md"));
        assert_eq!(renamed.changed, vec![Changed { start: 3, count: 1 }]);
        assert!(subject.file("gone.txt").unwrap().deleted);
        assert!(subject.file("logo.png").unwrap().binary);
        assert!(subject.file("logo.png").unwrap().changed.is_empty());
    }

    #[test]
    fn a_patch_that_names_other_files_is_refused() {
        let patch = b"diff --git a/x b/x\n@@ -1 +1 @@\ndiff --git a/y b/y\n";
        assert!(parse_subject(b"M\0x\0", patch).is_err());
        assert!(parse_subject(b"M\0x\0M\0y\0", b"diff --git a/x b/x\n").is_err());
    }

    #[test]
    fn a_change_touches_the_lines_it_changed_and_those_beside_a_removal() {
        let lines = Changed {
            start: 10,
            count: 3,
        };
        assert!(lines.touches(12, 20));
        assert!(lines.touches(1, 10));
        assert!(!lines.touches(13, 14));
        assert!(!lines.touches(1, 9));
        let removal = Changed { start: 5, count: 0 };
        assert!(removal.touches(5, 5));
        assert!(removal.touches(6, 8));
        assert!(!removal.touches(7, 8));
        assert!(Changed { start: 0, count: 0 }.touches(1, 1));
        assert_eq!(lines.describe(), "10–12");
        assert_eq!(removal.describe(), "removed after 5");
    }

    #[test]
    fn a_type_change_is_one_file_with_two_headers() {
        let names = b"T\0link\0M\0other.txt\0";
        let patch = b"diff --git a/link b/link\ndeleted file mode 100644\n--- a/link\n+++ /dev/null\n@@ -1 +0,0 @@\n-x\ndiff --git a/link b/link\nnew file mode 120000\n--- /dev/null\n+++ b/link\n@@ -0,0 +1 @@\n+target\ndiff --git a/other.txt b/other.txt\n--- a/other.txt\n+++ b/other.txt\n@@ -1 +1 @@\n-a\n+b\n";
        let subject = parse_subject(names, patch).unwrap();
        assert_eq!(subject.files.len(), 2);
        assert_eq!(subject.files[0].path, "link");
        assert_eq!(subject.files[1].changed.len(), 1);
    }
}
