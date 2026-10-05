#!/bin/bash
# Install the spike controller on the host in $SPIKE_SSH.
# The host name stays in that variable; this script does not name one.
set -euo pipefail

if [ -z "${SPIKE_SSH:-}" ]; then
  echo "set SPIKE_SSH to the lab host" >&2
  exit 2
fi

cd "$(dirname "$0")/.."
bin=/usr/local/bin/brainiac-spike
state=/var/lib/brainiac-spike
image="${RUST_IMAGE:-rust:1.88}"

ssh_cmd() {
  ssh -o BatchMode=yes -o ConnectTimeout=8 "$SPIKE_SSH" "$@"
}

user=$(ssh_cmd id -un)
group=$(ssh_cmd id -gn)
case "$user" in
  *[!a-zA-Z0-9_-]*|"") echo "unexpected remote user" >&2; exit 2 ;;
esac
case "$group" in
  *[!a-zA-Z0-9_-]*|"") echo "unexpected remote group" >&2; exit 2 ;;
esac

ssh_cmd sudo -n mkdir -p "$state/src"
ssh_cmd sudo -n chown -R "$user:$group" "$state"
ssh_cmd chmod 700 "$state"

COPYFILE_DISABLE=1 tar -C . -czf - --exclude target --exclude .git . | ssh_cmd tar -C "$state/src" -xzf -

ssh_cmd sudo -n docker build -t brainiac-spike-stub:local "$state/src/images/stub"
ssh_cmd sudo -n docker build -t brainiac-spike-claude:local "$state/src/images/claude"
ssh_cmd sudo -n docker run --rm -v "$state/src:/src" -w /src "$image" cargo build --release --locked
ssh_cmd sudo -n install -m 755 "$state/src/target/release/brainiac-spike" "$bin"

if ! ssh_cmd test -f "$state/token"; then
  ssh_cmd sh -c "umask 077; openssl rand -hex 32 > '$state/token'"
fi
ssh_cmd chmod 600 "$state/token"

install_unit() {
  local name="$1"
  sed -e "s/@USER@/${user}/" -e "s/@GROUP@/${group}/" -e "s|@BIN@|${bin}|" "deploy/${name}.service" \
    | ssh_cmd sudo -n tee "/etc/systemd/system/${name}.service" >/dev/null
}

install_unit brainiac-spike-controller
install_unit brainiac-spike-guard
ssh_cmd sudo -n systemctl daemon-reload
ssh_cmd sudo -n systemctl enable --now brainiac-spike-guard
ssh_cmd sudo -n systemctl restart brainiac-spike-controller
ssh_cmd systemctl is-active brainiac-spike-controller
echo "deployed"
