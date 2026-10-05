#!/bin/bash
# Stop a workload, collect its working tree from a read-only mount, then
# remove that volume. The controller service is not part of the run.
set -euo pipefail

if [ "$(id -u)" -ne 0 ]; then
  echo "FAIL collect must run as root" >&2
  exit 1
fi

vol=brainiac-spike-collect
reader=brainiac-spike-collect-reader

cleanup() {
  docker rm -f "$reader" >/dev/null 2>&1 || true
  docker volume rm -f "$vol" >/dev/null 2>&1 || true
}
trap cleanup EXIT

docker rm -f "$reader" >/dev/null 2>&1 || true
docker volume rm -f "$vol" >/dev/null 2>&1 || true
docker volume create "$vol" >/dev/null

# The writer may use the network to install Git. It exits before collection.
# Nothing here is a repository from the host.
docker run -i --rm -v "$vol:/work" debian:bullseye bash -s <<'EOS'
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive
cat > /etc/apt/sources.list <<'EOF'
deb http://archive.debian.org/debian bullseye main
deb http://archive.debian.org/debian-security bullseye-security main
EOF
echo 'Acquire::Check-Valid-Until "false";' > /etc/apt/apt.conf.d/99no-check-valid
apt-get update -qq
apt-get install -y -qq git >/dev/null
git config --global user.email spike@example.com
git config --global user.name spike
cd /work
git init -q
echo base > tracked.txt
git add tracked.txt
git commit -q -m base
echo edited > tracked.txt
git add tracked.txt
echo created > extra.txt
printf 'bin\000\377' > blob.bin
ln -s extra.txt link
test "$(git rev-list --count HEAD)" = 1
echo "WRITER commits $(git rev-list --count HEAD)"
EOS

# The collector has no network and cannot write the volume. It does not run
# the workload image's entrypoint or Git.
docker run --name "$reader" --network none -v "$vol:/work:ro" \
  --entrypoint python3 brainiac-spike-stub:local -c '
import hashlib, os
root = "/work"
for dirpath, dirnames, filenames in os.walk(root, followlinks=False):
    if ".git" in dirnames:
        dirnames.remove(".git")
        print("exclude .git")
    for name in sorted(filenames):
        path = os.path.join(dirpath, name)
        rel = os.path.relpath(path, root)
        if os.path.islink(path):
            print("symlink", rel, os.readlink(path))
            continue
        if not os.path.isfile(path):
            print("skip", rel)
            continue
        data = open(path, "rb").read()
        print("file", rel, hashlib.sha256(data).hexdigest(), len(data))
'

mode=$(docker inspect -f '{{.HostConfig.NetworkMode}}' "$reader")
rw=$(docker inspect -f '{{range .Mounts}}{{if eq .Destination "/work"}}{{.RW}}{{end}}{{end}}' "$reader")
if [ "$mode" != "none" ] || [ "$rw" != "false" ]; then
  echo "FAIL collector network=$mode writable=$rw" >&2
  exit 1
fi

manifest=$(docker logs "$reader" 2>&1)
edited=$(printf 'edited\n' | sha256sum | awk '{print $1}')
created=$(printf 'created\n' | sha256sum | awk '{print $1}')
blob=$(printf 'bin\000\377' | sha256sum | awk '{print $1}')
base=$(printf 'base\n' | sha256sum | awk '{print $1}')

require() {
  if ! printf '%s\n' "$manifest" | grep -q -F "$1"; then
    echo "FAIL manifest missing: $1" >&2
    printf '%s\n' "$manifest" >&2
    exit 1
  fi
}
require "exclude .git"
require "file tracked.txt $edited"
require "file extra.txt $created"
require "file blob.bin $blob"
require "symlink link extra.txt"
if printf '%s\n' "$manifest" | grep -q -F "file tracked.txt $base"; then
  echo "FAIL collector kept the committed bytes instead of the edit" >&2
  exit 1
fi
echo "collector network=none mount=ro"
echo "$manifest"

docker rm -f "$reader" >/dev/null
docker volume rm "$vol" >/dev/null
if docker volume inspect "$vol" >/dev/null 2>&1; then
  echo "FAIL volume remained after cleanup" >&2
  exit 1
fi
active=$(systemctl is-active brainiac-spike-controller || true)
if [ "$active" != "active" ]; then
  echo "FAIL controller is $active" >&2
  exit 1
fi
echo "PASS collect"
trap - EXIT
