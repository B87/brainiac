# Design: codebase maps

Notes on a navigable map of a whole repository, kept current as it changes: its parts, how they depend on each other, how a user action flows through them, and how a change moved any of that. Explain (v0.6) teaches one change; a map teaches the codebase the changes land in. Nothing here is on the roadmap or in scope for any release.

## Status: idea, not planned (written 10 Oct 2026, rewritten 11 Oct 2026)

Nothing in the app is built. On 10 October 2026 the first draft was written with the maintainer, reviewed by a client panel, and compared with existing tools, and `pnpm map:diff` started as an experiment. On 11 October a prototype map of Brainiac drawn with LikeC4 replaced the first draft's own diagrams and spec, and this file was rewritten around it: Brainiac computes and checks the facts, an agent composes the map, and LikeC4 draws it. The same day a diff of PR #13 was drawn on the prototype, and a design canvas explored how maps and their diffs would sit in Brainiac (The diff experience; Extending LikeC4).

UX design canvas: https://claude.ai/artifact/RKBX9GE1cVFwzBMhRciWuR (the Map tab, Architecture beside a pull request with Before / Diff / After, five ways to show a change, a flow marked by a change, and how it plugs into LikeC4)

## Why

With agents writing a growing share of the code, the hard part is no longer writing a change but knowing the codebase it lands in. Explain answers "what does this change do and why"; it does not answer "what is this part for, what depends on it, and what happens when I click this", and nobody reads a whole repository to find out. The maintainer found this while reshaping Brainiac's own modules (`modules.md`): the numbers that showed the problem were gathered by hand, once, and were stale a week later.

Two things the first attempts showed:

- **Computed alone is dumb.** `pnpm map:diff` is always right and hard to learn from: its graph draws every import the same way, has no idea which arrows matter, and cannot show a flow.
- **Written alone is not trusted.** An AI-written wiki is read once and skimmed (What exists already).

So a map is composed by an agent, the way a person who knows the codebase would draw it, from facts Brainiac computes, in a fixed vocabulary of shapes, and checked against those facts before it is shown.

## What the user gets

- **Map** on a repository, beside its other tabs, drawn by LikeC4 inside Brainiac. The first time, a dialog like Explain's names the agent and host, says plainly that the agent reads **the whole repository**, and gives an estimate (Cost, time, and failure).
- **Zoom levels:** the repository in its context (people, the systems it talks to) → its parts (frontend, backend layers, features, processes, stores) → each part's modules → a module's main files. Any box with a view of its own opens on click; the back button and ⌘K search work across all of them.
- **Flows:** a user action traced through the code ("Explain a commit": 41 steps from the button to the panel in the prototype), as a diagram or a sequence, with **Start** to step through it one arrow at a time. Each step names the file and line it happens at, and links to it.
- **Cross-cutting views:** where data lives (which part writes which store), and the exceptions to the repository's own rules where it has some (Brainiac's `ALLOWED`).
- **Start here:** the first view, with a paragraph on the repository and the order to read its parts in.
- **Learning on any element,** as in Explain's panel: what it is for in a few sentences citing lines, Concepts shared with Explain's ledger (**I know this**), and Questions with Reveal answer.
- **Architecture**, a tab beside Explanation in the side panel of a pull request, a branch compared with the default branch, a commit, and a run's result: the map with the change drawn on it, a **Before / Diff / After** switch, and the computed list of changes linked to the patch (The diff experience, below).
- **Out of date:** what Git computes updates without a run; elements whose files changed since the map was composed are marked, with **Rewrite**.
- **Save as note,** a map's views as images and its text as Markdown; **Delete map**; the map in Settings → Explanations with what it cost.

A map never runs by itself (Not automatic), and it reads offline once made.

## How it works

### The pipeline

1. **Facts.** Brainiac computes what it can from Git and the files, and stores it per file version (Facts, below).
2. **Composition.** An explain run of its own kind: the agent, in its container, reads the code with **fact tools** (below) and writes a **LikeC4 model**, a text file of elements, relationships, and views, in Brainiac's fixed vocabulary (The kit).
3. **Checks.** Brainiac reads back only the model, runs LikeC4's validation, exports the model as JSON, and checks every element, relationship, and flow step against the facts (Checks). What fails is dropped and counted, or gets one follow-up turn, as Explain's checks do.
4. **Drawing.** Brainiac renders the checked model with LikeC4's components in the app. For a diff, Brainiac itself adds the change to the model as tags, which the kit colours.

