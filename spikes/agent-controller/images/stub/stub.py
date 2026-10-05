"""Heartbeat on stdout once a second. Echo one follow-up line from stdin.

A line `permission` asks and then waits. Heartbeats continue. Nothing is
granted until stdin says `perm:allow` or `perm:cancel`. A second answer
prints `perm:late` and changes nothing.

Each beat and follow-up is appended to a file in /workspace before it is
printed, so every line in the journal is also in the run's work.

The controller owns this process. Stdin staying open after a client
disconnect is intentional: Docker is asked not to close it, so a dead
controller does not by itself end the process. The separate guard stops it.
"""

import select
import sys
import time


WORKSPACE = "/workspace"


def keep(name, line):
    with open(f"{WORKSPACE}/{name}", "a", encoding="utf-8") as out:
        out.write(line + "\n")


def stamp():
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())


BEATS = 0


def seq_beat():
    # A counter keeps beats unique; two beats in one second share a stamp.
    global BEATS
    BEATS += 1
    return f"{stamp()}#{BEATS}"


def main():
    pending = None
    seq = 0
    while True:
        ready, _, _ = select.select([sys.stdin], [], [], 1.0)
        if ready:
            line = sys.stdin.readline()
            if line == "":
                return
            line = line.strip()
            if line == "permission":
                if pending is None:
                    seq += 1
                    pending = f"perm-{seq}"
                    sys.stdout.write(f"perm:pending:{pending}\n")
                else:
                    sys.stdout.write("perm:busy\n")
            elif line in ("perm:allow", "perm:cancel"):
                if pending is None:
                    sys.stdout.write("perm:late\n")
                else:
                    outcome = "allowed" if line == "perm:allow" else "cancelled"
                    sys.stdout.write(f"perm:{outcome}:{pending}\n")
                    pending = None
            else:
                keep("follow.txt", line)
                sys.stdout.write(f"follow:{line}\n")
        else:
            beat = f"beat:{seq_beat()}"
            keep("beats.txt", beat)
            sys.stdout.write(f"{beat}\n")
        sys.stdout.flush()


if __name__ == "__main__":
    main()
