# Design: explaining changes

Design notes for v0.6: Brainiac explains a change to the person reading it, so they learn the software being built rather than only seeing what moved. An explanation is written by an agent in a run of its own kind, on v0.5's agent-run machinery, and shown beside the diff in v0.1's viewer.

v0.6 started on 8 October 2026. What it does is now `SPEC.md` section 14, and how it is built `docs/architecture.md`, Explaining changes — v0.6; where they differ from the notes below, they win. This file keeps the reasons, the spike, and the questions still open.

UX Design Artifact: https://claude.ai/artifact/R41zyUax5ikqMYXh7Y9TuA (the panel, tour, notes, concepts, staleness, and Settings apply as drawn; its Explain dialog and Working state predate the choice of agent runs)

## Why

Agents now write a growing share of the code (v0.5), and a diff shows what changed, not why, how it fits the rest, or which ideas it relies on. Brainiac already holds what a good explanation needs: the change, the repository it belongs to, the commit message and pull request, and the repository's own spec and decisions. Explaining with those, and remembering what the reader already knows, teaches; a generic summary does not.

## What the user gets

- **Explain** (button and `E`) on a commit, a branch compared with the default branch, and a collected run's result; working-tree changes and pull requests in 0.6.x. Never automatic. A short dialog names the agent and host that will read the code, the depth (Brief, Teach me, Deep), and whether to add questions.
- **A panel beside the patch**, not a separate view: a summary; a source line (what the agent read); and three tabs:
  - **Tour:** the files in reading order (the rule, then the fix, then its helpers, then bookkeeping), each with its role. The file list switches between Reading order and Path, and `[` / `]` follow the tour.
  - **Concepts:** ideas the change relies on (a language feature, a library, a system tool, a project pattern), each with where it appears. **Got it** adds it to the ledger, and later explanations skip it unless a change uses it in a new way.
  - **Check yourself:** two or three questions with Reveal answer.
- **Notes in the patch:** a note sits after the lines it explains, with links to its sources. Concept words in the code are underlined and open the concept.
- **Every claim cites its source.** A note links to the file and lines, or the doc section, it rests on. A disagreement between code and docs is shown only when both quotes are found in the files.
- **While it works:** the files the agent opens, from the run's journal, and Cancel. An explanation takes minutes, not seconds; a finished one is kept and opens at once.
- **Staleness:** each note keeps the hash of the lines it explains. When those lines change, only that note is marked out of date, with Re-explain.
- **Save as note:** the explanation becomes a Markdown note in the vault, with the repository, commit, date, and agent in its frontmatter.
- **Settings → Explanations:** the profile and host (by default those of the last run), the reader's level per language, the default depth, the concepts known, the repositories asked, and stored explanations with Delete all.

## How it works

