# Runtime for binaries built on Linux amd64 / glibc 2.39.
FROM node:22.14.0-bookworm-slim@sha256:1c18d9ab3af4585870b92e4dbc5cac5a0dc77dd13df1a5905cea89fc720eb05b AS node_runtime
FROM ubuntu:24.04@sha256:24737cef17f4e22b1977df7d9a05e77c4e28b7da5b8911f8d8ccda136bf3573c
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates libssl3t64 libstdc++6 libatomic1 && rm -rf /var/lib/apt/lists/*
COPY --from=node_runtime /usr/local/bin/node /usr/local/bin/node
WORKDIR /app
COPY bin/ /app/
COPY web/ /app/web/
COPY workers/ /app/workers/
COPY licenses/ /app/licenses/
ENV DAKIA_WORKERS=/app/workers DAKIA_MODEL_CACHE=/data/models
USER 1000:1000
RUN node /app/workers/translation/smoke.mjs && /app/dakia-classifier-check
ENTRYPOINT ["/app/dakia-server"]
