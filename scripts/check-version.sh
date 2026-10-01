#!/usr/bin/env bash
# Verifies that package.json, src-tauri/tauri.conf.json and src-tauri/Cargo.toml
# agree on the app version, and optionally that it equals the given version.
# Usage: scripts/check-version.sh [expected-version]
set -euo pipefail
cd "$(dirname "$0")/.."

pkg=$(node -p "require('./package.json').version")
conf=$(node -p "require('./src-tauri/tauri.conf.json').version")
cargo=$(sed -n 's/^version = "\(.*\)"/\1/p' src-tauri/Cargo.toml | head -n1)

if ! [[ "$pkg" == "$conf" && "$pkg" == "$cargo" ]]; then
  echo "version mismatch: package.json=$pkg tauri.conf.json=$conf Cargo.toml=$cargo" >&2
  exit 1
fi
if [[ $# -ge 1 && "$pkg" != "$1" ]]; then
  echo "version is $pkg but expected $1" >&2
  exit 1
fi
echo "$pkg"
