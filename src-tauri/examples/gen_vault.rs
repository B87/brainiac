//! Writes a synthetic Markdown vault for measuring indexing, search, and the
//! vault watcher at scale (`docs/architecture.md`, Quality and verification).
//!
//!     pnpm vault:gen <empty-or-missing-folder> [--notes 10000] [--seed 1]
//!
//! The same notes count and seed always produce the same vault, on any machine.
//! It contains folders up to three levels deep, a fifth of the notes as daily
//! journal notes, a log-normal spread of note sizes with a tail of long
//! articles and pasted logs (1, 3, 5, and 8 MiB), frontmatter with and without
//! `brainiac_id`, every kind of link (wikilinks, aliases, headings, relative
//! Markdown links with `%20`, web links, images, unresolved targets),
//! attachments, hidden folders, and odd files (empty, CRLF, not UTF-8).

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Small deterministic random generator (xorshift64), so no dependency is
/// needed and a seed reproduces the same vault everywhere.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn chance(&mut self, p: f64) -> bool {
        self.unit() < p
    }
    /// Standard normal sample (Box–Muller), for log-normal note sizes.
    fn normal(&mut self) -> f64 {
        let (a, b) = (self.unit().max(1e-12), self.unit());
        (-2.0 * a.ln()).sqrt() * (2.0 * std::f64::consts::PI * b).cos()
    }
    // `'a` ties the returned reference to the slice: the item lives as long as `xs`.
    fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[self.below(xs.len())]
    }
}

/// Everyday and programming vocabulary for titles and prose.
const WORDS: &[&str] = &[
    "account",
    "adapter",
    "agenda",
    "alert",
    "analysis",
    "archive",
    "backlog",
    "backup",
    "balance",
    "branch",
    "budget",
    "build",
    "cache",
    "calendar",
    "change",
    "checklist",
    "client",
    "cluster",
    "commit",
    "config",
    "contract",
    "cost",
    "customer",
    "dashboard",
    "database",
    "deadline",
    "decision",
    "deploy",
    "design",
    "draft",
    "error",
    "estimate",
    "event",
    "feature",
    "feedback",
    "field",
    "filter",
    "fixture",
    "garden",
    "goal",
    "handler",
    "health",
    "index",
    "invoice",
    "issue",
    "journal",
    "kitchen",
    "latency",
    "layout",
    "lesson",
    "library",
    "limit",
    "migration",
    "milestone",
    "module",
    "monitor",
    "network",
    "onboarding",
    "outline",
    "owner",
    "payment",
    "pipeline",
    "plan",
    "policy",
    "priority",
    "process",
    "project",
    "proposal",
    "query",
    "queue",
    "reading",
    "recipe",
    "refactor",
    "release",
    "report",
    "request",
    "research",
    "retry",
    "review",
    "risk",
    "roadmap",
    "rollout",
    "router",
    "schema",
    "search",
    "service",
    "session",
    "settings",
    "snapshot",
    "sprint",
    "storage",
    "summary",
    "supplier",
    "support",
    "system",
    "template",
    "testing",
    "timeline",
    "token",
    "travel",
    "update",
    "upgrade",
    "version",
    "vendor",
    "weekly",
    "workflow",
    "worker",
    "writing",
];

/// Joining words that make the prose read like sentences to the tokenizer.
const GLUE: &[&str] = &[
    "the", "a", "and", "or", "with", "for", "from", "after", "before", "when", "because", "into",
    "on", "of", "to", "is", "was", "should", "could", "needs", "keeps", "moves", "checks", "adds",
    "removes", "reads", "writes", "every", "each", "new", "old", "next", "first", "last", "small",
    "large", "slow", "fast", "shared", "local",
];

/// Accented titles and words, so search covers diacritics and non-ASCII paths.
const ACCENTED: &[&str] = &[
    "Reunió",
    "Planificació",
    "Migració",
    "Café",
    "Configuración",
    "Résumé",
    "Übersicht",
    "Año",
    "Façade",
    "Naïve",
];

