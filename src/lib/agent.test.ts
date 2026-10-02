import { describe, expect, it } from "vitest";
import { claudeCommand, connectedText, shellQuote } from "./agent";

describe("agent access", () => {
  it("builds the Claude Code command, quoting paths that need it", () => {
    expect(
      claudeCommand("/Applications/Brainiac.app/Contents/MacOS/brainiac"),
    ).toBe(
      "claude mcp add --scope user brainiac -- /Applications/Brainiac.app/Contents/MacOS/brainiac mcp",
    );
    expect(
      shellQuote("/Users/a/My Apps/Brainiac.app/Contents/MacOS/brainiac"),
    ).toBe("'/Users/a/My Apps/Brainiac.app/Contents/MacOS/brainiac'");
    expect(shellQuote("/tmp/it's")).toBe(`'/tmp/it'\\''s'`);
  });

  it("counts connected agents", () => {
    expect(connectedText(0, "read_only")).toBe("No agents connected");
    expect(connectedText(1, "read_write")).toBe("1 agent connected");
    expect(connectedText(2, "off")).toBe("2 agents connected, without access");
  });
});
