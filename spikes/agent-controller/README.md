# Agent-controller spike

Throwaway proof that a controller on another machine can own an agent
session while this Mac sleeps, quits, or loses SSH. It is not part of the
app and it is not built by `pnpm check`.

Set `SPIKE_SSH` to the lab host. Do not write that host name into the repository.

```sh
export SPIKE_SSH=user@host
spikes/agent-controller/scripts/deploy.sh
SPIKE_SSH="$SPIKE_SSH" cargo run --manifest-path spikes/agent-controller/Cargo.toml -- prove
```

`prove` checks reconnect, SSH loss, a duplicate follow-up, emergency stop,
controller `kill -9`, a permission held across SSH loss, a host deadline,
Git 2.30 export, a disk budget, collection of a stopped volume, and that an
interrupted run keeps its work. `prove interrupt` is only that last check.

Each run's `/workspace` is its own volume. Nothing on the host removes a run
container except an explicit discard: controller restart, the guard, emergency
stop, and expiry only stop it. A new start is refused while the previous run's
work is kept; the checks discard it first.
`prove limit` is only the permission
and deadline checks. `hold` is the sleep check: leave the process running,
sleep the Mac for at least 20 seconds, and let it print `PASS sleep` after wake.
`hold claude` is that wait during a Claude tool which writes one line a
second. Sleep only when it prints `SLEEP NOW`, for 20 to 30 seconds.

`prove claude` starts the pinned Claude adapter. Create the credential on
the Mac with `claude setup-token` and export it as `SPIKE_CLAUDE_TOKEN`.
That value is a Claude plan token, not an API key. It is sent once over
the authenticated channel and is not written into this repository.

The controller listens on `127.0.0.1:47321` of the host. The Mac reaches it
through an SSH local forward. The token is `/var/lib/brainiac-spike/token`
on the host, mode 600.

`prove local` runs that same controller on this Mac, against each Docker
context it finds (Docker Desktop and OrbStack). It checks the workspace
cap inside the engine VM, that a dropped client leaves the session running,
and, when `SPIKE_CLAUDE_TOKEN` is set, that a Claude tool permission stays
pending across that drop. `hold local` is the lid-close check: close the
lid only after it prints `SLEEP NOW`.
