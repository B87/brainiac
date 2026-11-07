import { expect, type Page, test } from "@playwright/test";
import type {} from "./main";

test.use({
  baseURL: "http://localhost:1431/",
  viewport: { width: 960, height: 900 },
});

/** Check exposed hit targets, not just the presence of a hidden drag attribute. */
async function checkTopDragArea(page: Page, screen: string) {
  await expect
    .poll(
      () =>
        page.getByRole("main").evaluate((main) => {
          const top = main.getBoundingClientRect().top;
          return [
            ...main.querySelectorAll<HTMLElement>("[data-tauri-drag-region]"),
          ].some((element) => {
            const rect = element.getBoundingClientRect();
            if (
              rect.width < 12 ||
              rect.height < 4 ||
              rect.top >= top + 48 ||
              rect.bottom <= top
            )
              return false;
            for (const x of [
              rect.left + 3,
              rect.left + rect.width / 2,
              rect.right - 3,
            ]) {
              for (const y of [
                Math.max(top + 2, rect.top + 2),
                Math.min(top + 46, rect.bottom - 2),
              ]) {
                if (y < rect.top || y >= rect.bottom) continue;
                const target = document.elementFromPoint(x, y);
                const drag = target?.getAttribute("data-tauri-drag-region");
                if (drag !== null && drag !== undefined && drag !== "false")
                  return true;
              }
            }
            return false;
          });
        }),
      {
        message: `${screen} must expose a draggable target in its top 48 pixels`,
      },
    )
    .toBe(true);
}

for (const collapsed of [false, true]) {
  test(`main screens expose window dragging with the sidebar ${collapsed ? "collapsed" : "expanded"}`, async ({
    page,
  }) => {
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    await page.goto("/");
    await expect(
      page.getByRole("button", { name: "Today", exact: true }),
    ).toBeVisible();
    if (collapsed)
      await page.getByRole("button", { name: "Collapse sidebar" }).click();
    const nav = () =>
      page.getByRole("navigation", {
        name: collapsed ? "Sections" : "Repositories and workspaces",
      });
    for (const screen of [
      "Today",
      "Tasks",
      "Notes",
      "Databases",
      "Runs",
      "Routines",
      "Terminal",
      "All repositories",
    ]) {
      await nav()
        .getByRole("button", { name: new RegExp(`^${screen}\\b`) })
        .first()
        .click();
      await checkTopDragArea(page, screen);
    }

    await page.evaluate(() => window.fake.useSampleRuns());
    await nav()
      .getByRole("button", { name: /^Runs\b/ })
      .click();
    await page
      .getByText("Fix the flaky invoice rounding test", { exact: true })
      .click();
    await expect(
      page.getByRole("heading", {
        name: "Fix the flaky invoice rounding test",
      }),
    ).toBeVisible();
    await checkTopDragArea(page, "Run detail");

    await page.evaluate(() => {
      window.fake.readyThisMac();
      window.fake.readyProfile("claude-code", ["local"]);
    });
    await nav()
      .getByRole("button", { name: /^Routines\b/ })
      .click();
    await page.getByRole("button", { name: "New Routine…" }).first().click();
    await expect(
      page.getByRole("heading", { name: "New routine" }),
    ).toBeVisible();
    await checkTopDragArea(page, "New routine");
    const form = page.getByRole("form", { name: "New routine" });
    await form.getByLabel("Name", { exact: true }).fill("Example routine");
    await form.getByLabel("Prompt").fill("Check the supplied information.");
    await form.getByRole("button", { name: "Create Routine" }).click();
    await expect(
      page.getByRole("heading", { name: "Example routine" }),
    ).toBeVisible();
    await checkTopDragArea(page, "Routine detail");
    await page.getByRole("tab", { name: "Definition", exact: true }).click();
    await page
      .getByRole("heading", { name: "Example routine" })
      .evaluate((heading) => {
        const scroller = heading.closest("header")?.parentElement;
        if (scroller) scroller.scrollTop = scroller.scrollHeight;
      });
    await checkTopDragArea(page, "Scrolled routine detail");

    // The rail has no individual repository rows; expand it for these views.
    if (collapsed)
      await nav().getByRole("button", { name: "Expand sidebar" }).click();
    const sidebar = page.getByRole("navigation", {
      name: "Repositories and workspaces",
    });
    await sidebar.getByRole("button", { name: /^parser\b/ }).click();
    await checkTopDragArea(page, "Repository");
    await sidebar.getByRole("button", { name: /^Team\b/ }).click();
    await checkTopDragArea(page, "Workspace");
    await page.getByRole("tab", { name: "PRs", exact: true }).click();
    const turnOn = page.getByRole("button", { name: "Turn On Pull Requests" });
    if (await turnOn.isVisible()) await turnOn.click();
    await page.getByRole("row", { name: "Parse nested lists #12" }).click();
    await expect(
      page.getByRole("heading", { name: /Parse nested lists/ }),
    ).toBeVisible();
    await checkTopDragArea(page, "Pull request");
    await page.evaluate(() => window.emitEvent("menu", { id: "settings" }));
    await expect(
      page.getByRole("navigation", { name: "Settings", exact: true }),
    ).toBeVisible();
    await checkTopDragArea(page, "Settings");
    expect(errors).toEqual([]);
  });
}
