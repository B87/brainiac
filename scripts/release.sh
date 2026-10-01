#!/usr/bin/env bash
# Cuts a release: bumps the version in the three manifests, commits, tags and
# pushes. The push of the tag starts .github/workflows/release.yml, which builds
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
if ! grep -q "^## \[$version\]" CHANGELOG.md; then
  echo "CHANGELOG.md has no '## [$version]' section; write the release notes first" >&2
  exit 1
fi
git fetch --quiet origin main
if [[ "$(git rev-parse HEAD)" != "$(git rev-parse origin/main)" ]]; then
  echo "local main differs from origin/main; pull or push first" >&2
  exit 1
fi

# --- Bump the three manifests -------------------------------------------------
node -e '
  const fs = require("fs");
  for (const file of ["package.json", "src-tauri/tauri.conf.json"]) {
    const json = JSON.parse(fs.readFileSync(file, "utf8"));
    json.version = process.argv[1];
    fs.writeFileSync(file, JSON.stringify(json, null, 2) + "\n");
  }
' "$version"
# Only the first `version =` line is the package version; dependency pins come later.
sed -i '' "0,/^version = \".*\"/s//version = \"$version\"/" src-tauri/Cargo.toml
# Refresh Cargo.lock's entry for this crate without touching dependency versions.
cargo update --workspace --offline --manifest-path src-tauri/Cargo.toml --quiet
scripts/check-version.sh "$version" >/dev/null

if [[ "$dry_run" == "--dry-run" ]]; then
  echo "dry run: would commit, tag $tag and push. Changes:"
  git --no-pager diff --stat
  git checkout -- .
  exit 0
fi

# --- Commit, tag, push --------------------------------------------------------
if [[ -n "$(git status --porcelain)" ]]; then
  git add package.json src-tauri/tauri.conf.json src-tauri/Cargo.toml src-tauri/Cargo.lock
  git commit --quiet -m "Release $tag"
fi
git tag -a "$tag" -m "Brainiac $tag"
git push --quiet origin main "$tag"

echo "pushed $tag. Watch the build with:  gh run watch"
echo "When it finishes, review the draft and publish with:  scripts/publish-release.sh $tag"
