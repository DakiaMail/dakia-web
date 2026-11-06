# syntax=docker/dockerfile:1
FROM node:24-bookworm-slim AS web
WORKDIR /src
COPY . .
RUN npm ci --ignore-scripts && npm run build && npm ci --prefix workers/translation --ignore-scripts --omit=dev && npm ci --prefix workers/push --ignore-scripts --omit=dev

FROM rust:1.98-bookworm AS rust
FROM ubuntu:24.04 AS build
RUN apt-get update && apt-get install -y --no-install-recommends build-essential pkg-config libssl-dev ca-certificates python3 && rm -rf /var/lib/apt/lists/*
COPY --from=rust /usr/local/cargo /usr/local/cargo
COPY --from=rust /usr/local/rustup /usr/local/rustup
ENV CARGO_HOME=/usr/local/cargo RUSTUP_HOME=/usr/local/rustup PATH=/usr/local/cargo/bin:$PATH
WORKDIR /src
COPY . .
RUN python3 scripts/fetch-classifier.py && cargo build --release --locked -p dakia-server && mkdir /runtime && cp target/release/dakia-server target/release/dakia-admin target/release/dakia-healthcheck /runtime/ && find target/release -maxdepth 1 -name 'libonnxruntime.so*' -exec cp -P {} /runtime/ \;

FROM ubuntu:24.04
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates libssl3t64 libstdc++6 libatomic1 && rm -rf /var/lib/apt/lists/* && mkdir -p /data /secrets && chown 1000:1000 /data /secrets
COPY --from=web /usr/local/bin/node /usr/local/bin/node
WORKDIR /app
COPY --from=build /runtime/ /app/
COPY --from=build /src/crates/inference/assets/email-classifier-v2/ /app/classifier/
COPY --from=web /src/apps/web/dist/ /app/web/
COPY --from=web /src/workers/ /app/workers/
COPY LICENSE THIRD_PARTY_NOTICES.md /app/licenses/
COPY crates/inference/assets/licenses/ /app/licenses/
COPY crates/inference/assets/DESKTOP_THIRD_PARTY_NOTICES.md /app/licenses/
COPY --from=web /src/node_modules/@fontsource/inter/LICENSE /app/licenses/Inter-OFL-1.1.txt
ENV LD_LIBRARY_PATH=/app DAKIA_DATA_DIR=/data DAKIA_CREDENTIAL_KEY_FILE=/secrets/credential-key DAKIA_WEB_ROOT=/app/web DAKIA_WORKERS=/app/workers DAKIA_CLASSIFIER_DIR=/app/classifier DAKIA_MODEL_CACHE=/data/models DAKIA_LISTEN=0.0.0.0:3025
USER 1000:1000
RUN node /app/workers/translation/smoke.mjs
EXPOSE 3025
HEALTHCHECK --interval=30s --timeout=5s CMD ["/app/dakia-healthcheck"]
ENTRYPOINT ["/app/dakia-server"]
