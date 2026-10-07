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

test("⌘B hides the sidebar and ⌥⌘B hides the side panel", async ({ page }) => {
  const sidebar = page.getByRole("navigation", {
    name: "Repositories and workspaces",
  });
  await expect(sidebar.getByRole("button", { name: "Today" })).toBeVisible();
  await page.keyboard.press("Meta+b");
  await expect(sidebar).toHaveCount(0);
  // Two presses inside 80 ms count as one (a menu accelerator and the key listener).
  await page.waitForTimeout(100);
  await page.keyboard.press("Meta+b");
  await expect(sidebar.getByRole("button", { name: "Today" })).toBeVisible();

  await page.getByRole("button", { name: "Today" }).click();
  const todayPanel = page.getByRole("complementary", {
    name: "Repositories in today's work",
  });
  await expect(todayPanel).toBeVisible();
  await page.keyboard.press("Alt+Meta+b");
  await expect(todayPanel).toBeHidden();
  await page.getByRole("button", { name: "Show side panel" }).click();
  await expect(todayPanel).toBeVisible();

  await page.getByRole("button", { name: "Hide sidebar" }).click();
  await expect(sidebar).toHaveCount(0);
  await page.keyboard.press("Meta+b");
  await expect(sidebar.getByRole("button", { name: "Notes" })).toBeVisible();
});

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

/** Settings replaces the sidebar and the view; its sidebar lists the sections. */
async function openSettings(page: Page, section: string) {
  await page.evaluate(() => window.emitEvent("menu", { id: "settings" }));
  await page
    .getByRole("navigation", { name: "Settings" })
    .getByRole("button", { name: section })
    .click();
  await expect(page.getByRole("heading", { name: section })).toBeVisible();
}

test("Settings shows the vault and turns note IDs off", async ({ page }) => {
  await openSettings(page, "Notes and Search");
  await page.getByRole("button", { name: "Choose Folder…" }).click();
  await expect(page.getByText("/tmp/Notes")).toBeVisible();
  await page.getByRole("switch").uncheck();
  const updates = await calls(page, "update_settings");
  expect(updates.at(-1)).toMatchObject({ settings: { write_note_ids: false } });
  await page.getByRole("button", { name: "Rebuild Index" }).click();

  const sections = page.getByRole("navigation", { name: "Settings" });
  await sections.getByRole("button", { name: "Backup" }).click();
  await expect(
    sections.getByRole("button", { name: "Backup" }),
  ).toHaveAttribute("aria-current", "true");
  await expect(page.getByRole("heading", { name: "Backup" })).toBeVisible();

  // Back returns to the view Settings was opened from.
  await sections.getByRole("button", { name: "Back", exact: true }).click();
  await expect(
    page.getByRole("navigation", { name: "Repositories and workspaces" }),
  ).toBeVisible();
  await expect(sections).toBeHidden();
});

test("Settings → Agents chooses This Mac's engine on its page and saves a pasted key, never showing it again", async ({
  page,
}) => {
  await openSettings(page, "Agents");
  await expect(page.getByText("Choose where runs execute.")).toBeVisible();
  await page.getByRole("button", { name: /^This Mac,/ }).click();
  await expect(
    page.getByRole("navigation", { name: "Breadcrumb" }),
  ).toContainText("Run hosts › This Mac");
  await page.getByRole("radio", { name: /OrbStack/ }).check();
  expect((await calls(page, "save_agent_settings")).at(-1)).toMatchObject({
    request: { engine_socket: "/Users/someone/.orbstack/run/docker.sock" },
  });
  await page
    .getByRole("navigation", { name: "Breadcrumb" })
    .getByRole("button", { name: "Agents" })
    .click();
  await expect(page.getByText("Choose where runs execute.")).toBeHidden();

  await page.getByRole("button", { name: "Add…" }).click();
  const key = "sk-ant-api03-test-key-for-the-fake-backend-only";
  await page
    .getByLabel("Anthropic API key")
    .fill(`${key.slice(0, 20)}\n${key.slice(20)}`);
  await page.getByRole("button", { name: "Save", exact: true }).click();
  expect((await calls(page, "save_agent_credential")).at(-1)).toMatchObject({
    request: { payment: "api_key", source: { kind: "store" } },
  });
  await expect(page.getByText("API key · Keychain")).toBeVisible();
  await expect(page.getByText(key)).toBeHidden();
});

