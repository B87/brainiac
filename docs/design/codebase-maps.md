# Design: codebase maps

Notes on a map of a whole repository, kept current as it changes: its modules, how they depend on each other, what each one offers, and how a change moved any of that. Explain (v0.6) teaches one change; a map teaches the codebase the changes land in. Nothing here is on the roadmap or in scope for any release.

## Status: idea, not planned (written 10 Oct 2026)

Nothing in the app is built. On 10 October 2026 the decisions below were made with the maintainer, a client panel reviewed the first draft (Panel review, below), and existing tools were surveyed (What exists already). An experiment on Brainiac's own repository runs first (The experiment on Brainiac itself), then a spike (Spike before building).

## Why

With agents writing a growing share of the code, the hard part is no longer writing a change but knowing the codebase it lands in. Explain answers "what does this change do and why"; it does not answer "what is this module for, what does it hide, and what depends on it", and nobody reads a whole repository to find out. The maintainer found this while reshaping Brainiac's own modules (`modules.md`): the numbers that showed the problem (lines in `lib.rs`, commands, the size of `models.rs`, imports between features) were gathered by hand, once, and were stale a week later.

A map has to be trusted to be read. So the design separates what code computes, which is then simply true, from what only a model can write, which is then checked, and says on screen which is which.

## What exists already

Surveyed on 10 October 2026. Each part of this design exists somewhere; no tool found combines them.

| Kind | Examples | What it does | What it lacks for this design |
| --- | --- | --- | --- |
| AI-written codebase wikis | DeepWiki (Cognition), DeepWiki-Open (self-hosted, local models, Mermaid diagrams) | A wiki per repository: architecture overview, module pages, diagrams, questions and answers | No diff between two commits, no check of its claims, and no use of history or pull requests, so the reasons behind a design are missing |
| Dependency graphs and boundary rules | dependency-cruiser and Madge (JavaScript), import-linter (Python), ArchUnit (Java), cargo-modules (Rust), Nx's project graph; depdog (Go) | A computed import graph, rules on it, and in depdog's case a diff against a Git ref that lists edges added and removed between components | One language each, rules rather than understanding, no prose |
| Change coupling from history | CodeScene; code-maat, after Adam Tornhill's *Your Code as a Crime Scene* | Modules that change in the same commits, by the same people, or under the same ticket, and whether that is getting stronger | No module pages or interfaces; a hosted product, or a command-line tool |
| Pull request summaries | CodeRabbit's walkthrough | Prose and sometimes a diagram per pull request | Nothing computed or checked behind it |
| Repository maps for agents | Aider's repo map | Definitions and references per file with tree-sitter, ranked, across many languages | Made for a model's context, not for a reader |

What this means for the design:

- **What is new is the combination:** facts kept per file version so any two commits compare, prose checked against those facts, learning that shares Explain's ledger, and all of it on the Mac.
- **The computed tools are the trusted ones.** The wikis are read once and skimmed; the import checkers run on every pull request. The design leans on computed facts and keeps prose small.
- **Change coupling is established,** with a name, a method, and a literature; this design uses CodeScene's framing that coupling is neither good nor bad by itself.
- **Aider shows tree-sitter extracts definitions and references cheaply across many languages,** which argues for a parser over the agent for interface items (Open questions).

