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
for pattern in '^latest\.json$' '\.app\.tar\.gz$' '\.app\.tar\.gz\.sig$' '\.dmg$' \
  '^brainiac-runner-linux-amd64$' '^brainiac-runner-linux-arm64$'; do
  if ! grep -qE "$pattern" <<<"$assets"; then
    echo "release $tag is missing an asset matching $pattern; did every build job finish and sign?" >&2
    exit 1
  fi
done
# The app downloads a run controller only when its SHA-256 is the one compiled
# into it, so a controller asset from another run of the workflow would leave
# this version unable to set up a host. Check each against the app's program.
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
gh release download "$tag" --dir "$work" --pattern '*.app.tar.gz' \
  --pattern 'brainiac-runner-linux-amd64' --pattern 'brainiac-runner-linux-arm64'
tar -xzf "$work"/*.app.tar.gz -C "$work"
program=$(find "$work" -path '*.app/Contents/MacOS/*' -type f -perm -u+x | head -n 1)
if [[ -z "$program" ]]; then
  echo "release $tag: no program found in the app archive" >&2
  exit 1
fi
for arch in amd64 arm64; do
  digest=$(shasum -a 256 "$work/brainiac-runner-linux-$arch" | cut -d' ' -f1)
  if ! grep -qaF "$digest" "$program"; then
    echo "release $tag: brainiac-runner-linux-$arch (SHA-256 $digest) is not the controller the app was built with" >&2
    exit 1
  fi
done

if [[ "$tag" == *-* ]]; then
  gh release edit "$tag" --draft=false --prerelease
else
  gh release edit "$tag" --draft=false --latest
fi
gh release view "$tag" --web