/** Add host…: the address, the key, then Trust this key and install. */
async function addHost(page: Page) {
  await page.getByRole("button", { name: "Add Host…" }).click();
  const dialog = page.getByRole("dialog", { name: "Add host" });
  await dialog.getByLabel("SSH user").fill("ada");
  await dialog.getByLabel("SSH host").fill("runner.example");
  await dialog.getByRole("button", { name: "Show Host Key" }).click();
  await expect(dialog.getByText(/SHA256:preview/)).toBeVisible();
  await expect(dialog.getByText(/Create the user brainiac/)).toBeVisible();
  return dialog;
}

test("Add host trusts the key and follows its setup as steps, and New run waits for it", async ({
  page,
}) => {
  await openSettings(page, "Agents");
  const dialog = await addHost(page);
  await dialog
    .getByRole("button", { name: "Trust This Key and Install" })
    .click();
  expect((await calls(page, "start_agent_host_job")).at(-1)).toMatchObject({
    id: "host-1",
    kind: "setup",
  });

  // The dialog closes on the host's page, where the job shows its steps.
  await expect(dialog).toBeHidden();
  const job = page.getByRole("region", { name: "Setting up" });
  await expect(
    job.getByText("Build the run controller for linux/amd64"),
  ).toBeVisible();
  await expect(job.getByText("214 crates compiled")).toBeVisible();
  await expect(job.getByLabel("Output, last lines")).toContainText(
    "Compiling bollard",
  );
  await expect(
    page.getByRole("button", { name: /runner\.example setting up/ }),
  ).toBeVisible();

  // New run shows the host with its step, and does not offer it.
  await page.evaluate(() => window.emitEvent("menu", { id: "palette" }));
  await page
    .getByRole("dialog", { name: "Search and switch" })
    .getByRole("button", { name: "New Run…" })
    .click();
  const newRun = page.getByRole("dialog", { name: "New run" });
  await expect(newRun.getByText(/setting up · step 2 of 9/)).toBeVisible();
  await expect(
    newRun.getByRole("button", { name: "runner.example · remote" }),
  ).toBeHidden();
  await newRun.getByRole("button", { name: "Show progress" }).click();
  await expect(job).toBeVisible();

  await page.evaluate(() => window.fake.endJob("host-1", "succeeded"));
  await expect(
    page.getByRole("status").getByText(/runner\.example is ready/),
  ).toBeVisible();
  await expect(page.getByText(/Emergency stop/)).toBeVisible();

  // A key is saved, so a run can start.
  await page.evaluate(() => {
    window.fake.agentSettings.missing = [];
  });
  await page.evaluate(() => window.emitEvent("menu", { id: "palette" }));
  await page
    .getByRole("dialog", { name: "Search and switch" })
    .getByRole("button", { name: "New Run…" })
    .click();
  await newRun.getByRole("button", { name: "runner.example · remote" }).click();
  await expect(
    newRun.getByText(/Anyone who administers it can read them/),
  ).toBeVisible();
  await expect(newRun.getByText(/over SSH/)).toBeVisible();

  // Start run opens the run at once, and its start shows as steps.
  await newRun
    .getByPlaceholder("What should the agent do?")
    .fill("Fix the flaky test");
  await newRun.getByRole("button", { name: "Start run" }).click();
  await expect(newRun).toBeHidden();
  const starting = page.getByRole("region", { name: "Starting the run" });
  await expect(starting.getByText("Starting on runner.example")).toBeVisible();
  await expect(
    starting
      .locator('[aria-current="step"]')
      .getByText(/Copy the start commit/),
  ).toBeVisible();
  await page.evaluate(() => window.fake.advanceStart("run-1", "send"));
  await expect(starting.locator('[aria-current="step"]')).toHaveText(
    "Send it to runner.example",
  );
  await page.evaluate(() => window.fake.advanceStart("run-1", null));
  await expect(starting.locator('[aria-current="step"]')).toHaveText(
    "Start the container and Claude Code",
  );
});

