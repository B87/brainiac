#!/bin/bash
# Measure a hard workspace limit. Docker's storage-opt is tried first. On
# engines that reject it, a fixed-size file is the limit that actually fails writes.
set -euo pipefail

if [ "$(id -u)" -ne 0 ]; then
  echo "FAIL quota probe must run as root" >&2
  exit 1
fi

mount=/mnt/brainiac-spike-quota
image_file=/var/lib/brainiac-spike/quota.img
cleanup() {
  umount "$mount" 2>/dev/null || true
  rm -f "$image_file"
  rmdir "$mount" 2>/dev/null || true
}
trap cleanup EXIT

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
if [ "$opt_code" -ne 0 ]; then
  echo "storage-opt rejected"
  printf '%s\n' "$opt" | head -8
else
  echo "$opt" | tail -1
  case "$opt" in
    *"storage-opt-fill-completed"*) echo "storage-opt does not enforce a 4m cap" ;;
  esac
fi

truncate -s 8M "$image_file"
mkfs.ext4 -F -q "$image_file"
mkdir -p "$mount"
mount -o loop "$image_file" "$mount"
chmod 777 "$mount"
result=$(docker run --rm --network none --entrypoint python3 \
  -v "$mount:/work" \
  brainiac-spike-stub:local -c '
import os
try:
    with open("/work/fill", "wb") as handle:
        for _ in range(10000):
            handle.write(b"x" * 65536)
    print("fill-completed")
except OSError as err:
    print("fill-errno", err.errno)
')
echo "$result"
case "$result" in
  *"fill-errno 28"*) echo "PASS quota loop-file" ;;
  *) echo "FAIL disk limit was not enforced" >&2; exit 1 ;;
esac
