// The collector (docs/architecture.md, Agent runs — v0.5, Artifacts). After
// the run's container is confirmed stopped, this runs in a container of its
// own: the run's workspace volume mounted read only at /work, no network, no
// credential. It builds one snapshot commit of the agent's working tree,
// whose only parent is the run's start, and writes result.bundle and a
// manifest to /out.
//
// The workspace's own Git data (.git) is never trusted: the start comes from
// the input bundle copied in again, and the snapshot is built with Git data
// this script owns. Tracked files are kept even when newly ignored; new
// files follow the start commit's ignore rules and Brainiac's fixed rules,
// so an edited ignore file cannot hide output; symbolic links are recorded
// as links and never followed; executable bits and deletions are kept.
//
// Paths are relative to the container's root; the environment overrides
// exist so the same script can be tested outside a container.

import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

const WORK = process.env.BRAINIAC_WORK || "/work";
const INPUT = process.env.BRAINIAC_INPUT || "/opt/brainiac/input";
const OUT = process.env.BRAINIAC_OUT || "/out";
const SCRATCH = process.env.BRAINIAC_SCRATCH || "/scratch";

// One file over this fails the collection when it is tracked, and is left
// out when it is new (SPEC.md, Collection failed).
const MAX_FILE_BYTES = 200 * 1024 * 1024;
const MAX_TOTAL_BYTES = 4 * 1024 * 1024 * 1024;
const MAX_FILES = 200_000;
// Left-out files are listed one by one up to this many; the rest are counted.
const MAX_LISTED = 2000;
// New files and folders with these names are never collected, at any depth:
// generated output, caches, Git's own data, and home folders. Tracked files
// among them are kept.
const FIXED_RULES = new Set([
  ".git",
  "node_modules",
  "__pycache__",
  ".venv",
  "venv",
  ".cache",
  ".npm",
  ".pytest_cache",
  ".mypy_cache",
  ".ruff_cache",
  "target",
  "dist",
  "build",
  ".next",
  ".turbo",
  "coverage",
  ".DS_Store",
  ".claude",
]);
const FIXED_REASON = "Brainiac's fixed rules";

const REPO = path.join(SCRATCH, "repo.git");
const RULES = path.join(SCRATCH, "rules");
const HOME = path.join(SCRATCH, "home");
const INDEX = path.join(SCRATCH, "index");

class Failure extends Error {}

const manifest = {
  start: null,
  result: null,
  changed_files: 0,
  left_out: [],
  left_out_more: 0,
  error: null,
};

function finish(code) {
  fs.mkdirSync(OUT, { recursive: true });
  fs.writeFileSync(path.join(OUT, "manifest.json"), JSON.stringify(manifest));
  process.exit(code);
}

// Git with nothing of a user's: no system or global configuration, no
// hooks, only this script's repository and index.
const ENV = {
  PATH: process.env.PATH,
  HOME,
  GIT_CONFIG_NOSYSTEM: "1",
  GIT_DIR: REPO,
  GIT_INDEX_FILE: INDEX,
  GIT_TERMINAL_PROMPT: "0",
  GIT_AUTHOR_NAME: "Brainiac",
  GIT_AUTHOR_EMAIL: "runs@brainiac.invalid",
  GIT_COMMITTER_NAME: "Brainiac",
  GIT_COMMITTER_EMAIL: "runs@brainiac.invalid",
  LC_ALL: "C",
};

function git(args, options = {}) {
  const env = { ...ENV, ...(options.env || {}) };
  for (const name of Object.keys(env)) {
    if (env[name] === undefined) delete env[name];
  }
  const result = spawnSync(
    "git",
    ["-c", "core.hooksPath=/dev/null", "-c", "core.fsmonitor=false", ...args],
    {
      cwd: options.cwd || SCRATCH,
      env,
      input: options.input,
      maxBuffer: 1 << 30,
      stdio: ["pipe", "pipe", "pipe"],
    },
  );
  if (result.error) {
    throw new Failure(`git ${args[0]}: ${result.error.message}`);
  }
  if (result.status !== 0 && !options.allowFailure) {
    const err = result.stderr.toString("utf8").trim().split("\n").pop() || "";
    throw new Failure(`git ${args[0]} failed: ${err}`);
  }
  return result;
}

function text(args, options) {
  return git(args, options).stdout.toString("utf8").trim();
}

function isUtf8(buf) {
  return Buffer.from(buf.toString("utf8"), "utf8").equals(buf);
}

function leftOut(file, reason) {
  if (manifest.left_out.length < MAX_LISTED) {
    manifest.left_out.push({ path: file, reason });
  } else {
    manifest.left_out_more += 1;
  }
}

