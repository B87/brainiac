#!/usr/bin/env bash
# Runs a command with scripts/codesign-run.sh installed as the Cargo runner for
# this Mac's target. `pnpm tauri dev` then signs the binary when
# BRAINIAC_CODESIGN_IDENTITY is set. Cargo test does not go through here.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)

if [[ "$(uname -s)" == "Darwin" ]]; then
  host=$(rustc -vV | awk '/^host: / { print $2 }')
  if [[ -z "$host" ]]; then
    echo "rustc did not report its host target" >&2
    exit 1
  fi
  # aarch64-apple-darwin -> CARGO_TARGET_AARCH64_APPLE_DARWIN_RUNNER
  upper=$(printf '%s' "$host" | tr '[:lower:]-' '[:upper:]_')
  export "CARGO_TARGET_${upper}_RUNNER=${root}/scripts/codesign-run.sh"
fi

exec "$@"
