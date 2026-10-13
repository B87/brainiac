import { describe, expect, it } from "vitest";
import type { RepositorySummary, Workspace, WorkspaceMember } from "./ipc";
import {
  folderName,
  isChoice,
  nextActive,
  quitQuestion,
  repositoryTarget,
  type TerminalTabState,
  tabKey,
  workspaceTarget,
} from "./terminal";

function member(
  name: string,
  status: WorkspaceMember["status"] = "ok",
): WorkspaceMember {
  return {
    origin: "manual",
    display_name: name,
    canonical_path: `/code/${name}`,
    repository_id: name,
    status,
  };
}

function workspace(over: Partial<Workspace>): Workspace {
  return {
    id: "w",
    name: "Acme",
    discovery_mode: "discovered",
    root_repository_id: null,
    discovery_root: "/code/product",
    discovery_path: null,
    members: [],
    activity: {
      watched_branches: ["main"],
      watched_tags: ["v*"],
      auto_fetch: false,
      notify_moves: false,
      morning_digest: false,
      warn_conflicts: true,
    },
    unseen_activity: 0,
    pull_requests: false,
    ...over,
  };
}

function repo(id: string, root: string): RepositorySummary {
  return { id, name: id, canonical_root: root } as RepositorySummary;
}

function tab(key: string): TerminalTabState {
  return {
    key,
    id: null,
    title: key,
    program: null,
    cwd: "",
    ended: null,
    error: null,
  };
}

describe("where a new terminal opens", () => {
  it("opens a repository in its working tree", () => {
    expect(repositoryTarget(repo("widgets", "/code/widgets"))).toEqual({
      cwd: "/code/widgets",
      title: "widgets",
      workspaceId: undefined,
    });
  });

  it("opens a discovered workspace with a root in the root repository", () => {
    const ws = workspace({ root_repository_id: "product" });
    const target = workspaceTarget(
      ws,
      { repositories: [repo("product", "/code/product")] },
      null,
    );
    expect(target).toEqual({
      cwd: "/code/product",
      title: "Acme",
      workspaceId: "w",
    });
  });

  it("opens a discovered workspace without a root in its selected folder", () => {
    const ws = workspace({ discovery_path: "services" });
    expect(workspaceTarget(ws, { repositories: [] }, null)).toMatchObject({
      cwd: "/code/product/services",
    });
  });

  it("asks which member of a manual workspace, then remembers it", () => {
    const ws = workspace({
      discovery_mode: "manual",
      discovery_root: null,
      members: [member("shop"), member("billing"), member("gone", "missing")],
    });
    const asked = workspaceTarget(ws, { repositories: [] }, null);
    expect(isChoice(asked)).toBe(true);
    if (isChoice(asked))
      expect(asked.members.map((m) => m.title)).toEqual(["shop", "billing"]);
    expect(workspaceTarget(ws, { repositories: [] }, "/code/billing")).toEqual({
      title: "billing",
      cwd: "/code/billing",
      workspaceId: "w",
    });
  });

  it("does not ask when a manual workspace has one member", () => {
    const ws = workspace({
      discovery_mode: "manual",
      discovery_root: null,
      members: [member("shop")],
    });
    expect(workspaceTarget(ws, { repositories: [] }, null)).toMatchObject({
      cwd: "/code/shop",
    });
  });

  it("names a folder by its last part", () => {
    expect(folderName("/code/acme/widgets/")).toBe("widgets");
  });
});

describe("keys in a focused tab", () => {
  const key = (k: string, mods: Partial<KeyboardEvent> = {}) => ({
    key: k,
    metaKey: false,
    ctrlKey: false,
    altKey: false,
    shiftKey: false,
    ...mods,
  });

  it("leaves with ⌘Esc on a Mac and Ctrl+Esc on Linux", () => {
    expect(tabKey(key("Escape", { metaKey: true }), true)).toBe("leave");
    expect(tabKey(key("Escape", { ctrlKey: true }), false)).toBe("leave");
    expect(tabKey(key("Escape"), true)).toBe(null);
  });

  it("gives ⌘ keys to Brainiac on a Mac and Ctrl keys to the tab", () => {
    expect(tabKey(key("t", { metaKey: true }), true)).toBe("app");
    expect(tabKey(key("b", { ctrlKey: true }), true)).toBe(null);
    // On Linux, Ctrl+J and Ctrl+T are the shell's.
    expect(tabKey(key("j", { ctrlKey: true }), false)).toBe(null);
    expect(tabKey(key("t", { ctrlKey: true }), false)).toBe(null);
  });
});

describe("the tab list", () => {
  const tabs = [tab("a"), tab("b"), tab("c")];

  it("moves to the next tab when the active one closes", () => {
    expect(nextActive(tabs, "b", "b")).toBe("c");
    expect(nextActive(tabs, "c", "c")).toBe("b");
    expect(nextActive(tabs, "a", "c")).toBe("c");
    expect(nextActive([tab("a")], "a", "a")).toBe(null);
  });

  it("names the busy programs when quitting", () => {
    expect(quitQuestion(["pnpm"])).toContain("running pnpm");
    expect(quitQuestion(["pnpm", "cargo", "pnpm"])).toContain(
      "3 terminal tabs",
    );
  });
});