The agent decides what is worth showing: which parts to group, which arrows matter, what each one is labelled, which flows to trace, and in what order to read. Brainiac decides what is true.

### Why LikeC4

LikeC4 is an open-source (MIT) tool for architecture as code, after the C4 model: one model, many views, written in a small language. The prototype (The prototype, below) showed it already does what an own spec and renderer would have to rebuild:

- **The kit is its `specification` block:** element kinds and relationship kinds with their shapes, colours, and line styles, declared once.
- **Zoom levels for free:** a view scoped to an element opens when the element is clicked.
- **Flows are first class:** dynamic views, as a diagram or a sequence, with a step-through.
- **Layout,** the hard part of drawing graphs, done.
- **A deterministic check on what the agent writes:** `likec4 validate` refuses unknown elements and broken references.
- **An MCP server** (`likec4 mcp`), so an agent can read and edit the model through tools instead of writing text blind.
- **Exports** to JSON (what Brainiac checks), images, draw.io, and Markdown, and a web component or React components for drawing it inside the app.

What it costs:

- A dependency of a few megabytes in the app (the prototype's single page was 3 MB), loaded only when a map opens. It is added only in the release that builds maps.
- A language Brainiac does not control. The version is pinned, and the model can always be composed again from the facts, so nothing is kept only in LikeC4's form.
- LikeC4's look, not Brainiac's, as far as its theming allows.
- Quirks the agent must be told about: some words are reserved (`notes` cannot name an element), and a sequence cannot show an element and its own child as two actors.

What it does not do, and Brainiac must: know whether an arrow is true.

### The kit

Brainiac owns one `specification` block, the same for every map, and the agent may use only its kinds. In the prototype:

| Elements | Relationships |
| --- | --- |
| person, app, layer, feature, module, file, ui, store, process, sandbox, external | uses (an import), invokes (IPC or an API), emits (an event), stores (a write to a store), spawns (a process or container), network (a call to the outside) |

and the tags Brainiac adds: `#exception` (outside the repository's rules), and for diffs `#added`, `#removed`, `#unknown`. A fixed kit keeps every map readable in the same way and gives the checks a fixed set of claims to verify.

### Facts

What Brainiac computes itself, for any language, with the same read-only Git the viewer uses:

- files, lines, and folders;
- who changed each part, and how much, over the last 30 and 90 days;
- **change coupling:** parts that keep changing in the same commits, leaving out commits that touch a large share of the repository (formatting, renames, dependency bumps, squashed merges of long branches).

What a parser computes, per file, where one exists for the language: definitions with their lines, and imports with their text. This is what tree-sitter gives for many languages (Aider's repository map does this), and what Brainiac's boundary test does for Rust today. Where no parser covers a language, the agent extracts the same facts and each is checked by finding its text in the file.

Facts are stored per repository and Git blob: a file's imports and definitions change only when its content does, so a later map reads only the blobs it has not seen, and facts at any two commits are lookups.

### Fact tools

The agent works with commands in its container, as the known-concepts lookup (`known`) already does in explain runs:

- `facts files [path]`, `facts imports <file>`, `facts defs <file>`: the computed facts;
- `facts refs <name>`: where a name is used;
- `facts coupling`, `facts owners <path>`: from history;
- LikeC4's MCP tools to read and edit the model, and `likec4 validate` to check it before finishing.

The tools save the agent reading every file to find structure, so its reading goes to what structure cannot say: what a part is for and how a flow runs.

### Checks

Before anything is shown:

- **Structure:** the model validates, uses only the kit's kinds, and every element is in a view.
- **Elements:** every element that names a path (a module, a file) names one that exists at the map's commit; a module's files are where the module list says.
- **Imports:** every `uses` relationship between two elements matches computed imports between their files.
- **Other relationships:** an `invokes`, `emits`, `stores`, or `spawns` relationship cites a file, a line, and a quote, and the quote must be found there (`invoke("start_explanation")`, `emit(...)`), as Explain's quotes are.
- **Flows:** every step cites a file and line in the same way, and its two ends must be the elements those files belong to.
- **Prose:** descriptions cite lines, checked like Explain's.

What fails is dropped and counted on the map, never shown; past a share of failures the run gets one follow-up turn with the errors. A check that passed is "matched to the code", never "verified".

### A missing fact is unknown, never removed

A dependency is removed only when the import text found in the old version of a file is no longer in the new one, which Brainiac checks by searching the new blob. A file with no facts yet makes its relationships unknown, shown grey, never absent. `pnpm map:diff` met the problem on its first day: compared with a commit from before Brainiac's boundary test, the shell's imports looked removed because the old commit did not say which modules were the shell.

### The parts of a repository

A diff compares parts across commits, so their identity must be stable:

- **A model in the repository, when there is one.** A team that wants one map for everyone checks in a LikeC4 model, or only its elements and the paths each covers. Brainiac only reads it, and it wins, so every member sees the same map and the same diff of a pull request. Map offers its own model as text for someone to commit; Brainiac never writes it.
- **Otherwise, the parts on this Mac.** The first map's agent proposes them. The user can rename, merge, or split them, and later maps keep those identities. This is the case of a contractor who cannot add files to a client's repository.
- Files no part covers go in **Other**, with the share of the code it holds; past a threshold the next map proposes where they belong, as a change the user can take, leave, or undo.

### The architecture diff

Computed by Brainiac between the facts at two commits (a pull request's base and head, a branch and the default branch, a commit and its parent):

- parts whose files appeared or disappeared;
- imports between parts added, removed, or unknown;
- definitions added, removed, or changed in signature;
- sizes beyond a threshold, and coupling that appeared.

A short narrative over the computed list is optional and folded under it. Explain on a pull request can extract the facts of the changed files in the same session, as an option in its dialog with its share of the estimate.

### The diff experience

Everything below is drawn from facts, with no model call.

**One merged model.** Brainiac builds the diff's model itself: the head's map, plus every element and relationship only the base had, each tagged `#added`, `#removed`, `#changed` (its files changed), or `#unknown` (it could not be matched). One model means one layout. LikeC4 lays out each model on its own, so drawing the base and the head as two models would rearrange the boxes between them and the reader could not tell a change from a new layout. The prototype also showed why it must be merged, not layered: removed parts exist only in the base, and LikeC4 relationships have no names, so their tags cannot be added from a file beside the model.

**Before / Diff / After** is the panel's switch, the way an image diff flips between two versions:

- **Diff:** added parts green with +, removed parts kept where they were as faded, dashed **ghosts** with −, changed parts amber with ~, unknown grey with ?.
- **Before:** the added parts hidden, the removed ones drawn as they were.
- **After:** the removed parts hidden, the added ones drawn plainly.

The modes only hide and restyle elements of the merged model, so nothing moves unless it changed. `[` and `]` flip between them, as they follow the tour in Explain. Badges and line styles carry the meaning as well as colour, so the marks read without telling green from red.

**Any view, not one generated view.** The overview, a feature, the storage map, or a flow can each be shown "with this change". The panel opens on a view of only what the change touched (the parts it changed and their neighbours, the prototype's PR #13 view), and the reader can switch to any view of the map. A view the change does not touch says so.

**The list drives the map.** The computed list comes first, and the map answers "where is this?": selecting a row dims the rest and centres its element, and selecting an element filters the list. The same link runs to the patch: a file in Files Changed highlights its element, and an element opens its hunks.

**Flows marked by a change.** Each flow step cites a file and line. A step whose cited lines the change touched is marked ~ and keeps its place, with Show the hunk and Rewrite this step; a step whose quote is no longer found is ? and stays dotted until the map is rewritten, never silently kept. Steps whose files are the same blob as before are not read again.

**Two more modes, later.** **Side by side** (before and after panning and zooming together) as a full-window mode opened from the panel, since it halves the space beside a patch. **Step through the commits** of a long pull request, cheap because facts are kept per file version, only if long pull requests ask for it.

What to build first: the overlay on any view with the list driving it, and the Before / Diff / After switch on top.

### Extending LikeC4

Three levels, each taken only when the one before falls short:

1. **The model only.** What the prototype did: the diff tags in the kit, and a generated view per change with a style per tag. No code in LikeC4. Enough for phase 0 on Brainiac itself; it gives one generated view, not any view, and no link between a list and the drawing.
2. **Wrap the renderer** (for the release). LikeC4's React components inside Brainiac's panel, with Brainiac's own diff layer owning the change list, the modes, and the selection, and driving focus and highlight in the drawing. Which props and events LikeC4's components offer for that is not yet checked, and is the first thing to read before building.
3. **Contribute upstream.** If hiding parts of the merged model still lets the layout move, a compare mode in LikeC4 itself (two models, one pinned layout, ghosts drawn natively), contributed to the project, which is MIT and would gain a feature every user reviewing architecture changes could use. A fork is the last resort: it would hold Brainiac to one version.

### Cost, time, and failure

- **The estimate of a first map** comes from the repository's size and this Mac's earlier maps; on a Claude plan it says "Uses your Claude plan".
- **Large repositories** are composed in parts: the overview first, then each part's views in a run of their own, each checked and kept as it finishes. A run that fails leaves the parts before it.
- **Sleep** pauses a run as for any explain run (SPEC section 14, Sleep and limits).

## Not automatic

The first draft had **Keep it current**, composing the map again whenever the default branch moved. It is out: it would start an agent with the repository's code and the user's token without anyone pressing anything, against the first rule of explanations (SPEC section 14: Never automatic); a cost cap means nothing on a Claude plan; and the default branch moves only on a fetch, which is off by default. What stays automatic costs nothing: facts from Git, and marking what the map has not read yet.

## Where it fits

- **On v0.6's machinery.** A map run is an explain run with another subject (a repository at a commit) and another output file (the model). It reuses the container, the input bundle, the checks' approach, the answer on sharing a repository's code (`sharing.rs`), the ledger, the panel, and Save as note.
- **In the explanations feature,** so it adds no import between features to the boundary test's `ALLOWED` list.
- **Code sharing:** a repository's answer covers maps, and the first Map dialog says the agent reads the whole repository.
- **Storage:** models, facts per blob, the parts, and prose in `history.db` with explanations; Delete map removes them and the conversations of the map's runs, which held the repository's code.
- **Concepts** join the ledger only as the reader marks them, per repository.
- **In the window:** Map is a tab on a repository; Architecture is a tab beside Explanation in the side panel wherever a patch is (a pull request's Files Changed, a branch's changes, a commit, a run's result). LikeC4 is loaded only when a map opens.

## What exists already

Surveyed on 10 October 2026.

| Kind | Examples | What it does | What it lacks for this design |
| --- | --- | --- | --- |
| AI-written codebase wikis | DeepWiki (Cognition), DeepWiki-Open | A wiki per repository with diagrams and questions | No diff between commits, no check of its claims, no history |
| Architecture as code | LikeC4, Structurizr (C4 model) | One model, many views: zoom levels, flows, layout, validation | Written by hand; nothing ties the model to the code |
| Dependency graphs and rules | dependency-cruiser, import-linter, ArchUnit, cargo-modules, depdog (Go, with a diff against a Git ref) | Computed imports, rules on them | One language each; rules, not understanding |
| Change coupling from history | CodeScene, code-maat | Parts that change together, and the trend | No model of the parts or their flows |
| Repository maps for agents | Aider | Definitions and references per file with tree-sitter, across languages | Made for a model's context, not a reader |

This design uses the second row to draw, the third and fifth to compute, the fourth for coupling, and an agent to compose what none of them can: which parts and flows matter and what they are for, checked against the computed facts. Sources: [DeepWiki](https://www.x-cmd.com/blog/250502), [DeepWiki-Open](https://dev.co/devops/open-source/deepwiki-open), [depdog](https://pkg.go.dev/github.com/matterpale/depdog), [CodeScene, change coupling](https://docs.enterprise.codescene.io/versions/6.0.0/guides/technical/change-coupling.html).

## On Brainiac itself

### `pnpm map:diff` (since 10 October 2026)

`scripts/module-diff.sh` and `src-tauri/examples/module_map.rs` print, as Markdown, how a change moved Brainiac's Rust modules: modules, imports against the boundary rules, `ALLOWED`, sizes, and a Mermaid graph with the change in colour, read with the boundary test's own reader. CI adds it to each pull request's run summary. It is the computed half of the architecture diff, and stays as that: what it lacks is everything the agent adds (which arrows matter, flows, the frontend).

### The prototype (11 October 2026)

A LikeC4 model of Brainiac, composed by an agent in a session with the maintainer, about 730 lines:

- **Facts it started from:** modules, files, and sizes from the sources, and for every import between modules the names it uses (`explain → agents` uses `AgentRunService`, `RunArtifacts`), computed by a script; the Explain flow traced through the code by a second agent, 47 steps with file and line.
- **What the agent composed:** thirteen views. Brainiac in its context; inside the app (frontend, shell, core, five features, the run controller, the container, four stores); the frontend, the core, and each feature; Explain in two flows (from the click to a started run, 19 steps; the run, the checks, and the panel, 22 steps); the exceptions in `ALLOWED`; and where data lives. Labels from the imported names ("an explanation is an agent run of its own kind"), `models` left out because nearly everything imports it.
- **What it showed:** zooming, flows, and search came from LikeC4 with no code; the inside-the-app view is busy, because arrows between modules add up to many arrows between layers, so choosing what a view leaves out is the agent's real work; and LikeC4's quirks (reserved words, nested actors) are worth telling the agent in its prompt.

- **A diff on it:** PR #13 drawn on the map from the computed facts: `hosting` added (green), the three imports into it added, the two old exceptions into `forge` removed (red, dashed), `forge` changed (amber), in a view of only the four modules the change touched. It read in seconds, and showed that a diff has to be one merged model (The diff experience).

The prototype had no checks: it is what the agent writes, not yet what Brainiac would show.

## Phases, if it happens

0. **On Brainiac itself:** `pnpm map:diff` in CI, and the prototype model kept and composed again after large changes, to see whether the maintainer reads them.
1. **A map on demand:** facts, fact tools, the kit, composition, the checks, and the views drawn in the app.
2. **The architecture diff:** the Architecture tab with the merged model, Before / Diff / After, the list driving the map, and flow steps marked; LikeC4 wrapped (Extending LikeC4, level 2).
3. **Learning on an element:** Concepts and Questions.

Each phase is useful without the next. Whether learning comes before the diff is open.

## Spike before building

- **Composition:** with Claude Code and with OpenCode, on three repositories of different languages and sizes, the agent writes a model with the fact tools and LikeC4's MCP server. Scored by hand: whether the parts are the ones a maintainer would draw, whether the flows are right, and whether the views can be read.
- **Checks:** on Brainiac, the boundary test's reader is ground truth for Rust imports; how many `uses` relationships are invented, how many flow steps cite a line that does not hold their quote.
- **Facts:** whether tree-sitter's definitions and imports are good enough for TypeScript and Python, and what the agent must extract where they are not.
- **Recorded:** time and cost of a first map per thousand files, and of a map's diff for a typical pull request.

## Panel review (10 October 2026)

A client panel (three fictional personas, so hypotheses to check with real users) reviewed the first draft: a CTO who reviews pull requests, a learner new to a monorepo, and a contractor working across client repositories.

- **What they would use:** the CTO the architecture diff; the learner the explanations of each part, daily at first; the contractor the overview, once per client repository.
- **Agreed:** change coupling is the useful statistic, and a grade-like label such as "shallow" is not; the first map's cost and size need an answer; the computed list of a diff is trusted more than its narrative; automatic rewriting comes last.
- **Disagreed:** the diff first (CTO) or learning first (learner); the parts defined in the repository (CTO, learner) or not (the contractor cannot add files to a client's repository).

What it changed, all kept in this rewrite: a missing fact is unknown, never removed; Keep it current is out; the parts come from the repository when it defines them and from this Mac otherwise; the first map gets an estimate and is composed in parts; Explain's names (I know this, Questions); Start here, Delete map, and facts per repository.

## Not in this design

- Maps written into the repository by Brainiac.
- Running without the user.
- Enforcing rules on a repository's structure; a map shows what is.
- Call graphs computed for every function; flows are traced for the actions the agent chooses.
- A score or grade for a codebase.

## Open questions

- **The diff or learning first.** The maintainer and the CTO persona would use the diff most; the learner persona would use learning most.
- **Which flows.** The agent chooses them from entry points (commands, routes, menu items, a CLI's subcommands); should the user ask for one ("what happens when I save a note?"), as a run of its own?
- **LikeC4's components:** which props and events they offer to focus, highlight, and hide elements by tag, and whether a merged model keeps its layout when parts are hidden or is laid out again (Extending LikeC4).
- **Theming:** how far LikeC4's components can take Brainiac's look, and whether that matters.
- **The size of a merged model** for a large repository, and of the view of only what a change touched.
- **The web component or the React components** inside the app, and what loading a few megabytes on first open costs.
- **A team's model in the repository:** whole models or only the parts and their paths, and how Brainiac merges its own views with a hand-written model.
- **Interface items** in languages without visibility keywords, and in configuration such as Terraform.
- **A command line for CI** in any repository: `brainiac map --diff` can print the computed diff from stored facts, but CI does not have the Mac's facts.
