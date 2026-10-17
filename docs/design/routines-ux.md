# Design: routines, the experience

Proposal, 16 Oct 2026; steps 1 to 3 built 16 and 17 Oct 2026. It reworks where routines live and how they look; it adds no feature beyond those already planned in [`scheduled-runs.md`](scheduled-runs.md) (Run now, schedules, approval, missed and overdue, cost and pause, the Today card, Save as note, Create task, memory and earlier results). Anything that would be new is in *Open questions*, not in the design.

## Terminology (decided 16 Oct 2026)

- **Routine:** the saved definition. Section "Routines", button "New Routine…", Runs tag **Routine**.
- **Briefing:** a result a routine leaves ("No briefing" when a run left nothing usable, `.brainiac/briefing.json`, the Briefings folder for notes).
- **Schedule:** when a routine fires on its host. Built (17 Oct 2026).
- Code uses these words too (`routines`, `RoutineService`, `list_routines`), since the rename of 16 Oct 2026; a briefing's rows stay in `briefing_results`.

## Why redo it

What exists today is the first slice, built to test the idea: a **Routines** block above the tasks on Today, and a dialog to add or edit one. It already shows three problems, and schedules will make them worse.

1. **There is no list.** The only way to see what is configured is to scroll Today. A routine that has never run, or whose latest result is old, is as prominent as one that needs attention. A user with five routines gets five cards on a page that is about *today*.
2. **Today does two jobs.** It is where you read results, and the only place to manage definitions. Today's own rule (open tasks that are due or planned for now) is about what needs you today; a definition is not that.
3. **The dialog is too small for the object.** A routine has a prompt, a start, an agent, a host, a time limit, a memory, earlier results, and soon a schedule, an approval and a cost. That does not fit a modal, and the schedule's approval needs room to say what it permits.

Two smaller ones: a routine's run appears in Runs as an unlabelled ordinary run, with nothing to tie it to its routine; and there is nowhere to see, for the host, which routines fire on it.

## Principles

- **Two words, two things.** A **routine** is the saved definition: what to run, where, and optionally when. A **briefing** is one result a routine leaves: a report. A *schedule* is a property of a routine, not another kind of object. The UI says "routine", "briefing" and "schedule", never "scheduled run". A routine may later also leave a change (a branch for a pull request), which is why the definition is not named for its report.
- **Today is the reading room, a section is the workshop.** Results are read on Today; definitions are managed in their own place.
- **Silence is never good news.** A routine that did not run, ran and failed, or is waiting for approval looks different from one that found nothing, in the list, on Today, and in the sidebar.
- **One place per fact.** The run's conversation stays in Runs, the host's state on the host's page, the secrets in Settings → Secrets. The routine links to them; it does not copy them.
- **Reuse the shapes Brainiac already has.** A sidebar section like Databases and Runs, a table with a side panel like the repository dashboards, a page with grouped settings like a host's page, tags like Explain's.

## The pieces

### 1. A Routines section in the sidebar

A new sidebar section after Runs (also View › Routines, `Cmd+K`, and in the collapsed rail with the same badge rule). It opens a table of every routine:

```
Routines                                               [ New Routine… ]
┌──────────────────────────────────────────────────────────────────────────────┐
│ Name            Repository  Runs on   Schedule         Last result   Cost    │
├──────────────────────────────────────────────────────────────────────────────┤
│▌Morning check   api         home-srv  Weekdays 07:00   ● 2 findings  $0.41   │
│                                       next Mon 07:00   today 07:02            │
│ Hacker News     blog        home-srv  Daily 08:00      ● Report      $0.12   │
│ Outdated docs   docs        This Mac  Not scheduled    ▲ No briefing  —      │
│                                                         failed: plan limit    │
│ Weekly bugs     api         home-srv  ⏸ Paused         ● Nothing found  …    │
└──────────────────────────────────────────────────────────────────────────────┘
```