Sources: [DeepWiki](https://www.x-cmd.com/blog/250502), [DeepWiki-Open](https://dev.co/devops/open-source/deepwiki-open), [depdog](https://pkg.go.dev/github.com/matterpale/depdog), [CodeScene, change coupling](https://docs.enterprise.codescene.io/versions/6.0.0/guides/technical/change-coupling.html), [architecture drift between releases](https://zof.ai/blog/the-graph-diff-detecting-architecture-drift-between-two-releases).

## What the user gets

- **Map** on a repository, beside its other tabs. The first time, a dialog like Explain's names the agent and host that will read the code, says plainly that the agent reads **the whole repository**, and gives an estimate from the repository's size (Cost, time, and failure).
- **Start here:** the repository in a paragraph, and the modules in the order to read them, for someone new to it.
- **The module graph:** each module as a box in its group, its dependencies as arrows. Every arrow says where it comes from (Two kinds of content). A module opens its page; the arrow keys move between modules and `Return` opens one.
- **A page per module:**
  - what it is for and what it hides, in a few sentences, each citing files and lines;
  - its interface: the items other modules use, with their signatures, and which modules use each;
  - what it depends on and what depends on it, and the modules it changes together with;
  - who changes it most often, from Git, so the reader knows whom to ask;
  - its files in reading order, each with its role, as the file list of an explanation;
  - **Concepts**, shared with Explain's ledger: **I know this** on a concept here leaves it out of explanations too, and the other way round;
  - **Questions:** two or three, with Reveal answer, as in Explain.
- **The architecture diff**, in the panel beside a pull request, a branch compared with the default branch, or a commit, next to its explanation:
  - modules added or removed;
  - dependencies added, removed, or unknown (A missing fact is unknown);
  - interface items added, removed, or changed, with the before and after signature;
  - sizes that moved, and modules that now change together;
  - a short narrative over those facts, folded under the list, which comes first.
- **Out of date:** a map knows the commit it describes. When the default branch moves, what Git computes (sizes, history, coupling) updates without a run; the graph and interfaces of the files that changed show as not read yet, and a module whose interface changed since its prose was written says so, with **Rewrite**.
- **Save as note:** a module page or an architecture diff becomes a Markdown note, its diagrams as Mermaid blocks, which Obsidian also renders.
- **Delete map,** and the map listed in Settings → Explanations with what it cost (Where it fits).

A map never runs by itself (Not automatic, below), and it reads offline once made, as explanations do.

## How it works

### Two kinds of content

| | Facts | Prose |
| --- | --- | --- |
| What | Files, sizes, history, imports, interface items | Purpose, what a module hides, Start here, the narrative of a diff, file roles, questions |
| Who produces it | Brainiac from Git; the agent for imports and interfaces, then checked | The agent, writing over the facts |
| When it changes | Whenever a file changes | Only when the facts it rests on change |
| Shown as | Each fact marked "from Git" or "read by the agent" | Each claim cites lines; quotes checked as Explain's are |

The words follow Explain's: a fact that passed its check is "matched to the code", never "verified", and the count of facts that failed their check is shown on the map, not only kept.

### Facts are per file version

A file's imports and interface items change only when its content does, and Git already names a file's content by its blob hash. So facts are stored per repository and blob:

- The first map extracts facts for every file. A later map, for any commit, extracts them only for blobs not yet read, which for a typical commit is a handful of files.
- Facts at any two commits are then lookups, and the architecture diff between them is a comparison Brainiac makes. The facts the agent read are as good as their checks; the comparison adds no model.
- A pull request's head usually needs facts for only the files it changed. Explain on a pull request can extract them in the same session, as an option in its dialog with its share of the estimate, so the diff arrives with the explanation.
- Facts are kept per repository even when two repositories hold the same blob (a vendored file), so one client's map never draws on another's.

What Brainiac computes itself, for any language, with the same read-only Git the viewer uses:

- files and lines per module;
- who changed each module, and how much, over the last 30 and 90 days;
- **change coupling:** pairs of modules that keep changing in the same commits. Commits that touch a large share of the repository (formatting, renames, dependency bumps, squashed merges of long branches) are left out, and the map says how many were.

What the agent extracts, in its container like any explain run, into `.brainiac/map.json`:

- each file's imports, as the text of the import and the path or module it names;
- each file's interface items: name, kind, signature, and the line it is declared on.

Each fact is checked before it is kept: an interface item's declaration line must hold its name in that blob, and an import's text must be found verbatim in the importing file. Where the import names a path, Brainiac resolves it to a module with the module list, not the agent; where it names a package or namespace, the agent's resolution is kept and marked as the agent's.

### A missing fact is unknown, never removed

An agent can find an import in a file at one commit and miss it in the next version of the file. Compared naively, that shows "dependency removed" as if it were computed. So a removal needs evidence:

- A dependency is **removed** only when the import text found in the old version is no longer in the new one. Brainiac checks that itself, by searching the new blob.
- If the text is still there, the dependency stays, and the missed fact is counted as a failed extraction.
- A file with no facts yet (not read, or its extraction failed) makes its dependencies **unknown**, shown as such in the graph and the diff, never as absent.

The same rule applies to interface items. The experiment met the problem on a small scale: compared with a commit from before Brainiac's boundary test, the shell's imports looked removed, because the old commit did not say which modules were the shell (The experiment on Brainiac itself).

### Modules: the list on this Mac, or a file in the repository

A repository Brainiac does not know has no module list, and a diff compares modules, so the list must be stable:

- **A file in the repository, when there is one.** A team that wants one map for everyone checks in a small file (`.brainiac/modules.toml`, say: a name, a group, and path patterns per module). Brainiac only reads it, and it wins over everything else, so every member's diff of a pull request is the same. Map offers the current list as that file's text, for someone to commit; Brainiac never writes it.
- **Otherwise, the list on this Mac.** The first map's agent proposes the modules, with a name and a one-line purpose each. The user can rename, merge, or split them, and later maps keep those identities. This is the case of a contractor who cannot add files to a client's repository.
- A file no pattern covers goes in **Other**, and the map says what share of the code that is. Past a threshold, the next map proposes where those files belong, as a change the user can see and take or leave; a list change taken can be undone.

A reader new to a repository is not asked to draw its modules: the proposal stands until someone who knows changes it.

### Prose is rewritten only where its facts changed

Each piece of prose records the facts it was written over: a module's interface, its dependencies, and its files' blobs. Rewrite sends the agent only the modules whose facts changed, with the previous prose, and asks it to keep what still holds. Prose stays stable between maps, so a reader sees what changed, not a new wording of the same thing.

The narrative of a diff may describe only facts in the computed diff, so it cannot mention a dependency that is not there.

### Stats

Per module and for the repository, with each term explained in a few words where it first appears, as Explain does:

- files, lines, interface items, dependencies in and out;
- who changes it, and how often;
- **changes together with:** the modules most often changed in the same commits. The one the panel valued most: it shows a dependency whatever the imports say.
- **modules touched per commit:** how far a typical change spreads.
- **lines per interface item:** how much code each public item stands for. Shown as a number with its explanation, never as a label such as "shallow", which reads as a grade of someone's code.

Each map keeps its stats. Trends are not on the module page; they belong to the experiment until someone asks for them.

### Cost, time, and failure

- **The estimate of a first map** comes from the repository's size (files and bytes the agent will read) and this Mac's earlier maps, since there are no earlier ones of this repository. On a Claude plan it says "Uses your Claude plan", as runs do.
- **Large repositories** are read in batches of files, one run each, with the facts of each batch checked and kept as it finishes. A batch that fails or runs out of time leaves the batches before it, so a failure costs what it spent and keeps what it read; Map then continues from the next file.
- **Sleep** pauses a run as for any explain run (SPEC section 14, Sleep and limits); batches keep that loss to one batch.

## Not automatic

The first draft had **Keep it current**: rewrite whatever changed each time the default branch moved, up to a monthly cost. It is out of the design:

- It contradicts the first rule of explanations (SPEC section 14: Never automatic). It would start an agent, with the repository's code, the user's token, and a container with network access, without anyone pressing anything.
- A monthly cost cap means nothing on a Claude plan, which reports no cost.
- The default branch moves only on a fetch, which is off by default, so it would run at surprising times.

What stays automatic is what costs nothing: sizes, history, and coupling from Git, and marking what the map has not read yet. Prose and agent facts change only on Map, Rewrite, or Explain.

## Where it fits

- **On v0.6's machinery.** A map run is an explain run with another subject (a repository at a commit instead of a change) and another output file. It reuses the container, the input bundle, the checks, the answer on sharing a repository's code (`sharing.rs`), the ledger, the panel's components, and Save as note.
- **In the explanations feature, not beside it.** A separate feature would import agent runs and explanations, two new entries in the boundary test's `ALLOWED` list. Maps are Explain at another scope, so they live in `explain/` and add no import between features.
- **Code sharing.** A repository's answer covers maps as it covers runs and explanations, but the first Map dialog says that the agent reads the whole repository, which is more than a run's "anything the agent reads" suggests.
- **Storage.** Maps, facts per blob, module lists, and prose in `history.db` with explanations: they cost runs to rebuild, so they are not in a rebuildable file. Settings → Explanations lists each map with its cost; Delete map removes its facts, prose, list, and the conversations of its runs, which held the repository's code.
- **Concepts.** A first map can find many project patterns at once. They join the ledger only as the reader marks them, per repository, as Explain's do; a map adds none by itself.

## The experiment on Brainiac itself

Before any of this is built, the cheapest test of the central claim (that an architecture diff beside each change is worth reading) runs on Brainiac's own repository, with no model:

- `pnpm map:diff [base] [head]` (`scripts/module-diff.sh`, `src-tauri/examples/module_map.rs`) prints, as Markdown, how a change moved the modules: modules added, removed, or placed in another group; imports between modules added and removed, each against the boundary rules; entries added to or removed from `ALLOWED`; the numbers `modules.md` tracked by hand (lines in `lib.rs`, commands, the size of `models.rs`, top-level modules, `ALLOWED` entries); sizes by module; and a Mermaid graph with what changed in color. With no arguments it compares the working tree with its merge base on `origin/main`.
- It reads the sources with the boundary test's own reader, and each commit's copy of the test's lists, so the map shows exactly what the test sees and needs no second parser. Commits are read with `git archive` into a temporary folder.
- CI adds the output to each pull request's run summary; it only reports.

What it showed on its first day:

- On PR #13 it said in one line what the pull request did to the structure: "1 module added, 4 dependencies added, 2 dependencies removed, 2 entries removed from `ALLOWED`".
- The full graph had 87 arrows and could not be read. The diff's graph now draws only what changed, the `ALLOWED` exceptions, and imports inside a feature (25 arrows); a map for learning needs the same choice of what to leave out.
- A false "removed" appeared on the first try (A missing fact is unknown).

What to watch for a few weeks: whether the maintainer opens the summary, which part they read, and what they wished it said. That decides the order of the phases.

## Phases, if it happens

0. **The experiment on Brainiac itself,** running now.
1. **A map on demand:** the module list (file or proposal), facts with their checks, Start here, the graph, module pages, and stats.
2. **The architecture diff** beside pull requests, branches, and commits, with facts extracted in the explain run.
3. **Learning on a module:** reading order, Concepts with the ledger, and Questions.
4. **Export** to a static HTML site.

Each phase is useful without the next. Whether learning (3) comes before the diff (2) is open (Open questions).

## Spike before building

No app code, as for Explain: ordinary runs with a prompt that asks for `map.json` and the module proposal, on three repositories of different languages and sizes, with Claude Code and with OpenCode.

- **Ground truth for Brainiac itself:** the boundary test's reader already finds every Rust import between top-level modules. The agent's edges are scored against it: missed, invented, and right, and how many of the misses the removal check would catch.
- **Scored by hand on the others:** whether the proposed modules are the ones a maintainer would draw, and whether the prose is correct, useful, and grounded.
- **Recorded:** time and cost of a first map per thousand files, of the facts for a typical commit, and how often a batch fails.

The spike decides whether the agent's facts are good enough, or whether a parser per language is needed.

## Panel review (10 October 2026)

A client panel (three fictional personas, so hypotheses to check with real users) reviewed the first draft: a CTO who reviews pull requests, a learner new to a monorepo, and a contractor working across client repositories.

- **What they would use:** the CTO the architecture diff, a few times a week; the learner the module pages, daily at first; the contractor the graph, once per client repository at the start of an engagement.
- **Agreed:** change coupling is the useful stat, and "depth" or "shallow" reads as a grade; the first map's cost and size were underspecified; the diff's computed list is trusted more than its narrative; Keep it current comes last.
- **Disagreed:** the CTO wants the diff first and the learner wants learning first; the CTO and the learner want the module list in the repository and the contractor cannot add one to a client's; the contractor sees HTML export as a deliverable and the CTO as code-derived text leaving the Mac.

What changed in this file because of it:

- A missing fact is unknown, never removed, with Brainiac's own check for a removal; each fact says where it comes from, and failed checks are counted on screen.
- Keep it current is out (Not automatic).
- The module list: a file in the repository when there is one, the list on this Mac otherwise, and a proposal the user can undo.
- The first map: an estimate from size, batches that keep what they read, and the dialog saying "the whole repository".
- Explain's names (I know this, Questions, the reading order in the file list), terms explained where they appear, and no grade-like labels.
- Start here, who changes a module, keyboard navigation of the graph, Delete map, facts per repository, and the explain run's fact extraction as an option with its estimate.

## Not in this design

- Maps written into the repository, or committed by Brainiac.
- Running without the user: no automatic rewrite on new commits.
- Rules a repository must follow (allowed dependencies between layers, as Brainiac's boundary test enforces). A map shows what is; deciding what should be is a later idea.
- Call graphs and data flow: imports and interfaces only.
- A score or grade for a codebase.

## Open questions

- **The diff or learning first.** The maintainer and the CTO persona would use the diff most; the learner persona would use learning most. The experiment's weeks are the evidence.
- **A parser instead of the agent for facts.** tree-sitter in Rust would make imports and interfaces computed and free for the languages it covers (Aider's repo map does this across many); it is a dependency and a grammar per language. The spike's missed and invented edges decide it.
- **Rendering diagrams.** Mermaid is large for the app's bundle; a graph drawn by the app (with a layout library) is lighter and interactive. Save as note needs Mermaid text either way.
- **The module file's format,** which, once read by Brainiac, has to stay compatible.
- **A command line for CI in any repository.** `pnpm map:diff` serves Brainiac only. A `brainiac map --diff base..head` that prints the computed diff from stored facts needs no model, but CI does not have the Mac's stored facts.
- **What counts as an interface item** in languages without visibility keywords (Python, JavaScript without exports), and in configuration such as Terraform.
- **HTML export:** a static site is a deliverable for a contractor and code-derived text leaving the Mac for a CTO; who it is for decides whether it is built.
