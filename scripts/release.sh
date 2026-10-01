#!/usr/bin/env bash
# Cuts a release: bumps the version in src-tauri/Cargo.toml (its only
# declaration), turns the Unreleased section of CHANGELOG.md into the release's
# section, commits, tags and pushes. The push of the tag starts .github/workflows/release.yml, which builds
# the signed macOS bundle and opens a draft GitHub release.
#
# Usage: scripts/release.sh <version>        e.g. scripts/release.sh 0.1.0
#        scripts/release.sh <version> --dry-run
set -euo pipefail
cd "$(dirname "$0")/.."

version="${1:-}"
dry_run="${2:-}"
if [[ -z "$version" ]]; then
  echo "usage: $0 <version> [--dry-run]" >&2
  exit 2
fi
if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]]; then
  echo "version must look like 1.2.3 or 1.2.3-beta.1" >&2
  exit 2
fi
tag="v$version"

# --- Preconditions ------------------------------------------------------------
if [[ -n "$(git status --porcelain)" ]]; then
  echo "working tree is not clean; commit or stash first" >&2
  exit 1
fi
branch=$(git rev-parse --abbrev-ref HEAD)
if [[ "$branch" != "main" ]]; then
  echo "releases are cut from main (currently on $branch)" >&2
  exit 1
fi
if git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
  echo "tag $tag already exists" >&2
  exit 1
fi
if grep -q "^## \[$version\]" CHANGELOG.md; then
  echo "CHANGELOG.md already has a '## [$version]' section" >&2
  exit 1
fi
# The release notes are whatever sits under Unreleased; refuse to ship none.
if ! awk '/^## \[Unreleased\]/ { inside = 1; next } /^## \[/ { inside = 0 } inside && /[^[:space:]]/ { found = 1 } END { exit !found }' CHANGELOG.md; then
  echo "CHANGELOG.md has nothing under '## [Unreleased]'; write the release notes first" >&2
  exit 1
fi
git fetch --quiet origin main
if [[ "$(git rev-parse HEAD)" != "$(git rev-parse origin/main)" ]]; then
  echo "local main differs from origin/main; pull or push first" >&2
  exit 1
fi

# --- Bump the version ----------------------------------------------------------
# Only the first `version =` line is the package version; dependency pins come later.
# perl rather than sed: the first-match address `0,/re/` is GNU-only and BSD sed ignores it.
perl -0pi -e 's/^version = "[^"]*"/version = "'"$version"'"/m' src-tauri/Cargo.toml
# Refresh Cargo.lock's entry for this crate without touching dependency versions.
cargo update --workspace --offline --manifest-path src-tauri/Cargo.toml --quiet
scripts/check-version.sh "$version" >/dev/null

# --- Promote the changelog -----------------------------------------------------
# "## [Unreleased]" gains a dated "## [X.Y.Z]" heading below it, and the
# comparison links at the bottom move on by one version.
# shellcheck disable=SC2016 # ${...} below is JavaScript, not shell.
node -e '
  const fs = require("fs");
  const [version, date] = process.argv.slice(1);
  let text = fs.readFileSync("CHANGELOG.md", "utf8");
  text = text.replace("## [Unreleased]\n", `## [Unreleased]\n\n## [${version}] - ${date}\n`);
  const link = /^\[Unreleased\]: (.*)\/compare\/(v\S+?)\.\.\.HEAD$/m;
  const match = text.match(link);
  if (!match) throw new Error("CHANGELOG.md: no [Unreleased]: .../compare/vX.Y.Z...HEAD link");
  const [line, base, previous] = match;
  text = text.replace(line,
    `[Unreleased]: ${base}/compare/v${version}...HEAD\n[${version}]: ${base}/compare/${previous}...v${version}`);
  fs.writeFileSync("CHANGELOG.md", text.replace(/\n{3,}/g, "\n\n"));
' "$version" "$(date +%Y-%m-%d)"

if [[ "$dry_run" == "--dry-run" ]]; then
  echo "dry run: would commit, tag $tag and push. Changes:"
  git --no-pager diff --stat
  git --no-pager diff -- CHANGELOG.md
  git checkout -- .
  exit 0
fi

# --- Commit, tag, push --------------------------------------------------------
if [[ -n "$(git status --porcelain)" ]]; then
  git add src-tauri/Cargo.toml src-tauri/Cargo.lock CHANGELOG.md
  git commit --quiet -m "Release $tag"
fi
git tag -a "$tag" -m "Brainiac $tag"
git push --quiet origin main "$tag"

echo "pushed $tag. Watch the build with:  gh run watch"
echo "When it finishes, review the draft and publish with:  scripts/publish-release.sh $tag"
