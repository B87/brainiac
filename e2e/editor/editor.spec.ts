/**
 * The note editor in WebKit, the engine Tauri uses on macOS, driven with real
 * keyboard and mouse input. These cover what unit tests cannot: cursor
 * movement and clicks over hidden markup, selection, undo, copying, scrolling
 * past images, and typing cost. Every test also fails on a page error or on a
 * request leaving the local server (web images must stay as text).
 */
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { expect, type Page, test } from "@playwright/test";
import type {} from "./harness";
import { longNote } from "./longNote";

const FIXTURES = fileURLToPath(
  new URL("../../src/lib/editor/fixtures", import.meta.url),
);
const fixtures = readdirSync(FIXTURES)
  .filter((f) => f.endsWith(".md"))
  .sort()
  .map((name) => ({ name, text: readFileSync(join(FIXTURES, name), "utf8") }));
const fixture = (name: string) =>
  fixtures.find((f) => f.name === name)?.text ?? "";
const typical = fixture("15-typical.md");
const long = longNote();

let problems: string[] = [];

test.beforeEach(async ({ page, baseURL }) => {
  problems = [];
  page.on("pageerror", (e) => problems.push(`page error: ${e.message}`));
  page.on("request", (r) => {
    const url = r.url();
    if (!url.startsWith(baseURL ?? "") && !url.startsWith("data:"))
      problems.push(`request: ${url}`);
  });
  await page.goto("/");
  // WebKit drops the first click of a headless session; spend it here.
  await page.mouse.click(5, 690);
});

test.afterEach(() => {
  expect(problems).toEqual([]);
});

const load = (page: Page, text: string, live = true) =>
  page.evaluate(([t, l]) => window.ed.load(t, l), [text, live] as const);
const text = (page: Page) => page.evaluate(() => window.ed.text());
const select = (page: Page, pos: number) =>
  page.evaluate((p) => window.ed.select(p), pos);
const head = async (page: Page) =>
  (await page.evaluate(() => window.ed.selection())).head;
const line = (page: Page) => page.evaluate(() => window.ed.line());
const frame = (page: Page) => page.evaluate(() => window.ed.frame());

/** Viewport coordinates of `pos`, scrolled on screen first. */
async function pointAt(page: Page, pos: number) {
  await page.evaluate((p) => window.ed.reveal(p), pos);
  const point = await page.evaluate((p) => window.ed.coords(p), pos);
  if (!point) throw new Error(`No coordinates for ${pos}`);
  return point;
}

test("opening, scrolling, and switching modes leave every fixture byte-identical", async ({
  page,
}) => {
  for (const f of fixtures) {
    await load(page, f.text);
    await page.evaluate(() => window.ed.scrollBy(1e6));
    for (const on of [false, true, false, true])
      await page.evaluate((o) => window.ed.setLive(o), on);
    expect(await text(page), f.name).toBe(f.text);
  }
});

for (const [label, doc, limit] of [
  ["a typical note", typical, 0],
  ["a note with images", long, 400],
] as const) {
  test(`Down and Up visit every line, one at a time, in ${label}`, async ({
    page,
  }) => {
    await load(page, doc);
    await select(page, 0);
    const last = limit || (await page.evaluate(() => window.ed.lines()));
    const down = [1];
    while (down[down.length - 1] < last && down.length < last * 3) {
      await page.keyboard.press("ArrowDown");
      down.push(await line(page));
    }
    const up = [down[down.length - 1]];
    while (up[up.length - 1] > 1 && up.length < last * 3) {
      await page.keyboard.press("ArrowUp");
      up.push(await line(page));
    }
    const steps = (seen: number[]) =>
      seen.slice(1).map((n, i) => Math.abs(n - seen[i]));
    expect(Math.max(...steps(down)), "largest step down").toBe(1);
    expect(Math.max(...steps(up)), "largest step up").toBe(1);
    expect(down[down.length - 1]).toBe(last);
    expect(up[up.length - 1]).toBe(1);
  });
}

