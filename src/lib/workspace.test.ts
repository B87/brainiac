import { describe, expect, it } from "vitest";
import type { Workspace, WorkspaceMember } from "./ipc";
import { discoveryLabel, orderedMembers, workspaceLayout } from "./workspace";

function member(
  name: string,
  origin: WorkspaceMember["origin"],
  repository_id: string | null = name,
): WorkspaceMember {
  return {
    origin,
    display_name: name,
    canonical_path: `/code/${name}`,
    repository_id,
    status: repository_id ? "ok" : "not_git",
  };
}

function workspace(over: Partial<Workspace>): Workspace {
  return {
    id: "w",
    name: "W",
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
    ...over,
  };
}

describe("workspace layout", () => {
  it("puts the root first and groups discovered members", () => {
    const ws = workspace({
      root_repository_id: "product",
      discovery_path: "services",
      members: [
        member("search", "discovered"),
        member("extra", "manual"),
        member("product", "discovered"),
        member("billing", "discovered"),
      ],
    });
    const layout = workspaceLayout(ws);
    expect(layout.root?.display_name).toBe("product");
    expect(layout.groupLabel).toBe("services/");
    expect(layout.grouped.map((m) => m.display_name)).toEqual([
      "billing",
      "search",
    ]);
    expect(orderedMembers(ws).map((m) => m.display_name)).toEqual([
      "product",
      "billing",
      "search",
      "extra",
    ]);
  });

  it("labels a plain discovery folder by its name", () => {
    expect(discoveryLabel(workspace({}))).toBe("product/");
  });

  it("keeps manual workspaces flat", () => {
    const ws = workspace({
      discovery_mode: "manual",
      discovery_root: null,
      members: [member("b", "manual"), member("a", "manual")],
    });
    const layout = workspaceLayout(ws);
    expect(layout.groupLabel).toBeNull();
    expect(layout.others.map((m) => m.display_name)).toEqual(["a", "b"]);
  });
});
