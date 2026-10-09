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
// Only an exact name is known: a similar name is reported as new. One name
// can be two kinds, and both are printed.

import fs from "node:fs";

const FILE =
  process.env.BRAINIAC_KNOWN_FILE || "/opt/brainiac/input/known-concepts.tsv";
const KINDS = new Set([
  "language",
  "library",
  "protocol",
  "tool",
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
  }
}
