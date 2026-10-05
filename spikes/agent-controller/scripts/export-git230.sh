#!/bin/bash
# Arbitrary-commit export with the Git that debian:bullseye ships (2.30.x).
# The source repo is created inside this container. No host repository is used.
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive
# Bullseye has moved to the archive. The live security mirror 404s the git package.
cat > /etc/apt/sources.list <<'EOF'
deb http://archive.debian.org/debian bullseye main
deb http://archive.debian.org/debian-security bullseye-security main
EOF
echo 'Acquire::Check-Valid-Until "false";' > /etc/apt/apt.conf.d/99no-check-valid
apt-get update -qq
apt-get install -y -qq git >/dev/null
version=$(git --version)
echo "git-version $version"
case "$version" in
  "git version 2.30"*) ;;
  *) echo "FAIL expected Git 2.30, got $version" >&2; exit 1 ;;
esac

export GIT_CONFIG_NOSYSTEM=1
export GIT_NO_LAZY_FETCH=1
export GIT_TERMINAL_PROMPT=0
export GIT_OPTIONAL_LOCKS=0
export HOME
HOME=$(mktemp -d)

flag() {
  if git -c "$1" --version >/dev/null 2>&1; then
    echo "flag-ok $1"
  else
    echo "flag-rejected $1"
  fi
}
flag "core.hooksPath=/dev/null"
flag "gc.auto=0"
flag "maintenance.auto=false"
flag "transfer.bundleURI=false"

home=$(mktemp -d)
HOME="$home" git config --global user.name from-home
empty=$(mktemp)
got=$(HOME="$home" GIT_CONFIG_GLOBAL="$empty" git config --global --get user.name || true)
if [ "$got" = "from-home" ]; then
  echo "flag-ignored GIT_CONFIG_GLOBAL"
else
  echo "flag-honored GIT_CONFIG_GLOBAL ($got)"
fi

configs=(-c core.hooksPath=/dev/null -c gc.auto=0)
if git -c maintenance.auto=false --version >/dev/null 2>&1; then
  configs+=(-c maintenance.auto=false)
fi

git230() {
  git --no-replace-objects "${configs[@]}" "$@"
}

# Hooks must not run when core.hooksPath is set.
hook_repo=$(mktemp -d)
git230 init -q "$hook_repo"
echo x > "$hook_repo/file"
git230 -C "$hook_repo" add file
mkdir -p "$hook_repo/.git/hooks"
printf '#!/bin/sh\ntouch /tmp/hook-ran\n' > "$hook_repo/.git/hooks/pre-commit"
chmod +x "$hook_repo/.git/hooks/pre-commit"
rm -f /tmp/hook-ran
git230 -C "$hook_repo" -c user.email=spike@example.com -c user.name=spike commit -q -m hooked
if [ -f /tmp/hook-ran ]; then
  echo "FAIL hooks ran" >&2
  exit 1
fi
echo "flag-ok hooks not run"

snapshot() {
  (
    cd "$1"
    find .git -type f \
      ! -path './.git/objects/*' \
      ! -path './.git/hooks/*' \
      -print0 | sort -z | xargs -0 -r sha256sum
  )
}

src=$(mktemp -d)
git230 init -q "$src"
git230 -C "$src" config user.email spike@example.com
git230 -C "$src" config user.name spike
echo first > "$src/file"
git230 -C "$src" add file
git230 -C "$src" commit -q -m first
first=$(git230 -C "$src" rev-parse HEAD)
echo second >> "$src/file"
git230 -C "$src" add file
git230 -C "$src" commit -q -m second
second=$(git230 -C "$src" rev-parse HEAD)
# The branch moves after the commit to export has been chosen.
git230 -C "$src" update-ref HEAD "$second"
before=$(snapshot "$src")
head_before=$(git230 -C "$src" rev-parse HEAD)

pack=$(mktemp)
git230 -C "$src" rev-list --objects "$first" | awk '{print $1}' | git230 -C "$src" pack-objects --stdout > "$pack"
dest=$(mktemp -d)
git230 init -q --bare "$dest"
git230 -C "$dest" index-pack --stdin < "$pack" >/dev/null
git230 -C "$dest" update-ref refs/brainiac/start "$first"
got=$(git230 -C "$dest" rev-parse refs/brainiac/start)
if [ "$got" != "$first" ]; then
  echo "FAIL export ref is $got, wanted $first" >&2
  exit 1
fi
if [ "$(git230 -C "$src" rev-parse HEAD)" != "$head_before" ]; then
  echo "FAIL source HEAD changed" >&2
  exit 1
fi
after=$(snapshot "$src")
if [ "$before" != "$after" ]; then
  echo "FAIL source refs, index, or config changed" >&2
  exit 1
fi
echo "PASS export kept $first while HEAD is $second"

# A raw object id fetch is recorded either way. It must not change the source.
fetch_dest=$(mktemp -d)
git230 init -q --bare "$fetch_dest"
set +e
git230 -C "$fetch_dest" fetch --no-tags "$src" "${first}:refs/brainiac/start" >/tmp/fetch.err 2>&1
fetch_code=$?
set -e
if [ "$fetch_code" -eq 0 ]; then
  echo "fetch-ok"
else
  echo "fetch-failed"
  head -3 /tmp/fetch.err || true
fi
after_fetch=$(snapshot "$src")
if [ "$before" != "$after_fetch" ]; then
  echo "FAIL fetch changed the source" >&2
  exit 1
fi

# A missing object fails here, instead of selecting another commit.
missing=$(mktemp -d)
git230 init -q "$missing"
echo blob > "$missing/file"
git230 -C "$missing" config user.email spike@example.com
git230 -C "$missing" config user.name spike
git230 -C "$missing" add file
git230 -C "$missing" commit -q -m only
oid=$(git230 -C "$missing" rev-parse HEAD)
blob=$(git230 -C "$missing" rev-parse HEAD:file)
rm -f "$missing/.git/objects/${blob:0:2}/${blob:2}"
set +e
git230 -C "$missing" rev-list --objects "$oid" >/dev/null 2>/tmp/missing.err
missing_code=$?
set -e
if [ "$missing_code" -eq 0 ]; then
  echo "FAIL missing object was packed" >&2
  exit 1
fi
echo "PASS missing object failed locally"
echo "PASS export"