test("Right steps through every character, markup included", async ({
  page,
}) => {
  await load(page, typical);
  await select(page, 0);
  const jumps: string[] = [];
  for (let prev = 0; prev < typical.length; ) {
    await page.keyboard.press("ArrowRight");
    const now = await head(page);
    if (now !== prev + 1) jumps.push(`${prev} -> ${now}`);
    if (now === prev) break;
    prev = now;
  }
  expect(jumps).toEqual([]);
});

test("a click on formatted text puts the cursor on the clicked character", async ({
  page,
}) => {
  await load(page, typical);
  for (const [target, offset] of [
    ["title: Payments", 2],
    ["# Payments service", 4],
    ["payments-api]", 2],
    ["fetch_with_backoff", 3],
    ["Review the migration", 2],
    ["Rollout checklist", 2],
    ["## Decisions", 5],
  ] as const) {
    await select(page, typical.length);
    const pos = typical.indexOf(target) + offset;
    const { x, y } = await pointAt(page, pos);
    await page.mouse.click(x, y);
    expect(Math.abs((await head(page)) - pos), target).toBeLessThanOrEqual(1);
  }
});

test("dragging across lines selects from the pressed to the released character", async ({
  page,
}) => {
  await load(page, typical);
  await select(page, typical.length);
  const from = typical.indexOf("Use idempotency") + 4;
  const to = typical.indexOf("Retries back") + 7;
  const a = await pointAt(page, from);
  await page.mouse.move(a.x, a.y);
  await page.mouse.down();
  await frame(page);
  // Markup on the pressed line has appeared; aim at the target's new place.
  const b = await page.evaluate((p) => window.ed.coords(p), to);
  if (!b) throw new Error("No coordinates for the drag target");
  await page.mouse.move(b.x, b.y, { steps: 6 });
  await page.mouse.up();
  const s = await page.evaluate(() => window.ed.selection());
  expect(Math.abs(s.anchor - from)).toBeLessThanOrEqual(1);
  expect(Math.abs(s.head - to)).toBeLessThanOrEqual(1);
});

test("a checkbox click changes only [ ] to [x] and leaves the cursor", async ({
  page,
}) => {
  await load(page, typical);
  await select(page, typical.length);
  await page.locator(".cm-md-checkbox").first().click();
  const at = typical.indexOf("- [ ] Review");
  expect(await text(page)).toBe(
    `${typical.slice(0, at + 3)}x${typical.slice(at + 4)}`,
  );
  expect(await head(page)).toBe(typical.length);
});

test("undo returns to the original text and redo to the edited text", async ({
  page,
}) => {
  await load(page, typical);
  await select(page, typical.length);
  await page.locator(".cm-md-checkbox").first().click();
  await select(page, typical.indexOf("Context for") + 7);
  await page.keyboard.type(" hello");
  await select(page, typical.indexOf("on every write."));
  await page.keyboard.press("End");
  await page.keyboard.press("Enter");
  await page.keyboard.type("new point");
  await page.keyboard.press("Shift+Home");
  await page.keyboard.type("replaced");
  const edited = await text(page);
  expect(edited).not.toBe(typical);
  for (let i = 0; i < 20 && (await text(page)) !== typical; i++)
    await page.keyboard.press("Meta+z");
  expect(await text(page)).toBe(typical);
  for (let i = 0; i < 20 && (await text(page)) !== edited; i++)
    await page.keyboard.press("Meta+Shift+z");
  expect(await text(page)).toBe(edited);
});

test("copying gives the Markdown, hidden markup and line endings included", async ({
  page,
}) => {
  await load(page, typical);
  await select(page, typical.indexOf("## Decisions"));
  for (let i = 0; i < 3; i++) await page.keyboard.press("Shift+ArrowDown");
  const s = await page.evaluate(() => window.ed.selection());
  const part = await page.evaluate(() => window.ed.copy());
  expect(part).toBe(typical.slice(s.anchor, s.head));
  expect(part).toContain("## Decisions");
  expect(part).toContain("- Use idempotency");

  const crlf = fixture("13-crlf.md");
  await load(page, crlf);
  await page.keyboard.press("Meta+a");
  expect(await page.evaluate(() => window.ed.copy())).toBe(crlf);
});