test("An upgrade can be cancelled before it installs, and a failed one says where and offers Try again", async ({
  page,
}) => {
  await openSettings(page, "Agents");
  const dialog = await addHost(page);
  await dialog
    .getByRole("button", { name: "Trust This Key and Install" })
    .click();
  await page.evaluate(() => {
    window.fake.endJob("host-1", "succeeded");
    const host = window.fake.agentSettings.hosts.find((h) => h.id === "host-1");
    if (host) {
      host.controller_build = "a1f3c9e";
      host.upgrade_available = true;
    }
  });
  await page
    .getByRole("navigation", { name: "Breadcrumb" })
    .getByRole("button", { name: "Run hosts" })
    .click();
  await page.getByRole("button", { name: /^runner\.example,/ }).click();

  const offer = page.getByRole("region", {
    name: "An upgrade of the run controller is available",
  });
  await expect(offer).toContainText("build a1f3c9e");
  await expect(offer).toContainText("7c2e51a");
  await offer.getByRole("button", { name: "Upgrade" }).click();
  await expect(
    page.getByRole("region", { name: "Upgrading the run controller" }),
  ).toContainText("Build a1f3c9e → 7c2e51a");
  await page.getByRole("button", { name: "Cancel upgrade" }).click();
  expect(await calls(page, "cancel_agent_host_job")).toHaveLength(1);
  await expect(
    page.getByRole("region", {
      name: "Upgrade of the run controller",
      exact: true,
    }),
  ).toContainText("cancelled");

  await offer.getByRole("button", { name: "Upgrade" }).click();
  await page.evaluate(() => window.fake.endJob("host-1", "failed"));
  await expect(
    page.getByText("Upgrade failed · still on its previous controller").first(),
  ).toBeVisible();
  await expect(page.getByLabel("What happened")).toContainText(
    "did not match the program Brainiac built",
  );
  await page.getByRole("button", { name: "Try Again" }).click();
  expect((await calls(page, "start_agent_host_job")).at(-1)).toMatchObject({
    kind: "upgrade",
  });
});

test("Add host can trust a host key that changed", async ({ page }) => {
  await openSettings(page, "Agents");
  let dialog = await addHost(page);
  await dialog
    .getByRole("button", { name: "Trust This Key and Install" })
    .click();
  await expect(dialog).toBeHidden();

  await page.evaluate(() => {
    window.fake.hostKey = "SHA256:changed";
  });
  await page
    .getByRole("navigation", { name: "Breadcrumb" })
    .getByRole("button", { name: "Run hosts" })
    .click();
  await page.getByRole("button", { name: "Add Host…" }).click();
  dialog = page.getByRole("dialog", { name: "Add host" });
  await dialog.getByLabel("SSH user").fill("ada");
  await dialog.getByLabel("SSH host").fill("runner.example");
  await dialog.getByRole("button", { name: "Show Host Key" }).click();
  await expect(dialog.getByText(/SHA256:changed/)).toBeVisible();
  await dialog
    .getByRole("button", { name: "Trust This Key and Install" })
    .click();
  await expect(dialog.getByRole("alert")).toContainText(
    "Approve the new fingerprint",
  );
  await dialog
    .getByRole("button", { name: "Trust the New Key and Install" })
    .click();
  expect((await calls(page, "approve_agent_host")).at(-1)).toMatchObject({
    request: { fingerprint: "SHA256:changed", accept_changed_key: true },
  });
});

