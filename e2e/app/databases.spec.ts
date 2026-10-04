/**
 * Databases (SPEC.md, section 11) clicked through in WebKit over the fake
 * backend: connections, the query view, results, saved queries with
 * parameters, transactions, and Health. Every test fails on a page error.
 * With SCREENSHOTS set to a folder, each test leaves a picture of the view.
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

async function shot(page: Page, name: string) {
  const folder = process.env.SCREENSHOTS;
  if (folder) await page.screenshot({ path: `${folder}/${name}.png` });
}

const calls = (page: Page, cmd: string) =>
  page.evaluate(
    (c) => window.fake.calls.filter((x) => x.cmd === c).map((x) => x.args),
    cmd,
  );

async function openDatabases(page: Page) {
  await page
    .getByRole("navigation", { name: "Repositories and workspaces" })
    .getByRole("button", { name: "Databases" })
    .click();
}

/** A connection added straight to the fake backend, before Databases opens. */
async function seed(
  page: Page,
  options: { environment?: string; access?: string; queries?: boolean } = {},
) {
  await page.evaluate((o) => {
    window.fake.dbConnections.push({
      id: "conn-billing",
      name: "billing",
      kind: "postgres",
      environment: (o.environment ?? "local") as "local",
      access: (o.access ?? "read_only") as "read_only",
      file_path: null,
      host: "db.example.com",
      port: 5432,
      database: "billing",
      user: "app",
      tls: "verify",
      ca_file: null,
      password: "keychain",
      password_ready: true,
      statement_timeout_seconds: 30,
      file_size: null,
      repository_ids: [],
      runs_on: null,
      version: 1,
    });
    if (o.queries)
      window.fake.savedQueries.push({
        id: "q-late",
        name: "Late invoices",
        folder: "Billing",
        description: "Unpaid for more than :days days",
        connection_id: "conn-billing",
        sql: "select * from invoices where due_on < now() - :days * interval '1 day';",
        parameters: [],
        version: 1,
        updated_at: "2026-10-02T09:00:00.000Z",
      });
  }, options);
}

const editor = (page: Page) => page.locator(".cm-content").last();

test("a connection is added and a query shows its rows", async ({ page }) => {
  await openDatabases(page);
  await expect(page.getByText("No connections yet")).toBeVisible();
  await page.getByRole("button", { name: "New Connection…" }).first().click();
  const dialog = page.getByRole("dialog", { name: "New Connection" });
  await dialog
    .getByLabel("Paste a URL", { exact: true })
    .fill("postgres://app:s3cret@db.example.com:5433/billing");
  await expect(dialog.getByLabel("Host", { exact: true })).toHaveValue(
    "db.example.com",
  );
  await expect(dialog.getByLabel("Port", { exact: true })).toHaveValue("5433");
  await expect(dialog.getByLabel("Name", { exact: true })).toHaveValue(
    "billing",
  );
  await expect(dialog.getByLabel("Paste a URL", { exact: true })).toHaveValue(
    "",
  );
  await dialog.getByRole("button", { name: "Test Connection" }).click();
  await expect(dialog.getByText("Connected: PostgreSQL 16.4.")).toBeVisible();
  await dialog.getByRole("button", { name: "Add Connection" }).click();
  const saved = await calls(page, "save_db_connection");
  expect(saved[0]).toMatchObject({
    request: { password: "s3cret", password_storage: "keychain", port: 5433 },
  });

  await page.getByRole("button", { name: "New Query" }).click();
  await expect(page.getByRole("tab", { name: /Untitled 1/ })).toBeVisible();
  await editor(page).click();
  await page.keyboard.type("select * from invoices;");
  await page.keyboard.press("Meta+Enter");
  const grid = page.getByRole("grid", { name: "Result" });
  await expect(grid.getByText("a@example.com")).toBeVisible();
  await expect(page.getByText(/3 rows · 18 ms/)).toBeVisible();
  // NULL is drawn apart from the text "NULL".
  await expect(grid.locator("td.null")).toHaveCount(1);
  await expect(
    grid.locator("td:not(.null)", { hasText: /^NULL$/ }),
  ).toHaveCount(1);
  // The inspector pretty-prints JSON.
  await grid.getByText('{"tier": "gold"}').click();
  await expect(
    page.getByRole("complementary", { name: "Inspector" }),
  ).toContainText('"tier": "gold"');
  await expect(
    page.getByRole("complementary", { name: /Schema, saved queries/ }),
  ).toContainText("invoices");
  await shot(page, "query-rows");

  // An error is shown with its position, which is underlined in the editor.
  await editor(page).click();
  await page.keyboard.press("Meta+a");
  await page.keyboard.type("select 1;\nselect * from nowhere;");
  await page.keyboard.press("Meta+Enter");
  await expect(page.getByRole("alert")).toContainText(
    'relation "nowhere" does not exist',
  );
  await expect(page.getByRole("alert")).toContainText(
    "42P01 · line 2, column 15",
  );
  await expect(page.locator(".cm-sql-error")).toHaveText("nowhere");
  await shot(page, "query-error");
});

