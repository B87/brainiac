# Design: note templates and Kindle highlights

Design notes for two v0.5 note features: note templates, and importing Kindle books and their highlights into a note built from a template. They go deeper than the summary in [`roadmap.md`](../roadmap.md) (Note templates and Kindle highlights — v0.5). They are not the specification yet. When v0.5 starts, the behavior moves into [`SPEC.md`](../../SPEC.md) section 5, the design into [`architecture.md`](../architecture.md), and this file keeps only the background and open questions.

Both features build on what v0.2 already guarantees for notes (`SPEC.md` section 5): notes are ordinary Markdown, opening or indexing a note never rewrites it, unknown frontmatter keys are preserved, and a save never overwrites a version Brainiac has not seen. The Kindle import is one adapter of the v0.5 import pipeline (`roadmap.md`, Capture and imports — v0.5) and follows its rules for provenance, duplicates, and user edits.

## Why these two

Templates and Kindle highlights are among the reasons people keep Obsidian. Obsidian's built-in Templates is small: it inserts a snippet at the cursor of the active note and knows three variables, `{{title}}`, `{{date}}`, and `{{time}}`, with Moment.js formats such as `{{date:YYYY-MM-DD}}`. It has no logic, and creating a new note from a template needs another plugin. Logic comes from community plugins: Templater (JavaScript) and the Kindle Highlights plugin (Nunjucks templates). Brainiac can cover both with one template engine in Rust and no plugin system.

## Note templates

### What the user gets

- **New Note from Template…** in the File menu, the command palette, and the Notes view: choose a template, enter a title, and the note is created from the rendered template in the current folder.
- **Insert Template…** in the editor: the rendered text is inserted at the cursor as an ordinary edit, so undo removes it and the note saves as usual.
- A preview of the rendered result before either action, with errors shown at their line and column. Nothing is created or inserted when rendering fails.
- `{{ cursor }}` in a template marks where the cursor goes afterwards; it renders as nothing.

### Where templates live

- A templates folder in the vault, chosen in Settings (default `Templates/`). Templates are ordinary `.md` files, edited like any note; opening one shows its template text and never renders it.
- Notes in the templates folder are still listed in the tree and found by search, labelled Template. Their links, mentions, and checkboxes are not indexed as the vault's own: they do not appear in backlinks, unresolved links, repository suggestions, or Today. A template full of `[[{{ title }}]]` must not create unresolved links.
- A template's own `brainiac_id` (a template created in Brainiac gets one like any new note) is never copied: the new note gets a fresh ID.
- An optional `brainiac_template` object in a template's frontmatter holds settings for that template and is removed from the rendered note. In v0.5 it has one key, `kind`: `note` (the default) or one of the Kindle kinds below. Templates of another kind are not offered in New Note from Template.

### Variables

| Variable | Value |
| --- | --- |
| `title` | The title entered for the new note; for Insert Template, the open note's title |
| `date`, `time` | Today's date and the current time, formatted as `YYYY-MM-DD` and `HH:mm` |
| `now` | The current date and time, for formatting with a date filter |
| `folder` | The vault-relative folder the note is created in |

Obsidian's `{{date:FORMAT}}` and `{{time:FORMAT}}` are not valid Jinja. A small pre-pass rewrites them before parsing, translating the common Moment.js tokens (`YYYY`, `YY`, `MMMM`, `MMM`, `MM`, `M`, `DD`, `D`, `dddd`, `ddd`, `HH`, `H`, `mm`, `ss`), so templates copied from an Obsidian vault work unchanged. An unknown token is reported in the preview rather than guessed.

### Frontmatter

A template may start with frontmatter, which becomes the new note's frontmatter after rendering. Inserting a template into an existing note adds only the frontmatter keys the note does not have; it never changes a key the note already has. As everywhere else, the note's existing frontmatter text is kept as written: Brainiac adds lines, it does not reserialize the block.

String values rendered into frontmatter go through a `yaml` filter that quotes them when needed, so a title such as `Notes: part 1` does not break the YAML.

### Engine: MiniJinja

