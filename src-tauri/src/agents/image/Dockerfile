# The image Brainiac builds for Claude Code runs (SPEC.md, Settings → Agents).
# Brainiac builds this file and the entrypoint beside it, nothing else: a
# repository's own Dockerfile or dev container is never used.
#
# Everything is pinned: the base image by its digest (node:22-bookworm-slim
# as of 23 Sep 2026), and the packages by package-lock.json. The Claude ACP
# adapter 0.85.1 depends on @anthropic-ai/claude-agent-sdk 0.3.286, which
# declares Claude Code 2.1.286, installed at that version too; the entrypoint
# selects the Claude Code binary. The collector is added with collection.
FROM node:22-bookworm-slim@sha256:43ac6c60b8f89723f746e8a92ce91abd5017e627ce1ddfe4238355d3a30b772c

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates git \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /opt/claude
COPY package.json package-lock.json /opt/claude/
# The lockfile cannot tell glibc from musl, so npm also installs the musl
# builds of the native binaries; this Debian image never uses them.
RUN npm ci --omit=dev --no-audit --no-fund \
    && rm -rf node_modules/@anthropic-ai/*-musl \
    && npm cache clean --force

COPY entrypoint.mjs /opt/claude/entrypoint.mjs

# Claude Code's managed settings win over a repository's own .claude
# settings, which runs otherwise use (CLAUDE.md, hooks, permissions). They
# keep the token or key going to Anthropic over verified TLS even if a
# repository's settings name another endpoint or certificate. They do not
# stop the repository's code or hooks from reading it: see SPEC.md, Agent
# runs, Boundaries. Root-owned, so the agent cannot change it.
COPY managed-settings.json /etc/claude-code/managed-settings.json

# A path, not a credential. The token or key is never an image or container
# variable: the entrypoint reads it once from stdin.
ENV CLAUDE_CODE_EXECUTABLE=/opt/claude/node_modules/.bin/claude
LABEL org.brainiac.adapter=0.85.1 \
      org.brainiac.claude-code=2.1.286 \
      org.brainiac.sdk=0.3.286

RUN mkdir -p /workspace && chown node:node /workspace
USER node
WORKDIR /workspace
ENTRYPOINT ["node", "/opt/claude/entrypoint.mjs"]
