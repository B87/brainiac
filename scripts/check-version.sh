#!/usr/bin/env bash
# Prints the app version, which is declared only in src-tauri/Cargo.toml (Tauri
# reads it from there when tauri.conf.json has no "version"). Fails if a second
# declaration reappears in package.json or tauri.conf.json, or, given an
# argument, if the version differs from it.
# Usage: scripts/check-version.sh [expected-version]
set -euo pipefail
cd "$(dirname "$0")/.."

for file in package.json src-tauri/tauri.conf.json; do
  if [[ "$(node -p "require('./$file').version ?? ''")" != "" ]]; then
    echo "$file declares a version; remove it, src-tauri/Cargo.toml is the only source" >&2
    exit 1
  fi
done
version=$(sed -n 's/^version = "\(.*\)"/\1/p' src-tauri/Cargo.toml | head -n1)
if [[ -z "$version" ]]; then
  echo "no version found in src-tauri/Cargo.toml" >&2
  exit 1
fi
if [[ $# -ge 1 && "$version" != "$1" ]]; then
  echo "version is $version but expected $1" >&2
  exit 1
fi
echo "$version"
