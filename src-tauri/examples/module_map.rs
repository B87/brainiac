//! Brainiac's module map, and how a change moved it (docs/design/codebase-maps.md,
//! The experiment on Brainiac itself).
//!
//!     pnpm map:diff [base] [head]      the diff, through scripts/module-diff.sh
//!     module_map map <src-tauri>       the map of one tree
//!     module_map diff <base> <head> [--base-label L] [--head-label L]
//!
//! A tree is a `src-tauri` folder: its `src/` and its copy of
//! `tests/module_boundaries.rs`, whose lists place each module. Everything is
//! computed from the sources, with the boundary test's own import reader, so
//! the map shows exactly what the test sees. Markdown goes to stdout.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

// `#[path]` compiles the test file as a module of this program, so the map
// and the test share one reader and one set of rules; its `#[test]`
// functions are left out outside `cargo test`.
#[path = "../tests/module_boundaries.rs"]
#[allow(dead_code)]
mod boundaries;

use boundaries::{imports_in, top_level_module, Rules, Verdict};

/// What the map knows about one tree.
struct Tree {
    rules: Rules,
    /// Lines of Rust per top-level module, tests included.
    lines: BTreeMap<String, usize>,
    /// Imports between top-level modules, the shell's included.
    edges: BTreeSet<(String, String)>,
    /// The numbers `docs/design/modules.md` tracks for change amplification.
    stats: Vec<(&'static str, usize)>,
}

fn read_tree(dir: &Path) -> Tree {
    let src = dir.join("src");
    let rules = Rules::from_source(
        &fs::read_to_string(dir.join("tests/module_boundaries.rs")).unwrap_or_default(),
    );
    let mut lines = BTreeMap::new();
    let mut commands = 0;
    for path in rust_files(&src) {
        let text = fs::read_to_string(&path).unwrap_or_default();
        *lines.entry(top_level_module(&path, &src)).or_insert(0) += text.lines().count();
        commands += text.matches("#[tauri::command").count();
    }
    let edges = imports_in(&src)
        .into_iter()
        .flat_map(|(from, names)| names.into_iter().map(move |to| (from.clone(), to)))
        .filter(|(_, to)| lines.contains_key(to))
        .collect();
    let file_lines = |name: &str| {
        fs::read_to_string(src.join(name))
            .map(|t| t.lines().count())
            .unwrap_or(0)
    };
    let stats = vec![
        ("`lib.rs` lines", file_lines("lib.rs")),
        ("Commands (`#[tauri::command]`)", commands),
        ("`models.rs` lines", file_lines("models.rs")),
        ("Top-level modules", lines.len()),
        ("Entries in `ALLOWED`", rules.allowed.len()),
    ];
    Tree {
        rules,
        lines,
        edges,
        stats,
    }
}

fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(rust_files(&path));
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
    out
}

fn group(tree: &Tree, module: &str) -> String {
    tree.rules
        .group(module)
        .unwrap_or_else(|| "unplaced".into())
}

fn verdict(tree: &Tree, from: &str, to: &str) -> &'static str {
    match tree.rules.verdict(from, to) {
        Verdict::ByRule => "by rule",
        Verdict::Listed => "listed in `ALLOWED`",
        Verdict::Crossing => "**crosses a boundary**",
        Verdict::Unplaced => "a module without a place",
    }
}

fn edge(tree: &Tree, (from, to): &(String, String)) -> String {
    format!(
        "`{from}` ({}) → `{to}` ({}), {}",
        group(tree, from),
        group(tree, to),
        verdict(tree, from, to)
    )
}

fn signed(n: i64) -> String {
    if n > 0 {
        format!("+{n}")
    } else {
        n.to_string()
    }
}

/// Whether either tree puts `module` in the shell. A commit from before the
/// boundary test places nothing, and its shell's imports would otherwise
/// show as removed.
fn in_shell(trees: &[&Tree], module: &str) -> bool {
    trees
        .iter()
        .any(|t| t.rules.group(module).as_deref() == Some("shell"))
}

/// The imports a diff compares: without the shell's, which may import anything.
fn compared(tree: &Tree, trees: &[&Tree]) -> BTreeSet<(String, String)> {
    tree.edges
        .iter()
        .filter(|(from, _)| !in_shell(trees, from))
        .cloned()
        .collect()
}

