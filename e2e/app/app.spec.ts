/**
 * The v0.2 views clicked through in WebKit over the fake backend in fake.ts:
 * every test fails on a page error. What the backend does with the requests
 * is tested by `cargo test`; this checks the frontend sends the right ones
 * and shows what comes back.
 */
import { expect, type Page, test } from "@playwright/test";
import type {} from "./main";

test.use({
  baseURL: "http://localhost:1431/",
  viewport: { width: 1440, height: 900 },
});

let errors: string[] = [];

test.beforeEach(async ({ page }) => {
  errors = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  await page.goto("/");
  await expect(page.getByRole("button", { name: "Today" })).toBeVisible();
});

test.afterEach(() => {
  expect(errors).toEqual([]);
});

const calls = (page: Page, cmd: string) =>
  page.evaluate(
    (c) => window.fake.calls.filter((x) => x.cmd === c).map((x) => x.args),
    cmd,
  );

async function setUpVault(page: Page) {
  await page
    .getByRole("navigation", { name: "Repositories and workspaces" })
    .getByRole("button", { name: "Notes" })
    .click();
  await expect(
    page.getByRole("heading", { name: "Set up your vault" }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Choose Folder…" }).click();
  await expect(page.getByRole("navigation", { name: "Notes" })).toBeVisible();
}

test("Today and Tasks work without a vault, and Notes offers the setup", async ({
  page,
}) => {
  await page.getByRole("button", { name: "Today" }).click();
  await expect(page.getByText("Nothing planned or due today.")).toBeVisible();
  await page
    .getByRole("navigation", { name: "Repositories and workspaces" })
    .getByRole("button", { name: "Tasks" })
    .click();
  await expect(page.getByText(/No open tasks/)).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Set up your vault" }),
  ).toBeVisible();
});

test("a task created with a date shows in Today and completes with the round check", async ({
  page,
}) => {
  await page.getByRole("button", { name: "Today" }).click();
  await page.getByRole("button", { name: "New Task" }).first().click();
  const dialog = page.getByRole("dialog", { name: "New Task" });
  await dialog.getByLabel("Title").fill("Review the parser");
  const today = await page.evaluate(() => {
    const d = new Date();
    return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
  });
  await dialog.getByLabel("Planned for").fill(today);
  await dialog.getByLabel("Repository").selectOption({ label: "parser" });
  await dialog.getByRole("button", { name: "Create Task" }).click();
  await expect(dialog).toBeHidden();

  const row = page
    .locator("[data-task-id]")
    .filter({ hasText: "Review the parser" });
  await expect(row).toBeVisible();
  await expect(row.getByText("Planned today")).toBeVisible();
  await expect(
    page
      .getByRole("complementary", { name: "Repositories in today's work" })
      .getByText("parser"),
  ).toBeVisible();
  await row.getByRole("button", { name: /Mark .* done/ }).click();
  await expect(page.getByText("Completed today")).toBeVisible();
  const updates = await calls(page, "update_task");
  expect(updates.at(-1)).toMatchObject({
    request: { expected_version: 1, fields: { status: "done" } },
  });
});

test("a new note is edited and autosaves with the version it read", async ({
  page,
}) => {
  await setUpVault(page);
  await page.getByRole("button", { name: "New note", exact: true }).click();
  const editor = page.locator(".note-editor .cm-content");
  await expect(editor).toContainText("Untitled");
  await editor.click();
  await page.keyboard.press("Meta+ArrowDown");
  await page.keyboard.type("First thoughts about the parser.");
  await expect(
    page.getByRole("status").filter({ hasText: /^Saved/ }),
  ).toBeVisible({ timeout: 5000 });
  const saves = await calls(page, "save_note");
  expect(saves.length).toBeGreaterThan(0);
  const last = saves.at(-1) as { request: { text: string } };
  expect(last.request.text).toContain("First thoughts about the parser.");
  expect(last.request.text.startsWith("---\nbrainiac_id: ")).toBe(true);

  // The context panel suggests the repository the note mentions.
  const panel = page.getByRole("complementary", { name: "Note context" });
  await expect(panel.getByText("Mentioned in this note")).toBeVisible();
  await panel.getByRole("button", { name: "Link", exact: true }).click();
  await expect(
    panel.getByRole("button", { name: "Open in Editor" }),
  ).toBeVisible();
});

test("a note changed on disk while edited offers Compare, Reload, and Save as Copy", async ({
  page,
}) => {
  await setUpVault(page);
  await page.getByRole("button", { name: "New note", exact: true }).click();
  const editor = page.locator(".note-editor .cm-content");
  await expect(editor).toContainText("Untitled");
  await page.evaluate(() =>
    window.fake.editOutside("Untitled.md", "# Untitled\n\nChanged elsewhere\n"),
  );
  await editor.click();
  await page.keyboard.press("Meta+ArrowDown");
  await page.keyboard.type("Mine");
  const banner = page.getByRole("alert").filter({ hasText: "changed on disk" });
  await expect(banner).toBeVisible({ timeout: 5000 });
  await expect(banner.getByRole("button", { name: "Compare…" })).toBeVisible();
  await banner.getByRole("button", { name: "Reload from Disk" }).click();
  await expect(editor).toContainText("Changed elsewhere");
  await expect(banner).toBeHidden();
});

test("the palette searches notes and tasks next to repositories", async ({
  page,
}) => {
  await setUpVault(page);
  await page.getByRole("button", { name: "New note", exact: true }).click();
  await expect(page.locator(".note-editor .cm-content")).toContainText(
    "Untitled",
  );
  await page.evaluate(() => window.emitEvent("menu", { id: "palette" }));
  const palette = page.getByRole("dialog", { name: "Search and switch" });
  await palette.getByRole("textbox").fill("untitled");
  await expect(
    palette.locator(".section-label", { hasText: "Notes" }),
  ).toBeVisible();
  await palette.getByRole("textbox").fill("pars");
  await expect(
    palette.locator(".section-label", { hasText: "Repositories" }),
  ).toBeVisible();
  await palette.getByRole("textbox").fill('"unbalanced');
  await expect(palette.getByText("No matches")).toBeVisible();
  await page.keyboard.press("Escape");
});

test("a repository's Notes tab creates a linked note", async ({ page }) => {
  await setUpVault(page);
  await page
    .getByRole("navigation", { name: "Repositories and workspaces" })
    .getByRole("button", { name: /parser/ })
    .first()
    .click();
  await page.getByRole("tab", { name: "Notes" }).click();
  await expect(page.getByText(/No notes are linked to parser/)).toBeVisible();
  await page.getByRole("button", { name: "New Note for parser" }).click();
  await expect(page.locator(".note-editor .cm-content")).toContainText(
    "Untitled",
  );
  const created = await calls(page, "create_note");
  expect(created.at(-1)).toMatchObject({
    request: { repository_id: "repo-1" },
  });
});

test("a note is renamed, pinned, and shown as missing when deleted outside", async ({
  page,
}) => {
  await setUpVault(page);
  await page.getByRole("button", { name: "New note", exact: true }).click();
  await expect(page.locator(".note-editor .cm-content")).toContainText(
    "Untitled",
  );

  await page.getByRole("button", { name: "Note actions" }).click();
  await page.getByRole("menuitem", { name: "Rename or Move…" }).click();
  const rename = page.getByRole("dialog", { name: "Rename or Move Note" });
  await rename.getByRole("textbox").fill("Ideas/Plan.md");
  await rename.getByRole("button", { name: "Rename" }).click();
  await expect(rename).toBeHidden();
  await expect(page.locator("header").getByText("Ideas/Plan.md")).toBeVisible();

  await page.getByRole("button", { name: "Note actions" }).click();
  await page.getByRole("menuitem", { name: "Pin" }).click();
  await expect(
    page.getByRole("navigation", { name: "Notes" }).getByText("Pinned"),
  ).toBeVisible();

  await page.evaluate(() => window.fake.deleteOutside("Ideas/Plan.md"));
  const banner = page.getByRole("alert").filter({ hasText: "file is gone" });
  await expect(banner).toBeVisible();
  await expect(
    banner.getByRole("button", { name: "Restore as New File" }),
  ).toBeVisible();
  await expect(
    banner.getByRole("button", { name: "Relink to a File…" }),
  ).toBeVisible();
});

test("Settings shows the vault and turns note IDs off", async ({ page }) => {
  await setUpVault(page);
  await page.evaluate(() => window.emitEvent("menu", { id: "settings" }));
  const settings = page.getByRole("dialog", { name: "Settings" });
  await expect(settings.getByText("/tmp/Notes")).toBeVisible();
  await settings.getByRole("checkbox").uncheck();
  const updates = await calls(page, "update_settings");
  expect(updates.at(-1)).toMatchObject({ settings: { write_note_ids: false } });
  await settings.getByRole("button", { name: "Rebuild Index" }).click();
  await settings.getByRole("button", { name: "Done" }).click();
  await expect(settings).toBeHidden();
});

test("Settings turns agent access on and shows how to add Brainiac to Claude Code", async ({
  page,
}) => {
  await setUpVault(page);
  await page.evaluate(() => window.emitEvent("menu", { id: "settings" }));
  const settings = page.getByRole("dialog", { name: "Settings" });
  const access = settings.getByRole("group", { name: "Agent access" });
  await expect(access.getByRole("button", { name: "Off" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await expect(settings.getByText("No agents connected")).toBeVisible();
  await expect(
    settings.getByText(
      "claude mcp add --scope user brainiac -- /Applications/Brainiac.app/Contents/MacOS/brainiac mcp",
    ),
  ).toBeVisible();
  await expect(
    settings.getByText("/plugin install brainiac@brainiac"),
  ).toBeVisible();

  await access.getByRole("button", { name: "Read and write" }).click();
  const updates = await calls(page, "update_settings");
  expect(updates.at(-1)).toMatchObject({
    settings: { agent_access: "read_write" },
  });
  await expect(
    access.getByRole("button", { name: "Read and write" }),
  ).toHaveAttribute("aria-pressed", "true");
  await expect(settings.getByText(/never delete anything/)).toBeVisible();
  await expect(settings.getByText("1 agent connected")).toBeVisible();
});

test("Settings adds a GitHub account and saves a Bitbucket token as read-only", async ({
  page,
}) => {
  await setUpVault(page);
  await page.evaluate(() => window.emitEvent("menu", { id: "settings" }));
  const settings = page.getByRole("dialog", { name: "Settings" });

  await settings.getByRole("button", { name: "Add Account…" }).click();
  const token = settings.getByLabel("Fine-grained personal access token");
  await token.fill("github_pat_example");
  await settings.getByRole("button", { name: "Check and Add" }).click();
  await expect(settings.getByText("octo")).toBeVisible();
  await expect(
    settings.getByText(/Fine-grained token · expires/),
  ).toBeVisible();
  expect((await calls(page, "save_forge_account")).at(-1)).toMatchObject({
    request: { kind: "github", token: "github_pat_example", read_only: false },
  });

  // The Bitbucket token is already in the Keychain; only the email is asked.
  await expect(settings.getByText("brainiac/bitbucket")).toBeVisible();
  await settings.getByRole("button", { name: "Use This Token" }).click();
  await settings.getByLabel("Atlassian account email").fill("jo@example.com");
  await settings.getByRole("button", { name: "Check and Add" }).click();
  await expect(
    settings.getByText(/missing write:pullrequest:bitbucket/),
  ).toBeVisible();
  await settings.getByRole("button", { name: "Save as Read-Only" }).click();
  await expect(settings.getByText(/^Read-only:/)).toBeVisible();
  expect((await calls(page, "save_forge_account")).at(-1)).toMatchObject({
    request: {
      kind: "bitbucket_cloud",
      token: null,
      email: "jo@example.com",
      read_only: true,
    },
  });

  await settings.getByRole("button", { name: "Remove" }).first().click();
  await settings.getByRole("button", { name: "Remove Account" }).click();
  await expect(settings.getByText("No account.")).toBeVisible();
});

test("edits typed after a conflict survive leaving the note", async ({
  page,
}) => {
  await setUpVault(page);
  await page.getByRole("button", { name: "New note", exact: true }).click();
  const editor = page.locator(".note-editor .cm-content");
  await expect(editor).toContainText("Untitled");

  await page.evaluate(() =>
    window.fake.editOutside("Untitled.md", "# Untitled\n\nChanged elsewhere\n"),
  );
  await editor.click();
  await page.keyboard.press("Meta+ArrowDown");
  await page.keyboard.type("Mine");
  await expect(
    page.getByRole("alert").filter({ hasText: "changed on disk" }),
  ).toBeVisible({ timeout: 5000 });
  await page.keyboard.type(" and more");
  // Leave the note for another one, then come back.
  await page.getByRole("button", { name: "New note", exact: true }).click();
  await expect(page.locator("header").getByText("Untitled 2.md")).toBeVisible();
  await page.evaluate(() => window.emitEvent("menu", { id: "palette" }));
  await page
    .getByRole("dialog", { name: "Search and switch" })
    .getByRole("textbox")
    .fill("changed elsewhere");
  await page
    .getByRole("dialog", { name: "Search and switch" })
    .getByText("Untitled.md")
    .click();
  await expect(editor).toContainText("Mine and more");
  await expect(
    page.getByRole("alert").filter({ hasText: "changed on disk" }),
  ).toBeVisible();
});

test("a new note's file follows its title once the cursor leaves the heading", async ({
  page,
}) => {
  await setUpVault(page);
  await page.getByRole("button", { name: "New note", exact: true }).click();
  const editor = page.locator(".note-editor .cm-content");
  await expect(editor).toContainText("Untitled");
  const path = page.locator("header .mono");
  await expect(path).toHaveText("Untitled.md");

  // Line 5 is the heading, below the frontmatter.
  await editor.click();
  await page.keyboard.press("Meta+ArrowUp");
  for (let i = 0; i < 4; i++) await page.keyboard.press("ArrowDown");
  await page.keyboard.press("End");
  for (let i = 0; i < "Untitled".length; i++)
    await page.keyboard.press("Backspace");
  await page.keyboard.type("Payment retries");
  await expect(
    page.getByRole("status").filter({ hasText: /^Saved/ }),
  ).toBeVisible({ timeout: 5000 });
  // Not while the title is being typed.
  await expect(path).toHaveText("Untitled.md");
  expect(await calls(page, "follow_note_title")).toEqual([]);

  await page.keyboard.press("ArrowDown");
  await expect(path).toHaveText("Payment retries.md");
  expect(await calls(page, "follow_note_title")).toEqual([
    { noteId: expect.any(String), fromTitle: "Untitled" },
  ]);
  // Listed by its title, with the new path on hover.
  await expect(
    page
      .getByRole("navigation", { name: "Notes" })
      .getByTitle("Payment retries.md")
      .first(),
  ).toHaveText("Payment retries");

  // Named otherwise, the file keeps its name and the header offers a rename.
  await page.getByRole("button", { name: "Note actions" }).click();
  await page.getByRole("menuitem", { name: "Rename or Move…" }).click();
  const rename = page.getByRole("dialog", { name: "Rename or Move Note" });
  await rename.getByRole("textbox").fill("Ideas/Plan.md");
  await rename.getByRole("button", { name: "Rename" }).click();
  await expect(path).toHaveText("Ideas/Plan.md");
  await page
    .getByRole("button", { name: "Rename File to Match Title…" })
    .click();
  await expect(rename.getByRole("textbox")).toHaveValue(
    "Ideas/Payment retries.md",
  );
});

test("a narrow window shows the context panel on request without changing the saved choice", async ({
  page,
}) => {
  await setUpVault(page);
  await page.getByRole("button", { name: "New note", exact: true }).click();
  await expect(page.locator(".note-editor .cm-content")).toContainText(
    "Untitled",
  );
  const panel = page.getByRole("complementary", { name: "Note context" });
  await expect(panel).toBeVisible();
  await page.getByRole("button", { name: "Hide context panel" }).click();
  await expect(panel).toBeHidden();
  // The header counts this note's links instead.
  await expect(page.getByText("0 repositories · 0 tasks")).toBeVisible();

  await page.setViewportSize({ width: 1000, height: 900 });
  await page.getByRole("button", { name: "Show context panel" }).click();
  await expect(panel).toBeVisible();
  await page.setViewportSize({ width: 1440, height: 900 });
  await expect(panel).toBeHidden();
});

test("a dialog opened from a menu takes focus, so Escape closes it", async ({
  page,
}) => {
  await setUpVault(page);
  await page.getByRole("button", { name: "New note", exact: true }).click();
  await expect(page.locator(".note-editor .cm-content")).toContainText(
    "Untitled",
  );
  await page.getByRole("button", { name: "Note actions" }).click();
  await page.getByRole("menuitem", { name: "Version History…" }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
});

test("Pull requests: a workspace turns them on, lists them, and opens one", async ({
  page,
}) => {
  await page
    .getByRole("navigation", { name: "Repositories and workspaces" })
    .getByRole("button", { name: /^Team\b/ })
    .click();
  await page.getByRole("tab", { name: "Pull requests" }).click();
  await page.getByRole("button", { name: "Turn On Pull Requests" }).click();
  expect(
    (await calls(page, "update_workspace_pull_requests")).at(-1),
  ).toMatchObject({ workspaceId: "ws-1", enabled: true });

  const row = page.getByRole("row", { name: "Parse nested lists #12" });
  await expect(row).toBeVisible();
  await expect(row.getByText("Needs your review")).toBeVisible();
  await expect(row.getByRole("img", { name: "Approved" })).toBeVisible();
  await expect(row.getByText("2 of 2 passed")).toBeVisible();
  await expect(row.getByText("+40 −3 in 2 files")).toBeVisible();
  // The fake has no GitHub account yet: the tab says so and offers Settings.
  await expect(page.getByText(/No GitHub account yet/)).toBeVisible();
  await expect(
    page.getByText("GitHub: 12 of 5000 requests used this hour"),
  ).toBeVisible();

  await page.getByRole("button", { name: "Yours" }).click();
  await expect(
    page.getByRole("row", { name: "Draft: faster tokenizer #13" }),
  ).toBeVisible();
  await expect(row).toBeHidden();
  await page.getByRole("button", { name: "All open" }).click();

  await row.click();
  await expect(
    page.getByRole("heading", { name: /Parse nested lists/ }),
  ).toBeVisible();
  // The description and comments are rendered Markdown; a resolved thread
  // is folded, an open one shows its file and line, and a review its verdict.
  await expect(page.locator(".md strong", { hasText: "lists" })).toBeVisible();
  await expect(page.getByText("2 new commits since your review")).toBeVisible();
  const conversation = page.getByRole("region", { name: "Conversation" });
  await expect(conversation.getByText("3 threads")).toBeVisible();
  await expect(conversation.getByText("src/parse.ts:12")).toBeVisible();
  await expect(conversation.getByText("Why not recurse here?")).toBeVisible();
  await expect(conversation.getByText("edited")).toBeVisible();
  await expect(conversation.getByText("approved")).toBeVisible();
  await expect(conversation.getByText("typo")).toBeHidden();
  await conversation.getByText("Resolved", { exact: true }).click();
  await expect(conversation.getByText("typo")).toBeVisible();
  await expect(page.getByText("1 unresolved thread")).toBeVisible();
  await expect(page.getByRole("button", { name: "Merge" })).toBeDisabled();
  // The side panel knows the local checkout.
  await expect(page.getByText("/code/parser")).toBeVisible();

  // Comment on the pull request, reply in a thread, resolve it: each is
  // posted at once and the conversation comes back with it.
  await conversation
    .getByRole("textbox", { name: "Comment on the pull request" })
    .fill("Nice work");
  await conversation.getByRole("button", { name: "Comment" }).click();
  expect((await calls(page, "comment_on_pull_request")).at(-1)).toMatchObject({
    request: { reference: "github.com/team/parser#12", body: "Nice work" },
  });
  await expect(conversation.getByText("Nice work")).toBeVisible();
  await expect(conversation.getByText("4 threads")).toBeVisible();
  const open = conversation.locator(".thread", { hasText: "Why not recurse" });
  await open.getByRole("button", { name: "Reply" }).click();
  await open.getByRole("textbox", { name: "Reply" }).fill("Fair enough");
  await open.getByRole("button", { name: "Reply" }).click();
  expect((await calls(page, "reply_to_thread")).at(-1)).toMatchObject({
    request: { thread_id: "T1", body: "Fair enough" },
  });
  await expect(open.getByText("Fair enough")).toBeVisible();
  await open.getByRole("button", { name: "Resolve" }).click();
  expect((await calls(page, "resolve_thread")).at(-1)).toMatchObject({
    request: { thread_id: "T1", resolved: true },
  });
  await expect(page.getByText("No unresolved threads")).toBeVisible();

  // Files Changed: the first file (not the lock file, which is folded as
  // generated) with the provider's diff, then one from local Git; a click
  // marks a file viewed.
  await page.getByRole("tab", { name: /Files Changed/ }).click();
  await expect(page.getByText("from GitHub", { exact: false })).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Ignore whitespace" }),
  ).toBeDisabled();
  expect((await calls(page, "get_pull_request_diff")).at(-1)).toMatchObject({
    request: { path: "docs/lists.md", old_path: "docs/list.md" },
  });
  await expect(page.getByText("1 generated file")).toBeVisible();
  const docs = page.getByRole("checkbox", { name: "Viewed docs/lists.md" });
  await expect(docs).not.toBeChecked();
  const parse = page.getByRole("checkbox", { name: "Viewed src/parse.ts" });
  await expect(parse).not.toBeChecked();
  await page.getByRole("button", { name: /parse\.ts/ }).click();
  await expect(parse).toBeChecked();
  await expect(
    page.getByText("from local Git", { exact: false }),
  ).toBeVisible();
  await expect(page.getByText("push(parse(item));")).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Ignore whitespace" }),
  ).toBeEnabled();
  // A comment on a line is a draft: written from the line's +, kept on the
  // Mac, counted on Review Changes, and sent by Finish Review.
  await page.getByRole("button", { name: "Comment on line 11" }).click();
  await page
    .getByRole("textbox", { name: "Comment on line 11" })
    .fill("Recursion depth?");
  await page.getByRole("button", { name: "Save Draft" }).click();
  expect((await calls(page, "save_review_draft")).at(-1)).toMatchObject({
    request: {
      reference: "github.com/team/parser#12",
      anchor: {
        path: "src/parse.ts",
        side: "new",
        line: 11,
        start_line: null,
        commit: "a".repeat(40),
      },
      body: "Recursion depth?",
    },
  });
  await expect(page.locator("[data-draft]")).toContainText("Recursion depth?");
  await expect(
    page.getByRole("button", { name: "Review Changes 1" }),
  ).toBeVisible();
  await page.getByRole("tab", { name: "Since your review" }).click();
  expect((await calls(page, "list_pull_request_files")).at(-1)).toMatchObject({
    since: "c".repeat(40),
  });
  await expect(page.getByText("1 file", { exact: true })).toBeVisible();

  // Finish Review: the draft, a summary, and the verdict, for the head on screen.
  await page.getByRole("button", { name: "Review Changes 1" }).click();
  const dialog = page.getByRole("dialog", { name: "Finish Review" });
  await expect(dialog.getByText("1 comment on a line to send")).toBeVisible();
  await expect(dialog.getByText("src/parse.ts:11")).toBeVisible();
  await dialog.getByRole("radio", { name: "Approve" }).check();
  await dialog.getByRole("textbox").fill("Ship it");
  await dialog.getByRole("button", { name: "Approve" }).click();
  expect((await calls(page, "submit_review")).at(-1)).toMatchObject({
    request: {
      reference: "github.com/team/parser#12",
      body: "Ship it",
      verdict: "approve",
      expected_head_sha: "a".repeat(40),
    },
  });
  await expect(dialog).toBeHidden();
  await expect(
    page.getByRole("button", { name: "Review Changes", exact: true }),
  ).toBeVisible();
  await page.getByRole("tab", { name: "Overview" }).click();
  await expect(conversation.getByText("Ship it")).toBeVisible();
  await expect(conversation.getByText("Recursion depth?")).toBeVisible();
  await page.getByRole("tab", { name: "Checks" }).click();
  await expect(page.getByRole("button", { name: "Log" })).toBeVisible();
  await page.getByRole("button", { name: "Back" }).click();
  await expect(
    page.getByRole("tab", { name: "Pull requests" }),
  ).toHaveAttribute("aria-selected", "true");
});

test("a repository's Pull requests tab names its forge and can point it elsewhere", async ({
  page,
}) => {
  await page
    .getByRole("navigation", { name: "Repositories and workspaces" })
    .getByRole("button", { name: "parser" })
    .click();
  await page.getByRole("tab", { name: "Pull requests" }).click();
  // Not tracked yet: the workspace it is in is offered.
  await page.getByRole("button", { name: "Turn On for Team" }).click();
  await expect(
    page.getByRole("row", { name: "Parse nested lists #12" }),
  ).toBeVisible();
  const panel = page.getByRole("complementary", {
    name: "Pull request settings",
  });
  await expect(panel.getByText("github.com/team/parser")).toBeVisible();
  await expect(panel.getByText("Team", { exact: true })).toBeVisible();

  await panel.getByRole("button", { name: "Change…" }).click();
  await panel.getByLabel("Owner (user or organization)").fill("upstream");
  await panel.getByLabel("Repository").fill("parser");
  await panel.getByRole("button", { name: "Use This Repository" }).click();
  expect((await calls(page, "set_repository_forge")).at(-1)).toMatchObject({
    request: {
      repository_id: "repo-1",
      forge: { kind: "github", owner: "upstream", name: "parser" },
    },
  });
  await expect(panel.getByText("github.com/upstream/parser")).toBeVisible();
  await panel.getByRole("button", { name: "Use origin" }).click();
  await expect(panel.getByText("github.com/team/parser")).toBeVisible();

  await page.getByRole("button", { name: "Merged and closed" }).click();
  expect((await calls(page, "list_pull_requests")).at(-1)).toMatchObject({
    request: { repository_id: "repo-1", closed: true },
  });
  await expect(
    page.getByText("No pull requests match this filter."),
  ).toBeVisible();
});
