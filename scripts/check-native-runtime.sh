#!/bin/sh
# Exercise the extracted archive with fresh state and no host Node installation.
set -eu
[ "$#" = 1 ] || { echo 'Usage: check-native-runtime.sh ARCHIVE' >&2; exit 1; }
source_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
work=$(mktemp -d)
pid=
trap 'if [ -n "$pid" ]; then kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true; fi; rm -rf "$work"' EXIT
mkdir "$work/runtime" "$work/data" "$work/secrets"
tar -xzf "$1" -C "$work/runtime"
runtime=$work/runtime
export PATH=/usr/bin:/bin
export LD_LIBRARY_PATH="$runtime"
export DAKIA_DATA_DIR="$work/data" DAKIA_CREDENTIAL_KEY_FILE="$work/secrets/credential-key"
export DAKIA_WEB_ROOT="$runtime/web" DAKIA_SEARCH_MODEL_DIR="$runtime/search-model"
export DAKIA_ORIGIN=https://mail.test DAKIA_RELEASE_CHECKS=false
DAKIA_EXPECTED_RELEASE_VERSION=$(cat "$runtime/VERSION")
export DAKIA_EXPECTED_RELEASE_VERSION
export DAKIA_GMAIL_CLIENT_ID=image-fixture.apps.googleusercontent.com
export DAKIA_GMAIL_CLIENT_SECRET=synthetic-image-smoke-only
export DAKIA_MICROSOFT_CLIENT_ID=microsoft-image-fixture
export DAKIA_MICROSOFT_CLIENT_SECRET=synthetic-image-smoke-only
"$runtime/node" --version
"$runtime/dakia-classifier-check"
"$runtime/dakia-search-model-check"
"$runtime/node" "$runtime/workers/translation/smoke.mjs"
start() {
    "$runtime/dakia" > "$work/server.log" 2>&1 &
    pid=$!
    ready=false
    for _ in $(seq 1 30); do
        if "$runtime/dakia-healthcheck"; then ready=true; break; fi
        sleep 1
    done
    if [ "$ready" != true ]; then cat "$work/server.log"; exit 1; fi
}
start
"$runtime/node" "$source_root/scripts/image-oauth-smoke.mjs"
before=$(sha256sum "$DAKIA_CREDENTIAL_KEY_FILE")
kill "$pid"
wait "$pid" || true
pid=
start
test "$before" = "$(sha256sum "$DAKIA_CREDENTIAL_KEY_FILE")"
"$runtime/node" "$source_root/scripts/image-oauth-smoke.mjs" --resume
