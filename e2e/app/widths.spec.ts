/**
 * Brainiac in half and three-quarter windows (SPEC.md, Main window): every
 * main view, with the sidebar collapsed to its rail, at 960 and 760 points
 * wide. A view fails when the page scrolls sideways, when two controls in its
 * header overlap, or when its header wraps onto a second row.
 */
import { expect, type Page, test } from "@playwright/test";
import type {} from "./main";

test.use({ baseURL: "http://localhost:1431/" });

let errors: string[] = [];

test.beforeEach(async ({ page }) => {
  errors = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
});

test.afterEach(() => {
  expect(errors).toEqual([]);
});

/** Problems with the view on screen, in words, so a failure says which. */
async function problems(page: Page): Promise<string[]> {
  return page.evaluate(() => {
    const out: string[] = [];
    const doc = document.documentElement;
    if (doc.scrollWidth > window.innerWidth + 1)
      out.push(`page is ${doc.scrollWidth}px wide in ${window.innerWidth}px`);
    const main = document.querySelector("main");
    if (main && main.scrollWidth > main.clientWidth + 1)
      out.push(`main scrolls sideways (${main.scrollWidth}px)`);
    // The traffic lights sit about 18 to 30px down and reach 72px in: a
    // control starting in that band starts clear of them.
    const top = [
      ...(main?.querySelectorAll("button, a, [role=tab], h1, h2") ?? []),
    ]
      .map((el) => ({ el, r: el.getBoundingClientRect() }))
      .filter(({ r }) => r.width > 0 && r.top < 36);
    for (const { el, r } of top)
      if (r.left < 80)
        out.push(
          `"${(el.textContent || el.getAttribute("aria-label") || el.tagName).trim().slice(0, 30)}" at ${Math.round(r.left)},${Math.round(r.top)} is under the traffic lights`,
        );
    const header = main?.querySelector("header");
    if (header) {
      const h = header.getBoundingClientRect();
      if (h.height > 49) out.push(`header wraps (${Math.round(h.height)}px)`);
      if (header.scrollWidth > header.clientWidth + 1)
        out.push("header content is wider than the header");
      const boxes = [...header.children]
        .map((el) => ({
          el,
          r: el.getBoundingClientRect(),
          text: (el.textContent ?? el.getAttribute("aria-label") ?? "").trim(),
        }))
        .filter(({ r }) => r.width > 0 && r.height > 0);
      for (let i = 0; i < boxes.length; i++)
        for (let j = i + 1; j < boxes.length; j++) {
          const a = boxes[i].r;
          const b = boxes[j].r;
          const overlap = Math.min(a.right, b.right) - Math.max(a.left, b.left);
          if (overlap > 1)
            out.push(
              `"${boxes[i].text.slice(0, 30)}" overlaps "${boxes[j].text.slice(0, 30)}"`,
            );
        }
    }
    return out;
  });
}

const rail = (page: Page) => page.getByRole("navigation", { name: "Sections" });
const sidebar = (page: Page) =>
  page.getByRole("navigation", { name: "Repositories and workspaces" });

/** Opens a sidebar item, then collapses the sidebar to its rail again. */
async function viaSidebar(page: Page, name: string | RegExp) {
  await rail(page).getByRole("button", { name: "Expand sidebar" }).click();
  await sidebar(page).getByRole("button", { name }).first().click();
  await page.getByRole("button", { name: "Collapse sidebar" }).click();
  await expect(rail(page)).toBeVisible();
}

for (const width of [960, 760]) {
  test(`every main view fits a ${width}-point window`, async ({ page }) => {
    await page.setViewportSize({ width, height: 900 });
    await page.goto("/");
    await expect(page.getByRole("button", { name: "Today" })).toBeVisible();
    await page.getByRole("button", { name: "Collapse sidebar" }).click();
    await expect(rail(page)).toBeVisible();

    const found: Record<string, string[]> = {};
    const check = async (view: string) => {
      // Let container queries and measured widths settle.
      await page.waitForTimeout(150);
      const p = await problems(page);
      if (p.length) found[view] = p;
    };

    for (const section of ["Today", "Tasks", "Notes", "Databases", "Runs"]) {
      await rail(page).getByRole("button", { name: section }).click();
      await check(section);
    }
    await rail(page).getByRole("button", { name: "All repositories" }).click();
    await check("All repositories");

    await viaSidebar(page, /^Team\b/);
    await check("Workspace");

    await viaSidebar(page, "parser");
    await check("Repository · Changes");
    const files = page.getByRole("region", { name: "Changed files" });
    if (width < 800) {
      // Narrow, the list folds to a strip and slides over the patch.
      await expect(files).toBeHidden();
      await page.getByRole("button", { name: "Files", exact: true }).click();
      await expect(files).toBeVisible();
      await files.press("Escape");
      await expect(files).toBeHidden();
    } else await expect(files).toBeVisible();
    for (const tab of ["Notes", "PRs"]) {
      const tabs = page.getByRole("tab", { name: tab });
      if (await tabs.isVisible()) await tabs.click();
      else {
        // Narrow, the tabs are one menu with their full names.
        await page.getByRole("button", { name: /^Repository view:/ }).click();
        await page
          .getByRole("menuitem", {
            name: tab === "PRs" ? /Pull requests/ : /Notes/,
          })
          .click();
      }
      await check(`Repository · ${tab}`);
    }

    expect(found).toEqual({});
  });
}
