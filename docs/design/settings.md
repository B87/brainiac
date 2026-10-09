# Design: Settings

Design notes for reworking Settings so it shows what needs attention first, keeps its lists usable as they grow, and lets a problem be fixed from where it shows. Not scheduled for a release yet; nothing here is in `SPEC.md`.

UX Design Artifact: https://claude.ai/artifact/9bRvjbJoFghLmxccnqu77a (nine screens, light and dark from the app's own tokens)

## Why

A review of today's Settings by a panel of three fictional users (`.claude/agents/client-panel.md`; reactions, not user data) found:

- **Nothing says what is broken** until each section is opened. The sidebar shows no status, and a failed secret test is grey, not red (`SecretsSection.tsx`).
- **A fix lives somewhere else.** Secrets says to change a source "where it is entered" with no link; approving restored items is split across Accounts, Secrets, and each agent's page, with three labels.
- **No search or keyboard path.** `Cmd+,` opens the last section; typing does nothing, arrow keys don't move, and ⌘K has no settings.
- **Lists are flat.** Secrets, known concepts, and stored explanations have no filter, grouping, or per-group delete, and each is one long 640 px column.
- **Labels collide.** Agent Access sits next to Agents; the Repositories and Databases panes list neither.
- **Agents hides its tests.** Whether an agent may run on a host is known only by opening each host's page.

## What the user gets

- **A sidebar in groups:** Overview; **Preferences** (General, Git and repositories, Notes and search, Databases, Backup and restore); **Services** (Accounts, Secrets, Agents, MCP server, which was Agent Access); **Explanations** (Agent and depth, Repositories asked, Concepts you know, Stored explanations).
- **Badges on the section where the fix is:** a red count for what failed, amber for what waits (an expiring token, restored items), read aloud as "1 need you" or "4 waiting".
- **Overview:** *Needs you*, each item with its fix (Test Again, Replace Token…, Review…); *Set up an agent*, a checklist with times, shown until the first test passes, since runs and explanations share the agent; *At a glance*, one line per area.
- **Search:** ⌘F anywhere in Settings, even in a field. Matches from every section show where they live and can be changed in place; ↩ opens the section. Former names still match ("agent access" finds MCP server). The same settings are in ⌘K after "settings".
- **Lists that grow, all built the same way:** one table with collapsible sections, each header counting its problems; a filter (`/`); **Group by**, remembered; **Problems first**; J/K and ↩; a side panel for the selected row with its fix; and "n more" for the rest. A section the user closes stays closed and still counts its problems.
  - **Secrets:** grouped by kind, source, or not at all. A source's panel shows what failed, the exact program and arguments it ran, **Test Again** and **Refresh**; its source is still changed where it is entered (**Edit Connection…**).
  - **Agents:** agents and run hosts in one table, as two sections (or grouped by host or provider), with the columns Name, Setup, Tests, and Status. **Tests** shows a chip per host for an agent, and per agent for a host, each in words ("build-01 failed", "OpenAI not tested"); a host's job shows its step and progress in the row. The side panel of an agent lists its test on each host, why one failed, **Update Key…**, **Test on Every Host**, and its setup with **Edit Setup…**; of a host, its running job's steps and output with **Cancel**, its address, host key, controller and image, and **Emergency Stop…** apart from the rest. ↩ opens today's full agent or host page; ⌘↩ tests the selected row. A restored agent or host links to the review.
  - **Stored explanations:** by repository or month, cost for a chosen period, delete per repository or for the selected ones.
  - **Concepts you know:** tabs per kind, **Merge into One…** and **Forget** for a selection; a project pattern names its repository.
  - **Repositories asked:** one row per repository and provider, its answer, how many were explained, and **Ask Again**.
- **After a restore, one review.** Everything restored waits in one dialog, with nothing ticked: each item says exactly what allowing it does (the program and its arguments, the Keychain item, or the host key with the command that prints it on the host). An item whose secret did not travel says so and offers to paste it. What is left waits under Backup and restore and on the Overview.
- **Style:** the app's colors, type, and spacing from `src/index.css`, light and dark; a visible focus ring; one primary button per screen; click areas of at least 24 px; status always in words as well as color.

## What it does not change

- One account per provider (`SPEC.md` section 10). Grouping by workspace was tried and dropped: connections belong to repositories, a repository can be in several workspaces, and items would show and count twice.
- A source is changed where it is entered (section 12). Secrets shows and tests; it links to the place that edits.
- Each restored item is allowed on its own. The review only gathers them; it never allows anything the user did not tick.

## Spec changes it needs

- Section 3: the Settings sidebar's groups, the Overview, badges, search, and `⌘F` in the keyboard table.
- Section 9: Agent Access is named MCP server in Settings.
- Sections 8, 12, and 13: the restore review that gathers what each section confirms today.
- Section 12: Settings → Secrets' grouping, filter, and source panel.
- Section 13: Settings → Agents as one table with the side panel, and Test on Every Host.
- Section 14: Settings → Explanations as four panes. v0.6 is building that pane now; agree on the split before either is built.

## Phases

1. Labels, badges, red failures, and the Overview.
2. Search and keyboard.
3. The list pattern (sections, filter, grouping, side panel): Secrets and Agents first, then the Explanations panes with v0.6.
4. The restore review.

## Open questions

- Which release: v0.6, because of the Explanations panes, or 0.6.x?
- Whether "expires in n days" can be known: the design assumes GitHub reports a token's expiry on its answers.
- Test on Every Host: one host job after another, or at once?
- Where a list's remembered grouping and closed sections are stored, and whether they are exported.
- Agents grouped by host repeats each agent under every host: whether that grouping earns its place.
- How Merge into One… treats a concept that two explanations' notes point to (with v0.6).
