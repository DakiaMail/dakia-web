#!/bin/sh
# Prepare a private Linux amd64 runtime bundle, without data or credentials.
set -eu
cd "$(dirname "$0")/.."
[ "$(uname -m)" = x86_64 ] || { printf 'Linux amd64 build required\n' >&2; exit 1; }
[ "$(uname -s)" = Linux ] || exit 1
npm ci --ignore-scripts
npm run build
npm test
npm ci --prefix workers/translation --ignore-scripts
npm ci --prefix workers/push --ignore-scripts
node --test workers/translation/test/*.test.mjs workers/push/test/*.test.mjs
cargo test --workspace --locked
cargo build --release --locked -p dakia-server
rm -rf deploy/bundle/bin deploy/bundle/licenses deploy/bundle/classifier
mkdir -p deploy/bundle/bin
for binary in dakia-server dakia-admin dakia-healthcheck dakia-classifier-check; do
    cp "target/release/$binary" "deploy/bundle/bin/$binary"
    strip "deploy/bundle/bin/$binary"
done
cp deploy/Dockerfile deploy/bundle/Dockerfile
mkdir -p deploy/bundle/licenses
cp LICENSE THIRD_PARTY_NOTICES.md deploy/bundle/licenses/
cp workers/translation/DESKTOP_THIRD_PARTY_NOTICES.md deploy/bundle/licenses/
cp node_modules/@fontsource/inter/LICENSE deploy/bundle/licenses/Inter-OFL-1.1.txt
# Empty the generated assets only, never application data or source.
python3 -c 'from pathlib import Path; import shutil; p=Path("deploy/bundle/web"); shutil.rmtree(p,ignore_errors=True); shutil.copytree("apps/web/dist",p)'
python3 -c 'from pathlib import Path; import shutil; p=Path("deploy/bundle/workers"); shutil.rmtree(p,ignore_errors=True); shutil.copytree("workers",p,ignore=shutil.ignore_patterns("test"))'
# Test copied dependencies and WASM, not only the source checkout.
node deploy/bundle/workers/translation/smoke.mjs
deploy/bundle/bin/dakia-classifier-check
tar -C deploy/bundle -czf deploy/bundle/runtime.tar.gz bin web workers licenses Dockerfile
sha256sum deploy/bundle/runtime.tar.gz