- **Syntax.** MiniJinja implements Jinja2, the same family as the Nunjucks that Obsidian's Kindle plugin uses, so its users' templates need few changes.
- **Bounded work.** The optional `fuel` feature (`Environment::set_fuel`) gives each render an instruction budget and fails with `OutOfFuel` when it runs out, which stops a runaway loop in a user's template. The budget is set from tests: well above what the shipped Kindle template needs for a book with 5,000 highlights.
- **No reach outside the template.** No template loader in v0.5, so no `include`, `import`, or `extends`; no custom functions that touch files, the network, or the clock beyond the variables above.
- **Bounded output.** Rendering stops at 5 MiB, the largest note Brainiac edits.
- **No HTML escaping.** Output is Markdown; autoescape is off. The `yaml` filter covers frontmatter.
- **Missing values render empty.** A variable a source does not provide (a Kindle field only Amazon's cloud has, for example) renders as nothing instead of failing the import; the preview lists the undefined names it met, so a typo is still visible.

Tera and handlebars-rust were not compared in depth; MiniJinja is chosen for its syntax match and fuel. Confirm the version and the contrib date filters when v0.5 starts.

### Commands

| Command | Input / output |
| --- | --- |
| `list_templates` | Templates of a kind in the templates folder, with title and path |
| `render_template` | Template path, kind, and context (title, folder, or an import job's item) → rendered text, cursor offset, undefined names, or an error with line and column |
| `create_note_from_template` | Template path, title, folder → the new note, created through `NoteService` |

Insert Template calls `render_template` and inserts the text in the editor, so it goes through the editor's normal save.

## Kindle highlights

### Where highlights come from

| Source | Books covered | Offline | Notes |
| --- | --- | --- | --- |
| `My Clippings.txt` on a Kindle e-reader | Every book annotated on that device, including sideloaded books and personal documents | Yes | One file per device; not written by the Kindle apps for Mac or iOS. No published format, and its wording follows the device's language. |
| Kindle Cloud Reader notebook (read.amazon.com/notebook) | Only books bought from the Kindle Store; never sideloaded or emailed documents | No | Needs the user's logged-in Amazon session. Breaks when Amazon changes its login or pages. Amazon's Conditions of Use (updated 14 August 2026) exclude "data mining, robots, or similar data gathering and extraction tools". |

v0.5 imports `My Clippings.txt` only. It covers the most books, it needs no account, and it fits the import rules: Brainiac does not borrow browser cookies, get past login walls, or run page JavaScript (`roadmap.md`, Extraction limits and search behavior). The cloud notebook stays out; if it is ever added, it is opt-in, runs in a webview session the user logs into, and accepts that Amazon may block it. Other routes (the Kindle app for Mac's local data, emailed notebook exports, the Readwise API for people who already use Readwise) were not verified and are candidates for later adapters.

### The user's flow

1. **Add to brain → Kindle highlights…**, or drop a `My Clippings.txt` on the window. The user picks the file; Brainiac does not assume where it sits on the device.
2. The preview lists the books in the file with, for each: new highlights, highlights already imported, revised highlights, truncated highlights, and whether a note for the book exists. The user chooses which books to import and the folder for new book notes (default `Inbox/`, remembered).
3. A book seen for the first time gets a new note rendered from the Kindle book template. A book already imported gets its new highlights added to its existing note, shown in the preview first.
4. New book notes appear in the Inbox like any other import.

One import job covers one file and many books. Retrying the job resumes it without duplicating notes or highlights.

### Parsing `My Clippings.txt`

Amazon publishes no format. What third-party parsers agree on:

- Entries are separated by a line of ten `=` signs. Each entry is a title line, a metadata line, a blank line, and the text.
- The title line ends with the author in parentheses: `Title (Author)`. The last parenthesized group is the author; several authors are separated by `;`. Without parentheses the author is unknown.
- The metadata line starts with `- ` and holds the kind (highlight, note, bookmark, clip), a page and/or location range, and the date added.
- The file is UTF-8 with a byte-order mark, and a U+FEFF can also appear before later title lines. Strip it from every title line, not only the first character of the file. Accept CRLF line endings.

Pitfalls the adapter handles:

- **The device's language.** The metadata words follow the Kindle's language (`Added on`, `Ajouté le`, `Añadido el`, `Posición`, …). A keyword table per language parses kind, location, page, and date. Ship only languages with a test fixture; when no keyword matches, parse by structure (separator, title, metadata, text) so the highlight still imports with less metadata, and say so in the preview. A file can mix languages if the device language changed. An unparsed date is kept as written.
- **Bookmarks** have no text and are skipped, with a count in the preview.
- **Notes** typed on the Kindle are separate entries at a single location. A note whose location matches the end of a highlight in the same book is attached to that highlight; otherwise it becomes its own block. This is the common heuristic, to be checked against real files.
- **Page or location.** Some entries have only a page, some only a location, some both. Identity and ordering use the location when present and the page otherwise.
- **Duplicates and edits.** The file is append-only: highlighting the same passage twice adds a second entry, and extending or editing a highlight adds a new entry with different text and range while the old one stays. Exact duplicates (same book, kind, location, text) are dropped. A highlight in the same book whose range overlaps an earlier one and whose text contains or extends it is a revision, shown as such in the preview (below).
- **Publisher export limits.** Publishers cap how much of a book can be exported, reported at roughly 10–20% and sometimes lower; past the limit the device writes a notice instead of the text. A highlight with the notice is imported as truncated, labelled in the note, and never treated as a revision of a complete highlight. Personal documents are reportedly not limited.
- **Ordering.** The file is in the order things were added; highlights are rendered in book order.

### Identity

- **Book key.** Clippings carry no ASIN, so a book is identified by its normalized title and author: Unicode NFC, case-folded, whitespace collapsed, byte-order marks removed. The same book can still appear under two titles (another edition, a corrected title on the device); the Obsidian Kindle plugin has an open bug from exactly this. The preview offers **Import into existing note…**, and that choice is remembered as an alias of the book key.
- **Note.** The book's note is found by its `brainiac_id` through the book key stored in `brainiac_source`, never by its path, so renaming or moving the note between imports does not create a second one (the failure Readwise has, which finds the note by file name).
- **Highlight.** `kh-` followed by 10 hex digits of SHA-256 over the book key, the location start (or page), and the normalized text. The same highlight imported from two devices gets the same ID.

### The book note

The Kindle import uses two templates, so the book note is laid out once and each highlight block is laid out the same way on every import. Brainiac ships both, and **Customize** copies them into the templates folder, marked with `brainiac_template.kind` `kindle_book` and `kindle_highlight`.

Book template variables: `title`, `author`, `authorsLastNames`, `highlightsCount`, `highlights` (the rendered highlight blocks), `date`. Highlight template variables: `text`, `location`, `page`, `note`, `added`, `truncated`, `title`, `author`. The names follow the Obsidian Kindle plugin's, so its templates paste in with small Jinja/Nunjucks differences. Fields only Amazon's cloud provides there (`color`, `asin`, `imageUrl`, `publicationDate`, `lastAnnotatedDate`) are not available from Clippings and render empty.

Default book template:

```markdown
---
title: {{ title | yaml }}
author: {{ author | yaml }}
tags: [book]
---

# {{ title }}

by {{ author }}

## My notes

## Highlights

{{ highlights }}
```

Default highlight template:

```markdown
> {{ text }}

{% if note %}{{ note }}

{% endif %}— {% if location %}location {{ location }}{% else %}page {{ page }}{% endif %}{% if truncated %} · truncated by the publisher's export limit{% endif %}
```

The book template renders once, when the note is created; what it writes is the user's from then on. Values that change, such as the highlight count, live in `brainiac_source`, which Brainiac owns:

```yaml
brainiac_source:
  type: kindle_clippings
  id: "kindle:moby-dick|herman melville"
  imported: 2026-11-02T09:14:00Z
  content_scope: excerpt
  hash: "sha256:…"
  kindle:
    highlights: 42
    truncated: 3
    last_added: 2026-10-30
```

### Markers and re-import

Brainiac appends each highlight block's ID as an Obsidian block reference, `^kh-1a2b3c4d5e`, so the block can be linked from other notes in Obsidian and Brainiac can find it again. The user's template cannot drop it, because Brainiac adds it after rendering. Its exact placement (end of the last line, or its own line after the block) follows Obsidian's rules for the block type and is checked in the fixture suite.

On re-import, for each book with something new:

1. Read the note. If it has unsaved edits open in Brainiac, stop and say so; the save rules of section 5 apply.
2. Find the markers. Each new highlight is inserted after the block of the nearest earlier highlight in book order, as the Obsidian Kindle plugin does, or before the nearest later one. A book note with no markers left gets the new highlights appended at the end, and the preview shows where.
3. A **revised** highlight replaces the earlier block only if that block is still exactly as Brainiac rendered it; if the user edited it, the new version is added beside it. The preview shows both cases and lets the user add, replace, or skip.
4. Replace only the `brainiac_source` lines of the frontmatter. Every other byte of the note, including the user's text around and between highlight blocks, stays as it was.
5. Save through `NoteService` with the expected version, so a change from outside since step 1 stops the save.

A highlight the user deleted from the note must not come back. Markers alone cannot tell "deleted" from "never imported", so a ledger of seen items is kept in `brainiac.db`, because it cannot be rebuilt from the vault: `import_items` with the adapter, the item ID, the note, when it was first seen, the hash of the block as rendered, and its disposition (`added`, `superseded`, `dismissed`). The table is generic, for any adapter whose source has items that recur. If `brainiac.db` is lost and restored without it, the markers are the fallback, and a deleted highlight shows in the preview as new rather than being added silently.

### Originals

The roadmap keeps raw originals in `attachments/imports/<import-id>/`. A `My Clippings.txt` holds every book on the device, so a copy is kept only when its hash differs from the last copy kept; the book notes refer to it from `brainiac_source`.

## Testing

- Clippings fixtures are written by the tests, never real files from a device: public-domain books (Moby-Dick, Pride and Prejudice), each shipped language, byte-order marks mid-file, CRLF, page-only and location-only entries, bookmarks, notes, exact duplicates, extended highlights, the export-limit notice, and a book under two titles.
- Re-import: new highlights only; a revised highlight with and without user edits; a note renamed and moved between imports; a deleted marker; user text between blocks; a note changed on disk during the import; a retried job.
- Templates: fuel exhausted, output cap, undefined names, Obsidian date syntax, frontmatter quoting, `brainiac_id` not copied, templates excluded from backlinks and Today.

## Open questions

| Question | How to settle it |
| --- | --- |
| Do current Kindle models mount as a USB drive on macOS, or use MTP? This decides whether Brainiac can offer the device's file directly. | Try a recent Paperwhite, Colorsoft, and Scribe; until then the file picker is the only path |
| Exact text of the export-limit notice in each language | Real files from limited books |
| Where Obsidian accepts a block ID for quotes, lists, and paragraphs | Fixture notes opened in Obsidian |
| Whether to keep the Clippings original in the vault at all, given the highlights are in the notes | Decide with the `.eml` original-retention design |
| Attaching Kindle notes to highlights by location | Real files with notes on short and long highlights |
| A later cloud or Readwise adapter | Only after Clippings import is used daily; needs its own terms and account review |

## Sources

- Obsidian Templates: <https://help.obsidian.md/Plugins/Templates>
- Obsidian Kindle Highlights plugin: <https://github.com/hadynz/obsidian-kindle-plugin>, <https://community.obsidian.md/plugins/obsidian-kindle-plugin>
- Readwise, Kindle import and Obsidian export: <https://docs.readwise.io/readwise/docs/importing-highlights/kindle>, <https://docs.readwise.io/readwise/docs/exporting-highlights/obsidian>
- Clippings.io, importing `My Clippings.txt`: <https://docs.clippings.io/importing/importing-your-myclippingstxt-file/>
- Clippings parsers: <https://github.com/lvzon/kindle-clippings>, <https://github.com/NightMachinery/fyodor>
- Amazon Conditions of Use: <https://www.amazon.com/fetch/kiosk/legal/conditionsOfUse>
- MiniJinja `Environment`: <https://docs.rs/minijinja/latest/minijinja/struct.Environment.html>