const CODE: &[&str] = &[
    "```rust\nfn fetch_with_backoff(url: &str, tries: u32) -> Result<String, Error> {\n    for attempt in 0..tries {\n        match get(url) {\n            Ok(body) => return Ok(body),\n            Err(e) if attempt + 1 < tries => sleep(delay(attempt)),\n            Err(e) => return Err(e),\n        }\n    }\n    unreachable!()\n}\n```",
    "```ts\nexport async function loadNotes(folder: string): Promise<Note[]> {\n  const entries = await listFolder(folder);\n  return entries.filter((e) => e.name.endsWith(\".md\")).map(toNote);\n}\n```",
    "```sql\nSELECT n.path, count(l.id) AS links\nFROM notes n LEFT JOIN links l ON l.target_id = n.id\nGROUP BY n.id ORDER BY links DESC LIMIT 20;\n```",
    "```sh\ngit fetch --prune origin\ngit log --oneline origin/main..HEAD\n```",
    "```json\n{ \"retries\": 3, \"timeout_ms\": 2500, \"regions\": [\"eu-west\", \"us-east\"] }\n```",
];

struct Meta {
    dir: String,
    title: String,
    stem: String,
    rel: String,
    journal: bool,
    size: usize,
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        // `to_uppercase` yields several chars for some letters, hence `chain`.
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn title(rng: &mut Rng) -> String {
    let mut words: Vec<&str> = (0..2 + rng.below(3)).map(|_| *rng.pick(WORDS)).collect();
    if rng.chance(0.04) {
        words[0] = rng.pick(ACCENTED);
    }
    capitalize(&words.join(" "))
}

fn sentence(rng: &mut Rng, words: usize) -> String {
    let mut out: Vec<&str> = Vec::with_capacity(words);
    for i in 0..words {
        out.push(if i % 2 == 1 || rng.chance(0.3) {
            rng.pick(GLUE)
        } else if rng.chance(0.02) {
            rng.pick(ACCENTED)
        } else {
            rng.pick(WORDS)
        });
    }
    capitalize(&out.join(" ")) + "."
}

fn paragraph(rng: &mut Rng) -> String {
    let n = 2 + rng.below(5);
    let parts: Vec<String> = (0..n)
        .map(|_| {
            let len = 6 + rng.below(13);
            sentence(rng, len)
        })
        .collect();
    parts.join(" ")
}

/// Relative Markdown link from a folder to a vault path, spaces as `%20`.
fn rel_link(from_dir: &str, to_rel: &str) -> String {
    let from: Vec<&str> = from_dir.split('/').filter(|x| !x.is_empty()).collect();
    let to: Vec<&str> = to_rel.split('/').collect();
    let common = from
        .iter()
        .zip(&to[..to.len() - 1])
        .take_while(|(a, b)| a == b)
        .count();
    let mut parts = vec![".."; from.len() - common];
    parts.extend(&to[common..]);
    parts.join("/").replace(' ', "%20")
}

fn link(rng: &mut Rng, metas: &[Meta], me: usize, attachments: &[String]) -> String {
    let target = &metas[rng.below(metas.len())];
    match rng.below(100) {
        0..=24 => format!("[[{}]]", target.stem),
        25..=34 => format!("[[{}|{}]]", target.stem, target.title.to_lowercase()),
        35..=39 => format!("[[{}#Notes]]", target.stem),
        40..=69 => format!(
            "[{}]({})",
            target.title,
            rel_link(&metas[me].dir, &target.rel)
        ),
        70..=84 => format!("[the docs](https://example.com/docs/{})", rng.below(500)),
        85..=89 if !attachments.is_empty() => {
            format!(
                "[spec]({})",
                rel_link(&metas[me].dir, rng.pick(attachments))
            )
        }
        _ => format!("[[Unwritten idea {}]]", rng.below(200)),
    }
}

fn uuid(rng: &mut Rng) -> String {
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&rng.next().to_le_bytes());
    bytes[8..].copy_from_slice(&rng.next().to_le_bytes());
    uuid::Builder::from_random_bytes(bytes)
        .into_uuid()
        .to_string()
}