1. **An explain run.** The same controller, container, input bundle, profile, key, and host as any run, with a kind of its own: Brainiac writes the prompt, the agent is autonomous inside the container, the time limit is short, and the run has no review, no patch, and no place in the Runs list. The agent reads whatever it needs (callers, definitions, `SPEC.md`, decisions, history) instead of a context Brainiac gathers.
2. **The prompt** holds the subject (a commit, or a range for a branch or a run's result), the reader's level and known concepts, the depth, the schema, and the rules: lines on the new side, quotes copied verbatim, no edits to the code, and the result in `.brainiac/explanation.json`.
3. **One file comes back.** The collector reads only that file; anything else the agent changed is ignored, and the run is cleaned up as soon as the file is imported.
4. **Checked before it is shown.** The file must match the schema; each note must anchor to lines in the change; each quote must be found verbatim in the commit's files, or its claim is dropped; a code–docs disagreement needs two verified quotes. A file that fails gets one follow-up prompt in the same session with the errors, then the explanation fails with what was wrong.
5. **Stored by Brainiac, never in the repository.** Explanations are kept per repository, subject, agent, and depth, and can be deleted. The level, the concepts known, and each repository's answer are user settings in `brainiac.db`.

**Privacy and cost.** The code goes to the provider of the chosen profile, as for any run; the first Explain in a repository asks once, and the answer is listed in Settings. A Claude Code profile can use the Claude subscription (subject to Anthropic's answer on the token's terms); on an API key, an agent reading many files costs more than one model call, and the run's reported usage is shown.

## Phases

- **0.6.0:** commits, branch comparisons, and collected run results; the explain run, the checks, the panel, notes, concepts with the ledger, questions, staleness, code–docs disagreement, Save as note, and Settings → Explanations.
- **0.6.x:** working-tree changes (sent as a patch the container applies to its copy, since Brainiac never writes the user's repository); pull requests; a question about one note as a follow-up prompt while the session lasts; and a direct model call for small changes, which would also allow a local model.

## Spike before building

No code: ordinary runs on this repository with a prompt that asks for `explanation.json`, over about five commits of different sizes, with Claude Code and with OpenCode. Score every note by hand (correct, useful, grounded) and record the time and cost. It decides the prompt, the schema, the default time limit, and whether the result teaches.

### Spike, first round (8 October 2026)

Claude Code only, run as the headless `claude -p` on a fresh clone of this repository checked out at each commit, not yet as a Brainiac run: no container, so the times below leave out its start and the image. The agent could read files, search, and run `git`, and write only the explanation. Five commits of different sizes, each with Opus 5.5 (Claude Code's default) and with Sonnet 5.5. Costs are what Claude Code reported for the run; on a Claude subscription they are notional.

The prompt, with the commit in place of `<SHA>`:

```
Explain commit <SHA> of this repository (checked out at that commit) to a reader who is new to Rust and comfortable with TypeScript. Do not edit any file except the one named below.

Read whatever you need: the diff (git show <SHA>), callers and definitions, SPEC.md, docs/architecture.md (especially its Decisions), and git history.

Write .brainiac/explanation.json, a single JSON object with:
- "summary": 2–4 sentences on why the change exists, not only what moved.
- "sources_read": the files and doc sections you relied on.
- "tour": the changed files in reading order (the rule, then the fix, then its helpers, then bookkeeping), each {"path", "role"}.
- "notes": each {"path", "new_start", "new_end", "text", "sources": [{"path", "start", "end", "quote"}]}. new_start/new_end are line numbers on the NEW side of the diff, inside a changed hunk. Each quote is copied verbatim from that file at this commit, and lies within start..end.
- "concepts": ideas the change relies on, each {"name", "kind": "language"|"library"|"system"|"project-pattern", "explanation", "appears": [{"path", "line"}]}.
- "questions": 2–3 {"question", "answer"} that check understanding.
- "disagreements": only where code and docs conflict, each {"claim", "code": {"path","start","end","quote"}, "doc": {"path","start","end","quote"}}. Empty array if none.

Every claim must cite a source. If you cannot quote it verbatim, leave the claim out.
```

Each file was then checked the way Brainiac would check it: every key present; each note's lines inside a changed hunk on the new side; each quote found verbatim within its cited lines at that commit (or, failing that, elsewhere in the file); each concept's location an existing line; every tour file part of the change.

| Commit | Change | Opus: time, cost | Sonnet: time, cost | Notes O / S | Quotes O / S | Concepts O / S | Disagreements O / S |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `5a98929` | 3 files, +12/−3 | 79 s, $0.49 | 42 s, $0.18 | 5 / 5 | 13 / 6 | 7 / 4 | 0 / 0 |
| `29bbc56` | 4 files, +35/−11 | 118 s, $0.68 | 61 s, $0.22 | 6 / 6 | 15 / 9 | 7 / 6 | 1 / 0 |
| `4aae4b2` | 6 files, +87/−24 | 142 s, $0.77 | 54 s, $0.22 | 11 / 8 | 30 / 9 | 10 / 6 | 0 / 0 |
| `287bdc9` | 5 files, +196/−17 | 250 s, $1.53 | 90 s, $0.37 | 14 / 12 | 38 / 18 | 13 / 8 | 1 / 1 |
| `7edf3c2` | 14 files, +726/−239 | 231 s, $1.50 | 110 s, $0.52 | 23 / 16 | 46 / 26 | 11 / 7 | 0 / 0 |
| **Total** | | **$4.97** | **$1.51** | 59 / 47 | 142 / 68 | 48 / 31 | 2 / 1 |

What it showed:

- **Both models follow the schema.** Every file parsed with every key, and every tour covered every changed file in a sensible order. The runs left the clone's tracked files unchanged.
- **Opus passed every check.** All 59 notes were inside changed lines and all 142 quotes were verbatim at their cited lines, so a follow-up turn would never have been needed. Sonnet failed two checks in five runs: one note anchored to unchanged lines (`5a98929`), and one quote verbatim but cited at the wrong lines (`4aae4b2`).
- **Disagreements are worth having.** Both of Opus's were real, and both docs were still out of date at the time of the spike: `architecture.md` said a pasted token has only its line breaks removed (`29bbc56`), and that a protocol mismatch is always a failure (`287bdc9`). Sonnet found the second and missed the first. One Opus note on `287bdc9` also found a small bug: Install over an earlier controller restarts the service but reports "the service started".
- **Sonnet is thinner, not wrong.** Its summaries, reading order, and questions were accurate and taught. It cited about 1.4 quotes per note against Opus's 2.4, gave fewer concepts, and caught less.
- **Cost follows what the agent reads** (files opened, turns) more than the size of the diff.
- **The checks cannot judge correctness or usefulness.** They prove that a note sits on the change and that its quotes exist, not that its claim is right. That still needs the hand scoring.

What it suggested for the build, taken into `SPEC.md` section 14 as defaults (the time limits doubled to allow for the container until runs in one are timed):

- The model follows the depth: Sonnet for Brief and Teach me, Opus for Deep, with an override in Settings → Explanations.
- The checker repairs what it can before a follow-up turn: a quote found verbatim elsewhere in its file is re-anchored there rather than reported. A note outside the changed lines still costs the follow-up turn, or is dropped.
- A default time limit of about 5 minutes for Sonnet and 10 for Opus, until runs in a container are timed.

Left for the spike: scoring every note by hand; the same five commits with OpenCode, with at least one provider; and the prompt as a real Brainiac run, to time the container.

## Open questions

- How long an explanation takes and costs on each agent, and the time limit to default to (the spike; Claude Code's first round is above, OpenCode is still to measure).
- Whether an explain run needs a smaller workspace than a coding run (it uses the profile's for now).
- Whether the panel and the canvas's Working state hold up with real progress from the journal.