/// A Mermaid flowchart of `head`, one box per group, with the edges `added`
/// in green and `removed` (drawn dashed) in red. The shell is left out, and
/// so are imports of the core that did not change: inside the core always,
/// and from the features too with `only_exceptions`, because nearly every
/// module imports `db` and `models` and drawing them hides everything else.
fn mermaid(
    head: &Tree,
    edges: &BTreeSet<(String, String)>,
    added: &BTreeSet<(String, String)>,
    removed: &BTreeSet<(String, String)>,
    only_exceptions: bool,
) -> String {
    let mut groups: BTreeMap<String, Vec<&String>> = BTreeMap::new();
    for module in head.lines.keys() {
        let g = group(head, module);
        if g != "shell" {
            groups.entry(g).or_default().push(module);
        }
    }
    let mut out = String::from("```mermaid\nflowchart LR\n");
    for (i, (name, members)) in groups.iter().enumerate() {
        let _ = writeln!(out, "  subgraph g{i}[\"{name}\"]");
        for m in members {
            let _ = writeln!(out, "    {m}");
        }
        out.push_str("  end\n");
    }
    let (mut green, mut red) = (Vec::new(), Vec::new());
    let hidden = |(from, to): &(String, String)| {
        group(head, to) == "core" && (only_exceptions || group(head, from) == "core")
    };
    let drawn = edges
        .iter()
        .filter(|e| !hidden(e) || added.contains(*e))
        .chain(removed);
    for (n, e @ (from, to)) in drawn.enumerate() {
        if removed.contains(e) {
            let _ = writeln!(out, "  {from} -.-> {to}");
            red.push(n.to_string());
        } else {
            let _ = writeln!(out, "  {from} --> {to}");
            if added.contains(e) {
                green.push(n.to_string());
            }
        }
    }
    if !green.is_empty() {
        let _ = writeln!(
            out,
            "  linkStyle {} stroke:#1a7f37,stroke-width:3px",
            green.join(",")
        );
    }
    if !red.is_empty() {
        let _ = writeln!(
            out,
            "  linkStyle {} stroke:#cf222e,stroke-width:3px",
            red.join(",")
        );
    }
    out.push_str("```\n");
    out
}

fn map(head: &Tree, label: &str) -> String {
    let mut out = format!("## Module map at {label}\n\n");
    out.push_str("| | |\n| --- | ---: |\n");
    for (name, value) in &head.stats {
        let _ = writeln!(out, "| {name} | {value} |");
    }
    out.push_str(
        "\nThe graph leaves out the shell (`lib`, `commands`), which may import \
         anything, and imports inside the core.\n\n",
    );
    let none = BTreeSet::new();
    out.push_str(&mermaid(
        head,
        &compared(head, &[head]),
        &none,
        &none,
        false,
    ));
    out
}

