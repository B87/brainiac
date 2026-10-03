import { describe, expect, it } from "vitest";
import { accessSummary, tokenSummary } from "./accounts";
import type { ForgeAccount } from "./ipc";

const account: ForgeAccount = {
  kind: "bitbucket_cloud",
  login: "jo",
  user_id: "{0a1b}",
  display_name: null,
  email: "jo@example.com",
  token_kind: "api_token",
  expires_at: null,
  scopes: [],
  read_only: false,
  missing: [],
  checked_at: "2026-10-03T09:00:00.000Z",
};

describe("accounts", () => {
  it("says when a token expires or expired", () => {
    const now = Date.parse("2026-10-03T09:00:00Z");
    expect(tokenSummary(account, now)).toBe("API token");
    const later = { ...account, token_kind: "fine_grained" as const };
    expect(
      tokenSummary({ ...later, expires_at: "2027-10-02T22:00:00Z" }, now),
    ).toMatch(/^Fine-grained token · expires /);
    expect(
      tokenSummary({ ...later, expires_at: "2026-01-02T22:00:00Z" }, now),
    ).toMatch(/^Fine-grained token · expired /);
  });

  it("names what a read-only token is missing", () => {
    expect(accessSummary(account)).toMatch(/^Reads, reviews/);
    expect(
      accessSummary({
        ...account,
        read_only: true,
        missing: ["write:pullrequest:bitbucket"],
      }),
    ).toContain("add write:pullrequest:bitbucket");
  });
});
