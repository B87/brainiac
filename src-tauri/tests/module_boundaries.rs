//! Which modules may import which (docs/design/modules.md, Preparation, New
//! features as leaves).
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

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// Wires everything together, so it may import anything.
const SHELL: &[&str] = &["lib", "commands", "main", "runner_main"];

/// What every feature needs and no feature owns (docs/design/modules.md,
/// The core). Backup, agent access (`mcp`), and Settings → Secrets are core
/// even though they name features today: each is meant to become a registry
/// that features join.
const CORE: &[&str] = &[
    "db",
    "models",
    "events",
    "git",
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
const FEATURES: &[(&str, &[&str])] = &[
    ("notes", &["notes", "tasks", "index", "vault"]),
    ("pull requests", &["forge"]),
    ("databases", &["databases"]),
    ("agent runs", &["agents"]),
    ("explanations", &["explain"]),
];

/// Imports outside the rules that exist today: (from, to, why).
const ALLOWED: &[(&str, &str, &str)] = &[
    // A repository's hosted identity lives in `forge` but is the core's.
    (
        "db",
        "forge",
        "ForgeRepository, the hosted repository an origin names, belongs with repositories",
    ),
    (
        "workspaces",
        "forge",
        "ForgeRepository, the hosted repository an origin names, belongs with repositories",
    ),
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

/// Where a module stands, by its top-level name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Place {
    Shell,
    Core,
    Feature(&'static str),
}

fn place(module: &str) -> Option<Place> {
    if SHELL.contains(&module) {
        return Some(Place::Shell);
    }
    if CORE.contains(&module) {
        return Some(Place::Core);
    }
    FEATURES
        .iter()
        .find(|(_, members)| members.contains(&module))
        .map(|(feature, _)| Place::Feature(feature))
}

/// The lines that count: without comments and without `#[cfg(test)]`
/// modules, which rustfmt closes with a `}` at the start of a line.
fn code_lines(text: &str) -> Vec<&str> {
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
fn imported(code: &str) -> BTreeSet<String> {
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
/// `src/`. A file in `src/<module>/` belongs to `<module>`.
fn imports() -> BTreeMap<String, BTreeSet<String>> {
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
            let first = path.strip_prefix(src).unwrap().components().next().unwrap();
            let module = first
                .as_os_str()
                .to_str()
                .unwrap()
                .trim_end_matches(".rs")
                .to_string();
            let code = code_lines(&std::fs::read_to_string(&path).unwrap()).join("\n");
            let mut names = imported(&code);
            names.remove(&module);
            out.entry(module).or_default().extend(names);
        }
    }
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut out = BTreeMap::new();
    walk(&src, &src, &mut out);
    out
}

/// Whether the rules allow `from` to import `to`, before `ALLOWED`.
fn allowed_by_rule(from: Place, to: Place) -> bool {
    match (from, to) {
        (Place::Shell, _) => true,
        (_, Place::Core) => true,
        (Place::Feature(a), Place::Feature(b)) => a == b,
        _ => false,
    }
}

#[test]
fn every_module_has_a_place() {
    let unplaced: Vec<String> = imports()
        .into_keys()
        .filter(|m| place(m).is_none())
        .collect();
    assert!(
        unplaced.is_empty(),
        "Put each new top-level module in SHELL, CORE, or a feature in \
         tests/module_boundaries.rs: {unplaced:?}"
    );
}

#[test]
fn modules_import_only_what_the_rules_or_the_list_allow() {
    let mut outside = Vec::new();
    for (from, names) in imports() {
        let Some(from_place) = place(&from) else {
            continue;
        };
        for to in names {
            // A name that is not a module (a macro's path, a typo) is left
            // to the compiler.
            let Some(to_place) = place(&to) else { continue };
            let listed = ALLOWED.iter().any(|(a, b, _)| *a == from && *b == to);
            if !allowed_by_rule(from_place, to_place) && !listed {
                outside.push(format!("{from} -> {to}"));
            }
        }
    }
    assert!(
        outside.is_empty(),
        "These imports cross a module boundary (docs/design/modules.md, New \
         features as leaves). Reach the other module through the core (an \
         event, a callback set in lib.rs, a type moved to its owner), or add \
         the import to ALLOWED with its reason: {outside:?}"
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
