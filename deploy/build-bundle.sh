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
for binary in dakia-server dakia-admin dakia-healthcheck dakia-classifier-check dakia-search-model-check; do
    cp "target/release/$binary" "deploy/bundle/bin/$binary"
    strip "deploy/bundle/bin/$binary"
done
cp deploy/Dockerfile deploy/bundle/Dockerfile
cp deploy/repair.Dockerfile deploy/bundle/repair.Dockerfile
cp LICENSE THIRD_PARTY_NOTICES.md deploy/bundle/
mkdir -p deploy/bundle/scripts
for tool in repair-search-index.py repair_search_postgres.py fetch-repair-sqlite.py search-repair-requirements.txt check-search-repair-package.py; do
    cp "scripts/$tool" "deploy/bundle/scripts/$tool"
done
cp deploy/SEARCH_REPAIR.md deploy/bundle/SEARCH_REPAIR.md
mkdir -p deploy/bundle/licenses
cp LICENSE THIRD_PARTY_NOTICES.md deploy/bundle/licenses/
cp crates/inference/assets/licenses/* deploy/bundle/licenses/
cp workers/translation/DESKTOP_THIRD_PARTY_NOTICES.md deploy/bundle/licenses/
cp node_modules/@fontsource/inter/LICENSE deploy/bundle/licenses/Inter-OFL-1.1.txt
# Empty the generated assets only, never application data or source.
python3 -c 'from pathlib import Path; import shutil; p=Path("deploy/bundle/web"); shutil.rmtree(p,ignore_errors=True); shutil.copytree("apps/web/dist",p)'
python3 -c 'from pathlib import Path; import shutil; [(shutil.rmtree(Path("deploy/bundle")/dst,ignore_errors=True),shutil.copytree(src,Path("deploy/bundle")/dst,ignore=shutil.ignore_patterns("test",".download-*",".model-*"))) for src,dst in [("workers","workers"),("crates/inference/assets/multilingual-e5-small","search-model")]]; [shutil.copy2(p,"deploy/bundle/bin") for p in Path("target/release").glob("libonnxruntime.so*")]'
# Test copied dependencies and WASM, not only the source checkout.
node deploy/bundle/workers/translation/smoke.mjs
deploy/bundle/bin/dakia-classifier-check
DAKIA_SEARCH_MODEL_DIR=deploy/bundle/search-model deploy/bundle/bin/dakia-search-model-check
tar -C deploy/bundle -czf deploy/bundle/runtime.tar.gz bin web workers search-model licenses scripts Dockerfile repair.Dockerfile SEARCH_REPAIR.md LICENSE THIRD_PARTY_NOTICES.md
sha256sum deploy/bundle/runtime.tar.gz
