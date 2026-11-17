# syntax=docker/dockerfile:1
# Offline operator image. Select a client base compatible with the target server.
ARG POSTGRES_IMAGE=postgres:17-bookworm@sha256:665dc7b37ac52a17e298511414c21d23ffa4e6a6ef6225b3e5d59c7afb35e994
FROM ${POSTGRES_IMAGE} AS sqlite_builder
RUN apt-get update && apt-get install -y --no-install-recommends build-essential python3 ca-certificates && rm -rf /var/lib/apt/lists/*
COPY scripts/fetch-repair-sqlite.py /build/fetch.py
RUN python3 /build/fetch.py /build/sqlite && cc -O2 -DSQLITE_ENABLE_FTS5 -DSQLITE_ENABLE_JSON1 -DSQLITE_THREADSAFE=1 /build/sqlite/shell.c /build/sqlite/sqlite3.c -ldl -lm -lpthread -o /build/sqlite3 && test "$(/build/sqlite3 :memory: 'SELECT sqlite_version();')" = 3.54.1

FROM ${POSTGRES_IMAGE}
RUN apt-get update && apt-get install -y --no-install-recommends python3 python3-venv ca-certificates && rm -rf /var/lib/apt/lists/*
COPY scripts/search-repair-requirements.txt /opt/dakia-repair/requirements.txt
RUN python3 -m venv /opt/dakia-repair/venv && /opt/dakia-repair/venv/bin/pip install --no-cache-dir -r /opt/dakia-repair/requirements.txt
COPY --from=sqlite_builder /build/sqlite3 /usr/local/bin/sqlite3
COPY scripts/repair-search-index.py scripts/repair_search_postgres.py scripts/check-search-repair-package.py /opt/dakia-repair/
COPY LICENSE THIRD_PARTY_NOTICES.md /opt/dakia-repair/licenses/
ENV PATH=/opt/dakia-repair/venv/bin:$PATH PYTHONDONTWRITEBYTECODE=1
WORKDIR /work
USER 1000:1000
RUN python3 /opt/dakia-repair/check-search-repair-package.py
ENTRYPOINT ["python3", "/opt/dakia-repair/repair-search-index.py"]