fn body(rng: &mut Rng, metas: &[Meta], me: usize, attachments: &[String]) -> String {
    let m = &metas[me];
    let mut s = String::new();
    if rng.chance(0.4) {
        s += "---\n";
        if rng.chance(0.5) {
            s += &format!("title: {}\n", m.title);
        }
        s += &format!("tags: [{}, {}]\n", rng.pick(WORDS), rng.pick(WORDS));
        s += &format!(
            "created: 2025-{:02}-{:02}\n",
            1 + rng.below(12),
            1 + rng.below(28)
        );
        if rng.chance(0.35) {
            s += &format!("brainiac_id: {}\n", uuid(rng));
        }
        if rng.chance(0.3) {
            s += "cssclasses: [wide]\nsource: \"https://example.com/article\"\n";
        }
        s += "---\n\n";
    }
    if m.journal || rng.chance(0.7) {
        s += &format!("# {}\n\n", m.title);
    }
    while s.len() < m.size {
        match rng.below(100) {
            0..=49 => {
                s += &paragraph(rng);
                if rng.chance(0.35) {
                    s += &format!(" See {}.", link(rng, metas, me, attachments));
                }
                s += "\n\n";
            }
            50..=59 => s += &format!("## {}\n\n", title(rng)),
            60..=71 => {
                for _ in 0..2 + rng.below(5) {
                    let len = 4 + rng.below(10);
                    s += &format!("- {}", sentence(rng, len));
                    if rng.chance(0.2) {
                        s += &format!(" {}", link(rng, metas, me, attachments));
                    }
                    s += "\n";
                }
                s += "\n";
            }
            72..=80 => {
                for _ in 0..1 + rng.below(5) {
                    let done = if rng.chance(0.4) { "x" } else { " " };
                    let len = 3 + rng.below(8);
                    s += &format!("- [{done}] {}\n", sentence(rng, len));
                }
                s += "\n";
            }
            81..=88 => s += &format!("{}\n\n", rng.pick(CODE)),
            89..=93 => {
                let len = 8 + rng.below(30);
                s += &format!("> {}\n\n", sentence(rng, len));
            }
            94..=96 if !attachments.is_empty() => {
                let a = rng.pick(attachments);
                if rng.chance(0.5) {
                    s += &format!("![diagram]({})\n\n", rel_link(&m.dir, a));
                } else {
                    let name = a.rsplit('/').next().unwrap_or(a);
                    s += &format!("![[{name}]]\n\n");
                }
            }
            _ => {
                s += "| Option | Cost | Notes |\n| --- | --- | --- |\n| A | low | fine |\n| B | high | later |\n\n"
            }
        }
    }
    s
}

fn pasted_log(rng: &mut Rng, bytes: usize) -> String {
    let mut s = String::from("# Pasted log\n\n```text\n");
    while s.len() < bytes - 200 {
        s += &format!(
            "2026-09-14T10:{:02}:{:02}Z INFO request id={:016x} path=/api/v1/orders/{} status={} ms={}\n",
            rng.below(60),
            rng.below(60),
            rng.next(),
            rng.below(100_000),
            [200, 200, 200, 404, 500][rng.below(5)],
            rng.below(900)
        );
    }
    s + "```\n"
}

/// Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