test("a large result is windowed and says more rows exist", async ({
  page,
}) => {
  await seed(page);
  await openDatabases(page);
  await page.getByRole("button", { name: "New Query" }).click();
  await editor(page).click();
  await page.keyboard.type("select generate_series(1, 5000)");
  await page.keyboard.press("Meta+Enter");
  await expect(
    page.getByText("first 1,000 rows · more available"),
  ).toBeVisible();
  const rendered = await page
    .getByRole("grid", { name: "Result" })
    .locator("tbody tr")
    .count();
  expect(rendered).toBeLessThan(200);
  await page.getByRole("button", { name: "Fetch All" }).click();
  await expect(page.getByText(/5,000 rows/)).toBeVisible();
  expect((await calls(page, "run_statement")).at(-1)).toMatchObject({
    request: { fetch_all: true },
  });
});

test("a saved query runs from Home and asks for its parameters", async ({
  page,
}) => {
  await seed(page, { queries: true });
  await openDatabases(page);
  await expect(page.getByText("Late invoices")).toBeVisible();
  await shot(page, "home");
  await page.getByRole("button", { name: "Run", exact: true }).click();
  const form = page.getByRole("form", { name: "Parameters" });
  await expect(form).toBeVisible();
  await form.locator('input[data-param="days"]').fill("30");
  await form.getByRole("button", { name: "Run" }).click();
  await expect(page.getByText("overdue 30 days")).toBeVisible();
  const remembered = await calls(page, "remember_query_parameters");
  expect(remembered.at(-1)).toMatchObject({
    id: "q-late",
    values: [{ name: "days", value: "30" }],
  });
  await shot(page, "saved-query-parameters");
});

test("a query is saved with ⌘S and found in ⌘K", async ({ page }) => {
  await seed(page);
  await openDatabases(page);
  await page.getByRole("button", { name: "New Query" }).click();
  await editor(page).click();
  await page.keyboard.type("select count(*) from invoices;");
  await page.evaluate(() => window.emitEvent("menu", { id: "save" }));
  const dialog = page.getByRole("dialog", { name: "Save Query As" });
  await dialog.getByLabel("Name", { exact: true }).fill("Invoice count");
  await dialog.getByLabel("Folder", { exact: true }).fill("Billing");
  await dialog.getByRole("button", { name: "Save", exact: true }).click();
  await expect(page.getByRole("tab", { name: /Invoice count/ })).toBeVisible();

  await page.evaluate(() => window.emitEvent("menu", { id: "palette" }));
  await page.keyboard.type("invoice count");
  await expect(page.getByText("Run Saved Query · billing")).toBeVisible();
});

test("writes on production ask first and stay in a manual transaction", async ({
  page,
}) => {
  await seed(page, { environment: "production", access: "read_write" });
  await openDatabases(page);
  await page.getByRole("button", { name: "New Query" }).click();
  await editor(page).click();
  await page.keyboard.type("update invoices set paid_at = now()");
  await page.keyboard.press("Meta+Enter");
  await expect(page.getByRole("alert")).toContainText(
    "This connection is read only",
  );

  await page.getByRole("button", { name: "Manual" }).click();
  await editor(page).click();
  await page.keyboard.press("Meta+Enter");
  await expect(page.getByRole("status")).toContainText("Transaction open");
  await expect(page.getByRole("status")).toContainText("1 statement");
  await expect(page.locator('.env-strip[data-writable="true"]')).toBeVisible();
  await shot(page, "manual-transaction");
  await page.getByRole("button", { name: "Commit", exact: true }).click();
  await expect(page.getByText("Transaction open")).toHaveCount(0);
  expect(await calls(page, "end_transaction")).toEqual([
    expect.objectContaining({ commit: true }),
  ]);
  // The mode was asked about once, for turning writes on in this tab.
  expect(
    (await calls(page, "plugin:dialog|message")).map(
      (a) => (a as { title: string }).title,
    ),
  ).toEqual(["Turn On Writes"]);
});

test("health shows connections, sessions, and who waits on whom", async ({
  page,
}) => {
  await seed(page, { access: "read_write" });
  await openDatabases(page);
  await page.getByRole("button", { name: "Health" }).click();
  await expect(
    page.getByRole("heading", { name: "billing health" }),
  ).toBeVisible();
  await expect(page.getByText("of 100")).toBeVisible();
  await expect(page.getByText("billing-worker").first()).toBeVisible();
  await expect(page.getByText("Waiting on locks")).toBeVisible();
  await expect(page.getByText("Where does this server run?")).toBeVisible();
  await shot(page, "health");
  await page.getByRole("button", { name: "Cancel Query" }).first().click();
  await expect
    .poll(async () => (await calls(page, "signal_db_backend"))[0])
    .toMatchObject({ pid: 4242, terminate: false });
});

test("completion names the schema's tables and columns", async ({ page }) => {
  await page.emulateMedia({ colorScheme: "dark" });
  await seed(page);
  await openDatabases(page);
  await page.getByRole("button", { name: "New Query" }).click();
  await expect(
    page.getByRole("complementary", { name: /Schema, saved queries/ }),
  ).toContainText("invoices");
  await editor(page).click();
  await page.keyboard.type("select * from invo");
  const list = page.locator(".cm-tooltip-autocomplete");
  const option = list.getByRole("option", { name: /invoices/ }).first();
  await expect(option).toContainText("table");
  // The list wears the app's colors, not CodeMirror's light default.
  const background = await list.evaluate(
    (e) => getComputedStyle(e).backgroundColor,
  );
  expect(background).not.toBe("rgb(245, 245, 245)");
  await shot(page, "completion");
});