test("Runs puts what needs you first, and a run asks for permission in its conversation", async ({
  page,
}) => {
  await page.evaluate(() => window.fake.useSampleRuns());
  await page.evaluate(() => window.emitEvent("menu", { id: "show_runs" }));
  const needsYou = page.getByRole("region", { name: "Needs you" });
  await expect(
    needsYou.getByText("Fix the flaky invoice rounding test"),
  ).toBeVisible();
  await expect(needsYou.getByText("Ready for your prompt")).toBeVisible();
  await expect(needsYou.getByText("Interrupted")).toBeVisible();
  await expect(
    page
      .getByRole("region", { name: "Ready to review" })
      .getByText("Ready to review · 2 files"),
  ).toBeVisible();
  await page.getByRole("button", { name: "Needs you 3" }).click();
  await expect(page.getByText("Upgrade the date library")).toBeHidden();
  await page.getByRole("button", { name: "All", exact: true }).click();

  await page.getByText("Fix the flaky invoice rounding test").click();
  await expect(
    page.getByRole("heading", { name: "Fix the flaky invoice rounding test" }),
  ).toBeVisible();
  const turn = page.getByRole("region", { name: "Turn 1" });
  await expect(turn.getByText("so half-cent totals round down")).toBeVisible();
  await expect(turn.getByText("**half-cent**")).toHaveCount(0);
  await expect(turn.getByRole("list")).toBeVisible();
  await expect(
    turn.getByText("Keep the signature of roundMoney"),
  ).toBeVisible();
  await expect(page.getByText("Waiting for you · permission")).toBeVisible();
  await expect(
    page.getByRole("button", { name: "2 steps · read 1 file, ran 1 command" }),
  ).toBeVisible();
  await expect(page.getByLabel("Next prompt")).toBeDisabled();
  const request = page
    .getByRole("alert")
    .filter({ hasText: "Claude Code asks to run a command" });
  await expect(
    request.getByText("rm -rf node_modules && pnpm install"),
  ).toBeVisible();
  await request.getByRole("button", { name: "Allow once" }).click();
  expect((await calls(page, "answer_run_permission")).at(-1)).toEqual({
    id: "run-ask",
    permissionId: "perm-1",
    allow: true,
  });

  await page
    .getByRole("navigation", { name: "Breadcrumb" })
    .getByRole("button", { name: "Runs" })
    .click();
  await page.getByText("Migrate config to TOML").click();
  const interrupted = page.getByRole("region", { name: "Interrupted" });
  await expect(
    interrupted.getByText(/OrbStack quit while the agent was working/),
  ).toBeVisible();
  await expect(
    interrupted.getByRole("button", { name: "Collect work" }),
  ).toBeEnabled();

  // A live run: the edit the agent reported, and Changes so far.
  await page
    .getByRole("navigation", { name: "Breadcrumb" })
    .getByRole("button", { name: "Runs" })
    .click();
  await page.getByText("Add pagination to the export endpoint").click();
  const edited = page.getByRole("region", { name: "Turn 1" });
  await edited.getByRole("button", { name: "1 step · edited 1 file" }).click();
  await expect(edited.getByText("limit: 500", { exact: false })).toBeVisible();
  await expect(edited.getByText("Removed:", { exact: false })).toHaveCount(2);
  await page.getByRole("tab", { name: "Changes so far" }).click();
  await expect(
    page.getByRole("list", { name: "Changed files" }).getByText("export.ts"),
  ).toBeVisible();
  await expect(
    page.getByText(/Provisional: the agent keeps working/),
  ).toBeVisible();
  await page.getByRole("button", { name: "Refresh" }).click();
  expect((await calls(page, "refresh_run_preview")).at(-1)).toEqual({
    id: "run-idle",
  });

  await page
    .getByRole("navigation", { name: "Breadcrumb" })
    .getByRole("button", { name: "Runs" })
    .click();
  await page.getByText("Upgrade the date library").click();
  await page.getByRole("tab", { name: /Changes/ }).click();
  await expect(
    page
      .getByRole("list", { name: "Changed files" })
      .getByText("parse.test.ts"),
  ).toBeVisible();
  await expect(page.getByRole("button", { name: "Copy patch" })).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Copy branch command" }),
  ).toBeVisible();
});

