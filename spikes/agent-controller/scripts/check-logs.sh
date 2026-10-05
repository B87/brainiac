#!/bin/bash
# Fail if the running spike container has a Docker json log. Logging was
# set to none, so the daemon's default json-file driver must not apply.
set -euo pipefail
id=$(docker ps -q --filter label=brainiac.spike=1 | head -1)
if [ -z "$id" ]; then
  echo "FAIL no running spike container" >&2
  exit 1
fi
log="/var/lib/docker/containers/${id}/${id}-json.log"
if [ -s "$log" ]; then
  echo "FAIL raw log has content" >&2
  exit 1
fi
echo "PASS no raw log file"
