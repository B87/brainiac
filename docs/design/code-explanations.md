# Design: explaining changes

Design notes for v0.6: Brainiac explains a change to the person reading it, so they learn the software being built rather than only seeing what moved. An explanation is written by an agent in a run of its own kind, on v0.5's agent-run machinery, and shown beside the diff in v0.1's viewer.

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

## Open questions

- How long an explanation takes and costs on each agent, and the time limit to default to (the spike).
- How a concept is identified across explanations, so Got it carries over (a name the agent gives, normalized by Brainiac, which the user can merge).
- Where explanations are stored (`history.db` or their own deletable file), and whether backups include them.
- Whether an explain run needs its own workspace filesystem, or a smaller one than a coding run.