fn diff(base: &Tree, head: &Tree, base_label: &str, head_label: &str) -> String {
    let modules = |t: &Tree| t.lines.keys().cloned().collect::<BTreeSet<_>>();
    let (before, after) = (modules(base), modules(head));
    let added_modules: Vec<_> = after.difference(&before).collect();
    let removed_modules: Vec<_> = before.difference(&after).collect();
    let (placed, regrouped): (Vec<_>, Vec<_>) = after
        .intersection(&before)
        .filter(|m| group(base, m) != group(head, m))
        .partition(|m| group(base, m) == "unplaced");
    let (base_edges, head_edges) = (compared(base, &[base, head]), compared(head, &[base, head]));
    let added: BTreeSet<_> = head_edges.difference(&base_edges).cloned().collect();
    let removed: BTreeSet<_> = base_edges.difference(&head_edges).cloned().collect();
    let allowed = |t: &Tree| t.rules.allowed.iter().cloned().collect::<BTreeSet<_>>();
    let listed: Vec<_> = allowed(head).difference(&allowed(base)).cloned().collect();
    let unlisted: Vec<_> = allowed(base).difference(&allowed(head)).cloned().collect();
    let crossings = added
        .iter()
        .filter(|(f, t)| head.rules.verdict(f, t) == Verdict::Crossing)
        .count();

    let mut out = format!("## Module map: {base_label} → {head_label}\n\n");
    let structural = !(added_modules.is_empty()
        && removed_modules.is_empty()
        && regrouped.is_empty()
        && placed.is_empty()
        && added.is_empty()
        && removed.is_empty()
        && listed.is_empty()
        && unlisted.is_empty());
    if structural {
        let mut parts = Vec::new();
        // "{n} {one or many} {rest}": "1 module added", "2 modules added".
        let mut count = |n: usize, one: &str, many: &str, rest: &str| {
            if n > 0 {
                let noun = if n == 1 { one } else { many };
                parts.push(format!("{n} {noun} {rest}").trim_end().to_string());
            }
        };
        count(added_modules.len(), "module", "modules", "added");
        count(removed_modules.len(), "module", "modules", "removed");
        count(
            regrouped.len(),
            "module",
            "modules",
            "moved to another group",
        );
        count(placed.len(), "module", "modules", "placed in a group");
        count(added.len(), "dependency", "dependencies", "added");
        count(removed.len(), "dependency", "dependencies", "removed");
        count(
            crossings,
            "new boundary crossing",
            "new boundary crossings",
            "",
        );
        count(listed.len(), "entry", "entries", "added to `ALLOWED`");
        count(unlisted.len(), "entry", "entries", "removed from `ALLOWED`");
        let _ = writeln!(out, "**{}.**\n", parts.join(", "));
    } else {
        out.push_str("**No change to the modules, their dependencies, or `ALLOWED`.**\n\n");
    }

    out.push_str("| | Before | After | Change |\n| --- | ---: | ---: | ---: |\n");
    for ((name, b), (_, a)) in base.stats.iter().zip(&head.stats) {
        let _ = writeln!(
            out,
            "| {name} | {b} | {a} | {} |",
            signed(*a as i64 - *b as i64)
        );
    }
    out.push('\n');

    if !(added_modules.is_empty()
        && removed_modules.is_empty()
        && regrouped.is_empty()
        && placed.is_empty())
    {
        out.push_str("### Modules\n\n");
        for m in &added_modules {
            let _ = writeln!(out, "- Added `{m}` ({})", group(head, m));
        }
        for m in &removed_modules {
            let _ = writeln!(out, "- Removed `{m}` (was {})", group(base, m));
        }
        for m in &placed {
            let _ = writeln!(out, "- `{m}` placed in {}", group(head, m));
        }
        for m in &regrouped {
            let _ = writeln!(
                out,
                "- `{m}` moved from {} to {}",
                group(base, m),
                group(head, m)
            );
        }
        out.push('\n');
    }
    if !(added.is_empty() && removed.is_empty()) {
        out.push_str("### Dependencies\n\nWithout the shell's, which may import anything.\n\n");
        for e in &added {
            let _ = writeln!(out, "- Added {}", edge(head, e));
        }
        for e in &removed {
            let _ = writeln!(out, "- Removed {}", edge(base, e));
        }
        out.push('\n');
    }
    if !(listed.is_empty() && unlisted.is_empty()) {
        out.push_str("### Exceptions in `ALLOWED`\n\n");
        for (f, t) in &listed {
            let _ = writeln!(out, "- Listed `{f}` → `{t}`");
        }
        for (f, t) in &unlisted {
            let _ = writeln!(out, "- No longer listed: `{f}` → `{t}`");
        }
        out.push('\n');
    }

    let mut sizes: Vec<(String, usize, usize)> = before
        .union(&after)
        .map(|m| {
            let get = |t: &Tree| t.lines.get(m).copied().unwrap_or(0);
            (m.clone(), get(base), get(head))
        })
        .filter(|(_, b, a)| b != a)
        .collect();
    sizes.sort_by_key(|(_, b, a)| std::cmp::Reverse((*a as i64 - *b as i64).abs()));
    if !sizes.is_empty() {
        out.push_str("### Size by module\n\n| Module | Group | Before | After | Change |\n| --- | --- | ---: | ---: | ---: |\n");
        for (m, b, a) in sizes.iter().take(15) {
            let g = if after.contains(m) {
                group(head, m)
            } else {
                group(base, m)
            };
            let _ = writeln!(
                out,
                "| `{m}` | {g} | {b} | {a} | {} |",
                signed(*a as i64 - *b as i64)
            );
        }
        if sizes.len() > 15 {
            let _ = writeln!(out, "\n{} more modules changed size.", sizes.len() - 15);
        }
        out.push('\n');
    }

    out.push_str("<details><summary>Module graph after the change</summary>\n\n");
    out.push_str(
        "What the change added is green, what it removed red and dashed. Unchanged imports of the core are left out, and so is the shell, so the other arrows are the exceptions in `ALLOWED` and imports inside a feature.\n\n",
    );
    out.push_str(&mermaid(head, &head_edges, &added, &removed, true));
    out.push_str("\n</details>\n");
    out
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let paths: Vec<&String> = args
        .iter()
        .enumerate()
        .filter(|(i, a)| !a.starts_with("--") && (*i == 0 || !args[i - 1].starts_with("--")))
        .map(|(_, a)| a)
        .collect();
    let usage = "usage: module_map map <src-tauri> | module_map diff <base> <head> [--base-label L] [--head-label L]";
    match paths.as_slice() {
        [cmd, dir] if *cmd == "map" => {
            print!(
                "{}",
                map(
                    &read_tree(Path::new(dir)),
                    &flag("--head-label").unwrap_or_else(|| dir.to_string())
                )
            );
        }
        [cmd, base, head] if *cmd == "diff" => {
            let base_label = flag("--base-label").unwrap_or_else(|| base.to_string());
            let head_label = flag("--head-label").unwrap_or_else(|| head.to_string());
            print!(
                "{}",
                diff(
                    &read_tree(Path::new(base)),
                    &read_tree(Path::new(head)),
                    &base_label,
                    &head_label
                )
            );
        }
        _ => {
            eprintln!("{usage}");
            std::process::exit(2);
        }
    }
}