test("Settings checks a value before saving it, and saves it when the field is left", async ({
  page,
}) => {
  await openSettings(page, "Repositories");
  const refresh = page.getByLabel("Refresh status every");
  await expect(refresh).toHaveValue("60");
  await refresh.fill("5");
  await refresh.press("Enter");
  await expect(page.getByText("At least 10.")).toBeVisible();
  expect(await calls(page, "update_settings")).toEqual([]);

  await refresh.fill("30");
  await refresh.blur();
  await expect(page.getByText("At least 10.")).toBeHidden();
  expect((await calls(page, "update_settings")).at(-1)).toMatchObject({
    settings: { refresh_interval_seconds: 30 },
  });

  const lines = page.getByLabel("and up to");
  await lines.fill("20,000");
  await lines.press("Enter");
  expect((await calls(page, "update_settings")).at(-1)).toMatchObject({
    settings: { diff_limits: { max_lines: 20000 } },
  });

  await page
    .getByRole("navigation", { name: "Settings" })
    .getByRole("button", { name: "General" })
    .click();
  // Brainiac's default is VS Code; a preset sets all three fields at once.
  const editors = page.getByRole("group", { name: "Editor" });
  await expect(
    editors.getByRole("button", { name: "VS Code" }),
  ).toHaveAttribute("aria-pressed", "true");
  await editors.getByRole("button", { name: "Warp" }).click();
  expect((await calls(page, "update_settings")).at(-1)).toMatchObject({
    settings: {
      editor: {
        executable: "/usr/bin/open",
        repo_args: ["warp://action/new_tab?path={path_url}"],
      },
    },
  });
  await expect(page.getByText(/new tab of Warp/)).toBeVisible();
  await editors.getByRole("button", { name: "Cursor" }).click();
  await editors.getByRole("button", { name: "Custom" }).click();
  await expect(page.getByLabel("Program")).toHaveValue(
    "/Applications/Cursor.app/Contents/Resources/app/bin/cursor",
  );

  const fileArgs = page.getByLabel("Arguments for a file at a line");
  await expect(fileArgs).toHaveValue("-g {path}:{line}");
  await fileArgs.fill("--wait");
  await fileArgs.press("Enter");
  await expect(page.getByText(/Include \{path\}/)).toBeVisible();
  await fileArgs.press("Escape");
  await expect(fileArgs).toHaveValue("-g {path}:{line}");

  // Leaving Settings from the menu, with no blur, still saves what was typed.
  await page
    .getByRole("navigation", { name: "Settings" })
    .getByRole("button", { name: "Repositories" })
    .click();
  await page.getByLabel("Stop a fetch after").fill("120");
  await page.evaluate(() => window.emitEvent("menu", { id: "show_today" }));
  await expect(page.getByRole("navigation", { name: "Settings" })).toBeHidden();
  await expect
    .poll(async () => (await calls(page, "update_settings")).at(-1))
    .toMatchObject({ settings: { fetch_timeout_seconds: 120 } });
});

