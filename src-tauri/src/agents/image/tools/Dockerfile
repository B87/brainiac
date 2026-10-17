# The tools layer (docs/design/scheduled-runs.md, The tools layer): Python and
# the Google Cloud CLI on top of the run image, for a profile that ticks
# "Python and Google Cloud CLI". Brainiac builds this file and nothing else;
# a repository's Dockerfile or requirements are never read. The base is the
# run image Brainiac built (BASE), already on the engine, so this build does
# not pull it.
#
# Pinned: google-cloud-cli by version, and the key its apt repository is
# signed with by SHA-256 (checked below, as downloaded 15 Oct 2026). No
# Python packages are installed: the scripts this was made for use only the
# standard library.
# A build that forgets BASE fails at the first step instead of pulling a name.
ARG BASE=scratch
FROM ${BASE}

USER root
RUN apt-get update \
    && apt-get install -y --no-install-recommends python3 ca-certificates curl gnupg \
    && curl -fsSL https://packages.cloud.google.com/apt/doc/apt-key.gpg -o /tmp/cloud.gpg \
    && echo "3ecc63922b7795eb23fdc449ff9396f9114cb3cf186d6f5b53ad4cc3ebfbb11f  /tmp/cloud.gpg" | sha256sum -c - \
    && gpg --dearmor -o /usr/share/keyrings/cloud.google.gpg /tmp/cloud.gpg \
    && echo "deb [signed-by=/usr/share/keyrings/cloud.google.gpg] https://packages.cloud.google.com/apt cloud-sdk main" > /etc/apt/sources.list.d/google-cloud-sdk.list \
    && apt-get update \
    && apt-get install -y --no-install-recommends google-cloud-cli=589.0.0-0 \
    && apt-get purge -y --auto-remove curl gnupg \
    && rm -rf /var/lib/apt/lists/* /tmp/cloud.gpg

# gcloud keeps its settings and logs under $HOME, which is tmpfs in a run.
ENV CLOUDSDK_CORE_DISABLE_PROMPTS=1 \
    CLOUDSDK_COMPONENT_MANAGER_DISABLE_UPDATE_CHECK=1 \
    CLOUDSDK_CORE_DISABLE_USAGE_REPORTING=1
LABEL org.brainiac.tools="python3 google-cloud-cli=589.0.0-0"
USER node
