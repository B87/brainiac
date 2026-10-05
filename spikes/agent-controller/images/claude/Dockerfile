# Pinned Claude ACP adapter for the subscription-token probe.
# Adapter 0.85.1 depends on @anthropic-ai/claude-agent-sdk 0.3.286,
# which declares Claude Code 2.1.286. Both packages are installed at
# those versions; the entrypoint selects the Claude Code binary.
FROM node:22-bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates git \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /opt/claude
RUN npm install --omit=dev --no-audit --no-fund \
        @agentclientprotocol/claude-agent-acp@0.85.1 \
        @anthropic-ai/claude-code@2.1.286 \
    && npm cache clean --force

COPY entrypoint.mjs /opt/claude/entrypoint.mjs

# A path, not a credential. The token is never an image or container env var.
ENV CLAUDE_CODE_EXECUTABLE=/opt/claude/node_modules/.bin/claude
LABEL org.brainiac.spike.adapter=0.85.1 \
      org.brainiac.spike.claude-code=2.1.286 \
      org.brainiac.spike.sdk=0.3.286

RUN mkdir -p /workspace && chown node:node /workspace
USER node
WORKDIR /workspace
ENTRYPOINT ["node", "/opt/claude/entrypoint.mjs"]
