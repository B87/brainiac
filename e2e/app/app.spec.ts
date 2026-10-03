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