- **Columns:** name; repository; host; schedule in words and the next time (before schedules exist: "Not scheduled" and Run Now only); the latest result's state and when; the cost of the latest run, and the month's total once the schedule exists. At the narrow widths Brainiac supports, the table drops to name, last result and status; the rest moves to the side panel.
- **Row states**, each with its words and not only a colour: **Working** (running since), **Report / Findings / Nothing found** (the agent's claim, as now), **No briefing** (failed, with the reason in one line; never "nothing found"), **Paused**, **Needs approval** (a change since it was approved), **Overdue** (the host has not reported a run within the window), **Host unreachable**.
- **Side panel** for the selected row (as the repository dashboards): the latest result's summary, **Run Now**, **Edit**, **Open the run**, and the last few results as a list of dates and states.
- **Sorting:** things that need the user first (No briefing, Needs approval, Overdue, Working), then by next run, then by name. No other sort in this design.
- **The badge** on the section counts routines that need the user: a failed latest result, a schedule that needs approval, an overdue one. It is the Runs badge's rule, and it is how silence becomes visible from anywhere in the app.
- **Empty state:** one paragraph and **New Routine…**, with a concrete example ("Check production errors every morning and tell me what is new") in the text, not as a template.

### 2. A routine's page

Opening a row opens its page, in place of the table, with a breadcrumb back (as a host's page does). It is a page and not a dialog, grouped like Settings → Agents:

```
Routines › Morning check                    [ Run Now ] [ Pause ] [ ⋯ ]
 ● 2 findings · today 07:02 · 4 min · $0.41            Open the run

 Latest │ History │ Definition │ Schedule
```

- **Latest** is the reading view, the same card Today shows, full width: summary, findings with links and "seen before", proposals with Create Task / Save as Note / Copy, Save as Note for the whole routine. A failed result shows the reason, what the run did (**Open the run**), and what to try (the run's own message; "Try again" is Run Now).
- **History** lists results by date with their state, cost and duration; selecting one shows it in the Latest layout. It is the list the memory's "earlier results" come from, so the user can see what the next run will be shown.
- **Definition** holds what is edited today, as grouped fields instead of a dialog: *What to check* (name, repository, start from, prompt); *Where it runs* (agent, host, time limit, with the same "not ready: …" line New run gives); *What it can use* (the agent's host secrets and tools layer, read from the agent's page and linked to it, with the same "this host does not hold SENTRY_TOKEN" warning New run shows); *Memory* and *Earlier results shown to a run* (as now).
- **Schedule** is the planned phase 4 and is where its states live (below). Until then the tab says "Not scheduled. Run Now starts it when you ask." and nothing else.
- **Delete** is in the **⋯** menu with a confirmation that names what goes (the definition, its results, its memory) and what stays (the runs in Runs).
- A **disclosure** line sits under the header on every tab, because every start sends code and a prompt out: "Sends this repository's commit and the prompt to Anthropic from home-srv. The network is unrestricted. It can read SENTRY_TOKEN." Same words as New run's *Before you start*, not a second wording.

### 3. Today becomes a digest

Today keeps only what is relevant to the day, above the tasks, as compact rows and not cards:

```
Routines
  Morning check   ● 2 findings · 07:02            Open
  Hacker News     ● Report · 08:01                Open
  Outdated docs   ▲ No briefing: plan limit       Open
```

- Shown: a routine whose latest result is from today, or that is working, or whose latest result failed, or that needs approval. Quiet routines (an old, ready result) are not shown on Today, so Today does not grow with the list. The rule uses the date, so no "seen" state is needed.
- A row opens the routine's page at Latest. Run Now, Edit, and **New Routine…** leave Today: they belong to the section. A one-line link, "All routines", sits at the end of the block.
- A routine's findings are not expanded on Today. Reading is on the page; Today answers "did anything happen" in one line each, and "did something not happen" in the failed and overdue rows.
- If nothing qualifies, the block is absent, not an empty state.

### 4. Runs and the host know about routines

- **Runs:** a routine's run carries a **Routine** tag (as Explain runs carry **Explain**), is titled by the routine's name, and the kind filter (runs, explanations) gains routines. Opening it shows the usual conversation with a **Open routine** link back. Nothing else about Runs changes.
- **A host's page** gains **Schedules on this host** next to Health and Secrets: each routine that fires there with its next time and state, and **Pause all** (planned in the cost and pause section). A routine's row says "Host unreachable" when the host's Health says so, which is how "the host is down on a Monday" reaches the table.
- **Settings → Secrets / Agents:** unchanged. Their lists do not show routines; the routine's page shows what it uses.

### 5. Schedules (phase 4), in these places

The planned behaviour is unchanged; this is only where it is seen.

- **Schedule tab:** days and time (a small fixed set: daily, weekdays, weekly on a day), the host, **Late start** (fire if the host was down at the time, off by default, as planned), a cost ceiling and the deadline (both required and shown), and **Pause**.
- **Approval:** saving a schedule, or changing anything it depends on, opens one **Approve schedule** step that lists what the unattended runs may do: the repository and commit rule, the prompt, the provider and who pays, the host, the network (unrestricted), the secrets it gets, the tools layer's digest, the deadline and ceiling. A change to any disables the schedule (**Needs approval** in the table) until approved again. It is the *Before you start* list of New run, once, with the schedule.
- **Missed and overdue** are states in the table and the page header, with the time they were expected. "Missed" is the host's record; "Overdue" is the app's, from the window since the host last reported.
- **Cost:** the latest run's cost on the row, the month's total and the ceiling on the Schedule tab, "uses your plan" with its usage as reported on a subscription, never a computed dollar figure.

## What changes in the existing build

| Today | After this design |
| --- | --- |
| Routines block on Today with full cards, New Routine…, Run Now, Edit | A compact digest on Today; everything else moves to the Routines section |
| Add/Edit in a modal dialog | The Definition tab of a routine's page; **New Routine…** opens a new page in the same layout |
| Result card inside Today | The Latest tab; Today shows one line per qualifying routine |
| Routine runs look like ordinary runs | **Routine** tag, the routine's name as title, filter, link back |
| No list, no states beyond the card | The table, its states, the sidebar badge |

Nothing in the data model changes for this: the section reads the same `list_briefings` (plus the result history, which is stored), and the Runs tag needs the run to remember its routine. Schedules add records on the host as `scheduled-runs.md` says.

## Order of work

1. The section, the table with its states, the page with Latest, History and Definition, and the dialog's removal. Today becomes the digest. These need no new backend beyond listing results and one link from a run to its routine.
2. The Runs tag, filter and link, and the sidebar badge.
3. With schedules (phase 4): the Schedule tab, approval, the host's **Schedules on this host**, missed and overdue.

## Not in this design

- Templates or a library of routines. The empty state gives an example sentence, no more.
- Reordering, folders or tags for routines; a search of results.
- Per-finding "seen" or dismissed state, comments, or assigning findings.
- A pull-request result type, continuing a routine's run, a Jira adapter, notifications from the host. They are planned elsewhere; this design leaves room (below) and builds nothing for them.

## Room left for what comes next

- **Pull-request results:** the Latest tab would gain a **Changes** block (files changed, Review, then Push and Create pull request on the user's action), and the table a column or tag for the result type. The Definition tab's *What it can use* would also list the push token as the user's, in the Mac's Keychain, never in the run.
- **Continuing the run:** **Open the run** already goes to the conversation; a kept-open run would show *Continue in the run* in its place, with its remaining minutes.
- **Notifications:** a finished or failed routine could use the same macOS notification Explain uses when Brainiac is not the active app; it is a setting on the routine, not a design here.

## Open questions

| Question | Decides |
| --- | --- |
| ~~Is "Briefing" the right word for the definition?~~ Decided 16 Oct 2026: the definition is a **Routine**, and **Briefing** names its report | Nothing left to decide; the section, the Runs tag and the strings use Routine |
| Should Today's digest also show routines whose latest result is *old but never opened*? It needs a seen state | Whether to add one small state (like Activity's Mark seen), which this design avoids |
| Should a finished Run Now notify when Brainiac is not the active app, as Explain does? | A per-routine notify setting, or none until host notifications exist |
| Does "Pause all on a host" belong on the host's page, the Routines table, or both? | Where the control is; the design puts it on the host's page |
| Should the section be a sidebar item of its own or a tab inside Runs? | The sidebar's length; the design chooses its own item because the objects and the questions asked of them differ from a run's |
