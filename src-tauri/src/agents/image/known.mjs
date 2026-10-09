// `known`: which of these concepts the reader already knows
// (docs/design/code-explanations.md, How the ledger reaches the run). Brainiac
// copies the reader's known concepts into the container as a tab-separated
// file, one concept per line: its folded name, its name, its kind, and the
// words an earlier explanation of this repository used. Names arrive one per
// line on standard input, so a name can contain spaces or punctuation.
// Arguments are the same, one name each. This reads that file and nothing
// else: no network, no repository.
//
// A name is folded as Brainiac folds it (`concept_key`): one code point at a
// time, kept when it is Unicode Alphabetic or Number, then lowercased on its
// own. Folding the whole string would turn a final sigma into ς and drop the
// dot in İ. A trailing "(kind)", as an earlier prompt wrote it, is ignored.
// Only an exact name is known. A name with no exact match is reported as
// new, followed by up to three `similar:` lines: known concepts whose words
// include all of this name's words, or the reverse, after dropping small
// words and a plural "s" ("Closure" and "Closures", "Result and ?" and
// "Result and the ? operator"). They are candidates for the agent to judge,
// never a match. One name can be two kinds, and both are printed.

import fs from "node:fs";

const FILE =
  process.env.BRAINIAC_KNOWN_FILE || "/opt/brainiac/input/known-concepts.tsv";
const KINDS = new Set([
  "language",
  "library",
  "protocol",
  "tool",
  "technique",
  "system",
  "project pattern",
]);
const ALPHABETIC = /^\p{Alphabetic}$/u;
const NUMBER = /^\p{N}$/u;

function fold(text) {
  let key = "";
  let gap = false;
  for (const c of text) {
    if (ALPHABETIC.test(c) || NUMBER.test(c)) {
      if (gap && key.length > 0) key += " ";
      gap = false;
      key += c.toLowerCase();
    } else {
      gap = true;
    }
  }
  return key;
}

// "go:embed (language)" is "go:embed": the kind word is dropped, and
// nothing else.
function withoutKind(name) {
  const trimmed = name.trim();
  const open = trimmed.lastIndexOf("(");
  if (open >= 0 && trimmed.endsWith(")")) {
    const inner = trimmed.slice(open + 1, -1).trim().toLowerCase();
    if (KINDS.has(inner)) {
      return trimmed.slice(0, open).trimEnd();
    }
  }
  return trimmed;
}

function readNames() {
  const args = process.argv.slice(2);
  if (args.length > 0) return args;
  const input = fs.readFileSync(0, "utf8");
  const lines = input.split("\n");
  if (lines.length > 0 && lines[lines.length - 1] === "") lines.pop();
  return lines.map((line) => line.replace(/\r$/, ""));
}

const SMALL = new Set(["a", "an", "and", "the", "of", "in", "with", "to", "for", "on", "or", "is", "as", "by"]);
const MAX_SIMILAR = 3;

// The words of a folded name, without small words and a plural "s".
function words(key) {
  const out = new Set();
  for (const w of key.split(" ")) {
    if (!w || SMALL.has(w)) continue;
    out.add(w.length > 3 && w.endsWith("s") && !w.endsWith("ss") ? w.slice(0, -1) : w);
  }
  return out;
}

// Known concepts sharing every word of the smaller name, closest first.
function similar(key, known) {
  const mine = words(key);
  if (mine.size === 0) return [];
  const found = [];
  for (const [other, list] of known) {
    const theirs = words(other);
    if (theirs.size === 0) continue;
    const [small, large] = mine.size <= theirs.size ? [mine, theirs] : [theirs, mine];
    let shared = 0;
    for (const w of small) if (large.has(w)) shared++;
    if (shared < small.size) continue;
    found.push({ score: shared / (mine.size + theirs.size - shared), list });
  }
  found.sort((a, b) => b.score - a.score);
  return found.slice(0, MAX_SIMILAR).flatMap((f) => f.list);
}

const names = readNames();
if (names.length === 0) {
  console.log(
    "Usage: run `known` with one concept name per line on standard input.",
  );
  process.exit(2);
}

let text = "";
try {
  text = fs.readFileSync(FILE, "utf8");
} catch {
  console.log(
    "There is no list of known concepts for this run: treat every concept as new.",
  );
  process.exit(0);
}

const known = new Map();
for (const line of text.split("\n")) {
  const [key, name, kind, ...rest] = line.split("\t");
  if (!key || !name) continue;
  const list = known.get(key) ?? [];
  const word = kind ?? "";
  if (!list.some((item) => item.kind === word)) {
    list.push({ name, kind: word, description: rest.join(" ") });
    known.set(key, list);
  }
}

for (const name of names) {
  const matches = known.get(fold(withoutKind(name)));
  if (matches && matches.length > 0) {
    for (const found of matches) {
      const description = found.description ? ` - ${found.description}` : "";
      console.log(`known: ${found.name} (${found.kind})${description}`);
    }
  } else {
    console.log(`new: ${name}`);
    for (const found of similar(fold(withoutKind(name)), known)) {
      const description = found.description ? ` - ${found.description}` : "";
      console.log(`similar: ${found.name} (${found.kind})${description}`);
    }
  }
}
