#!/bin/sh
# Shared native/container startup. Provision as the unprivileged server user.
set -eu
: "${DAKIA_CREDENTIAL_KEY_FILE:?Set the persistent credential key path}"
export DAKIA_VAPID_FILE="${DAKIA_VAPID_FILE:-$(dirname -- "$DAKIA_CREDENTIAL_KEY_FILE")/vapid.json}"
export DAKIA_VAPID_SUBJECT="${DAKIA_VAPID_SUBJECT:-${DAKIA_ORIGIN:-}}"
"${DAKIA_NODE:-node}" "${DAKIA_WORKERS:?Set DAKIA_WORKERS}/push/provision.mjs"
exec "$@"