test("Settings turns agent access on and shows how to add Brainiac to Claude Code", async ({
  page,
}) => {
  await openSettings(page, "Agent Access");
  const access = page.getByRole("group", { name: "Agent access" });
  await expect(access.getByRole("button", { name: "Off" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await expect(page.getByText("No agents connected")).toBeVisible();
  await expect(
    page.getByText(
      "claude mcp add --scope user brainiac -- /Applications/Brainiac.app/Contents/MacOS/brainiac mcp",
    ),
  ).toBeVisible();
  await expect(
    page.getByText("/plugin install brainiac@brainiac"),
  ).toBeVisible();

  await access.getByRole("button", { name: "Read and write" }).click();
  const updates = await calls(page, "update_settings");
  expect(updates.at(-1)).toMatchObject({
    settings: { agent_access: "read_write" },
  });
  await expect(
    access.getByRole("button", { name: "Read and write" }),
  ).toHaveAttribute("aria-pressed", "true");
  await expect(page.getByText(/never delete anything/)).toBeVisible();
  await expect(page.getByText("1 agent connected")).toBeVisible();
});

test("Settings adds a GitHub account and saves a Bitbucket token as read-only", async ({
  page,
}) => {
  await openSettings(page, "Accounts");
  const settings = page.getByRole("main");

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

test("an account's token comes from a command and is tested before it is saved", async ({
  page,
}) => {
  await openSettings(page, "Accounts");
  const settings = page.getByRole("main");
  await settings.getByRole("button", { name: "Add Account…" }).click();
  await settings.getByLabel("Token from").selectOption("command");
  await settings.getByLabel("Program").fill("gh");
  await settings.getByRole("button", { name: "Find…" }).click();
  for (const [i, arg] of [
    "auth",
    "token",
    "--hostname",
    "github.com",
  ].entries()) {
    await settings.getByRole("button", { name: "Add Argument" }).click();
    await settings.getByLabel(`Argument ${i + 1}`, { exact: true }).fill(arg);
  }
  await settings.getByRole("button", { name: "Test" }).click();
  await expect(settings.getByText("Belongs to octo.")).toBeVisible();
  await settings.getByRole("button", { name: "Check and Add" }).click();
  await expect(settings.getByText(/from Command gh auth token/)).toBeVisible();
  expect((await calls(page, "save_forge_account")).at(-1)).toMatchObject({
    request: {
      kind: "github",
      token: null,
      source: {
        kind: "command",
        program: "/opt/homebrew/bin/gh",
        args: ["auth", "token", "--hostname", "github.com"],
      },
    },
  });
});

test("Settings → Secrets reads nothing, allows a restored source, and retries a cleanup", async ({
  page,
}) => {
  await page.evaluate(() => {
    window.fake.dbConnections.push({
      id: "conn-billing",
      name: "billing",
      kind: "postgres",
      environment: "production",
      access: "read_only",
      file_path: null,
      host: "db.example.com",
      port: 5432,
      database: "billing",
      user: "app",
      tls: "verify",
      ca_file: null,
      password: {
        kind: "command",
        program: "/opt/homebrew/bin/op",
        args: ["read", "op://Work/billing/password"],
      },
      password_ready: true,
      credential: { needs_approval: true, pending: null, revision: 3 },
      statement_timeout_seconds: 30,
      file_size: null,
      repository_ids: [],
      runs_on: null,
      version: 1,
    });
    window.fake.accounts = window.fake.accounts.map((s) =>
      s.kind === "github"
        ? {
            ...s,
            account: {
              kind: "github",
              login: "octo",
              user_id: "42",
              display_name: null,
              email: null,
              token_kind: "fine_grained",
              expires_at: null,
              scopes: null,
              read_only: false,
              missing: [],
              checked_at: "2026-10-02T09:00:00.000Z",
              token_source: { kind: "environment", name: "GITHUB_TOKEN" },
              credential: {
                needs_approval: false,
                pending: "cleanup",
                revision: 2,
              },
            },
          }
        : s,
    );
  });
  await openSettings(page, "Secrets");
  const settings = page.getByRole("main");
  await expect(settings.getByText(/macOS login keychain/)).toBeVisible();
  const github = settings.getByRole("region", { name: "GitHub" });
  await expect(
    github.getByText(
      "Environment variable GITHUB_TOKEN → api.github.com as octo",
    ),
  ).toBeVisible();
  const billing = settings.getByRole("region", { name: "billing" });
  await expect(billing.getByText(/not read until you allow it/)).toBeVisible();
  // Opening the page asked for nothing but the list.
  const asked = await page.evaluate(() =>
    window.fake.calls
      .map((c) => c.cmd)
      .filter((c) => /secret|credential|test_/.test(c)),
  );
  expect(asked.length).toBeGreaterThan(0);
  expect(asked.every((c) => c === "list_secrets")).toBe(true);

  await billing.getByRole("button", { name: "Allow…" }).click();
  await expect(
    billing.getByText(
      '["/opt/homebrew/bin/op","read","op://Work/billing/password"]',
    ),
  ).toBeVisible();
  await billing.getByRole("button", { name: "Allow This Source" }).click();
  await expect(billing.getByText("Not tested.")).toBeVisible();
  expect((await calls(page, "approve_secret_source")).at(-1)).toEqual({
    owner: { kind: "db_connection", id: "conn-billing" },
    revision: 3,
  });

  await expect(
    github.getByText(/old Keychain item could not be deleted/),
  ).toBeVisible();
  await github.getByRole("button", { name: "Retry" }).click();
  await expect(github.getByText("Not tested.")).toBeVisible();
  await github.getByRole("button", { name: "Refresh" }).click();
  await expect(github.getByText(/read, or asked for, again/)).toBeVisible();
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
  // The sidebar counts the reviews waiting, and opening the workspace from
  // it lands on this tab while there are some.
  const sidebar = page.getByRole("navigation", {
    name: "Repositories and workspaces",
  });
  await expect(sidebar.getByText("1 to review")).toBeVisible();
  await page.getByRole("tab", { name: "Overview" }).click();
  await sidebar.getByRole("button", { name: /^Team\b/ }).click();
  await expect(
    page.getByRole("tab", { name: "Pull requests", selected: true }),
  ).toBeVisible();

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

  // Merge waits for the checklist: the line comment just sent opened a
  // thread, so it is resolved first. Then the confirmation opens with the
  // repository's methods and names the commit.
  const readiness = page.getByRole("complementary", {
    name: "Merge readiness and reviewers",
  });
  await expect(readiness.getByText("1 unresolved thread")).toBeVisible();
  await expect(readiness.getByRole("button", { name: "Merge" })).toBeDisabled();
  await conversation
    .locator(".thread", { hasText: "Recursion depth?" })
    .getByRole("button", { name: "Resolve" })
    .click();
  await expect(readiness.getByText("No unresolved threads")).toBeVisible();
  await readiness.getByRole("button", { name: "Merge" }).click();
  const merge = page.getByRole("dialog", { name: "Merge Pull Request" });
  await expect(
    merge.getByRole("radio", { name: "Squash and merge" }),
  ).toBeChecked();
  await expect(merge.getByLabel("Commit title")).toHaveValue(
    "Parse nested lists (#12)",
  );
  await merge.getByRole("radio", { name: "Create a merge commit" }).check();
  await expect(merge.getByLabel("Commit title")).toHaveValue(
    "Merge pull request #12 from team/nested-lists",
  );
  await expect(merge.getByRole("checkbox")).toBeChecked();
  await merge.getByRole("button", { name: "Merge aaaaaaaaaa" }).click();
  expect((await calls(page, "merge_pull_request")).at(-1)).toMatchObject({
    request: {
      reference: "github.com/team/parser#12",
      method: "merge_commit",
      commit_title: "Merge pull request #12 from team/nested-lists",
      delete_branch: true,
      expected_head_sha: "a".repeat(40),
    },
  });
  await expect(merge).toBeHidden();
  await expect(page.getByText("Merged", { exact: true }).first()).toBeVisible();
  // Merged: it no longer waits on anyone.
  await expect(sidebar.getByText("1 to review")).toBeHidden();
  await expect(readiness.getByRole("button", { name: "Merge" })).toBeDisabled();
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
