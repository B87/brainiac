//! Which modules may import which (docs/architecture.md, Decisions, 10 Oct
//! 2026; docs/design/modules.md, Preparation, New features as leaves). What to
//! do when it fails is a hard rule in AGENTS.md.
//!
//! The shell may import anything. The core imports only the core. A feature
//! imports the core and its own modules. Any other import between top-level
//! modules must be in `ALLOWED`, with the reason it exists, so a new one is a
//! decision a reviewer sees, not an accident. The list only shrinks: an entry
//! whose import is gone fails too, so it is removed with the import.
//!
//! The test reads the sources, not the compiled crate: an import is any
//! `crate::<module>` path outside comments and `#[cfg(test)]` modules, in
//! either form (`crate::notes::NoteService` or `use crate::{db, notes}`).
//!
//! `examples/module_map.rs` includes this file to draw the module graph and
//! compare it between two commits (`pnpm map:diff`), so the reader and the
//! rules below are `pub`, and the lists are read back from the text of any
//! commit's copy of this file (`Rules::from_source`).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// Wires everything together, so it may import anything.
pub const SHELL: &[&str] = &["lib", "commands", "main", "runner_main"];

/// What every feature needs and no feature owns (docs/design/modules.md,
/// The core). Backup, agent access (`mcp`), and Settings → Secrets are core
/// even though they name features today: each is meant to become a registry
/// that features join.
pub const CORE: &[&str] = &[
    "db",
    "models",
    "events",
    "git",
    "hosting",
    "credentials",
    "sharing",
    "machines",
    "workspaces",
    "activity",
    "fetcher",
    "watcher",
    "backup",
    "mcp",
    "secrets",
];

/// Each feature's top-level modules. Notes and tasks are one feature in
/// practice (docs/design/modules.md, Where the boundaries are today).
pub const FEATURES: &[(&str, &[&str])] = &[
    ("notes", &["notes", "tasks", "index", "vault"]),
    ("pull requests", &["forge"]),
    ("databases", &["databases"]),
    ("agent runs", &["agents"]),
    ("explanations", &["explain"]),
];

/// Imports outside the rules that exist today: (from, to, why).
pub const ALLOWED: &[(&str, &str, &str)] = &[
    // Core modules that name features until they become registries.
    ("backup", "notes", "export reads the vault through the notes service"),
    ("backup", "tasks", "export writes the tasks as JSON"),
    ("backup", "index", "restore rebuilds the search index"),
    ("mcp", "notes", "the note tools call the notes service"),
    ("mcp", "tasks", "the task tools call the tasks service"),
    ("mcp", "index", "the search tool searches notes and tasks"),
    ("secrets", "agents", "lists agent profiles with a secret"),
    ("secrets", "databases", "lists database connections with a password"),
    ("secrets", "forge", "lists pull request accounts with a token"),
    // Explanations are built on these (docs/design/modules.md, What v0.6 showed).
    ("explain", "agents", "an explanation is an agent run of its own kind"),
    ("explain", "forge", "Explain reads a pull request's facts"),
    (
        "explain",
        "notes",
        "Save as note, until notes contributes the action (Actions are contributed by the consumer)",
    ),
];

/// The lists above as data, so the rules can be applied to another commit's
/// copy of this file as well as to this one.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Rules {
    pub shell: Vec<String>,
    pub core: Vec<String>,
    /// (feature, its top-level modules)
    pub features: Vec<(String, Vec<String>)>,
    /// (from, to); the reasons stay in the source.
    pub allowed: Vec<(String, String)>,
}

/// How an import between two top-level modules stands against the rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// The shell imports anything, everyone imports the core, a feature its own modules.
    ByRule,
    /// Outside the rules, listed in `ALLOWED`.
    Listed,
    /// Outside the rules and not listed: the test fails.
    Crossing,
    /// One side has no place, which `every_module_has_a_place` reports.
    Unplaced,
}

impl Rules {
    /// The lists in this copy of the file.
    pub fn current() -> Rules {
        let names = |list: &[&str]| list.iter().map(|s| s.to_string()).collect();
        Rules {
            shell: names(SHELL),
            core: names(CORE),
            features: FEATURES
                .iter()
                .map(|(feature, members)| (feature.to_string(), names(members)))
                .collect(),
            allowed: ALLOWED
                .iter()
                .map(|(from, to, _)| (from.to_string(), to.to_string()))
                .collect(),
        }
    }

