#!/bin/bash
# Workspace cap inside the engine VM. storage-opt applies to the container
# writable layer. The loop file lives on a Docker volume, so both sit on the
# VM disk rather than a folder shared from the Mac.
set -uo pipefail

vol=brainiac-spike-vm-quota
writer=brainiac-spike-vm-writer
cleanup() {
  docker rm -f "$writer" >/dev/null 2>&1 || true
  docker volume rm -f "$vol" >/dev/null 2>&1 || true
}
trap cleanup EXIT
docker rm -f "$writer" >/dev/null 2>&1 || true

echo "engine $(docker info --format '{{.OperatingSystem}} driver {{.Driver}}')"

set +e
opt=$(docker run --rm --network none --entrypoint python3 --storage-opt size=4m \
  brainiac-spike-stub:local -c '
import os
written = 0
try:
    with open("/tmp/fill", "wb") as handle:
        chunk = b"x" * (1024 * 1024)
        for _ in range(64):
            handle.write(chunk)
            written += len(chunk)
        handle.flush()
        os.fsync(handle.fileno())
    print("storage-opt-fill-completed", written)
except OSError as err:
    print("storage-opt-errno", err.errno, written)
' 2>&1)
opt_code=$?
set -e
printf '%s\n' "$opt" | tail -4
storage_enforced=0
if [ "$opt_code" -ne 0 ]; then
  echo "storage-opt rejected"
elif printf '%s\n' "$opt" | grep -q "storage-opt-fill-completed"; then
  echo "storage-opt does not enforce a 4m cap"
elif printf '%s\n' "$opt" | grep -q "storage-opt-errno"; then
  echo "storage-opt enforced"
  storage_enforced=1
else
  echo "storage-opt returned no fill result"
fi

docker volume create "$vol" >/dev/null
set +e
loop=$(docker run -i --name "$writer" --privileged \
  -v "$vol":/work debian:bookworm-slim bash -s <<'EOS'
set -eu
apt-get update -qq
apt-get install -y -qq --no-install-recommends e2fsprogs >/dev/null
truncate -s 8M /work/quota.img
mkfs.ext4 -F -q /work/quota.img
mkdir -p /mnt
mount -o loop /work/quota.img /mnt
if dd if=/dev/zero of=/mnt/fill bs=65536 count=1000 status=none 2>/tmp/dd.err; then
  echo "loop-fill-completed"
else
  if grep -q "No space left" /tmp/dd.err; then
    echo "loop-enospc"
  else
    echo "loop-failed"
    cat /tmp/dd.err
  fi
fi
EOS
)
loop_code=$?
set -e
printf '%s\n' "$loop" | grep -E '^(loop-|mount:|mkfs|dd:)' || true
loop_enforced=0
if printf '%s\n' "$loop" | grep -q "loop-enospc"; then
  echo "loop enforced"
  loop_enforced=1
else
  echo "loop did not enforce (docker exit $loop_code)"
  printf '%s\n' "$loop" | tail -12
fi

if [ "$storage_enforced" -eq 1 ] || [ "$loop_enforced" -eq 1 ]; then
  echo "PASS vm-quota"
else
  echo "FAIL vm-quota" >&2
  exit 1
fi
