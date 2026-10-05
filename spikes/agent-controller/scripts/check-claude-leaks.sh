#!/bin/bash
# Print inspect data, daemon logs, and container-home names for the Mac-side
# leak check. This script does not receive the credential and must not be
# given it on the command line.
set -euo pipefail

id=$(docker ps -q --filter label=brainiac.spike=1 | head -1)
if [ -z "$id" ]; then
  echo "FAIL no running spike container" >&2
  exit 1
fi

image_id=$(docker image inspect brainiac-spike-claude:local --format '{{.Id}}')
echo "IMAGE ${image_id}"
echo "LABELS $(docker image inspect brainiac-spike-claude:local --format '{{json .Config.Labels}}')"

echo "TTY $(docker inspect "$id" --format '{{.Config.Tty}}')"
echo "LOG $(docker inspect "$id" --format '{{.HostConfig.LogConfig.Type}}')"
echo "USER $(docker inspect "$id" --format '{{.Config.User}}')"
echo "BINDS $(docker inspect "$id" --format '{{json .HostConfig.Binds}}')"
echo "MOUNTS $(docker inspect "$id" --format '{{json .Mounts}}')"
echo "ENV $(docker inspect "$id" --format '{{json .Config.Env}}')"
echo "CMD $(docker inspect "$id" --format '{{json .Config.Cmd}}')"
echo "ENTRY $(docker inspect "$id" --format '{{json .Config.Entrypoint}}')"
echo "CONTAINER_LABELS $(docker inspect "$id" --format '{{json .Config.Labels}}')"

log="/var/lib/docker/containers/${id}/${id}-json.log"
if [ -s "$log" ]; then
  echo "RAWLOG bytes=$(wc -c < "$log")"
else
  echo "RAWLOG empty"
fi

echo "DAEMON_BEGIN"
journalctl -u brainiac-spike-controller -n 200 --no-pager --output=cat || true
echo "DAEMON_END"

echo "FILES_BEGIN"
docker exec "$id" sh -c 'find "$HOME" /tmp /workspace -type f -print 2>/dev/null | sort'
echo "FILES_END"

echo "CLAUDE_JSON_BEGIN"
docker exec "$id" sh -c 'if [ -f "$HOME/.claude.json" ]; then cat "$HOME/.claude.json"; else echo MISSING; fi'
echo "CLAUDE_JSON_END"

echo "LOGIN_BEGIN"
docker exec "$id" sh -c 'find "$HOME" -type f \( -name credentials.json -o -name .credentials.json -o -name auth.json -o -name .claude.json.bak \) -print 2>/dev/null | sort'
echo "LOGIN_END"
