// `known <name>…`: which of these concepts the reader already knows
// (docs/design/code-explanations.md, How the ledger reaches the run). Brainiac
// copies the reader's known concepts into the container as a tab-separated
// file, one concept per line: its folded name, its name, its kind, and the
// words an earlier explanation used for it. This reads that file and nothing
// else: no network, no repository. Names are folded as Brainiac folds a
// concept's name (lowercase, every run of punctuation and spaces one space),
// so "Go:Embed" and "go embed" are the same name; a trailing "(kind)", as the
// prompt wrote it, is ignored. Only an exact name is known: a similar name is
// reported as new.

import fs from "node:fs";

const FILE =
  process.env.BRAINIAC_KNOWN_FILE || "/opt/brainiac/input/known-concepts.tsv";
const KINDS = new Set(["language", "library", "system", "project pattern"]);

const fold = (text) =>
  text
    .toLowerCase()
    .replace(/[^\p{L}\p{N}]+/gu, " ")
    .trim();

// "go:embed (language)" is "go:embed": the kind word the prompt's own list
// used is dropped, and nothing else.
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

const names = process.argv.slice(2);
if (names.length === 0) {
  console.log("Usage: known <name> [<name> …]");
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
  if (key && name && !known.has(key)) {
    known.set(key, { name, kind: kind ?? "", description: rest.join(" ") });
  }
}

for (const name of names) {
  const found = known.get(fold(withoutKind(name)));
  if (found) {
    const description = found.description ? ` - ${found.description}` : "";
    console.log(`known: ${found.name} (${found.kind})${description}`);
  } else {
    console.log(`new: ${name}`);
  }
}
