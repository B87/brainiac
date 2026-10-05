// The run's entrypoint (docs/architecture.md, Agent runs — v0.5,
// Credentials). It reads one credential frame from stdin, clones the run's
// start into the workspace, then starts the Claude ACP adapter with that one
// value in its environment.
//
// The frame is "BRB1", a 4-byte big-endian length, then JSON with exactly
// one key: CLAUDE_CODE_OAUTH_TOKEN (a Claude plan token) or
// ANTHROPIC_API_KEY. The read is exact, so the ACP bytes after it stay in
// the pipe for the adapter. Node strings cannot be wiped; the frame buffer
// is zeroed and the value is never written to a file.

import { spawn, spawnSync } from "node:child_process";
import fs from "node:fs";

const ADAPTER =
  "/opt/claude/node_modules/@agentclientprotocol/claude-agent-acp/dist/index.js";
const MAX_FRAME = 8192;
// The run's start, copied in by the run controller before this container
// started: one commit and its history, advertising refs/heads/start.
const INPUT = "/opt/brainiac/input/input.bundle";
const WORKSPACE = "/workspace";
const KEYS = ["CLAUDE_CODE_OAUTH_TOKEN", "ANTHROPIC_API_KEY"];
// Every variable that could carry another credential to Claude Code.
const CLEARED = [
  ...KEYS,
  "ANTHROPIC_AUTH_TOKEN",
  "CLAUDE_CODE_API_KEY",
  "CLAUDE_AGENT_LOGS",
];

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

// The reason is a fixed word: never part of the frame.
function fail(reason) {
  fs.writeSync(1, `bootstrap:rejected ${reason}\n`);
  process.exit(1);
}

let key;
let value;
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
  if (keys.length !== 1 || !KEYS.includes(keys[0])) {
    fail("keys");
  }
  key = keys[0];
  value = parsed[key];
  parsed[key] = "";
  if (typeof value !== "string" || value.length < 16 || /[\s\u0000]/.test(value)) {
    fail("value");
  }
} catch {
  fail("frame");
}

// The workspace volume is new and empty (a fresh ext4 filesystem has only
// lost+found); the run's start is fetched into it, with no remote left
// behind. Git's output goes to stderr, which is drained and never stored:
// stdout carries the protocol. The credential is not in Git's environment.
// A run without its start is refused rather than given an empty workspace.
if (!fs.existsSync(INPUT)) {
  fail("clone");
}
if (fs.readdirSync(WORKSPACE).some((name) => name !== "lost+found")) {
  fail("workspace");
}
const git = (args) =>
  spawnSync("git", ["-C", WORKSPACE, ...args], {
    stdio: ["ignore", 2, "inherit"],
  }).status === 0;
// Git refuses to fetch into the branch HEAD names, even an unborn one, so
// the repository starts on a throwaway branch that the checkout leaves.
if (
  !git(["init", "--quiet", "--initial-branch=brainiac-setup"]) ||
  !git(["fetch", "--quiet", INPUT, "refs/heads/start:refs/heads/start"]) ||
  !git(["checkout", "--quiet", "start"])
) {
  fail("clone");
}

// Claude Code ignores CLAUDE_CODE_OAUTH_TOKEN on a fresh home until this
// non-secret flag exists. It is not a login file and holds no credential.
const home = process.env.HOME || "/home/node";
fs.mkdirSync(home, { recursive: true });
fs.writeFileSync(`${home}/.claude.json`, '{"hasCompletedOnboarding":true}\n', {
  mode: 0o600,
});
fs.writeSync(1, "bootstrap:ready\n");

const env = { ...process.env };
for (const name of CLEARED) {
  delete env[name];
}
env[key] = value;
value = "";

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
