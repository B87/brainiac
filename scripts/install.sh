#!/usr/bin/env bash
# Downloads a Brainiac release and installs it into /Applications.
#
#   scripts/install.sh                 # latest published release
#   scripts/install.sh v0.1.0          # a specific tag (drafts work when `gh` is logged in)
#   scripts/install.sh --dmg path.dmg  # install a DMG you already have (e.g. a local build)
#
# Or without cloning:
#   curl -fsSL https://raw.githubusercontent.com/B87/brainiac/main/scripts/install.sh | bash -s -- v0.1.0
#
# Options: --dest <dir> (default /Applications), --no-launch
set -euo pipefail

REPO="B87/brainiac"
APP="Brainiac.app"
dest="/Applications"
tag=""
dmg=""
launch=1

while [[ $# -gt 0 ]]; do
  case "$1" in
    --dmg) dmg="$2"; shift 2 ;;
    --dest) dest="$2"; shift 2 ;;
    --no-launch) launch=0; shift ;;
    -h|--help) sed -n '2,12p' "$0"; exit 0 ;;
    v*) tag="$1"; shift ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "Brainiac only runs on macOS" >&2
  exit 1
fi

tmp=$(mktemp -d -t brainiac-install)
mount_point=""
cleanup() {
  if [[ -n "$mount_point" ]]; then hdiutil detach -quiet "$mount_point" || true; fi
  rm -rf "$tmp"
}
trap cleanup EXIT

# --- Get the DMG --------------------------------------------------------------
if [[ -z "$dmg" ]]; then
  if command -v gh >/dev/null && gh auth status >/dev/null 2>&1; then
    # gh can see draft releases and does not hit the anonymous API rate limit.
    if [[ -z "$tag" ]]; then
      tag=$(gh release view --repo "$REPO" --json tagName --jq .tagName)
    fi
    echo "Downloading $tag with gh..."
    gh release download "$tag" --repo "$REPO" --pattern '*.dmg' --dir "$tmp"
  else
    if [[ -z "$tag" ]]; then
      tag=$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" \
        | sed -n 's/^ *"tag_name": *"\([^"]*\)".*/\1/p' | head -n1)
      if [[ -z "$tag" ]]; then
        echo "could not determine the latest release of $REPO" >&2
        exit 1
      fi
    fi
    version="${tag#v}"
    url="https://github.com/$REPO/releases/download/$tag/Brainiac_${version}_universal.dmg"
    echo "Downloading $url..."
    curl -fL --progress-bar -o "$tmp/Brainiac.dmg" "$url"
  fi
  for candidate in "$tmp"/*.dmg; do dmg="$candidate"; break; done
fi
if [[ ! -f "$dmg" ]]; then
  echo "no DMG found at $dmg" >&2
  exit 1
fi

# --- Mount, copy, unmount -----------------------------------------------------
echo "Mounting $(basename "$dmg")..."
mount_point=$(hdiutil attach -nobrowse -readonly -noautoopen "$dmg" \
  | awk -F'\t' '/\/Volumes\// { print $NF }' | tail -n1)
if [[ -z "$mount_point" || ! -d "$mount_point/$APP" ]]; then
  echo "the DMG does not contain $APP" >&2
  exit 1
fi

if pgrep -xqi brainiac; then
  echo "Quitting the running Brainiac..."
  osascript -e 'tell application "Brainiac" to quit' >/dev/null 2>&1 || pkill -x Brainiac || true
  sleep 1
fi

mkdir -p "$dest"
if [[ -d "${dest}/${APP}" ]]; then
  echo "Replacing ${dest}/${APP}..."
  rm -rf "${dest:?}/${APP:?}"
fi
# ditto preserves bundle metadata and code signature; cp -R does not always.
ditto "$mount_point/$APP" "${dest}/${APP}"

# Releases are ad-hoc signed; without this Gatekeeper refuses to open the app.
xattr -dr com.apple.quarantine "${dest}/${APP}" 2>/dev/null || true

installed=$(defaults read "${dest}/${APP}/Contents/Info" CFBundleShortVersionString 2>/dev/null || echo "?")
echo "Installed Brainiac $installed to ${dest}/${APP}"

if [[ "$launch" -eq 1 ]]; then
  open "${dest}/${APP}"
fi