    /// The lists as written in `text`, a copy of this file from any commit.
    /// A list that is missing (a commit from before this file) is empty.
    pub fn from_source(text: &str) -> Rules {
        let flat = |name| const_items(text, name).into_iter().flatten().collect();
        Rules {
            shell: flat("SHELL"),
            core: flat("CORE"),
            features: const_items(text, "FEATURES")
                .into_iter()
                .filter_map(|mut item| {
                    (!item.is_empty()).then(|| {
                        let feature = item.remove(0);
                        (feature, item)
                    })
                })
                .collect(),
            allowed: const_items(text, "ALLOWED")
                .into_iter()
                .filter(|item| item.len() >= 2)
                .map(|item| (item[0].clone(), item[1].clone()))
                .collect(),
        }
    }

    /// "shell", "core", or the feature's name.
    pub fn group(&self, module: &str) -> Option<String> {
        if self.shell.iter().any(|m| m == module) {
            return Some("shell".into());
        }
        if self.core.iter().any(|m| m == module) {
            return Some("core".into());
        }
        self.features
            .iter()
            .find(|(_, members)| members.iter().any(|m| m == module))
            .map(|(feature, _)| feature.clone())
    }

    pub fn verdict(&self, from: &str, to: &str) -> Verdict {
        let (Some(a), Some(b)) = (self.group(from), self.group(to)) else {
            return Verdict::Unplaced;
        };
        if a == "shell" || b == "core" || a == b {
            return Verdict::ByRule;
        }
        if self.allowed.iter().any(|(f, t)| f == from && t == to) {
            Verdict::Listed
        } else {
            Verdict::Crossing
        }
    }
}

/// The string literals of a `const NAME: … = &[ … ];` list in `text`, one
/// group per element of the list (a name, or a tuple's strings), with
/// comments skipped.
fn const_items(text: &str, name: &str) -> Vec<Vec<String>> {
    let Some(start) = text.find(&format!("const {name}:")) else {
        return Vec::new();
    };
    let Some(open) = text[start..].find("= &[") else {
        return Vec::new();
    };
    let mut items = vec![Vec::new()];
    let mut depth = 1;
    let mut chars = text[start + open + "= &[".len()..].chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                let mut s = String::new();
                while let Some(c) = chars.next() {
                    match c {
                        '\\' => s.extend(chars.next()),
                        '"' => break,
                        _ => s.push(c),
                    }
                }
                items.last_mut().unwrap().push(s);
            }
            '/' if chars.peek() == Some(&'/') => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
            }
            '[' | '(' => depth += 1,
            ']' | ')' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            ',' if depth == 1 => items.push(Vec::new()),
            _ => {}
        }
    }
    items.retain(|item| !item.is_empty());
    items
}

/// The lines that count: without comments and without `#[cfg(test)]`
/// modules, which rustfmt closes with a `}` at the start of a line.
pub fn code_lines(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut in_tests = false;
    let mut previous = "";
    for line in text.lines() {
        let trimmed = line.trim();
        if in_tests {
            if line == "}" {
                in_tests = false;
            }
            continue;
        }
        if previous == "#[cfg(test)]" && trimmed.starts_with("mod ") && trimmed.ends_with('{') {
            in_tests = true;
            continue;
        }
        previous = trimmed;
        if !trimmed.starts_with("//") && trimmed != "#[cfg(test)]" {
            lines.push(line);
        }
    }
    lines
}

/// The top-level modules a piece of code names after `crate::`.
pub fn imported(code: &str) -> BTreeSet<String> {
    let ident = |s: &str| -> String {
        s.trim_start()
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect()
    };
    let mut names = BTreeSet::new();
    for (start, _) in code.match_indices("crate::") {
        let rest = &code[start + "crate::".len()..];
        if let Some(group) = rest.strip_prefix('{') {
            // `crate::{a, b::{C, D}}`: the first name of each item at depth one.
            let mut depth = 1;
            let mut item_start = true;
            for (i, c) in group.char_indices() {
                if item_start && depth == 1 {
                    let name = ident(&group[i..]);
                    if !name.is_empty() {
                        names.insert(name);
                        item_start = false;
                    }
                }
                match c {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    ',' if depth == 1 => item_start = true,
                    _ => {}
                }
            }
        } else {
            let name = ident(rest);
            if !name.is_empty() {
                names.insert(name);
            }
        }
    }
    names
}

