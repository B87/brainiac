#!/usr/bin/env bash
# How a change moved Brainiac's modules: the module graph, dependencies, the
# boundary test's exceptions, and sizes, before and after, as Markdown
# (docs/design/codebase-maps.md, The experiment on Brainiac itself).
#
#   pnpm map:diff                 the working tree against its merge base with origin/main
#   pnpm map:diff <base>          the working tree against <base>
#   pnpm map:diff <base> <head>   two commits
#
# Commits are read with `git archive` into a temporary folder, so nothing in
# the repository changes. The working tree includes uncommitted edits.
set -euo pipefail

root=$(git rev-parse --show-toplevel)
base=${1:-$(git -C "$root" merge-base origin/main HEAD)}
head=${2:-}

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# The tree the map reads: `src-tauri/src` and the boundary test's lists. A
# commit from before the test has no lists, so its modules show as unplaced.
extract() {
  mkdir -p "$2"
  git -C "$root" archive "$1" src-tauri/src | tar -x -C "$2"
  mkdir -p "$2/src-tauri/tests"
  git -C "$root" show "$1:src-tauri/tests/module_boundaries.rs" \
    >"$2/src-tauri/tests/module_boundaries.rs" 2>/dev/null || true
}

extract "$base" "$tmp/base"
base_label=$(git -C "$root" rev-parse --short "$base")
if [ -n "$head" ]; then
  extract "$head" "$tmp/head"
  head_dir="$tmp/head/src-tauri"
  head_label="\`$(git -C "$root" rev-parse --short "$head")\`"
else
  head_dir="$root/src-tauri"
  head_label="the working tree"
fi

cargo run --quiet --manifest-path "$root/src-tauri/Cargo.toml" --example module_map -- \
  diff "$tmp/base/src-tauri" "$head_dir" --base-label "\`$base_label\`" --head-label "$head_label"
