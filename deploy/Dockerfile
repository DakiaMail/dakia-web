# Runtime for binaries built on Linux amd64 / glibc 2.39.
FROM node:22.14.0-bookworm-slim@sha256:1c18d9ab3af4585870b92e4dbc5cac5a0dc77dd13df1a5905cea89fc720eb05b AS node_runtime
FROM ubuntu:24.04@sha256:24737cef17f4e22b1977df7d9a05e77c4e28b7da5b8911f8d8ccda136bf3573c
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates libssl3t64 libstdc++6 libatomic1 && rm -rf /var/lib/apt/lists/* && mkdir -p /data /secrets && chown 1000:1000 /data /secrets
COPY --from=node_runtime /usr/local/bin/node /usr/local/bin/node
WORKDIR /app
COPY bin/ /app/
COPY web/ /app/web/
COPY workers/ /app/workers/
COPY search-model/ /app/search-model/
COPY licenses/ /app/licenses/
ENV LD_LIBRARY_PATH=/app DAKIA_DATA_DIR=/data SQLITE_TMPDIR=/data DAKIA_CREDENTIAL_KEY_FILE=/secrets/credential-key DAKIA_WEB_ROOT=/app/web DAKIA_WORKERS=/app/workers DAKIA_SEARCH_MODEL_DIR=/app/search-model DAKIA_MODEL_CACHE=/data/models DAKIA_LISTEN=0.0.0.0:3025
USER 1000:1000
RUN node /app/workers/translation/smoke.mjs && /app/dakia-classifier-check && /app/dakia-search-model-check
ENTRYPOINT ["sh", "/app/workers/push/start.sh", "/app/dakia-server"]