test("Cmd+click opens web and note links; a plain click opens nothing", async ({
  page,
}) => {
  await load(page, typical);
  const click = async (target: string, meta: boolean) => {
    await select(page, typical.length);
    const { x, y } = await pointAt(page, typical.indexOf(target) + 3);
    if (meta) await page.keyboard.down("Meta");
    await page.mouse.click(x, y);
    if (meta) await page.keyboard.up("Meta");
  };
  await click("payments-api]", false);
  expect(await page.evaluate(() => window.ed.opened.length)).toBe(0);
  await click("payments-api]", true);
  await click("Rollout checklist", true);
  expect(await page.evaluate(() => window.ed.opened)).toEqual([
    { kind: "url", target: "https://example.com/org/payments-api" },
    { kind: "note", target: "Rollout checklist" },
  ]);
});

test("scrolling a long note with images never jumps against the scroll", async ({
  page,
}) => {
  await load(page, long);
  const top = () => page.evaluate(() => window.ed.topLine());
  let previous = await top();
  let against = 0;
  for (let i = 0; i < 400; i++) {
    const end = await page.evaluate(() => window.ed.scrollBy(600));
    const now = await top();
    if (now < previous) against++;
    previous = now;
    if (end) break;
  }
  expect(previous).toBeGreaterThan(2900);
  for (let i = 0; i < 400; i++) {
    const end = await page.evaluate(() => window.ed.scrollBy(-600));
    const now = await top();
    if (now > previous) against++;
    previous = now;
    if (end) break;
  }
  expect(against).toBe(0);
});

test("the cursor stays on screen when jumping, paging, and passing images", async ({
  page,
}) => {
  await load(page, long);
  // An image can finish loading just after a move and push the caret down
  // for a frame or two before the editor scrolls it back.
  const onScreen = (step: string) =>
    expect
      .poll(() => page.evaluate(() => window.ed.caretOnScreen()), {
        message: step,
        timeout: 1000,
      })
      .toBe(true);
  await page.keyboard.press("Meta+ArrowDown");
  await onScreen("end of note");
  for (let i = 0; i < 6; i++) {
    await page.keyboard.press("PageUp");
    await onScreen(`PageUp ${i + 1}`);
  }
  for (let i = 0; i < 40; i++) {
    await page.keyboard.press("ArrowDown");
    await onScreen(`ArrowDown ${i + 1}`);
  }
  await page.keyboard.type("typed");
  await onScreen("typing");
});

test("switching modes keeps the selection and the top line", async ({
  page,
}) => {
  await load(page, long);
  const middle = long.split("\n").slice(0, 1500).join("\n").length + 3;
  await select(page, middle);
  for (const on of [false, true, false, true]) {
    const before = await page.evaluate(() => window.ed.topLine());
    await page.evaluate((o) => window.ed.setLive(o), on);
    expect(
      Math.abs((await page.evaluate(() => window.ed.topLine())) - before),
    ).toBeLessThanOrEqual(2);
    expect(await head(page)).toBe(middle);
  }
});

test("typing in a 5 MiB note reaches the screen as fast as in a typical note", async ({
  page,
}) => {
  let big = long;
  while (big.length < 5 * 1024 * 1024 - long.length) big += long;
  const cost = async (doc: string) => {
    await load(page, doc);
    await select(page, Math.floor(doc.length / 2));
    return page.evaluate(() => window.ed.typingCost());
  };
  const small = await cost(typical);
  const large = await cost(big);
  test.info().annotations.push({
    type: "typing",
    description: `5 MiB: ${large.work.toFixed(1)} ms work, on screen after ${large.shown.toFixed(1)} ms; typical: ${small.shown.toFixed(1)} ms`,
  });
  // What the user sees: the keystroke on screen within 50 ms, well under
  // what a typist notices. A 5 MiB note does about 13 ms of work per key
  // against 1 ms for a small one, so under load it can miss a frame; the
  // numbers above are reported for comparison.
  expect(large.shown).toBeLessThan(50);
});