function main() {
  // Everything this script makes lives in scratch space of its own.
  fs.mkdirSync(SCRATCH, { recursive: true });
  for (const dir of [RULES, HOME]) {
    fs.mkdirSync(dir, { recursive: true });
  }
  fs.rmSync(REPO, { recursive: true, force: true });
  fs.rmSync(INDEX, { force: true });

  let params;
  try {
    params = JSON.parse(fs.readFileSync(path.join(INPUT, "collect.json"), "utf8"));
  } catch {
    throw new Failure("The collection's parameters are missing.");
  }
  const start = String(params.start || "");
  if (!/^[0-9a-f]{40}$/.test(start)) {
    throw new Failure("The run's start commit is not known.");
  }
  const include = new Set(
    Array.isArray(params.include) ? params.include.map(String) : [],
  );
  manifest.start = start;

  // The start: the input bundle, cloned into a bare repository of this
  // script's own (an empty template: no hooks), and checked.
  const bundle = path.join(INPUT, "input.bundle");
  if (!fs.existsSync(bundle)) {
    throw new Failure("The run's start is missing from the collector.");
  }
  const template = path.join(SCRATCH, "empty-template");
  fs.mkdirSync(template, { recursive: true });
  git(
    ["clone", "--quiet", "--bare", `--template=${template}`, bundle, REPO],
    { env: { GIT_DIR: undefined } },
  );
  if (text(["rev-parse", "--verify", "refs/heads/start^{commit}"]) !== start) {
    throw new Failure("The run's start is not the commit that was recorded.");
  }

  // Tracked paths at the start: mode and path, NUL separated.
  const tracked = new Map();
  const listing = git(["ls-tree", "-r", "-z", "start"]).stdout;
  for (const entry of listing.toString("utf8").split("\0")) {
    if (!entry) continue;
    const tab = entry.indexOf("\t");
    const [mode] = entry.slice(0, tab).split(" ");
    tracked.set(entry.slice(tab + 1), mode);
  }

  // The start commit's ignore rules, checked out on their own, decide what
  // new files are eligible: the workspace's edited ignore files do not.
  const ignoreFiles = [...tracked.keys()].filter(
    (p) => p === ".gitignore" || p.endsWith("/.gitignore"),
  );
  if (ignoreFiles.length > 0) {
    git(
      ["--work-tree", RULES, "checkout", "--quiet", "start", "--", ...ignoreFiles],
      { env: { GIT_INDEX_FILE: path.join(SCRATCH, "index-rules") } },
    );
  }

  // Walk the workspace. Entries are read as bytes so a name that is not
  // UTF-8 is refused rather than renamed.
  const files = []; // { path, kind: "file" | "link", mode, size }
  const present = new Set();
  let total = 0;
  const walk = (dir, rel) => {
    let entries;
    try {
      entries = fs.readdirSync(dir, { withFileTypes: true, encoding: "buffer" });
    } catch (e) {
      throw new Failure(`${rel || "the workspace"} could not be read: ${e.code || e.message}`);
    }
    entries.sort((a, b) => Buffer.compare(a.name, b.name));
    for (const entry of entries) {
      if (!isUtf8(entry.name)) {
        throw new Failure(`A file name in ${rel || "the workspace"} is not valid UTF-8.`);
      }
      const name = entry.name.toString("utf8");
      if (name.includes("\n")) {
        throw new Failure(`A file name in ${rel || "the workspace"} has a line break in it.`);
      }
      const relPath = rel ? `${rel}/${name}` : name;
      const full = path.join(dir, name);
      const isTracked = tracked.has(relPath);
      // .git anywhere is never collected, never looked inside, and not
      // worth listing: it is never output.
      if (name === ".git") {
        continue;
      }
      if (entry.isSymbolicLink()) {
        present.add(relPath);
        files.push({ path: relPath, kind: "link", tracked: isTracked });
        continue;
      }
      if (entry.isDirectory()) {
        if (FIXED_RULES.has(name) && !hasTrackedUnder(relPath) && !include.has(relPath)) {
          leftOut(`${relPath}/`, FIXED_REASON);
          continue;
        }
        walk(full, relPath);
        continue;
      }
      if (!entry.isFile()) {
        leftOut(relPath, "not a regular file");
        continue;
      }
      present.add(relPath);
      const stat = fs.lstatSync(full);
      files.push({
        path: relPath,
        kind: "file",
        mode: stat.mode & 0o111 ? "100755" : "100644",
        size: stat.size,
        tracked: isTracked,
        fixed: FIXED_RULES.has(name) || underFixed(relPath),
      });
      if (files.length > MAX_FILES) {
        throw new Failure(`The workspace has more than ${MAX_FILES} files.`);
      }
    }
  };
  // A tracked file inside a folder a fixed rule names (a repository that
  // tracks its build output) keeps the folder worth walking.
  const trackedDirs = new Set();
  for (const p of tracked.keys()) {
    let i = p.indexOf("/");
    while (i !== -1) {
      trackedDirs.add(p.slice(0, i));
      i = p.indexOf("/", i + 1);
    }
  }
  function hasTrackedUnder(dir) {
    return trackedDirs.has(dir);
  }
  function underFixed(p) {
    return p.split("/").slice(0, -1).some((part) => FIXED_RULES.has(part));
  }
  walk(WORK, "");

  // New files: eligible unless the start's ignore rules or the fixed rules
  // leave them out, and the user did not choose them.
  const candidates = files.filter((f) => !f.tracked);
  const ignored = new Map();
  if (candidates.length > 0 && ignoreFiles.length > 0) {
    const input = `${candidates.map((f) => f.path).join("\0")}\0`;
    const out = git(
      ["--work-tree", RULES, "check-ignore", "-z", "-v", "-n", "--no-index", "--stdin"],
      { input, allowFailure: true, cwd: RULES },
    );
    if (out.status !== 0 && out.status !== 1) {
      throw new Failure("The start commit's ignore rules could not be read.");
    }
    const fields = out.stdout.toString("utf8").split("\0");
    for (let i = 0; i + 3 < fields.length; i += 4) {
      const [source, line, pattern, p] = fields.slice(i, i + 4);
      if (source) ignored.set(p, `${source}:${line} ${pattern}`);
    }
  }

  const kept = [];
  for (const f of files) {
    if (f.tracked || include.has(f.path) || includedUnder(f.path)) {
      kept.push(f);
      continue;
    }
    if (f.fixed) {
      leftOut(f.path, FIXED_REASON);
    } else if (ignored.has(f.path)) {
      leftOut(f.path, `ignored by ${ignored.get(f.path)}`);
    } else if (f.kind === "file" && f.size > MAX_FILE_BYTES) {
      leftOut(f.path, "over the 200 MB limit for one file");
    } else {
      kept.push(f);
    }
  }
  function includedUnder(p) {
    for (const chosen of include) {
      if (p.startsWith(`${chosen}/`)) return true;
    }
    return false;
  }
  for (const f of kept) {
    if (f.kind === "file") {
      if (f.size > MAX_FILE_BYTES) {
        throw new Failure(`${f.path} is over the 200 MB limit for one file.`);
      }
      total += f.size;
      if (total > MAX_TOTAL_BYTES) {
        throw new Failure("The workspace's files are over the 4 GB limit.");
      }
    }
  }

  // Hash the kept files' bytes as they are: no filters, no text conversion.
  const regular = kept.filter((f) => f.kind === "file");
  const hashes = new Map();
  for (let i = 0; i < regular.length; i += 5000) {
    const batch = regular.slice(i, i + 5000);
    const out = text(["hash-object", "-w", "--no-filters", "--stdin-paths"], {
      input: `${batch.map((f) => f.path).join("\n")}\n`,
      cwd: WORK,
    });
    const ids = out.split("\n");
    if (ids.length !== batch.length) {
      throw new Failure("A file could not be read while it was collected.");
    }
    batch.forEach((f, j) => hashes.set(f.path, ids[j]));
  }
  for (const f of kept.filter((f) => f.kind === "link")) {
    const target = fs.readlinkSync(path.join(WORK, f.path), { encoding: "buffer" });
    hashes.set(f.path, text(["hash-object", "-w", "--stdin"], { input: target }));
  }

  // The snapshot's index: the start's tree, then every kept file at its
  // bytes now, and every tracked path that is gone removed.
  git(["read-tree", "start"]);
  const records = [];
  for (const f of kept) {
    const mode = f.kind === "link" ? "120000" : f.mode;
    records.push(`${mode} ${hashes.get(f.path)}\t${f.path}`);
  }
  for (const p of tracked.keys()) {
    if (!present.has(p)) {
      records.push(`0 0000000000000000000000000000000000000000\t${p}`);
    }
  }
  if (records.length > 0) {
    git(["update-index", "-z", "--index-info"], {
      input: `${records.join("\0")}\0`,
    });
  }
  const tree = text(["write-tree"]);
  const startTree = text(["rev-parse", "start^{tree}"]);
  if (tree === startTree) {
    // Nothing changed: the result is the start itself, and no bundle is needed.
    manifest.result = start;
    manifest.changed_files = 0;
    return;
  }
  const commit = text(
    ["commit-tree", tree, "-p", start, "-m", "Snapshot of the agent's working tree"],
  );
  git(["update-ref", "refs/heads/result", commit]);
  const changed = git(["diff-tree", "-r", "-z", "--name-only", "--no-commit-id", start, commit])
    .stdout.toString("utf8")
    .split("\0")
    .filter(Boolean).length;
  fs.mkdirSync(OUT, { recursive: true });
  // Only what the start does not have: the Mac's repository holds the rest.
  git(["bundle", "create", "--quiet", path.join(OUT, "result.bundle"), "start..refs/heads/result"]);
  manifest.result = commit;
  manifest.changed_files = changed;
}

try {
  main();
  finish(0);
} catch (e) {
  manifest.error =
    e instanceof Failure ? e.message : `The collector failed: ${e.message || e}`;
  finish(1);
}
