# Known-concepts spike

Throwaway measurement for v0.6.1 (`docs/design/code-explanations.md`,
Known-concepts spike). It is not part of the app and `pnpm check` does not
run it.

Question: with a ledger of a few hundred known concepts, how often does an
agent run the `known` lookup, how many known concepts does it still write
under Concepts, how many does it name in `known_used`, and what does that add
to time and cost?

```sh
node spikes/known-concepts/run.mjs --dry-run --commit 29bbc56   # prompts and ledgers only
node spikes/known-concepts/run.mjs --agent claude --model sonnet
node spikes/known-concepts/run.mjs --agent opencode --model anthropic/claude-sonnet-5-5
```

Run OpenCode runs one at a time: two at once fail with "database is locked". `--variant late` asks the agent to look up the names it will write; `--variant similar` explains the `similar:` lines the lookup prints; `--reuse <dir>` reruns on an earlier run's ledgers.

Options: `--agent` and `--commit` can repeat; `--ledger <n>` sets the second
run's size (300); `--out <dir>` keeps clones, prompts, logs, and
`results.tsv` (default: a folder in the temp directory).

Each commit and agent runs twice in a fresh clone checked out at that commit:
once with no ledger, once with a ledger of filler names plus every second
concept the first run wrote. The second run is the one to read: its
`known_kept` should be near zero, its `known_used` near the number of known
concepts the change relies on, and `lookups` above zero. Compare `secs` and
`cost_usd` between the two runs of a commit.

Limits: the agent runs headless on the host with no container, so the
container's start is not timed; the prompt is the first round's, with the
lookup paragraph of `explain/prompt.rs`, not the real prompt; the known
concepts are all given the kind technique; a run's cost is what the agent
reports. Each run costs real money (the first round's Sonnet runs were
$0.18 to $0.52). First results (Claude Code and OpenCode, Sonnet 5.5, three commits) are in the design doc. `known_used` counts any ledger name the agent lists, filler included.
