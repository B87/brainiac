# The image Brainiac builds for runs (SPEC.md, Settings → Agents), with
# every agent: Claude Code and OpenCode. Brainiac builds this file and the
# files beside it, nothing else: a repository's own Dockerfile or dev
# container is never used.
#
# Everything is pinned: the base image by its digest (node:22-bookworm-slim
# as of 23 Sep 2026), and the packages by package-lock.json. The Claude ACP
# adapter 0.85.1 depends on @anthropic-ai/claude-agent-sdk 0.3.286, which
# declares Claude Code 2.1.286, installed at that version too; the entrypoint
# selects the Claude Code binary. OpenCode 1.18.35 speaks ACP itself. The
# entrypoint starts the agent the run asks for (BRAINIAC_AGENT). The
# collector (collector.mjs) runs in a container of its own after a run
# stops, with no network and no credential.
FROM node:22-bookworm-slim@sha256:43ac6c60b8f89723f746e8a92ce91abd5017e627ce1ddfe4238355d3a30b772c

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates git \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /opt/claude
COPY package.json package-lock.json /opt/claude/
# The lockfile cannot tell glibc from musl, so npm also installs the musl
# builds of the native binaries; this Debian image never uses them.
RUN npm ci --omit=dev --no-audit --no-fund \
    && rm -rf node_modules/@anthropic-ai/*-musl node_modules/opencode-linux-*-musl \
    && npm cache clean --force

COPY entrypoint.mjs collector.mjs known.mjs /opt/claude/

# `known <name>…` tells an explain run which concepts the reader knows, from
# the file the controller copies in (a path, never a credential).
RUN printf '#!/bin/sh\nexec node /opt/claude/known.mjs "$@"\n' > /usr/local/bin/known \
    && chmod 755 /usr/local/bin/known

# Claude Code's managed settings win over a repository's own .claude
# settings, which runs otherwise use (CLAUDE.md, hooks, permissions). They
# keep the token or key going to Anthropic over verified TLS even if a
# repository's settings name another endpoint or certificate. They do not
# stop the repository's code or hooks from reading it: see SPEC.md, Agent
# runs, Boundaries. Root-owned, so the agent cannot change it.
COPY managed-settings.json /etc/claude-code/managed-settings.json

# OpenCode's managed configuration wins over every other, including the
# OPENCODE_CONFIG_CONTENT a run sets. It pins each provider's address,
# offers only the three providers Brainiac sets up (not OpenCode's own
# models), and turns off updates, sharing, and snapshots. A repository's
# OpenCode configuration is not read at all (OPENCODE_DISABLE_PROJECT_CONFIG):
# a setting there could put the key in a request to another server, which
# pinning the providers does not stop (docs/design/agent-runs.md, Record:
# OpenCode in a run's container). Root-owned, so the agent cannot change it.
COPY opencode.json /etc/opencode/opencode.json

# A path, not a credential. The token or key is never an image or container
# variable: the entrypoint reads it once from stdin.
ENV CLAUDE_CODE_EXECUTABLE=/opt/claude/node_modules/.bin/claude \
    OPENCODE_DISABLE_PROJECT_CONFIG=1 \
    OPENCODE_DISABLE_AUTOUPDATE=1 \
    OPENCODE_DISABLE_SHARE=1 \
    OPENCODE_DISABLE_LSP_DOWNLOAD=1 \
    OPENCODE_DISABLE_DEFAULT_PLUGINS=1
LABEL org.brainiac.adapter=0.85.1 \
      org.brainiac.claude-code=2.1.286 \
      org.brainiac.sdk=0.3.286 \
      org.brainiac.opencode=1.18.35

# The run controller copies the run's start (input.bundle) into
# /opt/brainiac/input before the container starts; the entrypoint clones it
# into the empty /workspace volume.
# /scratch and /out are the collector's own working and output folders.
RUN mkdir -p /workspace /opt/brainiac/input /scratch /out \
    && chown node:node /workspace /scratch /out
USER node
WORKDIR /workspace
ENTRYPOINT ["node", "/opt/claude/entrypoint.mjs"]
