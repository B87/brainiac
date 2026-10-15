// The run's entrypoint (docs/architecture.md, Agent runs — v0.5,
// Credentials). It reads one credential frame from stdin, clones the run's
// start into the workspace, then starts the run's agent with that one value
// in its environment: Claude Code's ACP adapter, or OpenCode when the
// container's BRAINIAC_AGENT says so.
//
// A run with extra secrets (docs/design/scheduled-runs.md) sends "BRB2"
// instead: JSON with "credential" (exactly the one key below) and "extra"
// (a few NAME: value secrets, exported to the agent's environment under the
// names EXTRA_NAME allows). A run without any sends "BRB1" as before.
//
// The frame is "BRB1", a 4-byte big-endian length, then JSON with exactly
// one key, one the agent reads: for Claude Code CLAUDE_CODE_OAUTH_TOKEN (a
// Claude plan token) or ANTHROPIC_API_KEY; for OpenCode the provider's
// ANTHROPIC_API_KEY, OPENAI_API_KEY, or OPENROUTER_API_KEY. The read is
// exact, so the ACP bytes after it stay in the pipe for the agent. Node
// strings cannot be wiped; the frame buffer is zeroed and the value is never
// written to a file.

import { spawn, spawnSync } from "node:child_process";
import fs from "node:fs";

const ADAPTER =
  "/opt/claude/node_modules/@agentclientprotocol/claude-agent-acp/dist/index.js";
const OPENCODE = "/opt/claude/node_modules/.bin/opencode";
const MAX_FRAME = 8192;
// A frame with extra secrets: up to 8 values of 16 KiB, JSON-escaped.
const MAX_FRAME_EXTRA = 256 * 1024;
const MAX_EXTRA = 8;
// The same rule as valid_secret_name in the run controller: capitals,
// digits and underscores, and nothing that loads code, changes Git or a
// proxy, or is read by an agent. Checked again here because this is the
// last place before the names reach an environment.
const EXTRA_NAME = /^[A-Z][A-Z0-9_]{0,63}$/;
const EXTRA_EXACT = new Set([
  "PATH", "HOME", "USER", "SHELL", "PWD", "TMPDIR", "IFS", "ENV", "BASH_ENV",
  "CDPATH", "PROXY", "TERM", "PROMPT_COMMAND", "SHELLOPTS", "BASHOPTS",
  "GLOBIGNORE", "ZDOTDIR", "REQUESTS_CA_BUNDLE", "LOCPATH", "NLSPATH",
  "HOSTALIASES",
]);
const EXTRA_PREFIXES = [
  "LD_", "DYLD_", "NODE_", "NPM_", "GIT_", "PYTHON", "CLAUDE", "ANTHROPIC_",
  "OPENAI_", "OPENROUTER_", "OPENCODE_", "BRAINIAC_", "SSL_", "CURL_",
  "SSH_", "DOCKER_", "CODEX_", "GEMINI_", "MALLOC_", "GCONV_",
];
function extraNameOk(name) {
  return (
    EXTRA_NAME.test(name) &&
    !EXTRA_EXACT.has(name) &&
    !name.endsWith("_PROXY") &&
    !name.endsWith("_OPTIONS") &&
    !name.endsWith("OPT") &&
    !EXTRA_PREFIXES.some((prefix) => name.startsWith(prefix))
  );
}
// The run's start, copied in by the run controller before this container
// started: one commit and its history, advertising refs/heads/start.
const INPUT = "/opt/brainiac/input/input.bundle";
const WORKSPACE = "/workspace";
// Each agent: how it starts, and the keys it reads.
const AGENTS = {
  claude: {
    command: process.execPath,
    args: [ADAPTER],
    keys: ["CLAUDE_CODE_OAUTH_TOKEN", "ANTHROPIC_API_KEY"],
  },
  opencode: {
    command: OPENCODE,
    args: ["acp"],
    keys: ["ANTHROPIC_API_KEY", "OPENAI_API_KEY", "OPENROUTER_API_KEY"],
  },
};
const AGENT = AGENTS[process.env.BRAINIAC_AGENT === "opencode" ? "opencode" : "claude"];
const KEYS = AGENT.keys;
// Every variable that could carry another credential to an agent.
const CLEARED = [
  "CLAUDE_CODE_OAUTH_TOKEN",
  "ANTHROPIC_API_KEY",
  "OPENAI_API_KEY",
  "OPENROUTER_API_KEY",
  "ANTHROPIC_AUTH_TOKEN",
  "CLAUDE_CODE_API_KEY",
  "CLAUDE_AGENT_LOGS",
  "OPENCODE_API_KEY",
  "OPENCODE_AUTH_CONTENT",
  "OPENCODE_CONSOLE_TOKEN",
  "OPENCODE_SERVER_PASSWORD",
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
let extra = {};
try {
  const magic = readExact(0, 4);
  const withExtra = magic.equals(Buffer.from("BRB2"));
  if (!withExtra && !magic.equals(Buffer.from("BRB1"))) {
    fail("magic");
  }
  const lenBuf = readExact(0, 4);
  const length = lenBuf.readUInt32BE(0);
  lenBuf.fill(0);
  if (length < 2 || length > (withExtra ? MAX_FRAME_EXTRA : MAX_FRAME)) {
    fail("length");
  }
  const payload = readExact(0, length);
  let parsed;
  try {
    parsed = JSON.parse(payload.toString("utf8"));
  } finally {
    payload.fill(0);
  }
  let credential = parsed;
  if (withExtra) {
    const top = parsed && typeof parsed === "object" ? Object.keys(parsed) : [];
    if (top.length !== 2 || !top.includes("credential") || !top.includes("extra")) {
      fail("keys");
    }
    extra = parsed.extra;
    credential = parsed.credential;
    const names =
      extra && typeof extra === "object" && !Array.isArray(extra)
        ? Object.keys(extra)
        : null;
    if (!names || names.length === 0 || names.length > MAX_EXTRA) {
      fail("extra");
    }
    for (const name of names) {
      const v = extra[name];
      if (
        !extraNameOk(name) ||
        typeof v !== "string" ||
        v.length < 8 ||
        v.length > 16 * 1024 ||
        v.includes("\u0000")
      ) {
        fail("extra");
      }
    }
  }
  const keys =
    credential && typeof credential === "object" ? Object.keys(credential) : [];
  if (keys.length !== 1 || !KEYS.includes(keys[0])) {
    fail("keys");
  }
  key = keys[0];
  value = credential[key];
  credential[key] = "";
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
if (AGENT === AGENTS.claude) {
  fs.writeFileSync(`${home}/.claude.json`, '{"hasCompletedOnboarding":true}\n', {
    mode: 0o600,
  });
}
fs.writeSync(1, "bootstrap:ready\n");

const env = { ...process.env };
for (const name of CLEARED) {
  delete env[name];
}
env[key] = value;
value = "";
for (const [name, secret] of Object.entries(extra)) {
  env[name] = secret;
  extra[name] = "";
}

const child = spawn(AGENT.command, AGENT.args, {
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
