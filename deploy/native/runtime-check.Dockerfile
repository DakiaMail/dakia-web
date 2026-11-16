FROM ubuntu:24.04
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates libssl3t64 libstdc++6 libatomic1 && rm -rf /var/lib/apt/lists/*
USER 1000:1000
