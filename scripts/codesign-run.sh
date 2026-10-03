#!/usr/bin/env bash
# Cargo runner for local development. When BRAINIAC_CODESIGN_IDENTITY is set,
# sign the binary with that certificate before executing it, so Keychain sees
# the same program across rebuilds (docs/keychain-access.md). With the variable
# unset this only executes the binary, which is what CI and other machines do.
set -euo pipefail

if [[ $# -lt 1 ]]; then
  echo "usage: $0 <binary> [args...]" >&2
  exit 2
fi

binary=$1
shift

# Same identifier as the packaged app (tauri.conf.json). Keychain trusts the
# certificate plus this identifier, so one "Always Allow" covers dev and the
# installed build.
identifier="dev.brainiac.desktop"

if [[ -n "${BRAINIAC_CODESIGN_IDENTITY:-}" ]]; then
  echo "Signing $(basename "$binary") with ${BRAINIAC_CODESIGN_IDENTITY}" >&2
  codesign --force --sign "$BRAINIAC_CODESIGN_IDENTITY" \
    --identifier "$identifier" "$binary"
fi

exec "$binary" "$@"
