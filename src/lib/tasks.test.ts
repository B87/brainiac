import { describe, expect, it } from "vitest";
import type { Task } from "./ipc";
import { dateNotes, fieldsOf, formatDay, localDate, toggled } from "./tasks";

const task = (over: Partial<Task> = {}): Task => ({
  id: "t1",
  title: "Ship",
  description: "",
  status: "todo",
  to_sort: false,
  planned_date: null,
  due_date: null,
  note: null,
  repository_id: null,
  created_at: "2026-10-01T10:00:00.000Z",
  updated_at: "2026-10-01T10:00:00.000Z",
  completed_at: null,
  version: 3,
  ...over,
});

describe("task dates", () => {
  it("uses the local calendar date", () => {
    expect(localDate(new Date(2026, 9, 2, 23, 59))).toBe("2026-10-02");
    expect(localDate(new Date(2026, 9, 3, 0, 0))).toBe("2026-10-03");
  });

  it("formats dates without shifting them", () => {
    expect(formatDay("2026-10-03", "2026-10-02")).toBe("Sat 3 Oct");
    expect(formatDay("2027-01-04", "2026-10-02")).toBe("Mon 4 Jan 2027");
  });

  it("says overdue in words", () => {
    const notes = dateNotes(task({ due_date: "2026-10-01" }), "2026-10-02");
    expect(notes).toEqual([{ text: "Overdue · due Thu 1 Oct", overdue: true }]);
    expect(
      dateNotes(
        task({ due_date: "2026-10-01", status: "done" }),
        "2026-10-02",
      )[0].overdue,
    ).toBe(false);
    expect(
      dateNotes(task({ due_date: "2026-10-02" }), "2026-10-02")[0].text,
    ).toBe("Due today");
    expect(
      dateNotes(task({ planned_date: "2026-10-02" }), "2026-10-02")[0].text,
    ).toBe("Planned today");
  });

  it("sends whole fields back, keeping a sorted task sorted", () => {
    expect(fieldsOf(task()).sorted).toBe(true);
    expect(fieldsOf(task({ to_sort: true })).sorted).toBe(false);
    expect(toggled(task()).status).toBe("done");
    expect(toggled(task({ status: "done" })).status).toBe("todo");
  });
});
