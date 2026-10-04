import { describe, expect, it } from "vitest";
import {
  byFolder,
  cellDisplay,
  cellText,
  completionSchema,
  connectionPlace,
  copyRows,
  defaultMode,
  formatBytes,
  formatDuration,
  lineAndColumn,
  partialCells,
  queryMarkdown,
  selectRows,
  sortRows,
  untitled,
} from "./databases";
import type { DbConnection, ResultColumn, SavedQuery } from "./ipc";

const connection = (over: Partial<DbConnection> = {}): DbConnection => ({
  id: "c1",
  name: "billing",
  kind: "postgres",
  environment: "local",
  access: "read_only",
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
  ...over,
});

const query = (over: Partial<SavedQuery>): SavedQuery => ({
  id: "q",
  name: "Late invoices",
  folder: "",
  description: "",
  connection_id: null,
  sql: "select 1",
  parameters: [],
  version: 1,
  updated_at: "",
  ...over,
});

const columns: ResultColumn[] = [
  { name: "id", type_name: "int4", kind: "number" },
  { name: "note", type_name: "text", kind: "text" },
  { name: "meta", type_name: "jsonb", kind: "json" },
];

describe("databases", () => {
  it("describes where a connection points", () => {
    expect(connectionPlace(connection())).toBe("app@db.example.com/billing");
    expect(connectionPlace(connection({ port: 6543 }))).toBe(
      "app@db.example.com:6543/billing",
    );
    expect(
      connectionPlace(
        connection({ kind: "sqlite", file_path: "/data/shop.db" }),
      ),
    ).toBe("/data/shop.db");
  });

  it("starts tabs read only on production and read-only connections", () => {
    expect(defaultMode(connection())).toBe("read_only");
    expect(defaultMode(connection({ access: "read_write" }))).toBe(
      "auto_commit",
    );
    expect(
      defaultMode(
        connection({ access: "read_write", environment: "production" }),
      ),
    ).toBe("read_only");
    expect(defaultMode(null)).toBe("read_only");
  });

  it("shows NULL apart from the text NULL and labels binary values", () => {
    expect(cellDisplay(null)).toBe("NULL");
    expect(cellText(null)).toBe("");
    expect(cellText("NULL")).toBe("NULL");
    expect(cellDisplay({ kind: "bytes", size: 2048, hex: "de" })).toBe(
      "2.0 KB binary",
    );
    expect(cellText({ kind: "cut", text: "abc", length: 99 })).toBe("abc");
  });

  it("sorts on screen with numbers as numbers and NULLs last", () => {
    const rows = [[10], [null], [9], ["100"]];
    expect(sortRows(rows, 0, "asc").map((r) => r[0])).toEqual([
      9,
      10,
      "100",
      null,
    ]);
    expect(sortRows(rows, 0, "desc").map((r) => r[0])).toEqual([
      "100",
      10,
      9,
      null,
    ]);
  });

  it("copies rows for spreadsheets, notes, programs, and SQL", () => {
    const rows = [
      [1, 'say "hi", ok', '{"a":1}'],
      [2, null, null],
    ];
    expect(copyRows("tsv", columns, rows)).toBe(
      'id\tnote\tmeta\n1\tsay "hi", ok\t{"a":1}\n2\t\t',
    );
    expect(copyRows("csv", columns, rows)).toBe(
      'id,note,meta\n1,"say ""hi"", ok","{""a"":1}"\n2,,',
    );
    expect(JSON.parse(copyRows("json", columns, rows))).toEqual([
      { id: 1, note: 'say "hi", ok', meta: { a: 1 } },
      { id: 2, note: null, meta: null },
    ]);
    expect(copyRows("markdown", columns, [[1, "a|b", null]])).toBe(
      "| id | note | meta |\n| ---: | --- | --- |\n| 1 | a\\|b |  |",
    );
    expect(copyRows("insert", columns, [[1, "it's", null]], "Notes")).toBe(
      `INSERT INTO "Notes" (id, note, meta) VALUES (1, 'it''s', NULL);`,
    );
  });

  it("formats sizes, durations, and positions", () => {
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(1536)).toBe("1.5 KB");
    expect(formatBytes(50 * 1024 * 1024)).toBe("50 MB");
    expect(formatDuration(18)).toBe("18 ms");
    expect(formatDuration(4200)).toBe("4.2 s");
    expect(formatDuration(125_000)).toBe("2 min 5 s");
    expect(lineAndColumn("select\n  nope", 9)).toEqual({ line: 2, column: 3 });
  });

  it("names new tabs and groups saved queries by folder", () => {
    expect(untitled([{ title: "Untitled 1" }, { title: "x" }])).toBe(
      "Untitled 2",
    );
    const groups = byFolder([
      query({ id: "1", name: "b", folder: "Support" }),
      query({ id: "2", name: "a", folder: "" }),
      query({ id: "3", name: "c", folder: "Billing" }),
    ]);
    expect(groups.map((g) => g.folder)).toEqual(["", "Billing", "Support"]);
  });

  it("puts a saved query in a fenced block for a note", () => {
    expect(
      queryMarkdown(
        query({ description: "Weekly.", sql: "select 1;\n" }),
        "billing",
      ),
    ).toBe("**Late invoices** · billing\n\nWeekly.\n\n```sql\nselect 1;\n```");
  });

  it("feeds completion with qualified and default-schema names", () => {
    const schema = {
      connection_id: "c1",
      default_schema: "public",
      read_at: "",
      schemas: [
        {
          name: "public",
          relations: [
            {
              name: "invoices",
              kind: "table" as const,
              estimated_rows: null,
              columns: [
                {
                  name: "id",
                  type_name: "int",
                  nullable: false,
                  default: null,
                  primary_key: true,
                },
              ],
              indexes: [],
              foreign_keys: [],
            },
          ],
        },
      ],
    };
    const invoices = {
      self: { label: "invoices", type: "table" },
      children: [{ label: "id", type: "column", detail: "int" }],
    };
    expect(completionSchema(schema)).toEqual({
      public: { self: { label: "public", type: "schema" }, children: {} },
      "public.invoices": invoices,
      invoices,
    });
    expect(selectRows("billing", "Invoices", false)).toBe(
      'select * from billing."Invoices" limit 100;',
    );
  });
});

describe("partialCells", () => {
  it("counts values held only in part", () => {
    expect(
      partialCells([
        [
          "whole",
          null,
          1,
          { kind: "cut", text: "abc", length: 99_000 },
          { kind: "bytes", size: 2, hex: "abcd" },
        ],
        [
          { kind: "bytes", size: 9_000, hex: "ab".repeat(4096) },
          { kind: "other", type_name: "polygon", size: 40 },
        ],
      ]),
    ).toBe(3);
  });
});
