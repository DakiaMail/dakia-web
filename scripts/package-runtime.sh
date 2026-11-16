#!/bin/sh
# Export the tested image; do not rebuild a different runtime for native installs.
set -eu
[ "$#" = 4 ] || { echo 'Usage: package-runtime.sh IMAGE VERSION amd64|arm64 OUTPUT_DIR' >&2; exit 1; }
image=$1 version=$2 arch=$3 output=$4
case "$arch" in amd64|arm64) ;; *) echo 'Unsupported architecture' >&2; exit 1 ;; esac
case "$version" in ''|*[!a-zA-Z0-9.-]*) echo 'Invalid version' >&2; exit 1 ;; esac
[ "$(docker image inspect --format '{{.Architecture}}' "$image")" = "$arch" ]
source_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
mkdir -p "$output"
output=$(CDPATH='' cd -- "$output" && pwd)
archive="dakia-$version-linux-$arch.tar.gz"
[ ! -e "$output/$archive" ] || { echo 'Archive already exists' >&2; exit 1; }
work=$(mktemp -d)
container=
trap 'if [ -n "$container" ]; then docker rm "$container" >/dev/null; fi; rm -rf "$work"' EXIT
container=$(docker create "$image")
mkdir "$work/runtime"
docker cp "$container:/app/." "$work/runtime/"
docker cp "$container:/usr/local/bin/node" "$work/runtime/node"
cp "$source_root/deploy/native/dakia" "$source_root/deploy/native/dakia.service" "$work/runtime/"
chmod 755 "$work/runtime/dakia" "$work/runtime/node"
for file in dakia-server dakia-admin dakia-healthcheck dakia-classifier-check dakia-search-model-check node; do
    test -x "$work/runtime/$file"
done
for file in web/index.html workers/translation/smoke.mjs search-model/model.onnx search-model/tokenizer.json licenses/Node-LICENSE; do
    test -s "$work/runtime/$file"
done
find "$work/runtime" -name 'libonnxruntime.so*' | grep -q .
printf '%s\n' "$version" > "$work/runtime/VERSION"
docker image inspect --format '{{index .Config.Labels "org.opencontainers.image.revision"}}' "$image" > "$work/runtime/REVISION"
printf '%s\n' "Setup: https://github.com/DakiaMail/dakia-web/blob/$(cat "$work/runtime/REVISION")/docs/native-hosting.md" > "$work/runtime/README.txt"
tar -C "$work/runtime" -czf "$output/$archive" .
(cd "$output" && sha256sum "$archive" > "$archive.sha256")
