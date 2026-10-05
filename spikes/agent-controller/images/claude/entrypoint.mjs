// Read one length-prefixed credential frame, then run the ACP adapter.
// The frame is "BRB1" + 4-byte big-endian length + JSON. Only
// CLAUDE_CODE_OAUTH_TOKEN is accepted. The read is exact, so later ACP
// bytes stay in the kernel pipe for the child. Node strings cannot be
// wiped; the frame buffer is zeroed and the value is not written to a file.

import { spawn } from "node:child_process";
import fs from "node:fs";

const ADAPTER =
  "/opt/claude/node_modules/@agentclientprotocol/claude-agent-acp/dist/index.js";
const MAX_FRAME = 8192;

function readExact(fd, length) {
  const buf = Buffer.alloc(length);
  let offset = 0;
  while (offset < length) {
    const got = fs.readSync(fd, buf, offset, length - offset, null);
    if (got === 0) {
      throw new Error("stdin closed during bootstrap");
    }
    offset += got;
  }
  return buf;
}

function fail(reason) {
  fs.writeSync(1, `bootstrap:rejected ${reason}\n`);
  process.exit(1);
}

let token;
try {
  const magic = readExact(0, 4);
  if (!magic.equals(Buffer.from("BRB1"))) {
    fail("magic");
  }
  const lenBuf = readExact(0, 4);
  const length = lenBuf.readUInt32BE(0);
  lenBuf.fill(0);
  if (length < 2 || length > MAX_FRAME) {
    fail("length");
  }
  const payload = readExact(0, length);
  let parsed;
  try {
    parsed = JSON.parse(payload.toString("utf8"));
  } finally {
    payload.fill(0);
  }
  const keys = parsed && typeof parsed === "object" ? Object.keys(parsed) : [];
  if (keys.length !== 1 || keys[0] !== "CLAUDE_CODE_OAUTH_TOKEN") {
    fail("keys");
  }
  token = parsed.CLAUDE_CODE_OAUTH_TOKEN;
  parsed.CLAUDE_CODE_OAUTH_TOKEN = "";
  if (typeof token !== "string" || token.length < 16 || /[\s\u0000]/.test(token)) {
    fail("token");
  }
} catch {
  fail("frame");
}

// Claude Code ignores CLAUDE_CODE_OAUTH_TOKEN on a fresh home until this
// non-secret flag exists. It is not a login file and does not hold the token.
const home = process.env.HOME || "/home/node";
fs.mkdirSync(home, { recursive: true });
fs.writeFileSync(`${home}/.claude.json`, '{"hasCompletedOnboarding":true}\n', {
  mode: 0o600,
});
fs.writeSync(1, "bootstrap:onboarding-flag\n");
fs.writeSync(1, "bootstrap:ready\n");

const env = { ...process.env, CLAUDE_CODE_OAUTH_TOKEN: token };
delete env.ANTHROPIC_API_KEY;
delete env.ANTHROPIC_AUTH_TOKEN;
delete env.CLAUDE_CODE_API_KEY;
delete env.CLAUDE_AGENT_LOGS;
token = "";

const child = spawn(process.execPath, [ADAPTER], {
  env,
  stdio: "inherit",
});
for (const signal of ["SIGTERM", "SIGINT"]) {
  process.on(signal, () => {
    if (!child.killed) {
      child.kill(signal);
    }
  });
}
child.on("exit", (code) => {
  process.exit(code ?? 1);
});
child.on("error", () => {
  process.exit(1);
});