/// Every top-level module's imports of other top-level modules, read from
/// `src`. A file in `src/<module>/` belongs to `<module>`.
pub fn imports_in(src: &Path) -> BTreeMap<String, BTreeSet<String>> {
    fn walk(dir: &Path, src: &Path, out: &mut BTreeMap<String, BTreeSet<String>>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, src, out);
                continue;
            }
            if path.extension().is_none_or(|e| e != "rs") || path.ends_with("tests.rs") {
                continue;
            }
            let module = top_level_module(&path, src);
            let code = code_lines(&std::fs::read_to_string(&path).unwrap()).join("\n");
            let mut names = imported(&code);
            names.remove(&module);
            out.entry(module).or_default().extend(names);
        }
    }
    let mut out = BTreeMap::new();
    walk(src, src, &mut out);
    out
}

/// The top-level module a file under `src` belongs to.
pub fn top_level_module(path: &Path, src: &Path) -> String {
    let first = path.strip_prefix(src).unwrap().components().next().unwrap();
    first
        .as_os_str()
        .to_str()
        .unwrap()
        .trim_end_matches(".rs")
        .to_string()
}

fn imports() -> BTreeMap<String, BTreeSet<String>> {
    imports_in(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"))
}

#[test]
fn every_module_has_a_place() {
    let rules = Rules::current();
    let unplaced: Vec<String> = imports()
        .into_keys()
        .filter(|m| rules.group(m).is_none())
        .collect();
    assert!(
        unplaced.is_empty(),
        "Put each new top-level module in SHELL, CORE, or a feature in \
         tests/module_boundaries.rs: {unplaced:?}"
    );
}

#[test]
fn modules_import_only_what_the_rules_or_the_list_allow() {
    let rules = Rules::current();
    let mut outside = Vec::new();
    for (from, names) in imports() {
        for to in names {
            // A name that is not a module (a macro's path, a typo) is left
            // to the compiler, and an unplaced module to the test above.
            if rules.verdict(&from, &to) == Verdict::Crossing {
                outside.push(format!("{from} -> {to}"));
            }
        }
    }
    assert!(
        outside.is_empty(),
        "These imports cross a module boundary (docs/design/modules.md, New \
         features as leaves). Reach the other module through the core (an \
         event, a callback set in lib.rs, a type moved to its owner), or, only \
         if the import is the right design, add it to ALLOWED with its reason \
         (AGENTS.md, Hard rules): {outside:?}"
    );
}

#[test]
fn every_allowed_import_still_exists() {
    let imports = imports();
    let gone: Vec<String> = ALLOWED
        .iter()
        .filter(|(from, to, _)| !imports.get(*from).is_some_and(|names| names.contains(*to)))
        .map(|(from, to, _)| format!("{from} -> {to}"))
        .collect();
    assert!(
        gone.is_empty(),
        "These imports are gone: remove them from ALLOWED so they cannot come \
         back unnoticed: {gone:?}"
    );
}

#[test]
fn the_reader_finds_both_forms_of_import() {
    let code = "use crate::{db::{self, Db}, models::X};\nlet a = crate::notes::mtime(1);";
    let names: Vec<String> = imported(code).into_iter().collect();
    assert_eq!(names, ["db", "models", "notes"]);
    let text = "use crate::db;\n// crate::notes in a comment\n#[cfg(test)]\nmod tests {\n    use crate::forge;\n}\nuse crate::git;\n";
    assert_eq!(code_lines(text), ["use crate::db;", "use crate::git;"]);
}

#[test]
fn the_lists_read_back_from_this_file_are_the_lists() {
    let text = include_str!("module_boundaries.rs");
    assert_eq!(Rules::from_source(text), Rules::current());
    assert_eq!(Rules::from_source("fn main() {}"), Rules::default());
}
