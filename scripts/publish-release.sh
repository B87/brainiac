#!/usr/bin/env bash
# Publishes the draft release created by the Release workflow. Publishing is
# what makes `releases/latest/download/latest.json` resolve, so installed apps
# only see the update after this step.
# Usage: scripts/publish-release.sh v0.1.0
set -euo pipefail
tag="${1:-}"
if [[ -z "$tag" ]]; then
  echo "usage: $0 <tag>" >&2
  exit 2
fi
assets=$(gh release view "$tag" --json assets --jq '.assets[].name')
for pattern in '^latest\.json$' '\.app\.tar\.gz$' '\.app\.tar\.gz\.sig$' '\.dmg$'; do
  if ! grep -qE "$pattern" <<<"$assets"; then
    echo "release $tag is missing an asset matching $pattern; was the build signed?" >&2
    exit 1
  fi
done
if [[ "$tag" == *-* ]]; then
  gh release edit "$tag" --draft=false --prerelease
else
  gh release edit "$tag" --draft=false --latest
fi
gh release view "$tag" --web