fn generate(root: &Path, n: usize, seed: u64) -> std::io::Result<()> {
    // xorshift never leaves zero, so force the seed odd.
    let mut rng = Rng(seed | 1);

    let mut dirs = vec![String::new()];
    for top in [
        "Projects",
        "Areas",
        "Resources",
        "Archive",
        "Meetings",
        "People",
        "Reading",
    ] {
        dirs.push(top.into());
        for _ in 0..2 + rng.below(6 + n / 1000) {
            let sub = format!("{top}/{}", title(&mut rng));
            dirs.push(sub.clone());
            if rng.chance(0.3) {
                for _ in 0..1 + rng.below(4) {
                    dirs.push(format!("{sub}/{}", title(&mut rng)));
                }
            }
        }
    }

    let mut metas = Vec::with_capacity(n);
    let mut used = HashSet::new();
    let journal = n / 5;
    for k in 0..n {
        let is_journal = k < journal;
        let (dir, title_s) = if is_journal {
            // Daily notes going back from 1 October 2026.
            let (y, mo, d) = civil(20_727 - k as i64);
            (
                format!("Journal/{y}/{mo:02}"),
                format!("{y}-{mo:02}-{d:02}"),
            )
        } else {
            (rng.pick(&dirs).clone(), title(&mut rng))
        };
        // Same name in the same folder, ignoring case (macOS file systems), gets a number.
        let mut stem = title_s.clone();
        let mut i = 2;
        while !used.insert(format!("{dir}/{stem}").to_lowercase()) {
            stem = format!("{title_s} {i}");
            i += 1;
        }
        let size = if is_journal {
            (500.0 * (0.8 * rng.normal()).exp()) as usize
        } else if rng.chance(0.004) {
            100_000 + rng.below(700_000)
        } else {
            (1500.0 * (1.15 * rng.normal()).exp()) as usize
        };
        let rel = if dir.is_empty() {
            format!("{stem}.md")
        } else {
            format!("{dir}/{stem}.md")
        };
        metas.push(Meta {
            dir,
            title: title_s,
            stem,
            rel,
            journal: is_journal,
            size: size.clamp(30, 800_000),
        });
    }

    let attachments: Vec<String> = (0..n / 25)
        .map(|k| format!("Attachments/image-{k}.png"))
        .collect();
    fs::create_dir_all(root.join("Attachments"))?;
    for a in &attachments {
        let len = 5_000 + rng.below(300_000);
        let bytes: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
        fs::write(root.join(a), bytes)?;
    }

    for k in 0..n {
        let text = body(&mut rng, &metas, k, &attachments);
        let path = root.join(&metas[k].rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, text)?;
    }

    let logs = root.join("Resources/Logs");
    fs::create_dir_all(&logs)?;
    for (name, mib) in [
        ("1 MiB", 1.0),
        ("3 MiB", 3.0),
        ("5 MiB", 4.99),
        ("8 MiB", 8.0),
    ] {
        let text = pasted_log(&mut rng, (mib * 1024.0 * 1024.0) as usize);
        fs::write(logs.join(format!("Pasted log {name}.md")), text)?;
    }
    fs::write(root.join("Empty note.md"), "")?;
    fs::write(
        root.join("Windows note.md"),
        "# Windows note\r\n\r\nWritten with CRLF line endings.\r\n",
    )?;
    fs::write(root.join("Latin-1 note.md"), b"# Caf\xe9\n\nNot UTF-8.\n")?;
    for (dir, file) in [
        (".obsidian", "app.json"),
        (".obsidian", "workspace.json"),
        (".trash", "Deleted note.md"),
    ] {
        fs::create_dir_all(root.join(dir))?;
        fs::write(root.join(dir).join(file), "{}\n")?;
    }

    let total: usize = metas.iter().map(|m| m.size).sum();
    println!(
        "{n} notes (about {} MB of text), {} attachments, {} folders in {}",
        total / 1_000_000,
        attachments.len(),
        dirs.len(),
        root.display()
    );
    Ok(())
}

fn usage() -> ! {
    eprintln!("usage: pnpm vault:gen <empty-or-missing-folder> [--notes 10000] [--seed 1]");
    std::process::exit(2)
}

fn main() {
    let mut args = std::env::args().skip(1);
    let (mut root, mut notes, mut seed) = (None::<PathBuf>, 10_000usize, 1u64);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--notes" => {
                notes = args
                    .next()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or_else(|| usage())
            }
            "--seed" => {
                seed = args
                    .next()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or_else(|| usage())
            }
            _ if root.is_none() && !arg.starts_with('-') => root = Some(arg.into()),
            _ => usage(),
        }
    }
    let Some(root) = root else { usage() };
    if notes < 25 {
        eprintln!("--notes must be at least 25");
        std::process::exit(2);
    }
    // Refuse to mix generated notes into a real folder.
    if fs::read_dir(&root).is_ok_and(|mut entries| entries.next().is_some()) {
        eprintln!("{} is not empty", root.display());
        std::process::exit(1);
    }
    if let Err(e) = generate(&root, notes, seed) {
        eprintln!("could not write the vault: {e}");
        std::process::exit(1);
    }
}
