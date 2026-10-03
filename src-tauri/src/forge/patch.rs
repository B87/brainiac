//! A provider's diff of a whole pull request, cut into one patch per file,
//! so each file is parsed by `git::parse_unified_diff` as a local patch is.

/// One file's part of a unified diff, from its `diff --git` line to the next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilePatch {
    /// The path after the change (the old one for a removed file).
    pub path: String,
    /// The path before a rename.
    pub old_path: Option<String>,
    pub text: String,
}

/// Split a unified diff at its `diff --git` headers. Paths come from the
/// `---`/`+++` lines when they are there (a rename without changes has
/// none), otherwise from the header itself.
pub fn split_patch(diff: &str) -> Vec<FilePatch> {
    let mut out: Vec<FilePatch> = Vec::new();
    let mut current: Option<Current> = None;
    for line in diff.split_inclusive('\n') {
        let bare = line.strip_suffix('\n').unwrap_or(line);
        if let Some(header) = bare.strip_prefix("diff --git ") {
            if let Some(c) = current.take() {
                out.push(c.finish());
            }
            let (old, new) = header_paths(header);
            current = Some(Current {
                header_old: old,
                header_new: new,
                old: None,
                new: None,
                text: String::new(),
            });
        }
        let Some(c) = current.as_mut() else {
            // Text before the first header (none from either provider).
            continue;
        };
        c.text.push_str(line);
        if c.text.contains("\n@@ ") || c.text.starts_with("@@ ") {
            // Inside hunks, `--- ` is a removed line, not a header.
            continue;
        }
        if let Some(p) = bare.strip_prefix("--- ") {
            c.old = strip_ab(p);
        } else if let Some(p) = bare.strip_prefix("+++ ") {
            c.new = strip_ab(p);
        } else if let Some(p) = bare.strip_prefix("rename from ") {
            c.old = Some(p.to_string());
        } else if let Some(p) = bare.strip_prefix("rename to ") {
            c.new = Some(p.to_string());
        }
    }
    if let Some(c) = current.take() {
        out.push(c.finish());
    }
    out
}

struct Current {
    header_old: String,
    header_new: String,
    old: Option<String>,
    new: Option<String>,
    text: String,
}

impl Current {
    fn finish(self) -> FilePatch {
        let old = self.old.unwrap_or(self.header_old);
        let new = self.new.unwrap_or(self.header_new);
        let path = if new.is_empty() { old.clone() } else { new };
        let old_path = (!old.is_empty() && old != path).then_some(old);
        FilePatch {
            path,
            old_path,
            text: self.text,
        }
    }
}

/// `a/x b/y` as `(x, y)`. A path may hold spaces, so the split is at the
/// ` b/` that leaves equal halves when there is one, otherwise the first.
fn header_paths(header: &str) -> (String, String) {
    let header = header.trim_end();
    if let Some(rest) = header.strip_prefix('"') {
        // Quoted paths (unusual characters): `"a/x y" "b/x y"`.
        let mut parts = Vec::new();
        let mut buf = String::new();
        let mut escaped = false;
        let mut chars = rest.chars();
        for c in chars.by_ref() {
            if escaped {
                // Kept as written (`\303`): Git's own escaping of the path.
                buf.push(c);
                escaped = false;
            } else if c == '\\' {
                buf.push(c);
                escaped = true;
            } else if c == '"' {
                parts.push(std::mem::take(&mut buf));
                if parts.len() == 2 {
                    break;
                }
            } else if !buf.is_empty() || c != ' ' {
                buf.push(c);
            }
        }
        let mut it = parts.into_iter().map(|p| strip_ab(&p).unwrap_or(p));
        return (it.next().unwrap_or_default(), it.next().unwrap_or_default());
    }
    let positions: Vec<usize> = header.match_indices(" b/").map(|(i, _)| i).collect();
    let split_at = positions
        .iter()
        .copied()
        .find(|&i| header[..i].strip_prefix("a/") == Some(&header[i + 3..]))
        .or_else(|| positions.first().copied());
    match split_at {
        Some(i) => (
            header[..i]
                .strip_prefix("a/")
                .unwrap_or(&header[..i])
                .to_string(),
            header[i + 3..].to_string(),
        ),
        None => (header.to_string(), header.to_string()),
    }
}

/// `a/path` or `b/path` as `path`; `/dev/null` as nothing.
fn strip_ab(p: &str) -> Option<String> {
    let p = p.split('\t').next().unwrap_or(p);
    if p == "/dev/null" {
        return None;
    }
    let p = p
        .strip_prefix("a/")
        .or_else(|| p.strip_prefix("b/"))
        .unwrap_or(p);
    Some(p.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIFF: &str = "diff --git a/src/lib.rs b/src/lib.rs\nindex 1..2 100644\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 +1,2 @@\n-old\n+new\n context\ndiff --git a/docs/old.md b/docs/new.md\nsimilarity index 100%\nrename from docs/old.md\nrename to docs/new.md\ndiff --git a/bin/x b/bin/x\nnew file mode 100644\nBinary files /dev/null and b/bin/x differ\ndiff --git a/gone.txt b/gone.txt\ndeleted file mode 100644\n--- a/gone.txt\n+++ /dev/null\n@@ -1 +0,0 @@\n-bye\n";

    #[test]
    fn files_are_cut_at_their_headers_with_their_paths() {
        let files = split_patch(DIFF);
        let summary: Vec<(&str, Option<&str>)> = files
            .iter()
            .map(|f| (f.path.as_str(), f.old_path.as_deref()))
            .collect();
        assert_eq!(
            summary,
            vec![
                ("src/lib.rs", None),
                ("docs/new.md", Some("docs/old.md")),
                ("bin/x", None),
                ("gone.txt", None),
            ]
        );
        assert!(files[0].text.starts_with("diff --git a/src/lib.rs"));
        assert!(files[0].text.ends_with(" context\n"));
        assert!(files[2].text.contains("Binary files"));
    }

    #[test]
    fn paths_with_spaces_and_a_removed_line_starting_with_dashes_survive() {
        let diff = "diff --git a/my dir/a b.txt b/my dir/a b.txt\n--- a/my dir/a b.txt\n+++ b/my dir/a b.txt\n@@ -1 +1 @@\n--- not a header\n+++ nor this\n";
        let files = split_patch(diff);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "my dir/a b.txt");
        assert_eq!(files[0].old_path, None);
        let quoted = "diff --git \"a/sp\\303\\244ce.txt\" \"b/sp\\303\\244ce.txt\"\n";
        assert_eq!(split_patch(quoted)[0].path, "sp\\303\\244ce.txt");
    }
}
